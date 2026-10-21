use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, Once, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LOG_BYTES: u64 = 256 * 1024;
static INITIALIZED: Once = Once::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_WRITE: Mutex<()> = Mutex::new(());
static READY: AtomicBool = AtomicBool::new(false);
static FAILURE_SHOWN: AtomicBool = AtomicBool::new(false);

pub(crate) fn initialize() {
    INITIALIZED.call_once(|| {
        let path = diagnostic_path();
        let _ = LOG_PATH.set(path);
        stage(&format!("starting Popper {}", env!("CARGO_PKG_VERSION")));
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Flush the backtrace before a native failure dialog blocks this
            // thread, so the complete diagnostic is immediately available.
            stage(&format!(
                "backtrace: {}",
                std::backtrace::Backtrace::force_capture()
            ));
            failure(&format!("panic: {info}"));
            previous(info);
        }));
    });
}

fn diagnostic_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(directory) = crate::app_paths::logs_directory() {
        return directory.join("startup.log");
    }
    std::env::temp_dir().join("Popper").join("startup.log")
}

pub(crate) fn stage(message: &str) {
    if let Some(path) = LOG_PATH.get() {
        let _guard = LOG_WRITE.lock().unwrap_or_else(|error| error.into_inner());
        if let Err(error) = write_entry(path, message) {
            eprintln!("failed to write Popper startup log: {error}");
        }
    }
}

fn write_entry(path: &Path, message: &str) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let truncate = fs::metadata(path).is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES);
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(!truncate)
        .truncate(truncate)
        .open(path)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let message: String = message.chars().take(32 * 1024).collect();
    writeln!(
        file,
        "{timestamp} process={} {}",
        std::process::id(),
        message.replace('\r', "\\r").replace('\n', "\\n")
    )
}

pub(crate) fn ready() {
    READY.store(true, Ordering::Release);
    stage("ready");
}

pub(crate) fn failure(message: &str) {
    stage(&format!("failure: {message}"));
    eprintln!("Popper: {message}");
    if !READY.load(Ordering::Acquire) && !FAILURE_SHOWN.swap(true, Ordering::AcqRel) {
        show_failure();
    }
}

#[cfg(target_os = "windows")]
fn show_failure() {
    let log_path = LOG_PATH.get().cloned().unwrap_or_else(diagnostic_path);
    show_failure_message(&format!(
        "Popper 启动失败。\n\n诊断日志：\n{}\n\n请将此日志提供给开发者以定位原因。",
        log_path.display()
    ));
}

#[cfg(target_os = "windows")]
pub(crate) fn storage_failure(error: &str) {
    // Storage selection failed before logging is initialized. Report the
    // actual error without silently writing to another user profile.
    eprintln!("Popper storage: {error}");
    show_failure_message(&format!("Popper 无法初始化数据目录。\n\n{error}"));
}

#[cfg(target_os = "windows")]
fn show_failure_message(message: &str) {
    use windows::{
        core::PCWSTR,
        Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK},
    };
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Popper 启动失败".encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn show_failure() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_log_preserves_previous_launch_and_escapes_multiline_errors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Popper/startup.log");
        write_entry(&path, "starting Popper").unwrap();
        write_entry(&path, "failure: 初始化失败\r\nsecond line").unwrap();
        let contents = fs::read_to_string(path).unwrap();
        assert_eq!(contents.lines().count(), 2);
        assert!(contents.contains("starting Popper"));
        assert!(contents.contains("failure: 初始化失败\\r\\nsecond line"));
        assert!(contents.contains(&format!("process={}", std::process::id())));
    }

    #[test]
    fn oversized_startup_log_is_rotated_before_next_launch_entry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("startup.log");
        fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
        write_entry(&path, "starting Popper").unwrap();
        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.len() < 1024);
        assert!(contents.ends_with("starting Popper\n"));
    }
}
