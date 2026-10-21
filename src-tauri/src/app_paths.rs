//! Windows storage selection, resolved once before any logger or WebView starts.
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

#[cfg(target_os = "windows")]
use std::sync::OnceLock;

#[derive(Debug)]
pub(crate) struct AppPaths {
    pub(crate) data_root: Option<PathBuf>,
    pub(crate) logs: PathBuf,
}

impl AppPaths {
    fn resolve(
        executable: &Path,
        arguments: impl IntoIterator<Item = OsString>,
        local_app_data: Option<OsString>,
        temporary_directory: PathBuf,
    ) -> Result<Self, String> {
        let mut arguments = arguments.into_iter();
        let mut explicit_root = None;
        while let Some(argument) = arguments.next() {
            let value = if argument == "--data-dir" {
                Some(arguments.next().ok_or("--data-dir 需要指定绝对路径")?)
            } else {
                argument
                    .to_str()
                    .and_then(|value| value.strip_prefix("--data-dir="))
                    .map(OsString::from)
            };
            if let Some(value) = value {
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err("--data-dir 必须为非空的绝对路径".to_owned());
                }
                if explicit_root.replace(path).is_some() {
                    return Err("只能指定一次 --data-dir".to_owned());
                }
            }
        }
        let directory = executable.parent().ok_or("无法确定 Popper 程序目录")?;
        let portable = directory
            .join("portable.flag")
            .try_exists()
            .map_err(|error| error.to_string())?;
        let data_root = explicit_root.or_else(|| portable.then(|| directory.to_path_buf()));
        let logs = match &data_root {
            Some(root) => root.join("logs"),
            None => local_app_data
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or(temporary_directory)
                .join("Popper"),
        };
        Ok(Self { data_root, logs })
    }

    fn prepare(&mut self) -> Result<(), String> {
        if let Some(root) = &self.data_root {
            // Resolve the executable/custom root once. Scoop links config and
            // logs individually; WebView data stays in the version directory.
            fs::create_dir_all(root)
                .map_err(|error| format!("无法创建数据目录 {}：{error}", root.display()))?;
            let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
            for directory in [root.join("config"), root.join("logs"), root.join("webview")] {
                ensure_writable(&directory)?;
            }
            self.logs = root.join("logs");
            self.data_root = Some(root);
        }
        Ok(())
    }

    pub(crate) fn settings_path(&self, installed_path: PathBuf) -> PathBuf {
        self.data_root
            .as_ref()
            .map(|root| root.join("config/settings.json"))
            .unwrap_or(installed_path)
    }

    pub(crate) fn accepts_launch(
        &self,
        executable: &Path,
        arguments: impl IntoIterator<Item = OsString>,
    ) -> Result<bool, String> {
        let requested = Self::resolve(executable, arguments, None, PathBuf::new())?;
        match (&self.data_root, &requested.data_root) {
            (None, None) => Ok(true),
            (Some(current), Some(requested)) => {
                let requested = fs::canonicalize(requested).map_err(|error| error.to_string())?;
                Ok(current == &requested)
            }
            _ => Ok(false),
        }
    }
}

fn ensure_writable(directory: &Path) -> Result<(), String> {
    let check = || -> std::io::Result<()> {
        fs::create_dir_all(directory)?;
        let probe = directory.join(format!(".write-check-{}", uuid::Uuid::new_v4().simple()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        drop(file);
        fs::remove_file(probe)
    };
    check().map_err(|error| format!("数据目录不可写 {}：{error}", directory.display()))
}

#[cfg(target_os = "windows")]
pub(crate) fn get() -> Result<&'static AppPaths, String> {
    static PATHS: OnceLock<Result<AppPaths, String>> = OnceLock::new();
    PATHS
        .get_or_init(|| {
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let mut paths = AppPaths::resolve(
                &executable,
                std::env::args_os().skip(1),
                std::env::var_os("LOCALAPPDATA"),
                std::env::temp_dir(),
            )?;
            paths.prepare()?;
            Ok(paths)
        })
        .as_ref()
        .map_err(Clone::clone)
}

#[cfg(target_os = "windows")]
pub(crate) fn logs_directory() -> Option<PathBuf> {
    get().ok().map(|paths| paths.logs.clone())
}

#[cfg(target_os = "windows")]
pub(crate) fn configure_webview<'a, R: tauri::Runtime, M: tauri::Manager<R>>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
) -> tauri::Result<tauri::WebviewWindowBuilder<'a, R, M>> {
    let paths = get().map_err(std::io::Error::other)?;
    Ok(match &paths.data_root {
        Some(root) => builder.data_directory(root.join("webview")),
        None => builder,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(executable: &Path, args: &[OsString]) -> Result<AppPaths, String> {
        AppPaths::resolve(
            executable,
            args.iter().cloned(),
            Some("local".into()),
            PathBuf::from("temporary"),
        )
    }

    #[test]
    fn installed_mode_preserves_existing_locations_even_when_data_directory_exists() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("data")).unwrap();
        let paths = resolve(&root.path().join("popper.exe"), &[]).unwrap();
        assert!(paths.data_root.is_none());
        assert_eq!(paths.logs, PathBuf::from("local/Popper"));
        assert_eq!(
            paths.settings_path(PathBuf::from("old/settings.json")),
            PathBuf::from("old/settings.json")
        );
    }

    #[test]
    fn portable_mode_uses_the_executable_directory_and_keeps_existing_data() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("portable.flag"), "").unwrap();
        let mut paths = resolve(&root.path().join("popper.exe"), &[]).unwrap();
        paths.prepare().unwrap();
        let settings = paths.settings_path(PathBuf::from("unused"));
        fs::write(&settings, "existing settings").unwrap();
        paths.prepare().unwrap();
        assert_eq!(fs::read_to_string(&settings).unwrap(), "existing settings");
        assert_eq!(paths.logs, root.path().canonicalize().unwrap().join("logs"));
        assert_eq!(
            settings,
            root.path()
                .canonicalize()
                .unwrap()
                .join("config/settings.json")
        );
        assert!(root.path().join("webview").is_dir());
        assert!(!root.path().join("data").exists());
        assert_eq!(fs::read_dir(settings.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn explicit_directory_overrides_portable_marker_and_accepts_unicode_spaces() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("portable.flag"), "").unwrap();
        let target = root.path().join("用户 data");
        let paths = resolve(
            &root.path().join("popper.exe"),
            &["--data-dir".into(), target.clone().into_os_string()],
        )
        .unwrap();
        assert_eq!(paths.data_root, Some(target.clone()));
        let argument = format!("--data-dir={}", target.display());
        assert_eq!(
            resolve(&root.path().join("popper.exe"), &[argument.into()])
                .unwrap()
                .data_root,
            Some(target)
        );
    }

    #[test]
    fn malformed_directory_arguments_are_rejected() {
        for arguments in [
            vec!["--data-dir"],
            vec!["--data-dir="],
            vec!["--data-dir", "relative"],
            vec!["--data-dir", "--other"],
        ] {
            assert!(resolve(
                Path::new("popper.exe"),
                &arguments
                    .into_iter()
                    .map(OsString::from)
                    .collect::<Vec<_>>()
            )
            .is_err());
        }
        let root = tempfile::tempdir().unwrap();
        assert!(resolve(
            &root.path().join("popper.exe"),
            &[
                "--data-dir".into(),
                root.path().into(),
                "--data-dir".into(),
                root.path().into()
            ]
        )
        .is_err());
    }

    #[test]
    fn invalid_portable_storage_fails_without_falling_back_to_installed_mode() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("portable.flag"), "").unwrap();
        fs::write(root.path().join("config"), "not a directory").unwrap();
        let mut paths = resolve(&root.path().join("popper.exe"), &[]).unwrap();
        assert!(paths.prepare().is_err());
        assert!(paths.data_root.is_some());
    }

    #[test]
    fn a_second_launch_may_only_open_the_active_profile() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("installed");
        let portable = root.path().join("portable");
        fs::create_dir(&installed).unwrap();
        fs::create_dir(&portable).unwrap();
        fs::write(portable.join("portable.flag"), "").unwrap();
        let installed_exe = installed.join("popper.exe");
        let portable_exe = portable.join("popper.exe");
        let mut paths = resolve(&portable_exe, &[]).unwrap();
        paths.prepare().unwrap();
        assert!(paths.accepts_launch(&portable_exe, []).unwrap());
        assert!(!paths.accepts_launch(&installed_exe, []).unwrap());
        assert!(paths
            .accepts_launch(
                &installed_exe,
                ["--data-dir".into(), paths.data_root.clone().unwrap().into()]
            )
            .unwrap());
        let installed_paths = resolve(&installed_exe, &[]).unwrap();
        assert!(installed_paths.accepts_launch(&installed_exe, []).unwrap());
        assert!(!installed_paths.accepts_launch(&portable_exe, []).unwrap());
    }

    #[test]
    fn a_relocated_portable_profile_preserves_provider_and_integration_secrets() {
        use crate::{
            models::{CreateProviderInput, ProviderKind},
            settings::SettingsRepository,
        };
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        fs::create_dir(&first).unwrap();
        fs::write(first.join("portable.flag"), "").unwrap();
        let mut paths = resolve(&first.join("popper.exe"), &[]).unwrap();
        paths.prepare().unwrap();
        let settings = SettingsRepository::new(paths.settings_path(PathBuf::new())).unwrap();
        let provider = settings
            .create_provider(CreateProviderInput {
                name: "portable provider".to_owned(),
                kind: ProviderKind::Openai,
                base_url: "https://example.com/v1".to_owned(),
                dsh_workspace: None,
            })
            .unwrap();
        let provider_id = provider.providers.last().unwrap().id.clone();
        settings
            .set_provider_api_key(&provider_id, "sk-portable")
            .unwrap();
        settings
            .set_eudic_authorization("test authorization")
            .unwrap();
        drop(settings);
        let second = root.path().join("second");
        fs::rename(first, &second).unwrap();
        let mut paths = resolve(&second.join("popper.exe"), &[]).unwrap();
        paths.prepare().unwrap();
        let settings = SettingsRepository::new(paths.settings_path(PathBuf::new())).unwrap();
        assert_eq!(
            settings.get_api_key(&provider_id).unwrap().as_deref(),
            Some("sk-portable")
        );
        assert_eq!(
            settings.eudic_authorization().unwrap().as_deref(),
            Some("test authorization")
        );
    }

    #[cfg(unix)]
    #[test]
    fn scoop_persists_config_and_logs_but_recreates_webview_for_each_version() {
        let root = tempfile::tempdir().unwrap();
        let persist = root.path().join("persist/popper");
        for name in ["config", "logs"] {
            fs::create_dir_all(persist.join(name)).unwrap();
        }
        for version in ["1.0.0", "2.0.0"] {
            let app = root.path().join("apps/popper").join(version);
            fs::create_dir_all(&app).unwrap();
            fs::write(app.join("portable.flag"), "").unwrap();
            for name in ["config", "logs"] {
                std::os::unix::fs::symlink(persist.join(name), app.join(name)).unwrap();
            }
            let mut paths = resolve(&app.join("popper.exe"), &[]).unwrap();
            paths.prepare().unwrap();
            let settings = paths.settings_path(PathBuf::new());
            if version == "1.0.0" {
                fs::write(&settings, "preserved settings").unwrap();
                fs::write(paths.logs.join("previous.log"), "preserved log").unwrap();
                fs::write(app.join("webview/cache"), "old cache").unwrap();
            } else {
                assert_eq!(fs::read_to_string(&settings).unwrap(), "preserved settings");
                assert_eq!(
                    fs::read_to_string(paths.logs.join("previous.log")).unwrap(),
                    "preserved log"
                );
                assert_eq!(fs::read_dir(app.join("webview")).unwrap().count(), 0);
            }
            assert!(paths.accepts_launch(&app.join("popper.exe"), []).unwrap());
            assert!(!app.join("data").exists());
        }
        assert!(!persist.join("webview").exists());
    }
}
