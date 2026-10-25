//! Native floating-image windows, menu actions and visibility lifecycle.
use super::{clipboard, manager, notify, pin_id, platform, PIN_PREFIX};
use crate::capture::{
    pins::{image_frame, resized_frame, Pin, SHADOW_INSET},
    CapturePoint, CaptureRect, PinUpdate, PinnedImageState,
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use uuid::Uuid;

pub(super) struct PinPlacement {
    pub position: CapturePoint,
    pub scale: f64,
}

pub(super) fn show_pin(window: &WebviewWindow) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())?;
    window
        .emit("popper:pin-shown", ())
        .map_err(|e| e.to_string())
}

pub(super) fn setup_menu(app: &AppHandle) {
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
}

pub(super) fn create_pin(
    app: &AppHandle,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    placement: Option<PinPlacement>,
) -> Result<(), String> {
    let state = manager(app);
    let id = Uuid::new_v4().to_string();
    let meta = PinnedImageState {
        id: id.clone(),
        width,
        height,
        zoom: 1.0,
        opacity: 1.0,
        click_through: false,
        shadow_inset: SHADOW_INSET,
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
        let PinPlacement { position, scale } = match placement {
            Some(placement) => placement,
            None => {
                let (_, screen, scale) = platform::screen()?;
                let (x, y) = platform::cursor()?;
                PinPlacement {
                    position: CapturePoint {
                        x: (x - width as i32 / 2).max(screen.x),
                        y: (y - height as i32 / 2).max(screen.y),
                    },
                    scale,
                }
            }
        };
        let frame = image_frame(
            CaptureRect {
                x: position.x,
                y: position.y,
                width,
                height,
            },
            SHADOW_INSET,
        );
        let label = format!("{PIN_PREFIX}{id}");
        let builder = WebviewWindowBuilder::new(
            app,
            &label,
            WebviewUrl::App("capture/index.html?mode=pin".into()),
        )
        .title("Popper 贴图")
        .inner_size(frame.width as f64 / scale, frame.height as f64 / scale)
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
        if let Ok(hwnd) = window.hwnd() {
            platform::configure_capture_surface(hwnd);
        }
        window
            .set_position(PhysicalPosition::new(frame.x, frame.y))
            .map_err(|e| e.to_string())?;
        // Moving between monitors can change DPI; apply the physical size afterwards.
        window
            .set_size(PhysicalSize::new(frame.width, frame.height))
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
            show_pin(&window)?;
            window.set_focus().map_err(|e| e.to_string())?;
            return Ok(());
        }
        state.pins.lock().items.remove(&id);
    }
    let image = clipboard::read()?;
    create_pin(
        app,
        clipboard::png(&image)?,
        image.width(),
        image.height(),
        None,
    )
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
    if update.zoom != meta.zoom {
        let frame = resized_frame(
            &meta,
            CaptureRect {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            },
            &update,
        );
        window
            .set_position(PhysicalPosition::new(frame.x, frame.y))
            .map_err(|e| e.to_string())?;
        window
            .set_size(PhysicalSize::new(frame.width, frame.height))
            .map_err(|e| e.to_string())?;
    }
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
                show_pin(&window)?;
            } else {
                window.hide().map_err(|e| e.to_string())?;
            }
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
