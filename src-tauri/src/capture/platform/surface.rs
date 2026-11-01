//! Native clipping: scrolling keeps its backdrop; recording exposes the desktop.
use super::frame;
use crate::capture::{CaptureRect, ScrollLayout};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ, HRGN, RGN_DIFF, RGN_OR,
};

struct Region(HRGN);
impl Drop for Region {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0 .0));
        }
    }
}
fn region(rect: CaptureRect) -> Result<Region, String> {
    let handle = unsafe {
        CreateRectRgn(
            rect.x,
            rect.y,
            rect.x + rect.width as i32,
            rect.y + rect.height as i32,
        )
    };
    if handle.0.is_null() {
        Err("无法创建屏幕捕捉边界".into())
    } else {
        Ok(Region(handle))
    }
}
fn surface_region(
    screen: CaptureRect,
    layout: &ScrollLayout,
    border: Option<u32>,
) -> Result<Region, String> {
    let selection = layout.selection;
    let (bounds, hole) = if let Some(border) = border {
        (
            selection,
            CaptureRect {
                x: selection.x + border as i32,
                y: selection.y + border as i32,
                width: selection.width.saturating_sub(border * 2),
                height: selection.height.saturating_sub(border * 2),
            },
        )
    } else {
        (
            CaptureRect {
                x: 0,
                y: 0,
                width: screen.width,
                height: screen.height,
            },
            selection,
        )
    };
    let outer = region(bounds)?;
    let hole = region(hole)?;
    let panel = region(layout.panel)?;
    let toolbar = region(layout.toolbar)?;
    unsafe {
        if CombineRgn(Some(outer.0), Some(outer.0), Some(hole.0), RGN_DIFF).0 == 0
            || CombineRgn(Some(outer.0), Some(outer.0), Some(panel.0), RGN_OR).0 == 0
            || CombineRgn(Some(outer.0), Some(outer.0), Some(toolbar.0), RGN_OR).0 == 0
        {
            return Err("无法创建屏幕捕捉区域".into());
        }
    }
    Ok(outer)
}
pub fn scroll_surface(
    window: &tauri::WebviewWindow,
    screen: CaptureRect,
    layout: Option<&ScrollLayout>,
) -> Result<(), String> {
    live_surface(window, screen, layout, None)
}
pub fn record_surface(
    window: &tauri::WebviewWindow,
    screen: CaptureRect,
    layout: &ScrollLayout,
    scale: f64,
) -> Result<(), String> {
    live_surface(
        window,
        screen,
        Some(layout),
        Some((2.0 * scale).ceil().max(1.0) as u32),
    )
}
fn live_surface(
    window: &tauri::WebviewWindow,
    screen: CaptureRect,
    layout: Option<&ScrollLayout>,
    border: Option<u32>,
) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    frame::remove_capture_frame(hwnd)?;
    let Some(layout) = layout else {
        return if unsafe { SetWindowRgn(hwnd, None, true) } == 0 {
            Err("无法恢复截图编辑窗口".into())
        } else {
            Ok(())
        };
    };
    let region = surface_region(screen, layout, border)?;
    if unsafe { SetWindowRgn(hwnd, Some(region.0), true) } == 0 {
        return Err("无法启用实时捕捉区域".into());
    }
    // SetWindowRgn takes ownership only on success.
    std::mem::forget(region);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::PtInRegion;
    #[test]
    fn recording_exposes_both_sides_of_the_border_and_keeps_controls_interactive() {
        let screen = CaptureRect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let selection = CaptureRect {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        };
        let toolbar = CaptureRect {
            x: 500,
            y: 720,
            width: 400,
            height: 48,
        };
        let layout = ScrollLayout {
            selection,
            toolbar,
            panel: toolbar,
        };
        for border in [2, 3, 4] {
            let region = surface_region(screen, &layout, Some(border)).unwrap();
            unsafe {
                assert!(!PtInRegion(region.0, 50, 50).as_bool());
                assert!(!PtInRegion(region.0, 200, 200).as_bool());
                assert!(!PtInRegion(region.0, 100 + border as i32, 200).as_bool());
                assert!(PtInRegion(region.0, 100, 200).as_bool());
                assert!(PtInRegion(region.0, 600, 740).as_bool());
            }
        }
        let scrolling = surface_region(screen, &layout, None).unwrap();
        assert!(unsafe { PtInRegion(scrolling.0, 50, 50) }.as_bool());
    }
    #[test]
    fn full_screen_recording_keeps_a_visible_inner_border() {
        let screen = CaptureRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let toolbar = CaptureRect {
            x: 1500,
            y: 1000,
            width: 400,
            height: 48,
        };
        let layout = ScrollLayout {
            selection: screen,
            panel: toolbar,
            toolbar,
        };
        let region = surface_region(screen, &layout, Some(2)).unwrap();
        unsafe {
            assert!(PtInRegion(region.0, 0, 300).as_bool());
            assert!(PtInRegion(region.0, 1919, 300).as_bool());
            assert!(!PtInRegion(region.0, 960, 540).as_bool());
        }
    }
}
