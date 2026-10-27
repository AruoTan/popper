//! Owned toolbar windows; moving a toolbar never changes its image window.
use super::{manager, pin_id, pinned, platform, PIN_PREFIX};
use crate::capture::{
    pins::toolbar::{HEIGHT, WIDTH},
    PinToolbarAction, PinToolbarState,
};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use uuid::Uuid;

const PREFIX: &str = "capture-pin-toolbar-";

fn owner(app: &AppHandle, window: &WebviewWindow) -> Result<WebviewWindow, String> {
    let id = window
        .label()
        .strip_prefix(PREFIX)
        .filter(|id| Uuid::parse_str(id).is_ok())
        .ok_or("贴图工具条窗口无效")?;
    if !manager(app).pins.lock().items.contains_key(id) {
        return Err("贴图已结束".into());
    }
    app.get_webview_window(&format!("{PIN_PREFIX}{id}"))
        .ok_or_else(|| "贴图已结束".into())
}

pub fn toolbar_state(app: &AppHandle, window: &WebviewWindow) -> Result<PinToolbarState, String> {
    let parent = owner(app, window)?;
    manager(app)
        .pins
        .lock()
        .items
        .get(pin_id(&parent)?)
        .and_then(|pin| pin.toolbar.state.clone())
        .ok_or_else(|| "工具条尚未就绪".into())
}

pub fn toolbar_publish(
    app: &AppHandle,
    window: &WebviewWindow,
    snapshot: PinToolbarState,
) -> Result<(), String> {
    let meta = pinned::active_pin_state(app, window)?;
    if !meta.toolbar_visible {
        return Ok(());
    }
    if !(1..=32).contains(&snapshot.line_width) || !valid_color(&snapshot.color) {
        return Err("工具条参数无效".into());
    }
    let label = format!("{PREFIX}{}", meta.id);
    let existing = app.get_webview_window(&label);
    let create = {
        let state = manager(app);
        let mut pins = state.pins.lock();
        let pin = pins.items.get_mut(&meta.id).ok_or("贴图已结束")?;
        pin.toolbar.state = Some(snapshot.clone());
        if existing.is_none() && !pin.toolbar.creating {
            pin.toolbar.creating = true;
            true
        } else {
            false
        }
    };
    if create {
        let result = (|| {
            let builder = WebviewWindowBuilder::new(
                app,
                &label,
                WebviewUrl::App("capture/index.html?mode=pin-toolbar".into()),
            )
            .title("Popper 贴图工具条")
            .theme(Some(tauri::Theme::Light))
            .owner(window)
            .map_err(|e| e.to_string())?
            .inner_size(WIDTH, HEIGHT)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(meta.always_on_top)
            .skip_taskbar(true)
            .visible(false)
            .focused(false);
            let tools = crate::app_paths::configure_webview(builder)
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| e.to_string())?;
            if let Ok(hwnd) = tools.hwnd() {
                platform::configure_capture_surface(hwnd);
            }
            let handle = app.clone();
            let id = meta.id.clone();
            tools.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let app = handle.clone();
                    let label = format!("{PREFIX}{id}");
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Some(window) = app.get_webview_window(&label) {
                            let _ = toolbar_action(&app, &window, PinToolbarAction::Hide);
                        }
                    });
                }
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    if let Some(pin) = manager(&handle).pins.lock().items.get_mut(&id) {
                        pin.toolbar.ready = false;
                        pin.toolbar.creating = false;
                    }
                }
            });
            if !manager(app).pins.lock().items.contains_key(&meta.id) {
                let _ = tools.destroy();
            }
            Ok::<_, String>(())
        })();
        if let Some(pin) = manager(app).pins.lock().items.get_mut(&meta.id) {
            pin.toolbar.creating = false;
        }
        result?;
    } else if let Some(tools) = existing {
        tools
            .emit("popper:pin-toolbar-state", snapshot)
            .map_err(|e| e.to_string())?;
    }
    sync(app, &meta.id)
}

pub fn toolbar_ready(app: &AppHandle, window: &WebviewWindow, height: f64) -> Result<(), String> {
    if !height.is_finite() || !(HEIGHT..=512.0).contains(&height) {
        return Err("工具条尺寸无效".into());
    }
    let parent = owner(app, window)?;
    let id = pin_id(&parent)?;
    if let Some(pin) = manager(app).pins.lock().items.get_mut(id) {
        pin.toolbar.ready = true;
        pin.toolbar.height = height;
    }
    sync(app, id)
}

pub fn toolbar_action(
    app: &AppHandle,
    window: &WebviewWindow,
    action: PinToolbarAction,
) -> Result<(), String> {
    let parent = owner(app, window)?;
    let meta = pinned::active_pin_state(app, &parent)?;
    if !meta.toolbar_visible {
        return Err("工具条已隐藏".into());
    }
    match &action {
        PinToolbarAction::Color { color } if !valid_color(color) => {
            return Err("标注颜色无效".into())
        }
        PinToolbarAction::Width { width } if !(1..=32).contains(width) => {
            return Err("标注线宽无效".into())
        }
        PinToolbarAction::Close { destroy } => return pinned::pin_close(app, &parent, *destroy),
        PinToolbarAction::Hide => {
            parent
                .emit("popper:pin-toolbar-action", PinToolbarAction::Hide)
                .map_err(|e| e.to_string())?;
            let updated = {
                let state = manager(app);
                let mut pins = state.pins.lock();
                let pin = pins.items.get_mut(&meta.id).ok_or("贴图已结束")?;
                pin.meta.toolbar_visible = false;
                pin.meta.clone()
            };
            parent
                .emit("popper:pin-updated", updated)
                .map_err(|e| e.to_string())?;
            return sync(app, &meta.id);
        }
        PinToolbarAction::Copy => return pinned::pin_action(app, &parent, "copy"),
        PinToolbarAction::Save => return pinned::pin_action(app, &parent, "save"),
        _ => {}
    }
    let enabled = meta.annotations_visible
        && manager(app)
            .pins
            .lock()
            .items
            .get(&meta.id)
            .and_then(|pin| pin.toolbar.state.as_ref())
            .is_some_and(|state| state.enabled && !state.busy);
    if !enabled {
        return Err("标注当前无法操作".into());
    }
    parent
        .emit("popper:pin-toolbar-action", action)
        .map_err(|e| e.to_string())
}

fn valid_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|c| c.is_ascii_hexdigit())
}

pub(super) fn sync(app: &AppHandle, id: &str) -> Result<(), String> {
    {
        let state = manager(app);
        let mut pins = state.pins.lock();
        let Some(pin) = pins.items.get_mut(id) else {
            return Ok(());
        };
        if pin.toolbar.state.is_none() {
            return Ok(());
        }
        if pin.toolbar.sync_pending {
            return Ok(());
        }
        pin.toolbar.sync_pending = true;
    }
    let handle = app.clone();
    let id = id.to_owned();
    let failed_id = id.clone();
    let result = app
        .run_on_main_thread(move || {
            if let Some(pin) = manager(&handle).pins.lock().items.get_mut(&id) {
                pin.toolbar.sync_pending = false;
            }
            if let Err(error) = sync_on_ui(&handle, &id) {
                if let Some(parent) = handle.get_webview_window(&format!("{PIN_PREFIX}{id}")) {
                    let _ = parent.emit("popper:pin-toolbar-error", error);
                }
            }
        })
        .map_err(|e| e.to_string());
    if result.is_err() {
        if let Some(pin) = manager(app).pins.lock().items.get_mut(&failed_id) {
            pin.toolbar.sync_pending = false;
        }
    }
    result
}

fn sync_on_ui(app: &AppHandle, id: &str) -> Result<(), String> {
    let Some(tools) = app.get_webview_window(&format!("{PREFIX}{id}")) else {
        return Ok(());
    };
    let Some(parent) = app.get_webview_window(&format!("{PIN_PREFIX}{id}")) else {
        let _ = tools.destroy();
        return Ok(());
    };
    let state = manager(app);
    // Preserve the visible desktop while a capture is in progress. In particular,
    // delayed toolbar geometry updates must not hide part of the source image.
    if state.active.load(Ordering::Acquire) {
        return Ok(());
    }
    let meta = {
        let pins = state.pins.lock();
        let Some(pin) = pins.items.get(id) else {
            return Ok(());
        };
        if state.stopped.load(Ordering::Acquire)
            || pin.closed
            || !pin.visible
            || !pin.meta.toolbar_visible
            || !pin.toolbar.ready
        {
            drop(pins);
            return hide(&tools);
        }
        pin.meta.clone()
    };
    if !parent.is_visible().unwrap_or(false) {
        return hide(&tools);
    }
    let (image, work, scale) = platform::pin_toolbar_geometry(&parent, &meta)?;
    let (rect, top_changed) = {
        let mut pins = state.pins.lock();
        let Some(pin) = pins.items.get_mut(id) else {
            return Ok(());
        };
        let height = pin.toolbar.height.max(HEIGHT);
        let rect = pin.toolbar.placement.layout(image, work, height, scale);
        let changed = pin.toolbar.topmost != Some(meta.always_on_top);
        pin.toolbar.topmost = Some(meta.always_on_top);
        (rect, changed)
    };
    if top_changed {
        tools
            .set_always_on_top(meta.always_on_top)
            .map_err(|e| e.to_string())?;
    }
    platform::position_pin_toolbar(&tools, rect)?;
    if !tools.is_visible().map_err(|e| e.to_string())? {
        tools.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn hide(window: &WebviewWindow) -> Result<(), String> {
    if window.is_visible().map_err(|e| e.to_string())? {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn refresh_visible(app: &AppHandle) {
    let ids: Vec<_> = manager(app).pins.lock().items.keys().cloned().collect();
    for id in ids {
        let _ = sync(app, &id);
    }
}
