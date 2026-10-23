use super::pins::{Pin, Pins};
use super::{
    clipboard, platform, recognition::Recognition, CaptureCandidate, CaptureRect, CaptureSession,
    PendingAttachment, PinUpdate, PinnedImageState,
};
use crate::{models::CaptureSettings, runtime::RuntimeState};
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
    targets: Vec<platform::TargetWindow>,
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
pub fn active(app: &AppHandle) -> bool {
    app.try_state::<CaptureManager>()
        .is_some_and(|m| m.active.load(Ordering::Acquire))
}

pub fn initialize(app: &AppHandle) {
    app.manage(CaptureManager::new());
}
pub fn setup(app: &AppHandle) {
    app.on_menu_event(|app, event| {
        let Some(value) = event.id().as_ref().strip_prefix("capture-pin:") else {
            return;
        };
        let Some((action, id)) = value.split_once(':') else {
            return;
        };
        if Uuid::parse_str(id).is_err() {
            return;
        }
        let Some(window) = app.get_webview_window(&format!("{PIN_PREFIX}{id}")) else {
            return;
        };
        let app = app.clone();
        let action = action.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let result = match action.as_str() {
                "close" => pin_close(&app, &window, false),
                "destroy" => pin_close(&app, &window, true),
                "reset" | "through" => pin_state(&app, &window).and_then(|state| {
                    pin_update(
                        &app,
                        &window,
                        PinUpdate {
                            zoom: if action == "reset" { 1.0 } else { state.zoom },
                            opacity: if action == "reset" {
                                1.0
                            } else {
                                state.opacity
                            },
                            click_through: action == "through",
                            anchor_x: 0.5,
                            anchor_y: 0.5,
                        },
                    )
                    .map(|updated| {
                        let _ = window.emit("popper:pin-updated", updated);
                    })
                }),
                _ => Ok(()),
            };
            if let Err(error) = result {
                notify(&app, &error);
            }
        });
    });
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
    let preferences = app.state::<RuntimeState>().settings.get_settings().capture;
    let meta = CaptureSession {
        id: id.clone(),
        screen,
        scale,
        color: preferences.color,
        line_width: preferences.line_width,
    };
    let foreground = platform::foreground();
    let targets = platform::windows();
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
        targets,
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
        session(app, window)?;
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
) -> Result<Vec<CaptureCandidate>, String> {
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
            return Ok(Vec::new());
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
        return Ok(Vec::new());
    };
    let state = manager(app);
    state.generation.fetch_max(generation, Ordering::AcqRel);
    let mut candidates = state
        .recognition
        .query(target.handle, screen.x + x, screen.y + y);
    candidates.push(CaptureCandidate {
        rect: target.rect,
        label: target.title,
    });
    if state.generation.load(Ordering::Acquire) > generation
        || !state
            .session
            .lock()
            .as_ref()
            .is_some_and(|s| s.meta.id == id)
    {
        return Ok(Vec::new());
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
            if !result
                .iter()
                .any(|value: &CaptureCandidate| value.rect == rect)
            {
                result.push(CaptureCandidate {
                    rect,
                    label: candidate.label,
                });
            }
        }
    }
    Ok(result)
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

fn create_pin(app: &AppHandle, bytes: Vec<u8>, width: u32, height: u32) -> Result<(), String> {
    let state = manager(app);
    let id = Uuid::new_v4().to_string();
    let meta = PinnedImageState {
        id: id.clone(),
        width,
        height,
        zoom: 1.0,
        opacity: 1.0,
        click_through: false,
    };
    {
        let mut pins = state.pins.lock();
        pins.insert(Pin {
            meta,
            png: Arc::new(bytes),
            visible: true,
            closed: false,
            ready: false,
        })?;
    }
    let result = (|| {
        let (_, screen, scale) = platform::screen()?;
        let (x, y) = platform::cursor()?;
        let label = format!("{PIN_PREFIX}{id}");
        let builder = WebviewWindowBuilder::new(
            app,
            &label,
            WebviewUrl::App("capture/index.html?mode=pin".into()),
        )
        .title("Popper 贴图")
        .inner_size(width as f64 / scale, height as f64 / scale)
        .decorations(false)
        .transparent(true)
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
        window
            .set_size(PhysicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
        window
            .set_position(PhysicalPosition::new(
                (x - width as i32 / 2).max(screen.x),
                (y - height as i32 / 2).max(screen.y),
            ))
            .map_err(|e| e.to_string())?;
        let app = app.clone();
        let closed_id = id.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let state = manager(&app);
                let mut pins = state.pins.lock();
                pins.items.remove(&closed_id);
                if pins.closed.as_deref() == Some(&closed_id) {
                    pins.closed = None;
                }
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if state.stopped.load(Ordering::Acquire) {
                return Err("应用正在退出".into());
            }
            let ready = state
                .pins
                .lock()
                .items
                .get(&id)
                .map(|pin| pin.ready)
                .ok_or("贴图窗口已关闭")?;
            if ready {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("贴图窗口未能就绪，请重试".into());
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        Ok::<_, String>(())
    })();
    if result.is_err() {
        state.pins.lock().items.remove(&id);
        if let Some(window) = app.get_webview_window(&format!("{PIN_PREFIX}{id}")) {
            let _ = window.destroy();
        }
    }
    result
}
pub fn paste(app: &AppHandle) -> Result<(), String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    if state.stopped.load(Ordering::Acquire) || state.active.load(Ordering::Acquire) {
        return Err("请先完成当前截图".into());
    }
    let restore = {
        let mut pins = state.pins.lock();
        pins.closed.take().and_then(|id| {
            pins.items.get_mut(&id).map(|pin| {
                pin.closed = false;
                pin.visible = true;
                id
            })
        })
    };
    if let Some(id) = restore {
        if let Some(window) = app.get_webview_window(&format!("{PIN_PREFIX}{id}")) {
            window
                .show()
                .and_then(|_| window.set_focus())
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        state.pins.lock().items.remove(&id);
    }
    let image = clipboard::read()?;
    create_pin(app, clipboard::png(&image)?, image.width(), image.height())
}
pub fn pin_state(app: &AppHandle, window: &WebviewWindow) -> Result<PinnedImageState, String> {
    let id = pin_id(window)?;
    let state = manager(app);
    let pins = state.pins.lock();
    pins.items
        .get(id)
        .map(|p| p.meta.clone())
        .ok_or_else(|| "贴图已结束".into())
}
pub fn pin_update(
    app: &AppHandle,
    window: &WebviewWindow,
    update: PinUpdate,
) -> Result<PinnedImageState, String> {
    let id = pin_id(window)?;
    if !update.zoom.is_finite()
        || !(0.1..=4.0).contains(&update.zoom)
        || !update.opacity.is_finite()
        || !(0.1..=1.0).contains(&update.opacity)
        || !update.anchor_x.is_finite()
        || !update.anchor_y.is_finite()
        || !(0.0..=1.0).contains(&update.anchor_x)
        || !(0.0..=1.0).contains(&update.anchor_y)
    {
        return Err("贴图参数无效".into());
    }
    let meta = {
        let state = manager(app);
        let pins = state.pins.lock();
        let pin = pins.items.get(id).ok_or("贴图已结束")?;
        if pin.closed {
            return Err("贴图已经关闭".into());
        }
        pin.meta.clone()
    };
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let width = (meta.width as f64 * update.zoom).round().max(1.0) as u32;
    let height = (meta.height as f64 * update.zoom).round().max(1.0) as u32;
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(PhysicalPosition::new(
            position.x + ((size.width as f64 - width as f64) * update.anchor_x).round() as i32,
            position.y + ((size.height as f64 - height as f64) * update.anchor_y).round() as i32,
        ))
        .map_err(|e| e.to_string())?;
    crate::windows::set_native_opacity(window, update.opacity).map_err(|e| e.to_string())?;
    window
        .set_ignore_cursor_events(update.click_through)
        .map_err(|e| e.to_string())?;
    let state = manager(app);
    let mut pins = state.pins.lock();
    let pin = pins.items.get_mut(id).ok_or("贴图已结束")?;
    pin.meta.zoom = update.zoom;
    pin.meta.opacity = update.opacity;
    pin.meta.click_through = update.click_through;
    Ok(pin.meta.clone())
}
pub fn pin_close(app: &AppHandle, window: &WebviewWindow, destroy: bool) -> Result<(), String> {
    let id = pin_id(window)?.to_owned();
    let state = manager(app);
    let discard = state.pins.lock().close(&id, destroy)?;
    if let Some(discard) = discard {
        if let Some(window) = app.get_webview_window(&format!("{PIN_PREFIX}{discard}")) {
            let _ = window.destroy();
        }
    }
    if destroy {
        window.destroy()
    } else {
        window.hide()
    }
    .map_err(|e| e.to_string())
}
pub fn toggle_pins(app: &AppHandle) -> Result<(), String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    if state.active.load(Ordering::Acquire) {
        return Err("请先完成当前截图".into());
    }
    let updates = {
        let mut pins = state.pins.lock();
        pins.hidden = !pins.hidden;
        let hidden = pins.hidden;
        pins.items
            .values_mut()
            .filter(|p| !p.closed)
            .map(|p| {
                p.visible = !hidden;
                (p.meta.id.clone(), p.visible)
            })
            .collect::<Vec<_>>()
    };
    for (id, visible) in updates {
        if let Some(window) = app.get_webview_window(&format!("{PIN_PREFIX}{id}")) {
            if visible {
                window.show()
            } else {
                window.hide()
            }
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub fn recover_click_through(app: &AppHandle) -> Result<(), String> {
    let ids = manager(app)
        .pins
        .lock()
        .items
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for id in ids {
        if let Some(window) = app.get_webview_window(&format!("{PIN_PREFIX}{id}")) {
            window
                .set_ignore_cursor_events(false)
                .map_err(|e| e.to_string())?;
            let updated = {
                let state = manager(app);
                let mut pins = state.pins.lock();
                pins.items.get_mut(&id).map(|pin| {
                    pin.meta.click_through = false;
                    pin.meta.clone()
                })
            };
            if let Some(updated) = updated {
                let _ = window.emit("popper:pin-updated", updated);
            }
        }
    }
    Ok(())
}

pub struct ShortcutChange {
    app: AppHandle,
    old: Vec<String>,
    next: Vec<String>,
    added: Vec<String>,
    committed: bool,
}
impl ShortcutChange {
    pub fn prepare(app: &AppHandle, next: &CaptureSettings) -> Result<Self, String> {
        use tauri_plugin_global_shortcut::{GlobalShortcut, ShortcutState};
        let shortcuts = app
            .try_state::<GlobalShortcut<tauri::Wry>>()
            .ok_or("全局快捷键服务未启动，请重启 Popper")?;
        let old = manager(app).shortcuts.lock().clone();
        let next = next
            .shortcuts()
            .iter()
            .map(|v| (*v).to_owned())
            .collect::<Vec<_>>();
        let mut change = Self {
            app: app.clone(),
            old,
            next,
            added: Vec::new(),
            committed: false,
        };
        for value in &change.next {
            if change.old.contains(value) {
                continue;
            }
            let expected = value.clone();
            shortcuts
                .on_shortcut(value.as_str(), move |app, _, event| {
                    if event.state() != ShortcutState::Pressed
                        || app.state::<RuntimeState>().global_shortcuts_suspended()
                    {
                        return;
                    }
                    let preferences = app.state::<RuntimeState>().settings.get_settings().capture;
                    let operation = if preferences.shortcut == expected {
                        "capture"
                    } else if preferences.paste_shortcut == expected {
                        "paste"
                    } else if preferences.toggle_pins_shortcut == expected {
                        "toggle"
                    } else {
                        return;
                    };
                    launch(app, operation);
                })
                .map_err(|e| format!("无法注册屏幕捕捉快捷键 {value}，可能已被占用：{e}"))?;
            change.added.push(value.clone());
        }
        Ok(change)
    }
    pub fn commit(mut self) {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        for value in &self.old {
            if !self.next.contains(value) {
                let _ = self.app.global_shortcut().unregister(value.as_str());
            }
        }
        *manager(&self.app).shortcuts.lock() = self.next.clone();
        self.app
            .state::<RuntimeState>()
            .set_capture_shortcut_error(None);
        self.committed = true;
    }
}
impl Drop for ShortcutChange {
    fn drop(&mut self) {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        if !self.committed {
            for value in &self.added {
                let _ = self.app.global_shortcut().unregister(value.as_str());
            }
        }
    }
}

pub fn pin_menu(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    use tauri::menu::{ContextMenu, Menu, MenuItem};
    let state = pin_state(app, window)?;
    let item = |action: &str, label: &str| {
        MenuItem::with_id(
            app,
            format!("capture-pin:{action}:{}", state.id),
            label,
            true,
            None::<&str>,
        )
        .map_err(|e| e.to_string())
    };
    let reset = item("reset", "恢复原始大小和透明度")?;
    let through = item("through", "鼠标穿透（从托盘恢复）")?;
    let close = item("close", "关闭 · 可恢复")?;
    let destroy = item("destroy", "销毁图片")?;
    Menu::with_items(app, &[&reset, &through, &close, &destroy])
        .and_then(|menu| menu.popup(window.as_ref().window().clone()))
        .map_err(|e| e.to_string())
}
