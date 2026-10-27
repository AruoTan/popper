//! Enforce per-window desktop visibility without changing user-controlled show/hide state.
use super::{active_desktop, desktop_registry, DesktopRegistry};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::WebviewWindow;
use uuid::Uuid;
use windows::{
    core::BOOL,
    Win32::{
        Foundation::HWND,
        Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK},
        UI::WindowsAndMessaging::IsWindow,
    },
};

#[derive(Default)]
pub struct DesktopVisibility {
    windows: Mutex<HashMap<usize, Guard>>,
    stopped: AtomicBool,
}
struct Guard {
    target: Option<Uuid>,
    image_cloaked: Option<bool>,
    toolbar: Option<(usize, Option<bool>)>,
}
fn needs_cloak(target: Option<Uuid>, active: Option<Uuid>) -> bool {
    target.is_some() && target != active
}
fn cloak(hwnd: usize, value: bool) -> Result<(), String> {
    let value = BOOL::from(value);
    unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd as *mut _),
            DWMWA_CLOAK,
            (&value as *const BOOL).cast(),
            std::mem::size_of_val(&value) as u32,
        )
    }
    .map_err(|e| e.to_string())
}
fn apply(
    hwnd: usize,
    current: &mut Option<bool>,
    value: bool,
    set: &mut impl FnMut(usize, bool) -> Result<(), String>,
) -> Result<(), String> {
    if *current != Some(value) {
        set(hwnd, value)?;
        *current = Some(value);
    }
    Ok(())
}
impl Guard {
    fn sync(
        &mut self,
        image: usize,
        active: Option<Uuid>,
        mut valid: impl FnMut(usize) -> bool,
        mut set: impl FnMut(usize, bool) -> Result<(), String>,
    ) -> Result<bool, String> {
        if !valid(image) {
            return Ok(false);
        }
        let hidden = needs_cloak(self.target, active);
        apply(image, &mut self.image_cloaked, hidden, &mut set)?;
        if let Some((toolbar, current)) = &mut self.toolbar {
            if valid(*toolbar) {
                apply(*toolbar, current, hidden, &mut set)?;
            } else {
                self.toolbar = None;
            }
        }
        Ok(self.target.is_some())
    }
}

impl DesktopVisibility {
    pub fn start(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tauri::async_runtime::spawn_blocking(move || {
            loop {
                let Some(state) = weak.upgrade() else {
                    break;
                };
                if state.stopped.load(Ordering::Acquire) {
                    break;
                }
                if !state.windows.lock().is_empty() {
                    let _ = state.sync(&desktop_registry());
                }
                // No AppHandle or image data is retained by this worker.
                drop(state);
                std::thread::sleep(Duration::from_millis(50));
            }
        });
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.windows.lock().clear();
    }
    pub fn remove(&self, hwnd: usize) {
        self.windows.lock().remove(&hwnd);
    }
    pub fn assign(
        &self,
        window: &WebviewWindow,
        target: Option<Uuid>,
        toolbar: Option<&WebviewWindow>,
    ) -> Result<(), String> {
        let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
        let toolbar = toolbar
            .map(|window| window.hwnd().map(|hwnd| hwnd.0 as usize))
            .transpose()
            .map_err(|e| e.to_string())?;
        {
            let mut windows = self.windows.lock();
            let guard = windows.entry(hwnd).or_insert(Guard {
                target,
                image_cloaked: None,
                toolbar: None,
            });
            guard.target = target;
            if let Some(toolbar) = toolbar {
                guard.toolbar = Some((toolbar, None));
            }
        }
        self.sync(&desktop_registry())
    }
    pub fn attach_toolbar(
        &self,
        parent: &WebviewWindow,
        toolbar: &WebviewWindow,
    ) -> Result<(), String> {
        let parent = parent.hwnd().map_err(|e| e.to_string())?.0 as usize;
        let toolbar = toolbar.hwnd().map_err(|e| e.to_string())?.0 as usize;
        if let Some(guard) = self.windows.lock().get_mut(&parent) {
            guard.toolbar = Some((toolbar, None));
        }
        self.sync(&desktop_registry())
    }
    fn sync(&self, registry: &DesktopRegistry) -> Result<(), String> {
        let active = active_desktop(registry);
        let mut windows = self.windows.lock();
        let mut error = None;
        windows.retain(|hwnd, guard| {
            // Removing a desktop lets the Shell choose a surviving destination.
            if guard
                .target
                .is_some_and(|target| !registry.ids.is_empty() && !registry.ids.contains(&target))
            {
                if let Ok(Some(target)) = super::desktop_at(HWND(*hwnd as *mut _)) {
                    if registry.ids.contains(&target) {
                        guard.target = Some(target);
                    }
                }
            }
            match guard.sync(
                *hwnd,
                active,
                |hwnd| unsafe { IsWindow(Some(HWND(hwnd as *mut _))) }.as_bool(),
                cloak,
            ) {
                Ok(retain) => retain,
                Err(message) => {
                    error = Some(message);
                    true
                }
            }
        });
        error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::{needs_cloak, Guard};
    use uuid::Uuid;
    #[test]
    fn desktop_switches_only_uncloak_the_assigned_desktop() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        assert!(!needs_cloak(Some(first), Some(first)));
        assert!(needs_cloak(Some(first), Some(second)));
        assert!(needs_cloak(Some(first), None));
        assert!(!needs_cloak(Some(first), Some(first)));
    }
    #[test]
    fn all_desktops_releases_the_cloak_even_when_activity_is_unknown() {
        for active in [None, Some(Uuid::from_u128(1)), Some(Uuid::from_u128(2))] {
            assert!(!needs_cloak(None, active));
        }
    }
    fn guard() -> Guard {
        Guard {
            target: Some(Uuid::from_u128(1)),
            image_cloaked: None,
            toolbar: Some((20, None)),
        }
    }
    #[test]
    fn image_and_toolbar_follow_desktop_switches_without_repeated_native_writes() {
        let mut guard = guard();
        let mut changes = vec![];
        for active in [2, 2, 1, 1, 3] {
            assert!(guard
                .sync(
                    10,
                    Some(Uuid::from_u128(active)),
                    |_| true,
                    |hwnd, hidden| {
                        changes.push((hwnd, hidden));
                        Ok(())
                    }
                )
                .unwrap());
        }
        assert_eq!(
            changes,
            [
                (10, true),
                (20, true),
                (10, false),
                (20, false),
                (10, true),
                (20, true)
            ]
        );
    }
    #[test]
    fn returning_to_all_desktops_releases_image_and_toolbar_before_removing_guard() {
        let mut guard = guard();
        guard.image_cloaked = Some(true);
        guard.toolbar = Some((20, Some(true)));
        guard.target = None;
        let mut changes = vec![];
        assert!(!guard
            .sync(
                10,
                None,
                |_| true,
                |hwnd, hidden| {
                    changes.push((hwnd, hidden));
                    Ok(())
                }
            )
            .unwrap());
        assert_eq!(changes, [(10, false), (20, false)]);
    }
    #[test]
    fn failed_native_updates_are_retried_and_do_not_cache_success() {
        let mut guard = guard();
        assert!(guard
            .sync(
                10,
                None,
                |_| true,
                |hwnd, _| if hwnd == 20 {
                    Err("DWM unavailable".into())
                } else {
                    Ok(())
                }
            )
            .is_err());
        assert_eq!(guard.image_cloaked, Some(true));
        assert_eq!(guard.toolbar, Some((20, None)));
        let mut changes = vec![];
        assert!(guard
            .sync(
                10,
                None,
                |_| true,
                |hwnd, hidden| {
                    changes.push((hwnd, hidden));
                    Ok(())
                }
            )
            .unwrap());
        assert_eq!(changes, [(20, true)]);
    }
    #[test]
    fn destroyed_image_is_removed_and_destroyed_toolbar_is_not_touched() {
        let mut guard = guard();
        assert!(!guard
            .sync(10, None, |_| false, |_, _| panic!("invalid HWND"))
            .unwrap());
        assert!(guard
            .sync(10, None, |hwnd| hwnd == 10, |_, _| Ok(()))
            .unwrap());
        assert!(guard.toolbar.is_none());
    }
    #[test]
    fn a_later_toolbar_inherits_the_images_existing_desktop_constraint() {
        let mut guard = guard();
        guard.toolbar = None;
        guard.sync(10, None, |_| true, |_, _| Ok(())).unwrap();
        guard.toolbar = Some((30, None));
        let mut changes = vec![];
        guard
            .sync(
                10,
                None,
                |_| true,
                |hwnd, hidden| {
                    changes.push((hwnd, hidden));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(changes, [(30, true)]);
    }
}
