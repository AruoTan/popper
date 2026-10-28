//! Temporary system animation settings owned by a capture session.
#[cfg(any(target_os = "windows", test))]
#[path = "animations/recovery.rs"]
mod recovery;
#[cfg(target_os = "windows")]
pub use recovery::{recover_pending, run_if_requested};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Effect {
    ClientArea,
    Window,
    Menu,
    MenuFade,
}
const EFFECTS: [Effect; 4] = [
    Effect::ClientArea,
    Effect::Window,
    Effect::Menu,
    Effect::MenuFade,
];

pub(super) trait Settings {
    fn read(&self, effect: Effect) -> Result<i32, String>;
    fn write(&self, effect: Effect, value: i32) -> Result<(), String>;
    fn arm_recovery(&self, _original: &[(Effect, i32)]) -> Result<(), String> {
        Ok(())
    }
    fn clear_recovery(&self) -> Result<(), String> {
        Ok(())
    }
}

pub(super) struct Pause<S: Settings> {
    system: S,
    changed: Vec<(Effect, i32)>,
    recovery_armed: bool,
}
impl<S: Settings> Pause<S> {
    pub(super) fn suspend(system: S) -> Result<Self, String> {
        // Read every original value before modifying any system setting.
        let original = EFFECTS
            .into_iter()
            .map(|effect| system.read(effect).map(|value| (effect, value)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut pause = Self {
            system,
            changed: Vec::new(),
            recovery_armed: false,
        };
        let original: Vec<_> = original
            .into_iter()
            .filter(|(_, value)| *value != 0)
            .collect();
        if !original.is_empty() {
            // Persist recovery before the first system change. Drop also cleans
            // up partially registered recovery when registration fails.
            pause.recovery_armed = true;
            pause.system.arm_recovery(&original)?;
        }
        for (effect, value) in original {
            // A failed setter may still have changed the value. Include it in rollback.
            pause.changed.push((effect, value));
            pause.system.write(effect, 0)?;
            if pause.system.read(effect)? != 0 {
                return Err("无法临时关闭系统动画".into());
            }
        }
        Ok(pause)
    }

    pub(super) fn restore(&mut self) -> Result<(), String> {
        restore_effects(&self.system, &mut self.changed)?;
        if self.recovery_armed {
            self.system.clear_recovery()?;
            self.recovery_armed = false;
        }
        Ok(())
    }
}

fn restore_effects(system: &impl Settings, changed: &mut Vec<(Effect, i32)>) -> Result<(), String> {
    let mut errors = Vec::new();
    changed.retain(|&(effect, value)| {
        let restored = system.write(effect, value).and_then(|()| {
            if system.read(effect)? == value {
                Ok(())
            } else {
                Err("系统动画未恢复原值".into())
            }
        });
        match restored {
            Ok(()) => false,
            Err(error) => {
                errors.push(error);
                true
            }
        }
    });
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("无法恢复系统动画：{}", errors.join("；")))
    }
}
impl<S: Settings> Drop for Pause<S> {
    fn drop(&mut self) {
        // Covers initialization errors, cancellation, shutdown and Rust unwinding.
        // Retain failed entries so a transient restore error can be retried.
        for _ in 0..3 {
            if self.restore().is_ok() {
                return;
            }
        }
        eprintln!("[capture] stage=restore-animations error_class=system-settings");
    }
}

#[cfg(target_os = "windows")]
pub(super) struct WindowsSettings {
    // The exclusive file handle is released by Windows even after a crash.
    // Unlike an owned mutex, it can move between capture worker threads.
    _lease: std::fs::File,
}
#[cfg(target_os = "windows")]
pub(super) type AnimationPause = Pause<WindowsSettings>;

#[cfg(target_os = "windows")]
impl Settings for WindowsSettings {
    fn arm_recovery(&self, original: &[(Effect, i32)]) -> Result<(), String> {
        recovery::arm(original)
    }

    fn clear_recovery(&self) -> Result<(), String> {
        recovery::clear()
    }

    fn read(&self, effect: Effect) -> Result<i32, String> {
        use windows::{core::BOOL, Win32::UI::WindowsAndMessaging::*};
        unsafe {
            if effect == Effect::Window {
                let mut info = ANIMATIONINFO {
                    cbSize: std::mem::size_of::<ANIMATIONINFO>() as u32,
                    ..Default::default()
                };
                SystemParametersInfoW(
                    SPI_GETANIMATION,
                    info.cbSize,
                    Some(&mut info as *mut _ as *mut _),
                    Default::default(),
                )
                .map_err(|e| format!("无法读取系统窗口动画：{e}"))?;
                return Ok(info.iMinAnimate);
            }
            let action = match effect {
                Effect::ClientArea => SPI_GETCLIENTAREAANIMATION,
                Effect::Menu => SPI_GETMENUANIMATION,
                Effect::MenuFade => SPI_GETMENUFADE,
                Effect::Window => unreachable!(),
            };
            let mut value = BOOL(0);
            SystemParametersInfoW(
                action,
                0,
                Some(&mut value as *mut _ as *mut _),
                Default::default(),
            )
            .map_err(|e| format!("无法读取系统动画：{e}"))?;
            Ok(value.0)
        }
    }

    fn write(&self, effect: Effect, value: i32) -> Result<(), String> {
        use windows::Win32::UI::WindowsAndMessaging::*;
        unsafe {
            if effect == Effect::Window {
                let mut info = ANIMATIONINFO {
                    cbSize: std::mem::size_of::<ANIMATIONINFO>() as u32,
                    iMinAnimate: value,
                };
                return SystemParametersInfoW(
                    SPI_SETANIMATION,
                    info.cbSize,
                    Some(&mut info as *mut _ as *mut _),
                    SPIF_SENDCHANGE,
                )
                .map_err(|e| format!("无法设置系统窗口动画：{e}"));
            }
            let action = match effect {
                Effect::ClientArea => SPI_SETCLIENTAREAANIMATION,
                Effect::Menu => SPI_SETMENUANIMATION,
                Effect::MenuFade => SPI_SETMENUFADE,
                Effect::Window => unreachable!(),
            };
            // These setters take the BOOL value itself, not a pointer to a BOOL.
            // Notify the shell without persisting changes to the user profile.
            SystemParametersInfoW(action, 0, Some(value as isize as *mut _), SPIF_SENDCHANGE)
                .map_err(|e| format!("无法设置系统动画：{e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    #[derive(Clone)]
    struct Fake(Rc<State>);
    struct State {
        values: RefCell<[i32; 4]>,
        writes: RefCell<Vec<(Effect, i32)>>,
        read_failure: Cell<Option<Effect>>,
        disable_failure: Cell<Option<Effect>>,
        restore_failure: Cell<Option<Effect>>,
    }
    impl Fake {
        fn new(values: [i32; 4]) -> Self {
            Self(Rc::new(State {
                values: RefCell::new(values),
                writes: RefCell::new(Vec::new()),
                read_failure: Cell::new(None),
                disable_failure: Cell::new(None),
                restore_failure: Cell::new(None),
            }))
        }
    }
    impl Settings for Fake {
        fn read(&self, effect: Effect) -> Result<i32, String> {
            if self.0.read_failure.get() == Some(effect) {
                return Err("read failed".into());
            }
            Ok(self.0.values.borrow()[effect as usize])
        }
        fn write(&self, effect: Effect, value: i32) -> Result<(), String> {
            self.0.writes.borrow_mut().push((effect, value));
            self.0.values.borrow_mut()[effect as usize] = value;
            if value == 0 && self.0.disable_failure.get() == Some(effect) {
                self.0.disable_failure.set(None);
                return Err("disable failed after changing state".into());
            }
            if value != 0 && self.0.restore_failure.get() == Some(effect) {
                self.0.restore_failure.set(None);
                self.0.values.borrow_mut()[effect as usize] = 0;
                return Err("restore failed".into());
            }
            Ok(())
        }
    }

    #[test]
    fn capture_disables_effects_and_drop_restores_exact_original_values() {
        let system = Fake::new([1, 1, 1, 1]);
        let guard = Pause::suspend(system.clone()).unwrap();
        assert_eq!(*system.0.values.borrow(), [0; 4]);
        drop(guard);
        assert_eq!(*system.0.values.borrow(), [1; 4]);
    }
    #[test]
    fn already_disabled_effects_are_never_enabled_or_written() {
        let system = Fake::new([0, 1, 0, 1]);
        let mut guard = Pause::suspend(system.clone()).unwrap();
        guard.restore().unwrap();
        let writes = system.0.writes.borrow().len();
        guard.restore().unwrap();
        drop(guard);
        assert_eq!(*system.0.values.borrow(), [0, 1, 0, 1]);
        assert_eq!(system.0.writes.borrow().len(), writes);
        assert!(system
            .0
            .writes
            .borrow()
            .iter()
            .all(|(effect, _)| matches!(effect, Effect::Window | Effect::MenuFade)));
    }
    #[test]
    fn read_failure_does_not_change_any_setting() {
        let system = Fake::new([1; 4]);
        system.0.read_failure.set(Some(Effect::Menu));
        assert!(Pause::suspend(system.clone()).is_err());
        assert!(system.0.writes.borrow().is_empty());
    }
    #[test]
    fn partial_disable_failure_rolls_back_even_the_failed_setter() {
        let system = Fake::new([1; 4]);
        system.0.disable_failure.set(Some(Effect::Menu));
        assert!(Pause::suspend(system.clone()).is_err());
        assert_eq!(*system.0.values.borrow(), [1; 4]);
        assert!(!system
            .0
            .writes
            .borrow()
            .iter()
            .any(|(effect, _)| *effect == Effect::MenuFade));
    }
    #[test]
    fn an_entirely_disabled_configuration_is_left_untouched() {
        let system = Fake::new([0; 4]);
        drop(Pause::suspend(system.clone()).unwrap());
        assert!(system.0.writes.borrow().is_empty());
        assert_eq!(*system.0.values.borrow(), [0; 4]);
    }
    #[test]
    fn repeated_capture_sessions_restore_the_original_configuration() {
        let system = Fake::new([1, 0, 1, 0]);
        for _ in 0..100 {
            let guard = Pause::suspend(system.clone()).unwrap();
            assert_eq!(*system.0.values.borrow(), [0; 4]);
            drop(guard);
            assert_eq!(*system.0.values.borrow(), [1, 0, 1, 0]);
        }
    }
    #[test]
    fn failed_restore_keeps_its_original_for_retry_and_restores_other_effects() {
        let system = Fake::new([1; 4]);
        let mut guard = Pause::suspend(system.clone()).unwrap();
        system.0.restore_failure.set(Some(Effect::ClientArea));
        assert!(guard.restore().is_err());
        assert_eq!(*system.0.values.borrow(), [0, 1, 1, 1]);
        drop(guard);
        assert_eq!(*system.0.values.borrow(), [1; 4]);
    }
    #[test]
    fn unwinding_during_capture_restores_system_settings() {
        let system = Fake::new([1; 4]);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = Pause::suspend(system.clone()).unwrap();
            panic!("capture initialization failed");
        }));
        assert_eq!(*system.0.values.borrow(), [1; 4]);
    }
}
