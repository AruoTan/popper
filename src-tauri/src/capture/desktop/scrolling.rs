//! Live capture control surface and session-scoped scrolling resources.
use super::{ensure_session, manager, platform, session_id};
use crate::capture::{
    clipboard, scroll::stream::ScrollStream, CaptureDocument, CapturePoint, CaptureRect,
    ScrollCaptureSession, ScrollProgress,
};
use std::{sync::Arc, time::Duration};
use tauri::{AppHandle, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub(super) struct ScrollSession {
    pub target: usize,
    rect: CaptureRect,
    stream: ScrollStream,
}
// Retained through output retries until capture ends.
pub(super) struct ScrollEscape {
    app: AppHandle,
}
impl Drop for ScrollEscape {
    fn drop(&mut self) {
        let _ = self.app.global_shortcut().unregister("Escape");
    }
}

fn restore_editor(app: &AppHandle, window: &WebviewWindow, screen: CaptureRect) -> Result<(), String> {
    platform::scroll_surface(window, screen, None)?;
    let _ = platform::exclude_from_capture(window, false);
    let window = window.clone();
    platform::on_ui(app, move |_| platform::present_capture(&window))?
}

pub fn scroll_begin(
    app: &AppHandle,
    window: &WebviewWindow,
    rect: CaptureRect,
    version: u64,
) -> Result<ScrollCaptureSession, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let id = session_id(window)?;
    let control = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let (screen, scale, target) = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, id)?;
        if session.scroll.is_some() {
            return Err("滚动截图已经开始".into());
        }
        if session.scroll_completed {
            return Err("长图已生成，请重新截图以开始新的滚动捕捉".into());
        }
        if version < session.version
            || !rect.valid_within(session.meta.screen.width, session.meta.screen.height)
            || rect.width < 32
            || rect.height < 96
        {
            return Err("请选择至少 32 × 96 像素的有效滚动区域".into());
        }
        let screen = session.meta.screen;
        let x = screen.x + rect.x + (rect.width / 2) as i32;
        let y = screen.y + rect.y + (rect.height / 2) as i32;
        let target = session
            .targets
            .iter()
            .find(|t| t.rect.contains(x, y))
            .map(|t| t.handle)
            .ok_or("选区中没有可滚动的应用窗口")?;
        (screen, session.meta.scale, target)
    };
    if app.global_shortcut().is_registered("Escape") {
        return Err("滚动截图取消按键已被占用，暂时无法开始".into());
    }
    let cancel_id = id.to_owned();
    app.global_shortcut()
        .on_shortcut("Escape", move |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                // Close an owned modal dialog so cancellation can acquire the output lock.
                platform::dismiss_owned_dialog(control);
                let app = app.clone();
                let id = cancel_id.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _ = super::cancel_id(&app, &id);
                });
            }
        })
        .map_err(|e| format!("无法注册滚动截图取消按键：{e}"))?;
    let escape = ScrollEscape { app: app.clone() };
    let layout = crate::capture::scroll::layout::layout(screen.width, screen.height, scale, rect);
    let setup = (|| {
        // Keep the frozen surface visible until the renderer receives the final layout.
        // Capture exclusion makes hiding and re-showing this window unnecessary.
        platform::prepare_scroll_frame(app, window)?;
        platform::exclude_from_capture(window, true)?;
        platform::scroll_surface(window, screen, Some(&layout))?;
        platform::restore_foreground(target);
        platform::flush();
        platform::capture_region(screen, rect)
    })();
    let image = match setup {
        Ok(image) => image,
        Err(error) => {
            let _ = restore_editor(app, window, screen);
            return Err(error);
        }
    };
    let stream = match ScrollStream::start(
        image,
        move || {
            let foreground = platform::foreground();
            if foreground != target && foreground != control {
                return Err("请回到原来的应用窗口后继续，或完成当前长图".into());
            }
            platform::capture_region(screen, rect).map(Some)
        },
        Duration::from_millis(8),
    ) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = restore_editor(app, window, screen);
            return Err(error);
        }
    };
    let progress = stream.snapshot();
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, id)?;
    session.version = version;
    session.selection = Some(rect);
    session.pin_annotations.clear();
    session.scroll = Some(ScrollSession {
        target,
        rect,
        stream,
    });
    session.scroll_escape = Some(escape);
    Ok(ScrollCaptureSession { progress, layout })
}

pub fn scroll_step(app: &AppHandle, window: &WebviewWindow) -> Result<ScrollProgress, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, session_id(window)?)?;
    let screen = session.meta.screen;
    if !platform::same_screen(screen, session.meta.scale) {
        return Err("显示器布局已变化，请重新截图".into());
    }
    let scroll = session.scroll.as_mut().ok_or("滚动截图已结束")?;
    if platform::foreground() != scroll.target && !platform::is_foreground(window)? {
        return Err("请回到原来的应用窗口后继续，或完成当前长图".into());
    }
    scroll.stream.progress()
}

pub fn scroll_preview(
    app: &AppHandle,
    window: &WebviewWindow,
    max_width: u32,
    max_height: u32,
) -> Result<Vec<u8>, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let snapshot = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, session_id(window)?)?;
        session
            .scroll
            .as_ref()
            .ok_or("滚动截图已结束")?
            .stream
            .preview()
    };
    // Filtering immutable strips leaves registration free to process new frames.
    let image = snapshot.render(max_width, max_height);
    // Metadata and thumbnail describe one atomic document snapshot.
    let metadata = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    let mut bytes = (metadata.len() as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(&metadata);
    bytes.extend_from_slice(&clipboard::png(&image)?);
    Ok(bytes)
}

pub fn scroll_finish(app: &AppHandle, window: &WebviewWindow) -> Result<CaptureDocument, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, session_id(window)?)?;
    if session.scroll_completed {
        let image = session.image.as_ref().ok_or("长图不可用")?;
        return Ok(CaptureDocument {
            width: image.width(),
            height: image.height(),
            version: session.version,
        });
    }
    let scroll = session.scroll.as_mut().ok_or("滚动截图已结束")?;
    // Include a final manual scroll even if the user finishes before the next poll.
    // Failures leave the last registered document available for output.
    scroll.stream.stop();
    let final_frame = if platform::same_screen(session.meta.screen, session.meta.scale)
        && (platform::foreground() == scroll.target || platform::is_foreground(window)?)
    {
        platform::capture_region(session.meta.screen, scroll.rect).ok()
    } else {
        None
    };
    let image = scroll.stream.compose(final_frame);
    let png = clipboard::png(&image)?;
    // Keep the live surface visible for direct output and save cancellation.
    let rect = scroll.rect;
    session.image_origin = CapturePoint {
        x: rect.x,
        y: rect.y,
    };
    session.version += 1;
    session.selection = Some(CaptureRect {
        x: 0,
        y: 0,
        width: image.width(),
        height: image.height(),
    });
    let result = CaptureDocument {
        width: image.width(),
        height: image.height(),
        version: session.version,
    };
    session.image = Some(Arc::new(image));
    session.png = Some(Arc::new(png));
    session.scroll_completed = true;
    session.scroll.take();
    Ok(result)
}
