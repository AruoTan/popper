//! Live capture control surface and session-scoped scrolling resources.
use super::{ensure_session, manager, platform, session_id};
use crate::capture::{
    clipboard, scroll::stream::ScrollStream, CaptureDocument, CapturePoint, CaptureRect,
    ScrollProgress,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{AppHandle, PhysicalPosition, PhysicalSize, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub(super) struct ScrollSession {
    pub target: usize,
    rect: CaptureRect,
    stream: ScrollStream,
}
// Retained until the full editor has regained native and WebView focus.
pub(super) struct EscapeShortcut {
    app: AppHandle,
}
impl Drop for EscapeShortcut {
    fn drop(&mut self) {
        let _ = self.app.global_shortcut().unregister("Escape");
    }
}

fn restore_editor(window: &WebviewWindow, screen: CaptureRect) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())?;
    window
        .set_position(PhysicalPosition::new(screen.x, screen.y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(PhysicalSize::new(screen.width, screen.height))
        .map_err(|e| e.to_string())?;
    let _ = platform::exclude_from_capture(window, false);
    Ok(())
}

pub fn scroll_begin(
    app: &AppHandle,
    window: &WebviewWindow,
    rect: CaptureRect,
    version: u64,
) -> Result<ScrollProgress, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let id = session_id(window)?;
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
        return Err("Esc 已被其他功能使用，暂时无法开始滚动截图".into());
    }
    let cancel_id = id.to_owned();
    app.global_shortcut()
        .on_shortcut("Escape", move |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                let app = app.clone();
                let id = cancel_id.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _ = super::cancel_id(&app, &id);
                });
            }
        })
        .map_err(|e| format!("无法注册滚动截图取消按键：{e}"))?;
    let escape = EscapeShortcut { app: app.clone() };
    let setup = (|| {
        window.hide().map_err(|e| e.to_string())?;
        platform::exclude_from_capture(window, true)?;
        let width = ((380.0 * scale).round() as u32).min(screen.width);
        let height = ((190.0 * scale).round() as u32).min(screen.height);
        let right = screen.width.saturating_sub(width + 16) as i32;
        let bottom = screen.height.saturating_sub(height + 16) as i32;
        let center = (
            rect.x + (rect.width / 2) as i32,
            rect.y + (rect.height / 2) as i32,
        );
        let (x, y) = [(right, 16), (right, bottom), (16, 16), (16, bottom)]
            .into_iter()
            .min_by_key(|&(x, y)| {
                let panel = CaptureRect {
                    x,
                    y,
                    width,
                    height,
                };
                let overlap = panel
                    .intersect(rect)
                    .map_or(0, |r| u64::from(r.width) * u64::from(r.height));
                (panel.contains(center.0, center.1), overlap)
            })
            .unwrap();
        window
            .set_size(PhysicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
        window
            .set_position(PhysicalPosition::new(screen.x + x, screen.y + y))
            .map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        platform::restore_foreground(target);
        platform::flush();
        platform::capture_region(screen, rect)
    })();
    let image = match setup {
        Ok(image) => image,
        Err(error) => {
            let _ = restore_editor(window, screen);
            let _ = window.show();
            let _ = platform::focus(window);
            return Err(error);
        }
    };
    let control = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let stream = match ScrollStream::start(
        image,
        move || {
            let foreground = platform::foreground();
            if foreground != target && foreground != control {
                return Err("请回到原来的应用窗口后继续，或完成当前长图".into());
            }
            platform::capture_region(screen, rect).map(Some)
        },
        Duration::from_millis(16),
    ) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = restore_editor(window, screen);
            let _ = window.show();
            let _ = platform::focus(window);
            return Err(error);
        }
    };
    let progress = stream.snapshot();
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, id)?;
    session.version = version;
    session.selection = Some(rect);
    session.scroll = Some(ScrollSession {
        target,
        rect,
        stream,
    });
    session.scroll_escape = Some(escape);
    Ok(progress)
}

pub fn scroll_step(
    app: &AppHandle,
    window: &WebviewWindow,
    automatic: bool,
) -> Result<ScrollProgress, String> {
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
    if automatic {
        let before = scroll.stream.position();
        platform::restore_foreground(scroll.target);
        platform::scroll_down(
            scroll.target,
            CapturePoint {
                x: screen.x + scroll.rect.x + (scroll.rect.width / 2) as i32,
                y: screen.y + scroll.rect.y + (scroll.rect.height / 2) as i32,
            },
        )?;
        std::thread::sleep(Duration::from_millis(350));
        return scroll.stream.progress_since(before);
    }
    scroll.stream.progress()
}

pub fn scroll_finish(app: &AppHandle, window: &WebviewWindow) -> Result<CaptureDocument, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, session_id(window)?)?;
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
    // Keep the scrolling document intact if changing the native surface fails.
    session.hidden_until = Instant::now() + Duration::from_secs(10);
    if let Err(error) = restore_editor(window, session.meta.screen) {
        let _ = platform::exclude_from_capture(window, true);
        let _ = window.show();
        return Err(error);
    }
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
