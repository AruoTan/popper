//! Native frame policy for the scrolling capture window.
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

const FRAME_SUBCLASS_ID: usize = 0x50504350;

fn excluded_styles(index: windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX) -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, WS_CAPTION, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME,
        WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
    };
    if index == GWL_STYLE {
        (WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX).0
    } else if index == GWL_EXSTYLE {
        (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME).0
    } else {
        0
    }
}

unsafe extern "system" fn frame_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    use windows::Win32::UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass},
        WindowsAndMessaging::{
            STYLESTRUCT, WINDOW_LONG_PTR_INDEX, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCDESTROY,
            WM_NCPAINT, WM_STYLECHANGING,
        },
    };
    match message {
        // Both NCCALCSIZE forms must leave the whole window as client space.
        WM_NCCALCSIZE | WM_NCPAINT => LRESULT(0),
        // Preserve framework activation/focus bookkeeping without native caption paint.
        WM_NCACTIVATE => DefSubclassProc(hwnd, message, wparam, LPARAM(-1)),
        WM_STYLECHANGING => {
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            if lparam.0 != 0 {
                let style = &mut *(lparam.0 as *mut STYLESTRUCT);
                style.styleNew &= !excluded_styles(WINDOW_LONG_PTR_INDEX(wparam.0 as i32));
            }
            result
        }
        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(hwnd, Some(frame_proc), FRAME_SUBCLASS_ID);
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        _ => DefSubclassProc(hwnd, message, wparam, lparam),
    }
}

/// Must run on the thread that owns this HWND. Removed on native window destruction.
pub(super) fn install(hwnd: HWND) -> Result<(), String> {
    use windows::Win32::{
        Graphics::Dwm::{DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY},
        System::Threading::GetCurrentThreadId,
        UI::{Shell::SetWindowSubclass, WindowsAndMessaging::GetWindowThreadProcessId},
    };
    unsafe {
        if GetWindowThreadProcessId(hwnd, None) != GetCurrentThreadId() {
            return Err("截图窗口边框操作必须在所属线程执行".into());
        }
        if !SetWindowSubclass(hwnd, Some(frame_proc), FRAME_SUBCLASS_ID, 0).as_bool() {
            return Err("无法设置截图窗口边框处理".into());
        }
        // Message handling also covers systems where DWM declines this attribute.
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &DWMNCRP_DISABLED as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&DWMNCRP_DISABLED) as u32,
        );
    }
    remove_capture_frame(hwnd)
}

pub(super) fn remove_capture_frame(hwnd: HWND) -> Result<(), String> {
    use windows::Win32::{
        Foundation::{GetLastError, SetLastError, WIN32_ERROR},
        UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            WINDOW_LONG_PTR_INDEX,
        },
    };
    fn strip(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> Result<bool, String> {
        unsafe {
            SetLastError(WIN32_ERROR(0));
            let current = GetWindowLongPtrW(hwnd, index) as u32;
            if current == 0 {
                let error = GetLastError();
                if error.0 != 0 {
                    return Err(std::io::Error::from_raw_os_error(error.0 as i32).to_string());
                }
            }
            let next = current & !excluded_styles(index);
            if next == current {
                return Ok(false);
            }
            SetLastError(WIN32_ERROR(0));
            if SetWindowLongPtrW(hwnd, index, next as isize) == 0 {
                let error = GetLastError();
                if error.0 != 0 {
                    return Err(std::io::Error::from_raw_os_error(error.0 as i32).to_string());
                }
            }
            Ok(true)
        }
    }
    // An undecorated WebView can retain native caption styles. Region and
    // activation redraws must not reveal a title bar over the live surface.
    let style_changed = strip(hwnd, GWL_STYLE)?;
    let edges_changed = strip(hwnd, GWL_EXSTYLE)?;
    if style_changed || edges_changed {
        unsafe {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{install, remove_capture_frame};
    use windows::{
        core::w,
        Win32::{
            Foundation::{HWND, LPARAM, POINT, RECT, WPARAM},
            Graphics::Gdi::{ClientToScreen, CreateRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ},
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetClientRect, GetWindowLongPtrW, GetWindowRect,
                SendMessageW, SetWindowLongPtrW, GWL_EXSTYLE, GWL_STYLE, NCCALCSIZE_PARAMS,
                WM_NCACTIVATE, WM_NCCALCSIZE, WS_CAPTION, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
                WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
                WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_SYSMENU,
                WS_THICKFRAME, WS_VISIBLE,
            },
        },
    };

    struct TestWindow(HWND);
    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    fn window() -> TestWindow {
        TestWindow(unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("STATIC"),
                w!("Popper capture frame test"),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                100,
                100,
                640,
                480,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        })
    }
    fn rect(hwnd: HWND) -> RECT {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }.unwrap();
        rect
    }
    fn assert_borderless(hwnd: HWND, expected: RECT) {
        let actual = rect(hwnd);
        assert_eq!(
            (actual.left, actual.top, actual.right, actual.bottom),
            (expected.left, expected.top, expected.right, expected.bottom)
        );
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        assert_eq!(
            style
                & (WS_CAPTION
                    | WS_SYSMENU
                    | WS_THICKFRAME
                    | WS_MINIMIZEBOX
                    | WS_MAXIMIZEBOX
                    | WS_VISIBLE)
                    .0,
            0
        );
        assert_eq!(
            style & (WS_CLIPCHILDREN | WS_CLIPSIBLINGS).0,
            (WS_CLIPCHILDREN | WS_CLIPSIBLINGS).0
        );
        let extended = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        assert_eq!(
            extended & (WS_EX_TOOLWINDOW | WS_EX_TOPMOST).0,
            (WS_EX_TOOLWINDOW | WS_EX_TOPMOST).0
        );
        let mut client = RECT::default();
        unsafe { GetClientRect(hwnd, &mut client) }.unwrap();
        assert_eq!(
            (client.left, client.top, client.right, client.bottom),
            (
                0,
                0,
                expected.right - expected.left,
                expected.bottom - expected.top
            )
        );
        let mut origin = POINT::default();
        assert!(unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool());
        assert_eq!((origin.x, origin.y), (expected.left, expected.top));
    }
    #[test]
    fn region_and_activation_changes_keep_the_capture_frame_borderless() {
        let window = window();
        let expected = rect(window.0);
        install(window.0).unwrap();
        assert_borderless(window.0, expected);
        let region = unsafe { CreateRectRgn(0, 0, 640, 32) };
        assert!(!region.0.is_null());
        if unsafe { SetWindowRgn(window.0, Some(region), true) } == 0 {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(region.0));
            }
            panic!("unable to set capture test region");
        }
        for active in [0, 1, 0] {
            unsafe {
                SendMessageW(
                    window.0,
                    WM_NCACTIVATE,
                    Some(WPARAM(active)),
                    Some(LPARAM(0)),
                );
            }
            assert_borderless(window.0, expected);
        }
        assert_ne!(unsafe { SetWindowRgn(window.0, None, true) }, 0);
        remove_capture_frame(window.0).unwrap();
        assert_borderless(window.0, expected);
    }
    #[test]
    fn framework_style_updates_cannot_restore_the_caption_or_borders() {
        let window = window();
        let expected = rect(window.0);
        install(window.0).unwrap();
        unsafe {
            let style = GetWindowLongPtrW(window.0, GWL_STYLE) as u32;
            SetWindowLongPtrW(
                window.0,
                GWL_STYLE,
                (style | WS_OVERLAPPEDWINDOW.0) as isize,
            );
            let extended = GetWindowLongPtrW(window.0, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(
                window.0,
                GWL_EXSTYLE,
                (extended | (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME).0) as isize,
            );
        }
        assert_borderless(window.0, expected);
        let extended = unsafe { GetWindowLongPtrW(window.0, GWL_EXSTYLE) } as u32;
        assert_eq!(
            extended & (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME).0,
            0
        );
        let mut single = expected;
        unsafe {
            SendMessageW(
                window.0,
                WM_NCCALCSIZE,
                Some(WPARAM(0)),
                Some(LPARAM(&mut single as *mut _ as isize)),
            );
        }
        assert_eq!(
            (single.left, single.top, single.right, single.bottom),
            (expected.left, expected.top, expected.right, expected.bottom)
        );
        let mut full = NCCALCSIZE_PARAMS::default();
        full.rgrc[0] = expected;
        unsafe {
            SendMessageW(
                window.0,
                WM_NCCALCSIZE,
                Some(WPARAM(1)),
                Some(LPARAM(&mut full as *mut _ as isize)),
            );
        }
        assert_eq!(
            (
                full.rgrc[0].left,
                full.rgrc[0].top,
                full.rgrc[0].right,
                full.rgrc[0].bottom
            ),
            (expected.left, expected.top, expected.right, expected.bottom)
        );
    }
    #[test]
    fn activation_is_forwarded_without_caption_repaint() {
        use windows::Win32::Foundation::LRESULT;
        use windows::Win32::UI::{
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{WM_APP, WM_NCPAINT},
        };
        #[derive(Default)]
        struct Events {
            active: usize,
            activation_lparam: isize,
            paints: usize,
            custom: usize,
        }
        unsafe extern "system" fn observe(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
            _: usize,
            data: usize,
        ) -> LRESULT {
            let events = &mut *(data as *mut Events);
            match message {
                WM_NCACTIVATE => {
                    events.active = wparam.0;
                    events.activation_lparam = lparam.0;
                }
                WM_NCPAINT => events.paints += 1,
                message if message == WM_APP + 35 => events.custom += 1,
                _ => {}
            }
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        // The window is destroyed before the observer's data is released.
        let mut events = Box::<Events>::default();
        let window = window();
        assert!(unsafe {
            SetWindowSubclass(
                window.0,
                Some(observe),
                1,
                events.as_mut() as *mut Events as usize,
            )
        }
        .as_bool());
        install(window.0).unwrap();
        unsafe {
            SendMessageW(window.0, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
            SendMessageW(window.0, WM_NCPAINT, Some(WPARAM(1)), Some(LPARAM(0)));
            SendMessageW(window.0, WM_APP + 35, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        assert_eq!(events.active, 1);
        assert_eq!(events.activation_lparam, -1);
        assert_eq!(events.paints, 0);
        assert_eq!(events.custom, 1);
    }
    #[test]
    fn frame_policy_cannot_be_installed_from_a_worker_thread() {
        let window = window();
        let hwnd = window.0 .0 as usize;
        assert!(std::thread::spawn(move || install(HWND(hwnd as *mut _)))
            .join()
            .unwrap()
            .is_err());
    }
    #[test]
    fn invalid_capture_window_reports_a_style_error() {
        assert!(remove_capture_frame(HWND::default()).is_err());
    }
}
