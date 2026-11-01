//! Recording shares the scrolling capture cutout and cancellation lifetime.
use super::{ensure_session, manager, platform, scrolling::register_escape, session_id};
use crate::capture::{recording::Recording, CaptureRect, RecordingProgress, ScrollLayout};
use tauri::{AppHandle, WebviewWindow};
use uuid::Uuid;

pub fn record_begin(
    app: &AppHandle,
    window: &WebviewWindow,
    rect: CaptureRect,
    version: u64,
) -> Result<ScrollLayout, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let id = session_id(window)?;
    let (screen, scale, foreground) = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, id)?;
        if session.scroll.is_some() || session.scroll_completed || session.recording.is_some() {
            return Err("屏幕捕捉已经开始，请先完成当前操作".into());
        }
        if version < session.version
            || !rect.valid_within(session.meta.screen.width, session.meta.screen.height)
            || rect.width < 32
            || rect.height < 32
        {
            return Err("请选择至少 32 × 32 像素的有效录屏区域".into());
        }
        (session.meta.screen, session.meta.scale, session.foreground)
    };
    let escape = register_escape(app, window, id)?;
    let mut layout =
        crate::capture::scroll::layout::layout(screen.width, screen.height, scale, rect);
    layout.toolbar.width = ((400.0 * scale).round() as u32).min(
        screen
            .width
            .saturating_sub((24.0 * scale).ceil() as u32)
            .max(1),
    );
    layout.toolbar.x = (rect.x + rect.width as i32 - layout.toolbar.width as i32)
        .clamp(0, screen.width.saturating_sub(layout.toolbar.width) as i32);
    layout.panel = layout.toolbar;
    let setup = (|| {
        platform::prepare_scroll_frame(app, window)?;
        platform::exclude_from_capture(window, true)?;
        platform::record_surface(window, screen, &layout, scale)?;
        platform::restore_foreground(foreground);
        platform::flush();
        let animations = {
            let mut slot = state.session.lock();
            ensure_session(&mut slot, id)?.animations.take()
        };
        // Restoring settings broadcasts window messages; release the session lock first.
        if let Some(mut pause) = animations {
            let restored = pause.restore();
            let mut slot = state.session.lock();
            ensure_session(&mut slot, id)?.animations = Some(pause);
            restored?;
        }
        Recording::start(screen, scale, rect)
    })();
    let recording = match setup {
        Ok(recording) => recording,
        Err(error) => {
            let _ = platform::scroll_surface(window, screen, None);
            let _ = platform::exclude_from_capture(window, false);
            let _ = super::show(app, window);
            return Err(error);
        }
    };
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, id)?;
    session.version = version;
    session.selection = Some(rect);
    session.recording = Some(recording);
    session.scroll_escape = Some(escape);
    Ok(layout)
}
pub fn record_status(app: &AppHandle, window: &WebviewWindow) -> Result<RecordingProgress, String> {
    let state = manager(app);
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, session_id(window)?)?;
    Ok(session.recording.as_ref().ok_or("录屏尚未开始")?.progress())
}
pub fn record_pause(
    app: &AppHandle,
    window: &WebviewWindow,
    paused: bool,
) -> Result<RecordingProgress, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, session_id(window)?)?;
    session
        .recording
        .as_ref()
        .ok_or("录屏尚未开始")?
        .pause(paused)
}
pub fn record_stop(app: &AppHandle, window: &WebviewWindow) -> Result<RecordingProgress, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, session_id(window)?)?;
    let recording = session.recording.as_mut().ok_or("录屏尚未开始")?;
    recording.stop()?;
    Ok(recording.progress())
}
pub fn record_save(app: &AppHandle, window: &WebviewWindow) -> Result<bool, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let source = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, session_id(window)?)?;
        let recording = session.recording.as_mut().ok_or("录屏尚未开始")?;
        recording.stop()?;
        recording.path()
    };
    let Some(path) =
        platform::save_video_dialog(app, window.hwnd().map_err(|e| e.to_string())?.0 as usize)?
    else {
        return Ok(false);
    };
    let temporary = path
        .parent()
        .ok_or("保存位置无效")?
        .join(format!(".popper-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut input = std::fs::File::open(&source)?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        crate::settings::atomic_file::replace_file(&temporary, &path)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("无法保存录屏，内容已保留，请重试：{error}"));
    }
    super::finish(app, true)?;
    Ok(true)
}
