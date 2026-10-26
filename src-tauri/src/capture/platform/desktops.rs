//! Assign a floating window to an existing Windows virtual desktop.
use tauri::WebviewWindow;
use windows::{
    core::{GUID, PCWSTR},
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED,
            },
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_SZ},
        },
        UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager},
    },
};

pub struct DesktopEntry {
    pub id: String,
    pub label: String,
    pub current: bool,
}
const ROOT: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\VirtualDesktops";

struct Apartment(bool);
impl Apartment {
    fn enter() -> Result<Self, String> {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok() {
            Ok(Self(true))
        } else if result == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(result.message())
        }
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

fn registry_value(path: &str, name: &str, string: bool) -> Result<Vec<u8>, String> {
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut bytes = vec![0u8; 65536];
    let mut size = bytes.len() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            PCWSTR(name.as_ptr()),
            if string {
                RRF_RT_REG_SZ
            } else {
                RRF_RT_REG_BINARY
            },
            None,
            Some(bytes.as_mut_ptr().cast()),
            Some(&mut size),
        )
    }
    .ok()
    .map_err(|e| e.to_string())?;
    bytes.truncate(size as usize);
    Ok(bytes)
}

fn desktop_ids(current: GUID) -> Vec<GUID> {
    let mut ids = registry_value(ROOT, "VirtualDesktopIDs", false)
        .ok()
        .map(|bytes| decode_ids(&bytes))
        .unwrap_or_default();
    ids.retain(|id| *id != GUID::zeroed());
    ids.dedup();
    if !ids.contains(&current) {
        ids.push(current);
    }
    ids
}

fn decode_ids(bytes: &[u8]) -> Vec<GUID> {
    if bytes.len() % 16 != 0 {
        return Vec::new();
    }
    bytes
        .chunks_exact(16)
        .map(|raw| {
            GUID::from_values(
                u32::from_le_bytes(raw[0..4].try_into().unwrap()),
                u16::from_le_bytes(raw[4..6].try_into().unwrap()),
                u16::from_le_bytes(raw[6..8].try_into().unwrap()),
                raw[8..16].try_into().unwrap(),
            )
        })
        .collect()
}

pub fn pin_desktops(window: &WebviewWindow) -> Result<Vec<DesktopEntry>, String> {
    let _apartment = Apartment::enter()?;
    let manager: IVirtualDesktopManager =
        unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| e.to_string())?;
    let current = unsafe { manager.GetWindowDesktopId(window.hwnd().map_err(|e| e.to_string())?) }
        .map_err(|e| e.to_string())?;
    if current == GUID::zeroed() {
        return Err("无法确定贴图所属桌面".into());
    }
    Ok(desktop_ids(current)
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let identifier = uuid::Uuid::from_u128(id.to_u128());
            let name = registry_value(&format!("{ROOT}\\Desktops\\{{{identifier}}}"), "Name", true)
                .ok()
                .map(|bytes| {
                    let units: Vec<u16> = bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|c| *c != 0)
                        .collect();
                    String::from_utf16_lossy(&units)
                })
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("桌面 {}", index + 1));
            DesktopEntry {
                id: identifier.to_string(),
                label: name.replace('&', "&&"),
                current: *id == current,
            }
        })
        .collect())
}

pub fn assign_pin_desktop(window: &WebviewWindow, value: &str) -> Result<(), String> {
    // Re-read membership so a removed desktop from an already open menu is rejected.
    if !pin_desktops(window)?.iter().any(|entry| entry.id == value) {
        return Err("目标桌面已不存在".into());
    }
    let id = GUID::from_u128(
        uuid::Uuid::parse_str(value)
            .map_err(|_| "桌面标识无效")?
            .as_u128(),
    );
    let _apartment = Apartment::enter()?;
    let manager: IVirtualDesktopManager =
        unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| e.to_string())?;
    unsafe { manager.MoveWindowToDesktop(window.hwnd().map_err(|e| e.to_string())?, &id) }
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_desktop_ids_keep_windows_byte_order_and_menu_order() {
        let first = uuid::Uuid::parse_str("12345678-9abc-def0-1234-567890abcdef").unwrap();
        let second = uuid::Uuid::parse_str("fedcba98-7654-3210-abcd-0123456789ab").unwrap();
        let bytes: Vec<u8> = [first, second]
            .iter()
            .flat_map(|id| id.to_bytes_le())
            .collect();
        let parsed: Vec<_> = decode_ids(&bytes)
            .iter()
            .map(|id| uuid::Uuid::from_u128(id.to_u128()))
            .collect();
        assert_eq!(parsed, [first, second]);
        assert!(decode_ids(&bytes[..31]).is_empty());
        assert!(decode_ids(&[]).is_empty());
    }
}
