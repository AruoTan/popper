//! Durable, current-user recovery for interrupted animation changes.
use super::Effect;

const FLAG: &str = "--restore-capture-animations";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    original: Vec<(Effect, i32)>,
}
impl Record {
    fn encode(original: &[(Effect, i32)]) -> Result<String, String> {
        serde_json::to_string(&Self {
            version: 1,
            original: original.to_vec(),
        })
        .map_err(|error| error.to_string())
    }

    fn decode(value: &str) -> Result<Self, String> {
        let record: Self = serde_json::from_str(value).map_err(|error| error.to_string())?;
        let valid = record.version == 1
            && !record.original.is_empty()
            && record.original.len() <= 4
            && record
                .original
                .iter()
                .enumerate()
                .all(|(index, (effect, value))| {
                    *value == 1
                        && !record.original[..index]
                            .iter()
                            .any(|(other, _)| other == effect)
                });
        if !valid {
            return Err("系统动画恢复记录无效".into());
        }
        Ok(record)
    }
}

fn command(executable: &str) -> Result<String, String> {
    if executable.is_empty() || executable.contains(['"', '\0', '\r', '\n']) {
        return Err("无法生成系统动画恢复命令".into());
    }
    let value = format!("\"{executable}\" {FLAG}");
    // Windows logon registry commands are limited to 260 characters.
    if value.encode_utf16().count() + 1 > 260 {
        return Err("程序路径过长，无法注册系统动画恢复项".into());
    }
    Ok(value)
}

fn recover_record(system: &impl super::Settings, value: Option<&str>) -> Result<(), String> {
    if let Some(value) = value {
        let mut original = Record::decode(value)?.original;
        let mut result = Ok(());
        for _ in 0..3 {
            result = super::restore_effects(system, &mut original);
            if result.is_ok() {
                break;
            }
        }
        result?;
    }
    system.clear_recovery()
}

#[cfg(target_os = "windows")]
pub(super) use native::{arm, clear};
#[cfg(target_os = "windows")]
pub use native::{recover_pending, run_if_requested};

#[cfg(target_os = "windows")]
mod native {
    use super::{command, recover_record, Record, FLAG};
    use crate::capture::animations::{Effect, WindowsSettings};
    use std::{
        fs::{File, OpenOptions},
        os::windows::fs::OpenOptionsExt,
        path::{Path, PathBuf},
    };
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE},
            Storage::FileSystem::{DELETE, FILE_FLAG_DELETE_ON_CLOSE},
            System::Registry::{
                RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegFlushKey, RegOpenKeyExW,
                RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
                KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
            },
        },
    };

    const STATE_KEY: PCWSTR = w!("Software\\Popper\\Capture");
    const STATE_VALUE: PCWSTR = w!("AnimationRecovery");
    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const RUN_VALUE: PCWSTR = w!("PopperCaptureAnimationRecovery");

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
    impl Key {
        fn open(path: PCWSTR, create: bool) -> Result<Option<Self>, String> {
            let mut key = HKEY::default();
            let status = unsafe {
                if create {
                    RegCreateKeyExW(
                        HKEY_CURRENT_USER,
                        path,
                        None,
                        None,
                        REG_OPTION_NON_VOLATILE,
                        KEY_QUERY_VALUE | KEY_SET_VALUE,
                        None,
                        &mut key,
                        None,
                    )
                } else {
                    RegOpenKeyExW(
                        HKEY_CURRENT_USER,
                        path,
                        None,
                        KEY_QUERY_VALUE | KEY_SET_VALUE,
                        &mut key,
                    )
                }
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            status
                .ok()
                .map_err(|e| format!("无法打开系统动画恢复项：{e}"))?;
            Ok(Some(Self(key)))
        }

        fn read(&self, name: PCWSTR) -> Result<Option<String>, String> {
            let mut kind = REG_VALUE_TYPE::default();
            let mut bytes = 0;
            let status = unsafe {
                RegQueryValueExW(self.0, name, None, Some(&mut kind), None, Some(&mut bytes))
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            status.ok().map_err(|e| e.to_string())?;
            if kind != REG_SZ || bytes < 2 || bytes > 8192 || bytes % 2 != 0 {
                return Err("系统动画恢复项格式无效".into());
            }
            let mut data = vec![0u16; bytes as usize / 2];
            unsafe {
                RegQueryValueExW(
                    self.0,
                    name,
                    None,
                    Some(&mut kind),
                    Some(data.as_mut_ptr().cast()),
                    Some(&mut bytes),
                )
            }
            .ok()
            .map_err(|e| e.to_string())?;
            data.truncate(bytes as usize / 2);
            if kind != REG_SZ || bytes % 2 != 0 || data.pop() != Some(0) || data.contains(&0) {
                return Err("系统动画恢复项格式无效".into());
            }
            String::from_utf16(&data)
                .map(Some)
                .map_err(|e| e.to_string())
        }

        fn write(&self, name: PCWSTR, value: &str) -> Result<(), String> {
            let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
            let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr().cast(), data.len() * 2) };
            unsafe { RegSetValueExW(self.0, name, None, REG_SZ, Some(bytes)) }
                .ok()
                .map_err(|e| e.to_string())?;
            // Commit both the backup and startup registration before changing
            // system settings, including when power is lost immediately after.
            unsafe { RegFlushKey(self.0) }
                .ok()
                .map_err(|e| e.to_string())?;
            if self.read(name)?.as_deref() != Some(value) {
                return Err("系统动画恢复项写入校验失败".into());
            }
            Ok(())
        }

        fn remove(&self, name: PCWSTR) -> Result<(), String> {
            let status = unsafe { RegDeleteValueW(self.0, name) };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            if status != ERROR_SUCCESS {
                return Err(format!("无法移除系统动画恢复项：{status:?}"));
            }
            unsafe { RegFlushKey(self.0) }
                .ok()
                .map_err(|e| e.to_string())
        }
    }

    fn lease() -> Result<Option<File>, String> {
        let root = std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or("无法确定系统动画恢复目录")?
            .join("Popper");
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        lock_file(&root.join("capture-animations.lock"))
    }

    fn lock_file(path: &Path) -> Result<Option<File>, String> {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .access_mode(GENERIC_READ.0 | GENERIC_WRITE.0 | DELETE.0)
            .share_mode(0)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE.0)
            .open(path)
        {
            Ok(file) => Ok(Some(file)),
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => Ok(None),
            Err(error) => Err(format!("无法锁定系统动画恢复记录：{error}")),
        }
    }

    impl WindowsSettings {
        pub(crate) fn acquire() -> Result<Self, String> {
            let system = Self {
                _lease: lease()?.ok_or("系统动画正在由另一截图会话使用")?,
            };
            recover(&system)?;
            Ok(system)
        }
    }

    pub(in crate::capture::animations) fn arm(original: &[(Effect, i32)]) -> Result<(), String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = command(executable.to_str().ok_or("程序路径无法用于系统动画恢复")?)?;
        let record = Record::encode(original)?;
        Record::decode(&record)?;
        Key::open(STATE_KEY, true)?
            .ok_or("无法创建系统动画恢复记录")?
            .write(STATE_VALUE, &record)?;
        // A temporary Run value stays registered if recovery fails. Unlike
        // RunOnce it is removed only after verified restoration succeeds.
        Key::open(RUN_KEY, true)?
            .ok_or("无法创建系统动画恢复项")?
            .write(RUN_VALUE, &command)
    }

    pub(in crate::capture::animations) fn clear() -> Result<(), String> {
        // Remove and flush the startup command first. If cleanup is interrupted,
        // the remaining backup can be safely consumed by the next normal launch.
        if let Some(key) = Key::open(RUN_KEY, false)? {
            key.remove(RUN_VALUE)?;
        }
        if let Some(key) = Key::open(STATE_KEY, false)? {
            key.remove(STATE_VALUE)?;
        }
        Ok(())
    }

    fn recover(system: &WindowsSettings) -> Result<(), String> {
        let value = Key::open(STATE_KEY, false)?
            .map(|key| key.read(STATE_VALUE))
            .transpose()?
            .flatten();
        recover_record(system, value.as_deref())
    }

    pub fn recover_pending() -> Result<(), String> {
        if let Some(lease) = lease()? {
            recover(&WindowsSettings { _lease: lease })?;
        }
        // An active capture owns the lease; never undo its animation pause.
        Ok(())
    }

    pub fn run_if_requested() -> bool {
        let mut args = std::env::args_os().skip(1);
        if args.next().as_deref() != Some(std::ffi::OsStr::new(FLAG)) || args.next().is_some() {
            return false;
        }
        // Runs before storage, single-instance forwarding, WebViews and tray.
        // The Windows subsystem entry point does not create a console window.
        if let Err(error) = recover_pending() {
            eprintln!("[capture] stage=recover-animations error={error}");
        }
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn exclusive_lock_can_move_threads_and_deletes_itself_after_release() {
            let path = std::env::temp_dir()
                .join(format!("popper-animation-{}.lock", uuid::Uuid::new_v4()));
            let file = lock_file(&path).unwrap().unwrap();
            assert!(lock_file(&path).unwrap().is_none());
            std::thread::spawn(move || drop(file)).join().unwrap();
            assert!(!path.exists());
            drop(lock_file(&path).unwrap().unwrap());
            assert!(!path.exists());
        }

        #[test]
        fn registry_backup_roundtrips_and_cleanup_is_idempotent() {
            use windows::Win32::System::Registry::RegDeleteKeyW;
            let path: Vec<u16> = format!(
                "Software\\Popper\\Tests\\AnimationRecovery-{}",
                uuid::Uuid::new_v4()
            )
            .encode_utf16()
            .chain(Some(0))
            .collect();
            struct Cleanup(PCWSTR);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    unsafe {
                        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, self.0);
                    }
                }
            }
            let _cleanup = Cleanup(PCWSTR(path.as_ptr()));
            let key = Key::open(PCWSTR(path.as_ptr()), true).unwrap().unwrap();
            assert!(key.read(STATE_VALUE).unwrap().is_none());
            let original = [(Effect::Window, 1), (Effect::MenuFade, 1)];
            let record = Record::encode(&original).unwrap();
            key.write(STATE_VALUE, &record).unwrap();
            assert_eq!(
                Record::decode(&key.read(STATE_VALUE).unwrap().unwrap())
                    .unwrap()
                    .original,
                original
            );
            key.remove(STATE_VALUE).unwrap();
            key.remove(STATE_VALUE).unwrap();
            assert!(key.read(STATE_VALUE).unwrap().is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Pause, Settings};
    use super::*;
    use std::cell::{Cell, RefCell};

    struct Durable {
        values: RefCell<[i32; 4]>,
        record: RefCell<Option<String>>,
        registered: Cell<bool>,
        fail_registration: Cell<bool>,
        fail_restore: Cell<bool>,
        fail_cleanup: Cell<bool>,
        writes: Cell<usize>,
    }
    impl Durable {
        fn new() -> Self {
            Self {
                values: RefCell::new([1, 0, 1, 0]),
                record: RefCell::new(None),
                registered: Cell::new(false),
                fail_registration: Cell::new(false),
                fail_restore: Cell::new(false),
                fail_cleanup: Cell::new(false),
                writes: Cell::new(0),
            }
        }
        fn recover(&self) -> Result<(), String> {
            let value = self.record.borrow().clone();
            recover_record(&self, value.as_deref())
        }
    }
    impl Settings for &Durable {
        fn read(&self, effect: Effect) -> Result<i32, String> {
            Ok(self.values.borrow()[effect as usize])
        }
        fn write(&self, effect: Effect, value: i32) -> Result<(), String> {
            if value == 0 {
                assert!(
                    self.registered.get(),
                    "settings changed before recovery was registered"
                );
            }
            self.writes.set(self.writes.get() + 1);
            if value != 0 && self.fail_restore.get() {
                return Err("restore failed".into());
            }
            self.values.borrow_mut()[effect as usize] = value;
            Ok(())
        }
        fn arm_recovery(&self, original: &[(Effect, i32)]) -> Result<(), String> {
            self.record.replace(Some(Record::encode(original)?));
            self.registered.set(true);
            if self.fail_registration.get() {
                return Err("registration failed".into());
            }
            Ok(())
        }
        fn clear_recovery(&self) -> Result<(), String> {
            if self.fail_cleanup.get() {
                return Err("cleanup failed".into());
            }
            self.registered.set(false);
            self.record.take();
            Ok(())
        }
    }

    #[test]
    fn successful_capture_restores_then_removes_both_recovery_items() {
        let system = Durable::new();
        let mut guard = Pause::suspend(&system).unwrap();
        assert_eq!(*system.values.borrow(), [0; 4]);
        assert!(system.registered.get());
        assert!(system.record.borrow().is_some());
        guard.restore().unwrap();
        assert_eq!(*system.values.borrow(), [1, 0, 1, 0]);
        assert!(!system.registered.get());
        assert!(system.record.borrow().is_none());
        drop(guard);
    }

    #[test]
    fn incomplete_registration_does_not_disable_animations() {
        let system = Durable::new();
        system.fail_registration.set(true);
        assert!(Pause::suspend(&system).is_err());
        assert_eq!(system.writes.get(), 0);
        assert_eq!(*system.values.borrow(), [1, 0, 1, 0]);
        assert!(!system.registered.get());
        assert!(system.record.borrow().is_none());
    }

    #[test]
    fn interrupted_capture_is_recovered_and_self_removes_on_next_launch() {
        let system = Durable::new();
        std::mem::forget(Pause::suspend(&system).unwrap());
        assert_eq!(*system.values.borrow(), [0; 4]);
        system.recover().unwrap();
        assert_eq!(*system.values.borrow(), [1, 0, 1, 0]);
        assert!(!system.registered.get());
        assert!(system.record.borrow().is_none());
        let writes = system.writes.get();
        system.recover().unwrap();
        assert_eq!(system.writes.get(), writes);
    }

    #[test]
    fn rebooted_capture_keeps_previously_disabled_effects_disabled() {
        let system = Durable::new();
        std::mem::forget(Pause::suspend(&system).unwrap());
        // A new login can already have reset these nonpersistent settings.
        system.values.replace([1, 0, 1, 0]);
        system.recover().unwrap();
        assert_eq!(*system.values.borrow(), [1, 0, 1, 0]);
        assert!(!system.registered.get());
    }

    #[test]
    fn interruption_before_registration_does_not_leave_a_startup_item() {
        let system = Durable::new();
        system.record.replace(Some(
            Record::encode(&[(Effect::ClientArea, 1), (Effect::Menu, 1)]).unwrap(),
        ));
        system.recover().unwrap();
        assert_eq!(*system.values.borrow(), [1, 0, 1, 0]);
        assert!(!system.registered.get());
        assert!(system.record.borrow().is_none());
    }

    #[test]
    fn invalid_backup_retains_recovery_registration_without_changing_settings() {
        let system = Durable::new();
        system.record.replace(Some("invalid".into()));
        system.registered.set(true);
        assert!(system.recover().is_err());
        assert_eq!(system.writes.get(), 0);
        assert!(system.registered.get());
        assert!(system.record.borrow().is_some());
    }

    #[test]
    fn failed_recovery_keeps_startup_registration_and_original_values_for_retry() {
        let system = Durable::new();
        std::mem::forget(Pause::suspend(&system).unwrap());
        system.fail_restore.set(true);
        assert!(system.recover().is_err());
        assert!(system.registered.get());
        assert!(system.record.borrow().is_some());
        system.fail_restore.set(false);
        system.recover().unwrap();
        assert_eq!(*system.values.borrow(), [1, 0, 1, 0]);
        assert!(!system.registered.get());
    }

    #[test]
    fn cleanup_failure_can_be_retried_without_disabling_restored_effects() {
        let system = Durable::new();
        let mut guard = Pause::suspend(&system).unwrap();
        system.fail_cleanup.set(true);
        assert!(guard.restore().is_err());
        assert_eq!(*system.values.borrow(), [1, 0, 1, 0]);
        assert!(system.registered.get());
        system.fail_cleanup.set(false);
        drop(guard);
        assert!(!system.registered.get());
        assert!(system.record.borrow().is_none());
    }

    #[test]
    fn no_recovery_item_is_registered_when_every_effect_is_already_off() {
        let system = Durable::new();
        system.values.replace([0; 4]);
        drop(Pause::suspend(&system).unwrap());
        assert!(!system.registered.get());
        assert!(system.record.borrow().is_none());
        assert_eq!(system.writes.get(), 0);
    }

    #[test]
    fn recovery_record_preserves_only_the_modified_effects() {
        let original = [(Effect::ClientArea, 1), (Effect::MenuFade, 1)];
        assert_eq!(
            Record::decode(&Record::encode(&original).unwrap())
                .unwrap()
                .original,
            original
        );
    }

    #[test]
    fn invalid_records_cannot_change_system_settings() {
        for value in [
            r#"{"version":2,"original":[["menu",1]]}"#,
            r#"{"version":1,"original":[]}"#,
            r#"{"version":1,"original":[["menu",0]]}"#,
            r#"{"version":1,"original":[["menu",1],["menu",1]]}"#,
            r#"{"version":1,"original":[["unknown",1]]}"#,
            r#"{"version":1,"original":[["menu",1]],"extra":true}"#,
            "invalid",
        ] {
            assert!(Record::decode(value).is_err(), "{value}");
        }
    }

    #[test]
    fn logon_command_quotes_paths_and_uses_only_the_recovery_entry_point() {
        assert_eq!(
            command(r"C:\Program Files\Popper\popper.exe").unwrap(),
            r#""C:\Program Files\Popper\popper.exe" --restore-capture-animations"#
        );
        assert!(command(r"D:\便携 程序\popper.exe").unwrap().ends_with(FLAG));
    }

    #[test]
    fn invalid_or_overlong_logon_commands_are_rejected_before_changes() {
        for executable in [
            "".to_owned(),
            "bad\"path.exe".into(),
            "bad\0path.exe".into(),
            "x".repeat(260),
        ] {
            assert!(command(&executable).is_err());
        }
    }
}
