#[cfg(target_os = "macos")]
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;

#[cfg(any(target_os = "windows", test))]
const WINDOWS_CLIPBOARD_ATTEMPTS: usize = 8;
#[cfg(any(target_os = "windows", test))]
const WINDOWS_CLIPBOARD_TEXT_LIMIT: usize = 1_000_000;

#[cfg(target_os = "windows")]
pub(crate) struct WindowsClipboardSnapshot {
    data_object: Option<windows::Win32::System::Com::IDataObject>,
    sequence: u32,
}

#[cfg(target_os = "windows")]
impl WindowsClipboardSnapshot {
    pub(crate) fn sequence(&self) -> u32 {
        self.sequence
    }

    /// Restores the complete OLE data object only while the synthetic copy is
    /// still the newest clipboard write. This deliberately does not flush the
    /// OLE clipboard, so delayed formats remain owned by their original source.
    pub(crate) fn restore_if_unchanged(&self, expected_sequence: u32) -> bool {
        if !windows_clipboard_sequence_matches(expected_sequence) {
            return false;
        }
        unsafe { windows::Win32::System::Ole::OleSetClipboard(self.data_object.as_ref()) }.is_ok()
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn write_text(text: &str) -> Result<(), String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    write_to_pasteboard(&pasteboard, text)
}

#[cfg(target_os = "windows")]
pub(crate) fn write_text(text: &str) -> Result<(), String> {
    write_to_windows_clipboard(text)
}

/// Captures the current Windows OLE clipboard object, including an explicitly
/// empty clipboard. Callers must run in an OLE-initialized STA.
#[cfg(target_os = "windows")]
pub(crate) fn snapshot_windows_clipboard() -> Option<WindowsClipboardSnapshot> {
    use windows::Win32::{
        Foundation::{GetLastError, SetLastError, WIN32_ERROR},
        System::{DataExchange::CountClipboardFormats, Ole::OleGetClipboard},
    };

    let sequence = unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() };
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let format_count = unsafe { CountClipboardFormats() };
    if format_count == 0 {
        if unsafe { GetLastError() }.0 != 0 {
            return None;
        }
        return Some(WindowsClipboardSnapshot {
            data_object: None,
            sequence,
        });
    }

    let data_object = unsafe { OleGetClipboard() }.ok()?;
    windows_clipboard_sequence_matches(sequence).then_some(WindowsClipboardSnapshot {
        data_object: Some(data_object),
        sequence,
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_clipboard_sequence() -> u32 {
    unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

#[cfg(target_os = "windows")]
pub(crate) fn read_windows_clipboard_text(expected_sequence: u32) -> Option<String> {
    use std::{slice, thread, time::Duration};

    use windows::Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::CF_UNICODETEXT,
        },
    };

    struct ClipboardGuard;
    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            let _ = unsafe { CloseClipboard() };
        }
    }

    struct GlobalLockGuard(HGLOBAL);
    impl Drop for GlobalLockGuard {
        fn drop(&mut self) {
            let _ = unsafe { GlobalUnlock(self.0) };
        }
    }

    if !windows_clipboard_sequence_matches(expected_sequence)
        || unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)) }.is_err()
    {
        return None;
    }

    retry_operation(
        WINDOWS_CLIPBOARD_ATTEMPTS,
        || unsafe { OpenClipboard(None) },
        |retry_index| {
            let delay_ms = 4_u64.saturating_mul((retry_index as u64) + 1).min(20);
            thread::sleep(Duration::from_millis(delay_ms));
        },
    )
    .ok()?;
    let _clipboard = ClipboardGuard;
    if !windows_clipboard_sequence_matches(expected_sequence) {
        return None;
    }

    let handle = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT.0)) }.ok()?;
    let global = HGLOBAL(handle.0);
    let byte_length = unsafe { GlobalSize(global) };
    if byte_length < std::mem::size_of::<u16>() || byte_length % std::mem::size_of::<u16>() != 0 {
        return None;
    }
    let pointer = unsafe { GlobalLock(global) }.cast::<u16>();
    if pointer.is_null() {
        return None;
    }
    let _lock = GlobalLockGuard(global);
    let available_units = byte_length / std::mem::size_of::<u16>();
    let inspected_units = available_units.min(WINDOWS_CLIPBOARD_TEXT_LIMIT.saturating_add(1));
    let units = unsafe { slice::from_raw_parts(pointer, inspected_units) };
    let text = decode_windows_clipboard_text(units, WINDOWS_CLIPBOARD_TEXT_LIMIT)?;
    (windows_clipboard_sequence_matches(expected_sequence)
        && text.chars().count() <= WINDOWS_CLIPBOARD_TEXT_LIMIT)
        .then_some(text)
}

#[cfg(target_os = "windows")]
fn windows_clipboard_sequence_matches(expected_sequence: u32) -> bool {
    windows_clipboard_sequence_is_stable(expected_sequence, unsafe {
        windows::Win32::System::DataExchange::GetClipboardSequenceNumber()
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn write_text(_text: &str) -> Result<(), String> {
    Err("当前平台不支持系统剪贴板".to_owned())
}

#[cfg(target_os = "macos")]
fn write_to_pasteboard(pasteboard: &NSPasteboard, text: &str) -> Result<(), String> {
    pasteboard.clearContents();
    let value = NSString::from_str(text);
    // SAFETY: `NSPasteboardTypeString` is an AppKit-provided immutable global
    // whose address is valid for the lifetime of the process.
    let string_type = unsafe { NSPasteboardTypeString };
    pasteboard
        .setString_forType(&value, string_type)
        .then_some(())
        .ok_or_else(|| "无法写入系统剪贴板".to_owned())
}

#[cfg(target_os = "windows")]
fn write_to_windows_clipboard(text: &str) -> Result<(), String> {
    use std::{mem::size_of, ptr, thread, time::Duration};

    use windows::{
        core::{w, Free},
        Win32::{
            Foundation::{GetLastError, SetLastError, HANDLE, HGLOBAL, HWND, WIN32_ERROR},
            System::{
                DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
                Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            },
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
            },
        },
    };

    const CF_UNICODETEXT: u32 = 13;

    struct ClipboardGuard;

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            // SAFETY: A guard is constructed only after OpenClipboard succeeds,
            // and exactly one guard owns that open operation.
            let _ = unsafe { CloseClipboard() };
        }
    }

    struct ClipboardOwnerWindow(HWND);

    impl Drop for ClipboardOwnerWindow {
        fn drop(&mut self) {
            // SAFETY: The window was created by this thread and is destroyed
            // only after the clipboard guard has closed the clipboard.
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }

    struct OwnedGlobalMemory(Option<HGLOBAL>);

    impl OwnedGlobalMemory {
        fn relinquish(&mut self) {
            self.0 = None;
        }
    }

    impl Drop for OwnedGlobalMemory {
        fn drop(&mut self) {
            if let Some(mut handle) = self.0.take() {
                // SAFETY: This handle came from GlobalAlloc and ownership has
                // not been transferred to SetClipboardData.
                unsafe { handle.free() };
            }
        }
    }

    let wide = encode_windows_clipboard_text(text)?;
    let allocation_size = wide
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| "复制内容过长".to_owned())?;
    // SAFETY: The requested byte count was checked and GMEM_MOVEABLE is the
    // allocation form required by SetClipboardData.
    let global = unsafe { GlobalAlloc(GMEM_MOVEABLE, allocation_size) }
        .map_err(|error| format!("无法分配剪贴板内存：{error}"))?;
    let mut memory = OwnedGlobalMemory(Some(global));

    // SAFETY: `global` is a live allocation owned by `memory`.
    let destination = unsafe { GlobalLock(global) }.cast::<u16>();
    if destination.is_null() {
        return Err("无法锁定剪贴板内存".to_owned());
    }
    // SAFETY: The allocation is exactly `wide.len() * size_of::<u16>()`
    // bytes, the source is valid, and the regions cannot overlap.
    unsafe { ptr::copy_nonoverlapping(wide.as_ptr(), destination, wide.len()) };
    // GlobalUnlock returns zero both when the final lock is successfully
    // released and when it fails. Clear/read last-error to distinguish them.
    unsafe { SetLastError(WIN32_ERROR(0)) };
    if unsafe { GlobalUnlock(global) }.is_err() {
        let error = unsafe { GetLastError() };
        if error.0 != 0 {
            return Err(format!(
                "无法解锁剪贴板内存：{}",
                std::io::Error::from_raw_os_error(error.0 as i32)
            ));
        }
    }

    // EmptyClipboard requires a real owner HWND; opening with NULL would make
    // the subsequent SetClipboardData fail. A hidden built-in STATIC window
    // gives this synchronous operation an owner without surfacing UI.
    let owner = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("TextLens Clipboard Owner"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )
    }
    .map(ClipboardOwnerWindow)
    .map_err(|error| format!("无法创建剪贴板所有者窗口：{error}"))?;

    retry_operation(
        WINDOWS_CLIPBOARD_ATTEMPTS,
        || {
            // SAFETY: `owner` remains alive until after ClipboardGuard closes
            // the clipboard. A successful call creates exactly one guard.
            unsafe { OpenClipboard(Some(owner.0)) }
        },
        |retry_index| {
            let delay_ms = 4_u64.saturating_mul((retry_index as u64) + 1).min(20);
            thread::sleep(Duration::from_millis(delay_ms));
        },
    )
    .map_err(|error| format!("系统剪贴板正忙：{error}"))?;
    let _clipboard = ClipboardGuard;

    // Allocate and populate memory before clearing the clipboard so allocation
    // failures cannot destroy the user's existing clipboard contents.
    unsafe { EmptyClipboard() }.map_err(|error| format!("无法清空系统剪贴板：{error}"))?;
    // SAFETY: `global` is an unlocked GMEM_MOVEABLE allocation containing a
    // NUL-terminated UTF-16 string. On success Windows owns the allocation.
    unsafe { SetClipboardData(CF_UNICODETEXT, Some(HANDLE(global.0))) }
        .map_err(|error| format!("无法写入系统剪贴板：{error}"))?;
    memory.relinquish();
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn encode_windows_clipboard_text(text: &str) -> Result<Vec<u16>, String> {
    let mut encoded = Vec::with_capacity(text.len().saturating_add(1));
    for unit in text.encode_utf16() {
        if unit == 0 {
            return Err("复制内容包含不支持的空字符".to_owned());
        }
        encoded.push(unit);
    }
    encoded.push(0);
    Ok(encoded)
}

#[cfg(any(target_os = "windows", test))]
fn decode_windows_clipboard_text(units: &[u16], limit: usize) -> Option<String> {
    let inspected = units.len().min(limit.saturating_add(1));
    let end = units[..inspected].iter().position(|unit| *unit == 0)?;
    if end > limit {
        return None;
    }
    let text = String::from_utf16(&units[..end]).ok()?;
    (text.chars().count() <= limit).then_some(text)
}

#[cfg(any(target_os = "windows", test))]
fn windows_clipboard_sequence_is_stable(expected: u32, current: u32) -> bool {
    current == expected
}

#[cfg(any(target_os = "windows", test))]
fn retry_operation<T, E>(
    attempts: usize,
    mut operation: impl FnMut() -> Result<T, E>,
    mut before_retry: impl FnMut(usize),
) -> Result<T, E> {
    let attempts = attempts.max(1);
    for attempt in 0..attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt + 1 == attempts => return Err(error),
            Err(_) => before_retry(attempt),
        }
    }
    unreachable!("attempts is always at least one")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_clipboard_encoding_is_unicode_and_nul_terminated() {
        let encoded = encode_windows_clipboard_text("TextLens 中文 🍎\n第二行").unwrap();
        assert_eq!(encoded.last(), Some(&0));
        assert_eq!(
            String::from_utf16(&encoded[..encoded.len() - 1]).unwrap(),
            "TextLens 中文 🍎\n第二行"
        );
    }

    #[test]
    fn windows_clipboard_encoding_supports_empty_text() {
        assert_eq!(encode_windows_clipboard_text("").unwrap(), vec![0]);
    }

    #[test]
    fn windows_clipboard_encoding_rejects_embedded_nul() {
        assert_eq!(
            encode_windows_clipboard_text("before\0after").unwrap_err(),
            "复制内容包含不支持的空字符"
        );
    }

    #[test]
    fn windows_clipboard_decoding_requires_a_bounded_nul_terminated_value() {
        let encoded = "TextLens 中文 🍎"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        assert_eq!(
            decode_windows_clipboard_text(&encoded, WINDOWS_CLIPBOARD_TEXT_LIMIT).as_deref(),
            Some("TextLens 中文 🍎")
        );
        assert!(decode_windows_clipboard_text(&[b'a' as u16, b'b' as u16], 2).is_none());
        assert!(decode_windows_clipboard_text(&[b'a' as u16, b'b' as u16, 0], 1).is_none());
        assert!(decode_windows_clipboard_text(&[0xD800, 0], 2).is_none());
    }

    #[test]
    fn windows_clipboard_restore_never_overwrites_a_newer_clipboard_write() {
        assert!(windows_clipboard_sequence_is_stable(42, 42));
        assert!(!windows_clipboard_sequence_is_stable(42, 43));
        assert!(windows_clipboard_sequence_is_stable(0, 0));
        assert!(!windows_clipboard_sequence_is_stable(0, 1));
    }

    #[test]
    fn retry_operation_stops_after_success_without_an_extra_delay() {
        let mut calls = 0;
        let mut retries = Vec::new();
        let value = retry_operation(
            WINDOWS_CLIPBOARD_ATTEMPTS,
            || {
                calls += 1;
                if calls < 3 {
                    Err("busy")
                } else {
                    Ok("opened")
                }
            },
            |attempt| retries.push(attempt),
        )
        .unwrap();

        assert_eq!(value, "opened");
        assert_eq!(calls, 3);
        assert_eq!(retries, vec![0, 1]);
    }

    #[test]
    fn retry_operation_returns_the_last_error() {
        let mut calls = 0;
        let mut retries = 0;
        let error = retry_operation(
            3,
            || {
                calls += 1;
                Err::<(), _>(calls)
            },
            |_| retries += 1,
        )
        .unwrap_err();

        assert_eq!(error, 3);
        assert_eq!(calls, 3);
        assert_eq!(retries, 2);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn writes_unicode_text_to_an_isolated_pasteboard() {
        let pasteboard = NSPasteboard::pasteboardWithUniqueName();
        let expected = "TextLens 剪贴板测试 🍎\n第二行";

        write_to_pasteboard(&pasteboard, expected).unwrap();

        // SAFETY: See the matching use in `write_to_pasteboard`.
        let string_type = unsafe { NSPasteboardTypeString };
        let actual = pasteboard
            .stringForType(string_type)
            .expect("unique pasteboard should contain a string");
        assert_eq!(actual.to_string(), expected);
    }
}
