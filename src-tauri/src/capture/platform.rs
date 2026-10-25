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
                EnumChildWindows, EnumWindows, GetForegroundWindow, GetPhysicalCursorPos,
                GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
                IsWindowVisible, SetForegroundWindow, SetPhysicalCursorPos,
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
pub fn focus(window: &tauri::WebviewWindow) -> Result<(), String> {
    window.set_focus().map_err(|e| e.to_string())?;
    // Activating the frame alone does not direct keyboard input to WebView2.
    window.as_ref().set_focus().map_err(|e| e.to_string())
}
pub fn restore_foreground(handle: usize) {
    let handle = HWND(handle as *mut _);
    unsafe {
        if IsWindow(Some(handle)).as_bool() {
            let _ = SetForegroundWindow(handle);
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
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == std::process::id() {
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
/// Run hide/restore as a single event-loop operation before synchronizing the compositor.
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
    on_ui(app, move |_| {
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
    })?
}
