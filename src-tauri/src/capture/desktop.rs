mod pinned;
mod shortcuts;
use pinned::create_pin;
pub use pinned::{
    paste, pin_close, pin_menu, pin_state, pin_update, recover_click_through, toggle_pins,
};
pub use shortcuts::ShortcutChange;

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
    image: Option<Arc<RgbaImage>>,
    png: Option<Arc<Vec<u8>>>,
    selection: Option<CaptureRect>,
    version: u64,
    targets: Vec<Arc<platform::TargetWindow>>,
    previous: Vec<String>,
    foreground: usize,
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
        state.session.lock().take();
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
        .title("Popper 屏幕捕捉")
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
                    let _ = window.destroy();
                });
            }
        }
    });
}
pub fn launch(app: &AppHandle, operation: &'static str) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = match operation {
            "capture" => start(&handle),
            "paste" => paste(&handle),
            "toggle" => toggle_pins(&handle),
            "recover" => recover_click_through(&handle),
            _ => Ok(()),
        };
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
        if let Some(id) = state.session.lock().as_ref().map(|s| s.meta.id.clone()) {
            if let Some(window) = app.get_webview_window(&format!("{CAPTURE_PREFIX}{id}")) {
                let _ = window.set_focus();
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
    let previous = platform::on_ui(app, |handle| {
        handle
            .webview_windows()
            .into_iter()
            .filter_map(|(label, window)| {
                if window.is_visible().unwrap_or(false) && !label.starts_with("capture-notice-") {
                    Some(label)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    })?;
    *manager(app).session.lock() = Some(Session {
        meta,
        image: None,
        png: None,
        selection: None,
        version: 0,
        targets: targets.into_iter().map(Arc::new).collect(),
        previous: previous.clone(),
        foreground,
    });
    app.state::<RuntimeState>()
        .set_screen_capture_active(app, true);
    platform::on_ui(app, move |handle| {
        for label in previous {
            if let Some(window) = handle.get_webview_window(&label) {
                let _ = window.hide();
            }
        }
    })?;
    platform::flush();
    let image = monitor
        .capture_image()
        .map_err(|e| format!("无法捕捉屏幕：{e}"))?;
    if image.width() != screen.width || image.height() != screen.height {
        return Err("显示器布局已变化，请重新截图".into());
    }
    let png = Arc::new(clipboard::png(&image)?);
    {
        let state = manager(app);
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, &id)?;
        session.image = Some(Arc::new(image));
        session.png = Some(png);
    }
    let label = format!("{CAPTURE_PREFIX}{id}");
    let builder = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App("capture/index.html?mode=capture".into()),
    )
    .title("Popper 屏幕捕捉")
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
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let current = manager(&handle)
                .session
                .lock()
                .as_ref()
                .is_some_and(|s| s.meta.id == id);
            if !current {
                break;
            }
            let ready = handle
                .get_webview_window(&label)
                .is_some_and(|window| window.is_visible().unwrap_or(false));
            let geometry_ok = platform::same_screen(screen, scale);
            if !geometry_ok
                || platform::foreground() == 0
                || (!ready && std::time::Instant::now() >= deadline)
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
    let result = platform::on_ui(app, move |handle| {
        if let Some(window) = handle.get_webview_window(&format!("{CAPTURE_PREFIX}{id}")) {
            let _ = window.destroy();
        }
        for label in previous.previous {
            if let Some(window) = handle.get_webview_window(&label) {
                if label.starts_with(PIN_PREFIX) {
                    let state = manager(&handle);
                    let pins = state.pins.lock();
                    if !pins.items.values().any(|pin| {
                        format!("{PIN_PREFIX}{}", pin.meta.id) == label
                            && pin.visible
                            && !pin.closed
                    }) {
                        continue;
                    }
                }
                let _ = window.show();
            }
        }
        if restore_focus {
            platform::restore_foreground(foreground);
        }
    });
    state.active.store(false, Ordering::Release);
    app.state::<RuntimeState>()
        .set_screen_capture_active(app, false);
    result
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
        validate_session(app, window)?;
        if let Ok(hwnd) = window.hwnd() {
            platform::configure_capture_surface(hwnd);
        }
        window
            .show()
            .and_then(|_| window.set_focus())
            .map_err(|e| e.to_string())
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
    if !rect.valid_within(session.meta.screen.width, session.meta.screen.height) {
        return Err("选区超出屏幕范围".into());
    }
    if version < session.version {
        return Err("选区版本已过期".into());
    }
    session.selection = Some(rect);
    session.version = version;
    Ok(())
}
pub fn crop(
    app: &AppHandle,
    window: &WebviewWindow,
    rect: CaptureRect,
    version: u64,
) -> Result<Vec<u8>, String> {
    select(app, window, rect, version)?;
    let image = {
        let state = manager(app);
        let mut slot = state.session.lock();
        ensure_session(&mut slot, session_id(window)?)?
            .image
            .clone()
            .ok_or("屏幕图像不可用")?
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
    let rect = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, id)?;
        if session.version != version {
            return Err("选区已经变化，请重新输出".into());
        }
        session.selection.ok_or("请先选择截图区域")?
    };
    let image = clipboard::decode(&bytes, ImageFormat::Png)?;
    if image.width() != rect.width || image.height() != rect.height {
        return Err("输出图片尺寸与选区不一致".into());
    }
    match output {
        "clipboard" => clipboard::write(window.hwnd().map_err(|e| e.to_string())?, &image, &bytes)?,
        "file" => {
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
                file.write_all(&bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|e| e.to_string())?;
                drop(file);
                crate::settings::atomic_file::replace_file(&temporary, &path)
                    .map_err(|e| e.to_string())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&temporary);
            }
            result?;
        }
        "pin" => {
            create_pin(app, bytes, image.width(), image.height())?;
        }
        "ask" => {
            deliver_attachment(app, bytes)?;
        }
        _ => return Err("截图输出方式无效".into()),
    }
    finish(app, output != "ask")?;
    if output == "ask" {
        crate::windows::show_quick_ask_window(app).map_err(|e| e.to_string())?;
    }
    Ok(true)
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
