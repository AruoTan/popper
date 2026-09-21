use std::{io, path::Path};

#[cfg(any(not(windows), unix, test))]
use std::fs;

/// Atomically commits a fully written replacement over `destination`.
///
/// Both paths must be on the same filesystem. Callers deliberately create the
/// replacement next to the destination, so a successful call never exposes a
/// partially written file.
pub(crate) fn replace_file(replacement: &Path, destination: &Path) -> io::Result<()> {
    replace_file_platform(replacement, destination)?;
    sync_parent_best_effort(destination);
    Ok(())
}

/// Atomically publishes a new file without overwriting an existing one.
///
/// This is used for the raw local encryption key: concurrent first launches
/// must agree on one complete key instead of replacing one another's key.
pub(crate) fn publish_new_file(replacement: &Path, destination: &Path) -> io::Result<()> {
    publish_new_file_platform(replacement, destination)?;
    sync_parent_best_effort(destination);
    Ok(())
}

#[cfg(not(windows))]
fn replace_file_platform(replacement: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(replacement, destination)
}

#[cfg(not(windows))]
fn publish_new_file_platform(replacement: &Path, destination: &Path) -> io::Result<()> {
    fs::hard_link(replacement, destination)?;
    // Publication already succeeded. Failure to clean up the temporary hard
    // link must not make the caller believe the destination was not created.
    let _ = fs::remove_file(replacement);
    Ok(())
}

#[cfg(windows)]
fn replace_file_platform(replacement: &Path, destination: &Path) -> io::Result<()> {
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            REPLACE_FILE_FLAGS,
        },
    };

    let replacement = wide_path(replacement)?;
    let destination = wide_path(destination)?;
    let replacement = PCWSTR::from_raw(replacement.as_ptr());
    let destination = PCWSTR::from_raw(destination.as_ptr());

    // ReplaceFileW keeps replacement atomic while preserving the destination's
    // metadata and ACL where Windows supports it. It requires the destination
    // to exist, so an initial save falls back to MoveFileExW. The fallback also
    // closes the race where another process creates the destination between
    // these calls.
    match unsafe {
        ReplaceFileW(
            destination,
            replacement,
            PCWSTR::null(),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    } {
        Ok(()) => Ok(()),
        Err(error) if windows_error_code(&error).is_some_and(is_missing_path_error) => unsafe {
            MoveFileExW(
                replacement,
                destination,
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
            .map_err(windows_io_error)
        },
        Err(error) => Err(windows_io_error(error)),
    }
}

#[cfg(windows)]
fn publish_new_file_platform(replacement: &Path, destination: &Path) -> io::Result<()> {
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH},
    };

    let replacement = wide_path(replacement)?;
    let destination = wide_path(destination)?;
    unsafe {
        MoveFileExW(
            PCWSTR::from_raw(replacement.as_ptr()),
            PCWSTR::from_raw(destination.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
        .map_err(windows_io_error)
    }
}

#[cfg(windows)]
fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows path contains an embedded null character",
        ));
    }
    wide.push(0);
    Ok(wide)
}

#[cfg(windows)]
fn windows_error_code(error: &windows::core::Error) -> Option<u32> {
    let code = error.code().0 as u32;
    // HRESULT_FROM_WIN32 stores the original Win32 status in the low word.
    ((code & 0xffff_0000) == 0x8007_0000).then_some(code & 0xffff)
}

#[cfg(windows)]
fn windows_io_error(error: windows::core::Error) -> io::Error {
    windows_error_code(&error)
        .map(|code| io::Error::from_raw_os_error(code as i32))
        .unwrap_or_else(|| io::Error::other(error.to_string()))
}

#[cfg(windows)]
fn is_missing_path_error(code: u32) -> bool {
    // ERROR_FILE_NOT_FOUND or ERROR_PATH_NOT_FOUND.
    matches!(code, 2 | 3)
}

fn sync_parent_best_effort(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
    }

    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn replaces_an_existing_file_and_consumes_the_replacement() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("settings.json");
        let replacement = directory.path().join("settings.next");
        fs::write(&destination, b"old complete value").unwrap();
        fs::write(&replacement, b"new complete value").unwrap();

        replace_file(&replacement, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"new complete value");
        assert!(!replacement.exists());
    }

    #[test]
    fn creates_a_missing_destination() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("settings.json");
        let replacement = directory.path().join("settings.next");
        fs::write(&replacement, b"first complete value").unwrap();

        replace_file(&replacement, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"first complete value");
        assert!(!replacement.exists());
    }

    #[test]
    fn failed_replace_keeps_the_old_destination_intact() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("settings.json");
        let missing_replacement = directory.path().join("missing.next");
        fs::write(&destination, b"old complete value").unwrap();

        assert!(replace_file(&missing_replacement, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"old complete value");
    }

    #[test]
    fn publishing_a_new_file_never_overwrites_an_existing_destination() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("secret.key");
        let replacement = directory.path().join("secret.next");
        fs::write(&destination, b"established key").unwrap();
        fs::write(&replacement, b"competing key").unwrap();

        let error = publish_new_file(&replacement, &destination).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&destination).unwrap(), b"established key");
        assert_eq!(fs::read(&replacement).unwrap(), b"competing key");
    }

    #[test]
    fn publishing_a_new_file_consumes_the_replacement() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("secret.key");
        let replacement = directory.path().join("secret.next");
        fs::write(&replacement, b"complete key").unwrap();

        publish_new_file(&replacement, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"complete key");
        assert!(!replacement.exists());
    }
}
