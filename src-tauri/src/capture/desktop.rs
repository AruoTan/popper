mod pinned;
mod scrolling;
mod shortcuts;
mod toolbar;
use pinned::{create_pin, PinPlacement};
pub use pinned::{
    paste, pin_action, pin_annotations, pin_close, pin_drag, pin_menu, pin_output, pin_state,
    pin_update, toggle_pins,
};
pub use scrolling::{scroll_begin, scroll_finish, scroll_preview, scroll_step};
pub use shortcuts::ShortcutChange;
pub use toolbar::{toolbar_action, toolbar_publish, toolbar_ready, toolbar_state};

use super::pins::Pins;
use super::{
    clipboard, hit_test::candidate_chain, platform, recognition::Recognition, CaptureCandidate,
    CaptureHitTest, CapturePoint, CaptureRect, CaptureSession, CaptureTarget, PendingAttachment,
};
use crate::runtime::RuntimeState;
use base64::Engine;
use image::{ImageFormat, RgbaImage};
use parking_lot::Mutex;
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use uuid::Uuid;

const CAPTURE_PREFIX: &str = "capture-session-";
const PIN_PREFIX: &str = "capture-pin-";
struct Session {
    meta: CaptureSession,
    animations: super::animations::AnimationPause,
    presented: bool,
    image: Option<Arc<RgbaImage>>,
    png: Option<Arc<Vec<u8>>>,
    selection: Option<CaptureRect>,
    version: u64,
    targets: Vec<Arc<platform::TargetWindow>>,
    foreground: usize,
    image_origin: CapturePoint,
    scroll: Option<scrolling::ScrollSession>,
    scroll_completed: bool,
    scroll_escape: Option<scrolling::ScrollEscape>,
    hidden_until: std::time::Instant,
    pin_annotations: Vec<super::CaptureAnnotation>,
}
struct AttachmentDelivery {
    payload: PendingAttachment,
    reply: mpsc::SyncSender<Result<(), String>>,
}
pub struct CaptureManager {
    operation: Mutex<()>,
    active: AtomicBool,
    stopped: AtomicBool,
    session: Mutex<Option<Session>>,
    pins: Mutex<Pins>,
    recognition: Recognition,
    generation: AtomicU64,
    shortcuts: Mutex<Vec<String>>,
    attachment: Mutex<Option<AttachmentDelivery>>,
}
impl CaptureManager {
    fn new() -> Self {
        Self {
            operation: Mutex::new(()),
            active: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            session: Mutex::new(None),
            pins: Mutex::new(Pins::default()),
            recognition: Recognition::new(),
            generation: AtomicU64::new(0),
            shortcuts: Mutex::new(Vec::new()),
            attachment: Mutex::new(None),
        }
    }
}
fn manager(app: &AppHandle) -> tauri::State<'_, CaptureManager> {
    app.state::<CaptureManager>()
}
fn session_id(window: &WebviewWindow) -> Result<&str, String> {
    window
        .label()
        .strip_prefix(CAPTURE_PREFIX)
        .filter(|id| Uuid::parse_str(id).is_ok())
        .ok_or_else(|| "截图会话窗口无效".into())
}
fn pin_id(window: &WebviewWindow) -> Result<&str, String> {
    window
        .label()
        .strip_prefix(PIN_PREFIX)
        .filter(|id| Uuid::parse_str(id).is_ok())
        .ok_or_else(|| "贴图窗口无效".into())
}
fn ensure_session<'a>(slot: &'a mut Option<Session>, id: &str) -> Result<&'a mut Session, String> {
    slot.as_mut()
        .filter(|s| s.meta.id == id)
        .ok_or_else(|| "截图会话已结束".into())
}
pub fn initialize(app: &AppHandle) {
    app.manage(CaptureManager::new());
}
pub fn setup(app: &AppHandle) {
    pinned::setup_menu(app);
    let settings = app.state::<RuntimeState>().settings.get_settings().capture;
    match ShortcutChange::prepare(app, &settings) {
        Ok(change) => change.commit(),
        Err(error) => app
            .state::<RuntimeState>()
            .set_capture_shortcut_error(Some(error)),
    }
}
pub fn shutdown(app: &AppHandle) {
    if let Some(state) = app.try_state::<CaptureManager>() {
        state.stopped.store(true, Ordering::Release);
        state.recognition.stop();
        let session = state.session.lock().take();
        // Restoring system settings broadcasts messages. Never hold a session
        // lock while other windows process that notification.
        drop(session);
        state.pins.lock().items.clear();
        state.attachment.lock().take();
        state.active.store(false, Ordering::Release);
    }
}
pub fn notify(app: &AppHandle, message: &str) {
    if app
        .try_state::<CaptureManager>()
        .is_some_and(|state| state.stopped.load(Ordering::Acquire))
    {
        return;
    }
    let handle = app.clone();
    let message = message.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let label = format!("capture-notice-{}", Uuid::new_v4());
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("mode", "notice")
            .append_pair("message", &message)
            .finish();
        let builder = WebviewWindowBuilder::new(
            &handle,
            &label,
            WebviewUrl::App(format!("capture/index.html?{query}").into()),
        )
        .title("Popper 屏幕提示")
        .inner_size(380.0, 84.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focusable(false)
        .focused(false);
        if let Ok(builder) = crate::app_paths::configure_webview(builder) {
            if let Ok(window) = builder.build() {
                if let Ok(hwnd) = window.hwnd() {
                    platform::configure_capture_surface(hwnd);
                }
                if let Ok((_, screen, scale)) = platform::screen() {
                    let _ = window.set_position(PhysicalPosition::new(
                        screen.x + ((screen.width as f64 - 380.0 * scale) / 2.0) as i32,
                        screen.y + (40.0 * scale) as i32,
                    ));
                }
                let _ = window.set_ignore_cursor_events(true);
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    if let Err(error) = window.destroy() {
                        eprintln!("[capture] failed to release notice: {error}");
                    }
                });
            }
        }
    });
}
pub fn pins_visible(app: &AppHandle) -> bool {
    !manager(app).pins.lock().hidden
}

pub fn launch(app: &AppHandle, operation: &'static str) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = match operation {
            "capture" => start(&handle),
            "paste" => paste(&handle),
            "toggle" => toggle_pins(&handle),
            _ => Ok(()),
        };
        if operation == "toggle" {
            crate::runtime::refresh_tray(&handle);
        }
        if let Err(error) = result {
            notify(&handle, &error);
        }
    });
}

pub fn start(app: &AppHandle) -> Result<(), String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    if state.stopped.load(Ordering::Acquire) {
        return Err("应用正在退出".into());
    }
    if state.active.load(Ordering::Acquire) {
        let active = state
            .session
            .lock()
            .as_ref()
            .map(|s| {
                (
                    s.meta.id.clone(),
                    s.presented,
                    s.scroll.is_some() || s.scroll_completed,
                )
            });
        if let Some((id, presented, scrolling)) = active {
            if let Some(window) = app.get_webview_window(&format!("{CAPTURE_PREFIX}{id}")) {
                // The renderer alone commits the first reveal after decoding.
                if presented {
                    if scrolling {
                        platform::focus(&window)?;
                    } else {
                        show(app, &window)?;
                    }
                }
            }
        }
        return Ok(());
    }
    state.active.store(true, Ordering::Release);
    let result = start_inner(app);
    if result.is_err() {
        let _ = finish(app, true);
    }
    result
}
fn start_inner(app: &AppHandle) -> Result<(), String> {
    let (monitor, screen, scale) = platform::screen()?;
    if u64::from(screen.width) * u64::from(screen.height) > clipboard::MAX_PIXELS {
        return Err("显示器图像尺寸超过当前限制".into());
    }
    let id = Uuid::new_v4().to_string();
    manager(app).generation.store(0, Ordering::Release);
    let foreground = platform::foreground();
    let mut targets = platform::windows();
    platform::snapshot_controls(&mut targets, screen);
    let pointer = platform::cursor()
        .ok()
        .filter(|&(x, y)| screen.contains(x, y));
    let snapshot = targets
        .iter()
        .position(|target| target.handle == foreground)
        .and_then(|index| {
            targets[index].rect.intersect(screen).map(|bounds| {
                (index, manager(app).recognition.snapshot(&id, foreground, bounds))
            })
        });
    app.state::<RuntimeState>()
        .set_screen_capture_active(app, true);
    // Pixel capture and provider prefetch run concurrently. Do not create or
    // activate the overlay until transient controls have been copied.
    platform::flush();
    let image = monitor
        .capture_image()
        .map_err(|e| format!("无法捕捉屏幕：{e}"))?;
    if image.width() != screen.width || image.height() != screen.height {
        return Err("显示器布局已变化，请重新截图".into());
    }
    let png = Arc::new(clipboard::png(&image)?);
    if let Some((index, snapshot)) = snapshot {
        targets[index].controls.extend(snapshot.finish());
    }
    if let Some((x, y)) = pointer {
        if let Some(target) = targets.iter().find(|target| target.rect.contains(x, y)) {
            manager(app).recognition.warm_up(&id, target.handle, x, y);
        }
    }
    let preferences = app.state::<RuntimeState>().settings.get_settings().capture;
    let meta = CaptureSession {
        id: id.clone(),
        screen,
        scale,
        color: preferences.color,
        line_width: preferences.line_width,
        pointer: pointer.map(|(x, y)| CapturePoint {
            x: x - screen.x,
            y: y - screen.y,
        }),
        targets: targets
            .iter()
            .filter_map(|target| {
                let local = |rect: CaptureRect| {
                    rect.intersect(target.rect)?
                        .intersect(screen)
                        .map(|rect| CaptureRect {
                            x: rect.x - screen.x,
                            y: rect.y - screen.y,
                            ..rect
                        })
                };
                Some(CaptureTarget {
                    rect: local(target.rect)?,
                    label: target.title.clone(),
                    controls: target
                        .controls
                        .iter()
                        .filter_map(|control| {
                            Some(CaptureCandidate {
                                rect: local(control.rect)?,
                                label: control.label.clone(),
                            })
                        })
                        .collect(),
                })
            })
            .collect(),
    };
    // Keep the screenshot and its control snapshot unchanged; suspend effects
    // only after they are captured, before creating or activating the overlay.
    let animations = super::animations::AnimationPause::suspend(
        super::animations::WindowsSettings::acquire()?,
    )?;
    let state = manager(app);
    let mut slot = state.session.lock();
    if state.stopped.load(Ordering::Acquire) {
        drop(slot);
        return Err("应用正在退出".into());
    }
    *slot = Some(Session {
        meta,
        animations,
        presented: false,
        image: Some(Arc::new(image)),
        png: Some(png),
        selection: None,
        version: 0,
        targets: targets.into_iter().map(Arc::new).collect(),
        foreground,
        image_origin: CapturePoint { x: 0, y: 0 },
        scroll: None,
        scroll_completed: false,
        scroll_escape: None,
        hidden_until: std::time::Instant::now() + Duration::from_secs(10),
        pin_annotations: Vec::new(),
    });
    drop(slot);
    let label = format!("{CAPTURE_PREFIX}{id}");
    let builder = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App("capture/index.html?mode=capture".into()),
    )
    .title("Popper 屏幕捕捉")
    .transparent(true)
    .inner_size(screen.width as f64 / scale, screen.height as f64 / scale)
    .decorations(false)
    .shadow(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .focused(false);
    let window = crate::app_paths::configure_webview(builder)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    if let Ok(hwnd) = window.hwnd() {
        platform::configure_capture_surface(hwnd);
    }
    window
        .set_position(PhysicalPosition::new(screen.x, screen.y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(PhysicalSize::new(screen.width, screen.height))
        .map_err(|e| e.to_string())?;
    let handle = app.clone();
    let window_id = id.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let handle = handle.clone();
            let id = window_id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = cancel_id(&handle, &id);
            });
        }
    });
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let pending = manager(&handle)
                .session
                .lock()
                .as_ref()
                .filter(|s| s.meta.id == id)
                .map(|s| (s.hidden_until, s.presented));
            let Some((hidden_until, presented)) = pending else {
                break;
            };
            let ready = handle
                .get_webview_window(&label)
                .is_some_and(|window| window.is_visible().unwrap_or(false));
            let geometry_ok = platform::same_screen(screen, scale);
            if !geometry_ok
                || (presented && platform::foreground() == 0)
                || (!ready && std::time::Instant::now() >= hidden_until)
            {
                let app = handle.clone();
                let id = id.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _ = cancel_id(&app, &id);
                });
                break;
            }
        }
    });
    Ok(())
}
fn finish(app: &AppHandle, restore_focus: bool) -> Result<(), String> {
    let state = manager(app);
    let previous = state.session.lock().take();
    let Some(previous) = previous else {
        state.active.store(false, Ordering::Release);
        app.state::<RuntimeState>()
            .set_screen_capture_active(app, false);
        return Ok(());
    };
    state.generation.fetch_add(1, Ordering::AcqRel);
    let id = previous.meta.id;
    let foreground = previous.foreground;
    let mut animations = previous.animations;
    let (reply, completed) = mpsc::sync_channel(1);
    let handle = app.clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    // Cleanup must still run if the UI is busy longer than the caller waits.
    // Unlike presentation, discarding a late cleanup would orphan its WebView.
    let result = app.run_on_main_thread(move || {
        let mut result = Ok(());
        if let Some(window) = handle.get_webview_window(&format!("{CAPTURE_PREFIX}{id}")) {
            // Queue destruction through the framework. Closing its browser
            // synchronously here can re-enter native callbacks on the UI thread.
            result = window.destroy().map_err(|e| e.to_string());
        }
        // A delayed cleanup must not steal focus from a subsequent operation.
        if restore_focus && std::time::Instant::now() < deadline {
            platform::restore_foreground(foreground);
        }
        let _ = reply.send(result);
    })
    .map_err(|e| e.to_string())
    .and_then(|()| {
        completed
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .map_err(|_| "截图窗口清理超时".to_owned())?
    });
    let restored = animations.restore();
    state.active.store(false, Ordering::Release);
    app.state::<RuntimeState>()
        .set_screen_capture_active(app, false);
    toolbar::refresh_visible(app);
    result.and(restored)
}
pub fn fail(app: &AppHandle, window: &WebviewWindow, message: &str) -> Result<(), String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let id = session_id(window)?;
    {
        let mut slot = state.session.lock();
        ensure_session(&mut slot, id)?;
    }
    finish(app, true)?;
    notify(app, &message.chars().take(256).collect::<String>());
    Ok(())
}
pub fn cancel(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    cancel_id(app, session_id(window)?)
}
fn cancel_id(app: &AppHandle, id: &str) -> Result<(), String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    if !state
        .session
        .lock()
        .as_ref()
        .is_some_and(|s| s.meta.id == id)
    {
        return Ok(());
    }
    finish(app, true)
}
pub fn session(app: &AppHandle, window: &WebviewWindow) -> Result<CaptureSession, String> {
    let id = session_id(window)?;
    let state = manager(app);
    let mut slot = state.session.lock();
    Ok(ensure_session(&mut slot, id)?.meta.clone())
}
pub fn validate_session(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    let id = session_id(window)?;
    let state = manager(app);
    let mut slot = state.session.lock();
    ensure_session(&mut slot, id).map(|_| ())
}
pub fn is_foreground(app: &AppHandle, window: &WebviewWindow) -> Result<bool, String> {
    let state = manager(app);
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, session_id(window)?)?;
    if let Some(scroll) = &session.scroll {
        return Ok(platform::foreground() == scroll.target || platform::is_foreground(window)?);
    }
    platform::is_foreground(window)
}
pub fn move_pointer(
    app: &AppHandle,
    window: &WebviewWindow,
    dx: i32,
    dy: i32,
) -> Result<CapturePoint, String> {
    let id = session_id(window)?;
    let state = manager(app);
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, id)?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    if session.image.is_none() || platform::foreground() != hwnd.0 as usize {
        return Err("截图窗口未激活".into());
    }
    platform::move_pointer(session.meta.screen, dx, dy)
}
pub fn copy_color(
    app: &AppHandle,
    window: &WebviewWindow,
    point: CapturePoint,
    format: super::CaptureColorFormat,
) -> Result<(), String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let value = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, session_id(window)?)?;
        if session.scroll.is_some() || session.scroll_completed {
            return Err("滚动截图不支持取色".into());
        }
        let image = session.image.as_ref().ok_or("屏幕图像不可用")?;
        super::color::sample(image, point, format)?
    };
    crate::clipboard::write_text(&value)?;
    finish(app, true)
}
pub fn image(app: &AppHandle, window: &WebviewWindow) -> Result<Vec<u8>, String> {
    if let Ok(id) = session_id(window) {
        let state = manager(app);
        let mut slot = state.session.lock();
        return ensure_session(&mut slot, id)?
            .png
            .as_ref()
            .map(|p| p.as_ref().clone())
            .ok_or_else(|| "屏幕图像尚未就绪".into());
    }
    let id = pin_id(window)?;
    let state = manager(app);
    let pins = state.pins.lock();
    pins.items
        .get(id)
        .map(|p| p.png.as_ref().clone())
        .ok_or_else(|| "贴图已结束".into())
}
pub fn show(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    if session_id(window).is_ok() {
        super::activation::wait_for_activation(|initial| {
            let window = window.clone();
            platform::on_ui(app, move |app| {
                let state = manager(&app);
                let mut slot = state.session.lock();
                let session = ensure_session(&mut slot, session_id(&window)?)?;
                if session.scroll.is_some() || session.scroll_completed {
                    return Err("滚动截图仅支持直接输出".into());
                }
                // Revalidate on every UI turn, including after a pending
                // foreground transfer. Cancellation makes the next attempt fail.
                let active = platform::present_capture(&window, initial)?;
                if active {
                    session.presented = true;
                }
                Ok(active)
            })?
        })
    } else {
        let id = pin_id(window)?;
        let visible = {
            let state = manager(app);
            let mut pins = state.pins.lock();
            let pin = pins.items.get_mut(id).ok_or("贴图已结束")?;
            pin.visible && !pin.closed
        };
        if visible {
            if let Ok(hwnd) = window.hwnd() {
                platform::configure_capture_surface(hwnd);
            }
            window.show().map_err(|e| e.to_string())?;
            if !manager(app).active.load(Ordering::Acquire) {
                platform::focus(window)?;
            }
        }
        if let Some(pin) = manager(app).pins.lock().items.get_mut(id) {
            pin.ready = true;
        }
        Ok(())
    }
}

pub fn select(
    app: &AppHandle,
    window: &WebviewWindow,
    rect: CaptureRect,
    version: u64,
) -> Result<(), String> {
    let id = session_id(window)?;
    let state = manager(app);
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, id)?;
    if session.scroll.is_some() {
        return Err("请先完成滚动截图".into());
    }
    let image = session.image.as_ref().ok_or("屏幕图像不可用")?;
    if !rect.valid_within(image.width(), image.height()) {
        return Err("选区超出图片范围".into());
    }
    if version < session.version {
        return Err("选区版本已过期".into());
    }
    session.selection = Some(rect);
    session.version = version;
    session.pin_annotations.clear();
    Ok(())
}
pub fn crop(
    app: &AppHandle,
    window: &WebviewWindow,
    rect: CaptureRect,
    version: u64,
    annotations: Option<Vec<super::CaptureAnnotation>>,
) -> Result<Vec<u8>, String> {
    let annotations = annotations.unwrap_or_default();
    super::validate_annotations(&annotations)?;
    select(app, window, rect, version)?;
    let image = {
        let state = manager(app);
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, session_id(window)?)?;
        if session.version != version || session.selection != Some(rect) {
            return Err("选区已经变化，请重新输出".into());
        }
        session.pin_annotations = annotations;
        session.image.clone().ok_or("屏幕图像不可用")?
    };
    clipboard::png(
        &image::imageops::crop_imm(
            image.as_ref(),
            rect.x as u32,
            rect.y as u32,
            rect.width,
            rect.height,
        )
        .to_image(),
    )
}
pub fn candidates(
    app: &AppHandle,
    window: &WebviewWindow,
    x: i32,
    y: i32,
    generation: u64,
) -> Result<CaptureHitTest, String> {
    let id = session_id(window)?;
    let (target, screen) = {
        let state = manager(app);
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, id)?;
        if x < 0
            || y < 0
            || x as u32 >= session.meta.screen.width
            || y as u32 >= session.meta.screen.height
        {
            return Ok(CaptureHitTest::default());
        }
        let screen = session.meta.screen;
        (
            session
                .targets
                .iter()
                .find(|t| t.rect.contains(screen.x + x, screen.y + y))
                .cloned(),
            screen,
        )
    };
    let Some(target) = target else {
        return Ok(CaptureHitTest::default());
    };
    let state = manager(app);
    state.generation.fetch_max(generation, Ordering::AcqRel);
    let detected = state
        .recognition
        .query(id, target.handle, screen.x + x, screen.y + y);
    let pending = detected.pending;
    let mut candidates = detected.candidates;
    candidates.extend(
        target
            .controls
            .iter()
            .filter(|candidate| candidate.rect.contains(screen.x + x, screen.y + y))
            .cloned(),
    );
    candidates.push(CaptureCandidate {
        rect: target.rect,
        label: target.title.clone(),
    });
    if state.generation.load(Ordering::Acquire) > generation
        || !state
            .session
            .lock()
            .as_ref()
            .is_some_and(|s| s.meta.id == id)
    {
        return Ok(CaptureHitTest::default());
    }
    let mut result = Vec::new();
    for candidate in candidates {
        if let Some(mut rect) = candidate
            .rect
            .intersect(screen)
            .and_then(|r| r.intersect(target.rect))
        {
            rect.x -= screen.x;
            rect.y -= screen.y;
            result.push(CaptureCandidate {
                rect,
                label: candidate.label,
            });
        }
    }
    Ok(CaptureHitTest {
        candidates: candidate_chain(result, x, y),
        pending,
    })
}

pub fn export(
    app: &AppHandle,
    window: &WebviewWindow,
    bytes: Vec<u8>,
    version: u64,
    output: &str,
) -> Result<bool, String> {
    let id = session_id(window)?;
    let state = manager(app);
    let _operation = state.operation.lock();
    let (rect, screen, scale, origin) = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, id)?;
        if session.scroll.is_some() {
            return Err("请先完成滚动截图".into());
        }
        if session.version != version {
            return Err("选区已经变化，请重新输出".into());
        }
        (
            session.selection.ok_or("请先选择截图区域")?,
            session.meta.screen,
            session.meta.scale,
            session.image_origin.clone(),
        )
    };
    let image = clipboard::decode(&bytes, ImageFormat::Png)?;
    if image.width() != rect.width || image.height() != rect.height {
        return Err("输出图片尺寸与选区不一致".into());
    }
    let mut new_pin = None;
    match output {
        "clipboard" => clipboard::write(window.hwnd().map_err(|e| e.to_string())?, &image, &bytes)?,
        "file" => {
            if !save_image(app, window, &bytes)? {
                return Ok(false);
            }
        }
        "pin" => {
            let annotations = {
                let mut slot = state.session.lock();
                let session = ensure_session(&mut slot, id)?;
                if session.version != version || session.selection != Some(rect) {
                    return Err("选区已经变化，请重新输出".into());
                }
                session.pin_annotations.clone()
            };
            new_pin = Some(create_pin(
                app,
                bytes,
                image.width(),
                image.height(),
                annotations,
                Some(PinPlacement {
                    position: CapturePoint {
                        x: screen.x + origin.x + rect.x,
                        y: screen.y + origin.y + rect.y,
                    },
                    scale,
                }),
            )?);
        }
        "ask" => {
            deliver_attachment(app, bytes)?;
        }
        _ => return Err("截图输出方式无效".into()),
    }
    finish(app, output != "ask" && output != "pin")?;
    if let Some(window) = new_pin {
        platform::focus(&window)?;
    }
    if output == "ask" {
        crate::windows::show_quick_ask_window(app).map_err(|e| e.to_string())?;
    }
    Ok(true)
}
fn save_image(app: &AppHandle, window: &WebviewWindow, bytes: &[u8]) -> Result<bool, String> {
    let Some(path) =
        platform::save_dialog(app, window.hwnd().map_err(|e| e.to_string())?.0 as usize)?
    else {
        return Ok(false);
    };
    let parent = path.parent().ok_or("保存位置无效")?;
    let temporary = parent.join(format!(".popper-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        crate::settings::atomic_file::replace_file(&temporary, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map(|_| true)
}
fn deliver_attachment(app: &AppHandle, bytes: Vec<u8>) -> Result<(), String> {
    if bytes.len() > crate::quick_ask::MAX_ATTACHMENT_BYTES {
        return Err("截图超过 8 MiB，请缩小选区后重试".into());
    }
    let payload = PendingAttachment {
        id: Uuid::new_v4().to_string(),
        attachment: crate::quick_ask::Attachment {
            name: "Popper-capture.png".into(),
            mime: "image/png".into(),
            kind: "image".into(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        },
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let state = manager(app);
    *state.attachment.lock() = Some(AttachmentDelivery { payload, reply: tx });
    let delivery = (|| {
        crate::windows::show_quick_ask_window(app).map_err(|e| e.to_string())?;
        let _ = app.emit_to("quick-ask", "popper:capture-attachment", ());
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "快捷提问未能接收截图，请重试".to_owned())?
    })();
    state.attachment.lock().take();
    delivery
}
pub fn pending_attachment(app: &AppHandle) -> Result<Option<PendingAttachment>, String> {
    Ok(manager(app)
        .attachment
        .lock()
        .as_ref()
        .map(|v| v.payload.clone()))
}
pub fn ack_attachment(app: &AppHandle, id: &str, error: Option<String>) -> Result<(), String> {
    let state = manager(app);
    let mut slot = state.attachment.lock();
    let delivery = slot
        .as_ref()
        .filter(|d| d.payload.id == id)
        .ok_or("图片交接已结束")?;
    let result = error.map_or(Ok(()), |message| Err(message.chars().take(256).collect()));
    let _ = delivery.reply.try_send(result);
    slot.take();
    Ok(())
}
