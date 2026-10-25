//! Screen image sessions and temporary floating images.
#[cfg(target_os = "windows")]
use tauri::Manager;
use tauri::{AppHandle, WebviewWindow};

#[cfg(target_os = "windows")]
mod accessibility;
#[cfg(target_os = "windows")]
mod clipboard;
#[cfg(target_os = "windows")]
mod desktop;
#[cfg(target_os = "windows")]
mod platform;
#[cfg(target_os = "windows")]
mod recognition;
#[cfg(target_os = "windows")]
pub use desktop::{initialize, launch, setup, shutdown, ShortcutChange};
#[cfg(target_os = "windows")]
pub use recognition::run_helper_if_requested;

#[cfg(any(target_os = "windows", test))]
mod bitmap;
#[cfg(any(target_os = "windows", test))]
mod hit_test;
#[cfg(any(target_os = "windows", test))]
mod pins;
mod types;
pub use types::*;

macro_rules! desktop_call {
    ($($call:tt)*) => {{
        #[cfg(target_os = "windows")] { $($call)* }
        #[cfg(not(target_os = "windows"))] { Err("当前平台暂不支持屏幕捕捉".to_owned()) }
    }};
}

#[tauri::command]
pub async fn capture_start(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    if !matches!(window.label(), "settings" | "quick-ask") {
        return Err("无法从该窗口开始截图".into());
    }
    desktop_call!(
        tauri::async_runtime::spawn_blocking(move || desktop::start(&app))
            .await
            .map_err(|e| e.to_string())?
    )
}
#[tauri::command]
pub fn capture_session(app: AppHandle, window: WebviewWindow) -> Result<CaptureSession, String> {
    desktop_call!(desktop::session(&app, &window))
}
#[tauri::command]
pub async fn capture_image(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<tauri::ipc::Response, String> {
    desktop_call!(tauri::async_runtime::spawn_blocking(
        move || desktop::image(&app, &window).map(tauri::ipc::Response::new)
    )
    .await
    .map_err(|e| e.to_string())?)
}
#[tauri::command]
pub fn capture_show(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    desktop_call!(desktop::show(&app, &window))
}
#[tauri::command]
pub fn capture_move_pointer(
    app: AppHandle,
    window: WebviewWindow,
    dx: i32,
    dy: i32,
) -> Result<CapturePoint, String> {
    desktop_call!(desktop::move_pointer(&app, &window, dx, dy))
}
#[tauri::command]
pub async fn capture_candidates(
    app: AppHandle,
    window: WebviewWindow,
    x: i32,
    y: i32,
    generation: u64,
) -> Result<CaptureHitTest, String> {
    desktop_call!(
        tauri::async_runtime::spawn_blocking(move || desktop::candidates(
            &app, &window, x, y, generation
        ))
        .await
        .map_err(|e| e.to_string())?
    )
}
#[tauri::command]
pub fn capture_preferences(
    app: AppHandle,
    window: WebviewWindow,
    color: String,
    line_width: u8,
) -> Result<(), String> {
    desktop_call!({
        desktop::validate_session(&app, &window)?;
        app.state::<crate::runtime::RuntimeState>()
            .settings
            .update_capture_preferences(color, line_width)
            .map_err(|e| e.to_string())
    })
}
#[tauri::command]
pub fn capture_select(
    app: AppHandle,
    window: WebviewWindow,
    rect: CaptureRect,
    version: u64,
) -> Result<(), String> {
    desktop_call!(desktop::select(&app, &window, rect, version))
}
#[tauri::command]
pub async fn capture_crop(
    app: AppHandle,
    window: WebviewWindow,
    rect: CaptureRect,
    version: u64,
) -> Result<tauri::ipc::Response, String> {
    desktop_call!(tauri::async_runtime::spawn_blocking(move || desktop::crop(
        &app, &window, rect, version
    )
    .map(tauri::ipc::Response::new))
    .await
    .map_err(|e| e.to_string())?)
}
#[tauri::command]
pub async fn capture_export(
    app: AppHandle,
    window: WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<bool, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("图片必须使用二进制传输".into());
    };
    let bytes = bytes.clone();
    let version = request
        .headers()
        .get("x-capture-version")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or("图片版本无效")?;
    let output = request
        .headers()
        .get("x-capture-output")
        .and_then(|v| v.to_str().ok())
        .ok_or("输出方式无效")?
        .to_owned();
    desktop_call!(
        tauri::async_runtime::spawn_blocking(move || desktop::export(
            &app, &window, bytes, version, &output
        ))
        .await
        .map_err(|e| e.to_string())?
    )
}
#[tauri::command]
pub async fn capture_fail(
    app: AppHandle,
    window: WebviewWindow,
    message: String,
) -> Result<(), String> {
    desktop_call!(tauri::async_runtime::spawn_blocking(move || desktop::fail(
        &app, &window, &message
    ))
    .await
    .map_err(|e| e.to_string())?)
}
#[tauri::command]
pub async fn capture_cancel(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    desktop_call!(
        tauri::async_runtime::spawn_blocking(move || desktop::cancel(&app, &window))
            .await
            .map_err(|e| e.to_string())?
    )
}
#[tauri::command]
pub async fn capture_paste(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("无法从该窗口创建贴图".into());
    }
    desktop_call!(
        tauri::async_runtime::spawn_blocking(move || desktop::paste(&app))
            .await
            .map_err(|e| e.to_string())?
    )
}
#[tauri::command]
pub fn capture_pin_state(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<PinnedImageState, String> {
    desktop_call!(desktop::pin_state(&app, &window))
}
#[tauri::command]
pub fn capture_pin_update(
    app: AppHandle,
    window: WebviewWindow,
    update: PinUpdate,
) -> Result<PinnedImageState, String> {
    desktop_call!(desktop::pin_update(&app, &window, update))
}
#[tauri::command]
pub fn capture_pin_menu(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    desktop_call!(desktop::pin_menu(&app, &window))
}
#[tauri::command]
pub fn capture_pin_close(
    app: AppHandle,
    window: WebviewWindow,
    destroy: bool,
) -> Result<(), String> {
    desktop_call!(desktop::pin_close(&app, &window, destroy))
}
#[tauri::command]
pub fn capture_pending_attachment(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<PendingAttachment>, String> {
    if window.label() != "quick-ask" {
        return Err("只能在快捷提问中接收图片".into());
    }
    desktop_call!(desktop::pending_attachment(&app))
}
#[tauri::command]
pub fn capture_ack_attachment(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    error: Option<String>,
) -> Result<(), String> {
    if window.label() != "quick-ask" {
        return Err("只能在快捷提问中接收图片".into());
    }
    desktop_call!(desktop::ack_attachment(&app, &id, error))
}
