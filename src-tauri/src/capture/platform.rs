#[path = "platform/frame.rs"]
mod frame;
#[path = "platform/presentation.rs"]
mod presentation;
use super::{CaptureCandidate, CapturePoint, CaptureRect};
use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use tauri::AppHandle;
use windows::{
    core::{BOOL, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, POINT, RECT},
        Graphics::Dwm::{
            DwmFlush, DwmGetWindowAttribute, DwmSetWindowAttribute, DWMSBT_NONE,
            DWMWA_BORDER_COLOR, DWMWA_CLOAKED, DWMWA_COLOR_NONE, DWMWA_EXTENDED_FRAME_BOUNDS,
            DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_TRANSITIONS_FORCEDISABLED,
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        },
        UI::{
            Controls::Dialogs::{
                CommDlgExtendedError, GetSaveFileNameW, OFN_EXPLORER, OFN_NOCHANGEDIR,
                OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
            },
            WindowsAndMessaging::{
                EnumChildWindows, EnumWindows, GetAncestor, GetForegroundWindow,
                GetPhysicalCursorPos, GetWindowRect, GetWindowTextW, IsChild, IsIconic,
                IsWindow, IsWindowVisible, SetForegroundWindow, SetPhysicalCursorPos, GA_ROOT,
                GA_ROOTOWNER,
            },
        },
    },
};

#[derive(Clone, Serialize, Deserialize)]
pub struct TargetWindow {
    pub handle: usize,
    pub rect: CaptureRect,
    pub title: String,
    pub controls: Vec<CaptureCandidate>,
}
pub fn cursor() -> Result<(i32, i32), String> {
    let mut point = POINT::default();
    unsafe { GetPhysicalCursorPos(&mut point) }.map_err(|e| e.to_string())?;
    Ok((point.x, point.y))
}
pub fn pin_toolbar_geometry(
    window: &tauri::WebviewWindow,
    meta: &super::PinnedImageState,
) -> Result<(CaptureRect, CaptureRect, f64), String> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let mut frame = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut frame) }.map_err(|e| e.to_string())?;
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) }
        .as_bool()
    {
        return Err("无法读取贴图所在屏幕".into());
    }
    let width = (frame.right - frame.left).max(1) as f64;
    let height = (frame.bottom - frame.top).max(1) as f64;
    let inset_x = (width * meta.shadow_inset as f64
        / ((meta.width as f64 * meta.zoom).round().max(1.0) + 2.0 * meta.shadow_inset as f64))
        .round();
    let inset_y = (height * meta.shadow_inset as f64
        / ((meta.height as f64 * meta.zoom).round().max(1.0) + 2.0 * meta.shadow_inset as f64))
        .round();
    Ok((
        CaptureRect {
            x: frame.left + inset_x as i32,
            y: frame.top + inset_y as i32,
            width: (width - 2.0 * inset_x).max(1.0) as u32,
            height: (height - 2.0 * inset_y).max(1.0) as u32,
        },
        CaptureRect {
            x: info.rcWork.left,
            y: info.rcWork.top,
            width: (info.rcWork.right - info.rcWork.left).max(1) as u32,
            height: (info.rcWork.bottom - info.rcWork.top).max(1) as u32,
        },
        window.scale_factor().map_err(|e| e.to_string())?,
    ))
}
pub fn position_pin_toolbar(
    window: &tauri::WebviewWindow,
    rect: CaptureRect,
) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let mut current = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut current) }.map_err(|e| e.to_string())?;
    if current.left == rect.x
        && current.top == rect.y
        && (current.right - current.left) as u32 == rect.width
        && (current.bottom - current.top) as u32 == rect.height
    {
        return Ok(());
    }
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            rect.x,
            rect.y,
            rect.width as i32,
            rect.height as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    }
    .map_err(|e| e.to_string())
}
pub fn drag_pin(
    window: &tauri::WebviewWindow,
    anchor: Option<CapturePoint>,
) -> Result<Option<CapturePoint>, String> {
    use windows::Win32::UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
        WindowsAndMessaging::{
            GetSystemMetrics, SetWindowPos, SM_SWAPBUTTON, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
        },
    };
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let primary = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } == 0 {
        VK_LBUTTON
    } else {
        VK_RBUTTON
    };
    // A queued renderer move must not reposition a released or hidden image.
    if !unsafe { IsWindowVisible(hwnd) }.as_bool()
        || unsafe { GetAsyncKeyState(primary.0 as i32) } >= 0
    {
        return Ok(None);
    }
    let (x, y) = cursor()?;
    if let Some(anchor) = anchor {
        // Bypass the caption move loop: it constrains windows at the work-area top.
        unsafe {
            SetWindowPos(
                hwnd,
                None,
                x.saturating_sub(anchor.x),
                y.saturating_sub(anchor.y),
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
        .map_err(|e| e.to_string())?;
        Ok(Some(anchor))
    } else {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|e| e.to_string())?;
        Ok(Some(CapturePoint {
            x: x.saturating_sub(rect.left),
            y: y.saturating_sub(rect.top),
        }))
    }
}
pub fn move_pointer(screen: CaptureRect, dx: i32, dy: i32) -> Result<CapturePoint, String> {
    let (x, y) = cursor()?;
    let point = CapturePoint { x, y }.step_within(screen, dx, dy)?;
    unsafe { SetPhysicalCursorPos(point.x, point.y) }.map_err(|e| e.to_string())?;
    let (x, y) = cursor()?;
    Ok(CapturePoint {
        x: (i64::from(x) - i64::from(screen.x)).clamp(0, i64::from(screen.width) - 1) as i32,
        y: (i64::from(y) - i64::from(screen.y)).clamp(0, i64::from(screen.height) - 1) as i32,
    })
}
pub fn foreground() -> usize {
    unsafe { GetForegroundWindow().0 as usize }
}
pub fn capture_region(screen: CaptureRect, rect: CaptureRect) -> Result<image::RgbaImage, String> {
    if !rect.valid_within(screen.width, screen.height) {
        return Err("滚动选区超出屏幕范围".into());
    }
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONULL,
    };
    let point = POINT {
        x: screen.x + (screen.width / 2) as i32,
        y: screen.y + (screen.height / 2) as i32,
    };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // Validate physical bounds in one query. Repeated display-mode queries here
    // duplicate the checks performed by the image capture path on every frame.
    let handle = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONULL) };
    if !unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool()
        || info.rcMonitor.left != screen.x
        || info.rcMonitor.top != screen.y
        || i64::from(info.rcMonitor.right) - i64::from(info.rcMonitor.left)
            != i64::from(screen.width)
        || i64::from(info.rcMonitor.bottom) - i64::from(info.rcMonitor.top)
            != i64::from(screen.height)
    {
        return Err("显示器布局已变化，请重新截图".into());
    }
    let monitor = xcap::Monitor::from_point(point.x, point.y).map_err(|e| e.to_string())?;
    monitor
        .capture_region(rect.x as u32, rect.y as u32, rect.width, rect.height)
        .map_err(|e| e.to_string())
}
pub fn exclude_from_capture(window: &tauri::WebviewWindow, exclude: bool) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
    };
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let affinity = if exclude {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    unsafe { SetWindowDisplayAffinity(hwnd, affinity) }
        .map_err(|_| "当前系统无法排除滚动预览，请使用 Windows 10 2004 或更新版本".to_owned())?;
    let mut actual = 0;
    unsafe { GetWindowDisplayAffinity(hwnd, &mut actual) }.map_err(|e| e.to_string())?;
    if actual != affinity.0 {
        return Err("无法将滚动预览排除在截图之外".into());
    }
    Ok(())
}
pub fn prepare_scroll_frame(app: &AppHandle, window: &tauri::WebviewWindow) -> Result<(), String> {
    let handle = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    on_ui(app, move |_| {
        let hwnd = HWND(handle as *mut _);
        frame::install(hwnd)?;
        presentation::mark_fullscreen(hwnd, false)
    })?
}
pub fn scroll_surface(
    window: &tauri::WebviewWindow,
    screen: CaptureRect,
    layout: Option<&super::ScrollLayout>,
) -> Result<(), String> {
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
            Err("无法创建滚动选区边界".into())
        } else {
            Ok(Region(handle))
        }
    }
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    frame::remove_capture_frame(hwnd)?;
    let Some(layout) = layout else {
        return if unsafe { SetWindowRgn(hwnd, None, true) } == 0 {
            Err("无法恢复截图编辑窗口".into())
        } else {
            Ok(())
        };
    };
    let outer = region(CaptureRect {
        x: 0,
        y: 0,
        width: screen.width,
        height: screen.height,
    })?;
    let hole = region(layout.selection)?;
    let panel = region(layout.panel)?;
    let toolbar = region(layout.toolbar)?;
    // The native hole forwards input to the real page, including drag and wheel input.
    // Keep the preview interactive even when a full-screen selection leaves no free margin.
    unsafe {
        if CombineRgn(Some(outer.0), Some(outer.0), Some(hole.0), RGN_DIFF).0 == 0
            || CombineRgn(Some(outer.0), Some(outer.0), Some(panel.0), RGN_OR).0 == 0
            || CombineRgn(Some(outer.0), Some(outer.0), Some(toolbar.0), RGN_OR).0 == 0
            || SetWindowRgn(hwnd, Some(outer.0), true) == 0
        {
            return Err("无法启用滚动选区，请重新截图".into());
        }
    }
    // Successful SetWindowRgn transfers this handle to the operating system.
    std::mem::forget(outer);
    Ok(())
}
pub fn focus(window: &tauri::WebviewWindow) -> Result<(), String> {
    window.set_focus().map_err(|e| e.to_string())?;
    // Activating the frame alone does not direct keyboard input to WebView2.
    window.as_ref().set_focus().map_err(|e| e.to_string())
}
/// Called on the UI thread, after the renderer has painted the frozen image.
pub fn present_capture(window: &tauri::WebviewWindow) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    configure_capture_surface(hwnd);
    frame::install(hwnd)?;
    presentation::mark_fullscreen(hwnd, true)?;
    window.show().map_err(|e| e.to_string())?;
    focus(window)?;
    presentation::raise(hwnd)
}
pub fn is_foreground(window: &tauri::WebviewWindow) -> Result<bool, String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let foreground = unsafe { GetForegroundWindow() };
    // A null foreground occurs during activation; it does not confirm an external switch.
    Ok(foreground.0.is_null()
        || foreground == hwnd
        || unsafe { IsChild(hwnd, foreground).as_bool() }
        || unsafe { GetAncestor(foreground, GA_ROOT) } == hwnd
        || unsafe { GetAncestor(foreground, GA_ROOTOWNER) } == hwnd)
}
pub fn restore_foreground(handle: usize) {
    let handle = HWND(handle as *mut _);
    unsafe {
        if IsWindow(Some(handle)).as_bool() {
            let _ = SetForegroundWindow(handle);
        }
    }
}
pub fn dismiss_owned_dialog(owner: usize) {
    use windows::Win32::{
        Foundation::WPARAM,
        UI::{
            Input::KeyboardAndMouse::IsWindowEnabled,
            WindowsAndMessaging::{GetLastActivePopup, PostMessageW, WM_CLOSE},
        },
    };
    let owner = HWND(owner as *mut _);
    unsafe {
        if IsWindowEnabled(owner).as_bool() {
            return;
        }
        let dialog = GetLastActivePopup(owner);
        if !dialog.0.is_null() && dialog != owner {
            let _ = PostMessageW(Some(dialog), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}
pub fn configure_capture_surface(hwnd: HWND) {
    use std::ffi::c_void;

    unsafe fn set_attribute<T: Copy>(
        hwnd: HWND,
        attribute: windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE,
        value: &T,
    ) {
        let _ = DwmSetWindowAttribute(
            hwnd,
            attribute,
            value as *const T as *const c_void,
            std::mem::size_of::<T>() as u32,
        );
    }

    unsafe {
        set_attribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &BOOL::from(true));
        set_attribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_DONOTROUND);
        set_attribute(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
        set_attribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_NONE);
    }
}
pub fn screen() -> Result<(xcap::Monitor, CaptureRect, f64), String> {
    let (x, y) = cursor()?;
    let monitor = xcap::Monitor::from_point(x, y).map_err(|e| e.to_string())?;
    let rect = CaptureRect {
        x: monitor.x().map_err(|e| e.to_string())?,
        y: monitor.y().map_err(|e| e.to_string())?,
        width: monitor.width().map_err(|e| e.to_string())?,
        height: monitor.height().map_err(|e| e.to_string())?,
    };
    let scale = monitor.scale_factor().map_err(|e| e.to_string())? as f64;
    if !scale.is_finite() || scale <= 0.0 {
        return Err("显示器缩放比例无效".into());
    }
    Ok((monitor, rect, scale))
}
pub fn windows() -> Vec<TargetWindow> {
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return BOOL(1);
            }
            let mut cloaked: u32 = 0;
            let _ = DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut _,
                size_of::<u32>() as u32,
            );
            if cloaked != 0 {
                return BOOL(1);
            }
            let mut rect = RECT::default();
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut rect as *mut _ as *mut _,
                size_of::<RECT>() as u32,
            )
            .is_err()
                && GetWindowRect(hwnd, &mut rect).is_err()
            {
                return BOOL(1);
            }
            if rect.right <= rect.left || rect.bottom <= rect.top {
                return BOOL(1);
            }
            let mut title = [0u16; 256];
            let count = GetWindowTextW(hwnd, &mut title).max(0) as usize;
            let targets = &mut *(data.0 as *mut Vec<TargetWindow>);
            targets.push(TargetWindow {
                handle: hwnd.0 as usize,
                rect: CaptureRect {
                    x: rect.left,
                    y: rect.top,
                    width: (rect.right - rect.left) as u32,
                    height: (rect.bottom - rect.top) as u32,
                },
                title: String::from_utf16_lossy(&title[..count]),
                controls: Vec::new(),
            });
        }
        BOOL(1)
    }
    let mut targets = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut targets as *mut _ as isize));
    }
    targets
}

/// Snapshot native children before the overlay exists. This path needs no UIA provider.
pub fn snapshot_controls(targets: &mut [TargetWindow], screen: CaptureRect) {
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        let controls = unsafe { &mut *(data.0 as *mut Vec<CaptureCandidate>) };
        if controls.len() >= 1024 {
            return BOOL(0);
        }
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return BOOL(1);
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err()
            || rect.right <= rect.left
            || rect.bottom <= rect.top
        {
            return BOOL(1);
        }
        controls.push(CaptureCandidate {
            rect: CaptureRect {
                x: rect.left,
                y: rect.top,
                width: (i64::from(rect.right) - i64::from(rect.left)) as u32,
                height: (i64::from(rect.bottom) - i64::from(rect.top)) as u32,
            },
            label: String::new(),
        });
        BOOL(1)
    }
    let mut remaining = 4096;
    for target in targets {
        if remaining == 0 {
            break;
        }
        if target.rect.intersect(screen).is_none() {
            continue;
        }
        unsafe {
            let _ = EnumChildWindows(
                Some(HWND(target.handle as *mut _)),
                Some(visit),
                LPARAM(&mut target.controls as *mut _ as isize),
            );
        }
        target.controls.truncate(remaining);
        remaining -= target.controls.len();
    }
}
/// Complete a window operation on the owning UI thread before returning.
pub fn on_ui<T: Send + 'static>(
    app: &AppHandle,
    action: impl FnOnce(AppHandle) -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = app.clone();
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancelled_on_ui = cancelled.clone();
    app.run_on_main_thread(move || {
        if !cancelled_on_ui.load(std::sync::atomic::Ordering::Acquire) {
            let _ = tx.send(action(handle));
        }
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|_| {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
            "窗口操作超时".into()
        })
}
pub fn flush() {
    unsafe {
        let _ = DwmFlush();
    }
}
pub fn same_screen(expected: CaptureRect, scale: f64) -> bool {
    xcap::Monitor::all().is_ok_and(|monitors| {
        monitors.iter().any(|m| {
            m.x().ok() == Some(expected.x)
                && m.y().ok() == Some(expected.y)
                && m.width().ok() == Some(expected.width)
                && m.height().ok() == Some(expected.height)
                && m.scale_factor()
                    .is_ok_and(|value| (value as f64 - scale).abs() < 0.001)
        })
    })
}
pub fn save_dialog(app: &AppHandle, owner: usize) -> Result<Option<std::path::PathBuf>, String> {
    let (started, ready) = mpsc::sync_channel(0);
    let (reply, result) = mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        // A timed-out caller drops the receiver, preventing a late modal from opening.
        if started.send(()).is_ok() {
            let _ = reply.send(native_save_dialog(owner));
        }
    })
    .map_err(|e| e.to_string())?;
    ready
        .recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|_| "保存对话框启动超时".to_owned())?;
    // Choosing a path is a user interaction, not a three-second window operation.
    result.recv().map_err(|_| "保存对话框已结束".to_owned())?
}
fn native_save_dialog(owner: usize) -> Result<Option<std::path::PathBuf>, String> {
    let mut filename = [0u16; 32768];
    let initial: Vec<u16> = "Popper-capture.png".encode_utf16().collect();
    filename[..initial.len()].copy_from_slice(&initial);
    let filter: Vec<u16> = "PNG 图片 (*.png)\0*.png\0\0".encode_utf16().collect();
    let extension: Vec<u16> = "png\0".encode_utf16().collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND(owner as *mut _),
        lpstrFile: windows::core::PWSTR(filename.as_mut_ptr()),
        nMaxFile: filename.len() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
        ..Default::default()
    };
    if unsafe { GetSaveFileNameW(&mut dialog) }.as_bool() {
        use std::os::windows::ffi::OsStringExt;
        let len = filename
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(filename.len());
        Ok(Some(std::path::PathBuf::from(
            std::ffi::OsString::from_wide(&filename[..len]),
        )))
    } else {
        let code = unsafe { CommDlgExtendedError() }.0;
        if code == 0 {
            Ok(None)
        } else {
            Err(format!("无法打开保存对话框：{code}"))
        }
    }
}
