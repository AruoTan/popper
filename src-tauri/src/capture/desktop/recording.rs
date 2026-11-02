//! Recording shares the scrolling capture cutout and cancellation lifetime.
use super::{ensure_session, manager, platform, scrolling::register_escape, session_id};
use crate::capture::{
    recording::Recording, CaptureRect, RecordingOptions, RecordingProgress, RecordingSurface,
    ScrollLayout,
};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
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
    let (screen, scale) = {
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
        (session.meta.screen, session.meta.scale)
    };
    let escape = register_escape(app, window, id)?;
    let layout = recording_layout(screen, scale, rect);
    let setup = (|| {
        platform::exclude_from_capture(window, true)?;
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
        Recording::prepare(screen, scale, rect)
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
fn recording_layout(screen: CaptureRect, scale: f64, rect: CaptureRect) -> ScrollLayout {
    let mut layout =
        crate::capture::scroll::layout::layout(screen.width, screen.height, scale, rect);
    layout.toolbar.width = ((480.0 * scale).round() as u32).min(
        screen
            .width
            .saturating_sub((24.0 * scale).ceil() as u32)
            .max(1),
    );
    layout.toolbar.x = (rect.x + rect.width as i32 - layout.toolbar.width as i32)
        .clamp(0, screen.width.saturating_sub(layout.toolbar.width) as i32);
    layout.panel = layout.toolbar;
    layout
}

pub fn record_ready(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let id = session_id(window)?;
    let (screen, scale, rect, foreground) = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, id)?;
        session.recording.as_ref().ok_or("录屏尚未开始")?;
        (
            session.meta.screen,
            session.meta.scale,
            session.selection.ok_or("录屏选区不可用")?,
            session.foreground,
        )
    };
    let result = (|| {
        // The renderer has painted the recording controls before this changes the native clip.
        platform::prepare_scroll_frame(app, window)?;
        platform::record_surface(
            window,
            screen,
            &recording_layout(screen, scale, rect),
            scale,
        )?;
        platform::restore_foreground(foreground);
        platform::flush();
        Ok(())
    })();
    if result.is_err() {
        let (recording, escape) = {
            let mut slot = state.session.lock();
            let session = ensure_session(&mut slot, id)?;
            (session.recording.take(), session.scroll_escape.take())
        };
        drop(recording);
        drop(escape);
        let _ = platform::scroll_surface(window, screen, None);
        let _ = platform::exclude_from_capture(window, false);
        let _ = super::show(app, window);
    }
    result
}

pub fn record_start(
    app: &AppHandle,
    window: &WebviewWindow,
    options: RecordingOptions,
) -> Result<RecordingProgress, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, session_id(window)?)?;
    session
        .recording
        .as_mut()
        .ok_or("录屏尚未准备")?
        .start(options)
}

pub fn record_status(app: &AppHandle, window: &WebviewWindow) -> Result<RecordingProgress, String> {
    let state = manager(app);
    let mut slot = state.session.lock();
    let session = ensure_session(&mut slot, session_id(window)?)?;
    let mut progress = session.recording.as_ref().ok_or("录屏尚未开始")?.progress();
    progress.surface = session.recording_surface.clone();
    Ok(progress)
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
    if !recording.progress().started {
        return Err("请先开始录制".into());
    }
    recording.stop()?;
    let mut progress = recording.progress();
    progress.surface = session.recording_surface.clone();
    Ok(progress)
}
pub fn record_save(app: &AppHandle, window: &WebviewWindow) -> Result<bool, String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let source = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, session_id(window)?)?;
        let recording = session.recording.as_mut().ok_or("录屏尚未开始")?;
        if !recording.progress().started {
            return Err("请先开始录制".into());
        }
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

pub(super) fn interrupt(app: &AppHandle, id: &str, message: &str) -> Result<(), String> {
    let state = manager(app);
    let _operation = state.operation.lock();
    let retained = {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, id)?;
        if let Some(recording) = session
            .recording
            .as_mut()
            .filter(|recording| recording.progress().started)
        {
            recording.interrupt(message);
            true
        } else {
            false
        }
    };
    if !retained {
        return super::finish(app, true);
    }
    let window = app
        .get_webview_window(&format!("{}{id}", super::CAPTURE_PREFIX))
        .ok_or("录屏窗口不可用，视频仍已保留")?;
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or("暂无可用显示器，视频仍已保留")?;
    let screen = CaptureRect {
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
    };
    let scale = monitor.scale_factor();
    let layout = recovery_layout(screen, scale);
    platform::prepare_scroll_frame(app, &window)?;
    window
        .set_position(PhysicalPosition::new(screen.x, screen.y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(PhysicalSize::new(screen.width, screen.height))
        .map_err(|e| e.to_string())?;
    platform::record_surface(&window, screen, &layout, scale)?;
    {
        let mut slot = state.session.lock();
        let session = ensure_session(&mut slot, id)?;
        session.meta.screen = screen;
        session.meta.scale = scale;
        session.recording_surface = Some(RecordingSurface { screen, layout });
    }
    window.show().map_err(|e| e.to_string())
}

fn recovery_layout(screen: CaptureRect, scale: f64) -> ScrollLayout {
    let mut layout = recording_layout(screen, scale, CaptureRect::default());
    layout.toolbar.x = (screen.width.saturating_sub(layout.toolbar.width) / 2) as i32;
    layout.toolbar.y = ((12.0 * scale).round() as u32)
        .min(screen.height.saturating_sub(layout.toolbar.height)) as i32;
    layout.panel = layout.toolbar;
    layout
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_controls_fit_a_remaining_display_without_the_old_selection() {
        for (width, height, scale) in [(1920, 1080, 1.0), (800, 600, 2.0), (1080, 1920, 1.5)] {
            let screen = CaptureRect {
                x: -1920,
                y: -100,
                width,
                height,
            };
            let layout = recovery_layout(screen, scale);
            assert!(layout.toolbar.valid_within(width, height));
            assert_eq!(layout.selection, CaptureRect::default());
            assert_eq!(layout.toolbar, layout.panel);
        }
    }
}
