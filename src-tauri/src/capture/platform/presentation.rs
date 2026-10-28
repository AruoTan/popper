//! Native presentation of the frozen capture surface without changing its geometry.
use windows::Win32::{
    Foundation::HWND,
    System::{
        Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        Threading::GetCurrentThreadId,
    },
    UI::{
        Shell::{ITaskbarList2, TaskbarList},
        WindowsAndMessaging::{
            GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, IsChild,
            SetForegroundWindow, SetWindowPos, GA_ROOT, HWND_TOPMOST, SWP_NOMOVE,
            SWP_NOOWNERZORDER, SWP_NOSIZE,
        },
    },
};

fn check_thread(hwnd: HWND) -> Result<(), String> {
    if unsafe { GetWindowThreadProcessId(hwnd, None) } != unsafe { GetCurrentThreadId() } {
        return Err("截图窗口显示操作必须在所属线程执行".into());
    }
    Ok(())
}

pub(super) fn mark_fullscreen(hwnd: HWND, fullscreen: bool) -> Result<(), String> {
    check_thread(hwnd)?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| format!("无法初始化截图窗口显示：{e}"))?;
        let _apartment = Apartment;
        let taskbar: ITaskbarList2 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| e.to_string())?;
        taskbar.HrInit().map_err(|e| e.to_string())?;
        // This changes shell stacking policy, not display mode, bounds or DPI.
        taskbar
            .MarkFullscreenWindow(hwnd, fullscreen)
            .map_err(|e| e.to_string())
    }
}

pub(super) fn raise(hwnd: HWND) -> Result<bool, String> {
    check_thread(hwnd)?;
    unsafe {
        // The snapshot already includes transient shell UI. Activating the frozen
        // surface now dismisses that live UI without losing its captured pixels.
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOOWNERZORDER,
        )
        .map_err(|e| e.to_string())?;
        // The shell can reject a request while dismissing transient UI. Report
        // the observed foreground state and allow the caller to yield and retry.
        let _ = SetForegroundWindow(hwnd);
        let foreground = GetForegroundWindow();
        Ok(!foreground.0.is_null()
            && (foreground == hwnd
                || IsChild(hwnd, foreground).as_bool()
                || GetAncestor(foreground, GA_ROOT) == hwnd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        core::w,
        Win32::{
            Foundation::RECT,
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetWindowLongPtrW, GetWindowRect, IsWindowVisible,
                GWL_EXSTYLE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
            },
        },
    };

    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    fn window(x: i32, topmost: bool) -> Window {
        Window(unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW
                    | if topmost {
                        WS_EX_TOPMOST
                    } else {
                        Default::default()
                    },
                w!("STATIC"),
                w!("Popper capture presentation test"),
                WS_POPUP | WS_VISIBLE,
                x,
                100,
                160,
                120,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        })
    }
    fn rect(hwnd: HWND) -> (i32, i32, i32, i32) {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }.unwrap();
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop with foreground activation permission"]
    fn presenting_capture_preserves_other_windows_and_all_geometry() {
        let previous = unsafe { GetForegroundWindow() };
        let settings = window(100, false);
        let pin = window(300, true);
        let overlay = window(500, true);
        let before = [rect(settings.0), rect(pin.0), rect(overlay.0)];
        let targets = super::super::windows();
        for window in [&settings, &pin] {
            assert!(targets
                .iter()
                .any(|target| target.handle == window.0 .0 as usize));
        }
        // Simulate an existing topmost window being above the newly created surface.
        assert!(raise(pin.0).unwrap());
        assert!(raise(overlay.0).unwrap());
        assert_eq!(unsafe { GetForegroundWindow() }, overlay.0);
        assert_eq!(before, [rect(settings.0), rect(pin.0), rect(overlay.0)]);
        for window in [&settings, &pin, &overlay] {
            assert!(unsafe { IsWindowVisible(window.0) }.as_bool());
        }
        assert_ne!(
            unsafe { GetWindowLongPtrW(overlay.0, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0,
            0
        );
        let targets = super::super::windows();
        let index = |hwnd: HWND| {
            targets
                .iter()
                .position(|target| target.handle == hwnd.0 as usize)
                .unwrap()
        };
        assert!(index(overlay.0) < index(pin.0));
        unsafe {
            let _ = SetForegroundWindow(previous);
        }
    }

    #[test]
    fn presentation_rejects_destroyed_windows_and_the_wrong_thread() {
        assert!(raise(HWND::default()).is_err());
        assert!(mark_fullscreen(HWND::default(), true).is_err());
        let window = window(100, false);
        let handle = window.0 .0 as usize;
        std::thread::spawn(move || {
            let hwnd = HWND(handle as *mut _);
            assert!(raise(hwnd).is_err());
            assert!(mark_fullscreen(hwnd, true).is_err());
        })
        .join()
        .unwrap();
    }
}
