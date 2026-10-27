//! Assign a floating window to an existing Windows virtual desktop.
#[path = "desktops/pinning.rs"]
mod pinning;
#[path = "desktops/visibility.rs"]
mod visibility;
use pinning::Pinning;
use tauri::WebviewWindow;
use uuid::Uuid;
pub use visibility::DesktopVisibility;
use windows::{
    core::{GUID, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, HANDLE, RPC_E_CHANGED_MODE},
        Security::{GetTokenInformation, TokenSessionId, TOKEN_QUERY},
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
                COINIT_APARTMENTTHREADED,
            },
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_SZ},
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
        UI::{
            Shell::{IVirtualDesktopManager, VirtualDesktopManager},
            WindowsAndMessaging::GetForegroundWindow,
        },
    },
};

pub struct DesktopEntry {
    pub id: String,
    pub label: String,
    pub current: bool,
}
pub struct DesktopMenu {
    pub all: Option<bool>,
    pub active: Option<String>,
    pub items: Vec<DesktopEntry>,
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

fn registry_paths() -> Vec<String> {
    let mut paths = vec![ROOT.to_owned()];
    let mut session = 0u32;
    let mut returned = 0;
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return paths;
    }
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenSessionId,
            Some((&mut session as *mut u32).cast()),
            std::mem::size_of_val(&session) as u32,
            &mut returned,
        )
    };
    unsafe {
        let _ = CloseHandle(token);
    }
    if result.is_ok() {
        paths.push(format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\SessionInfo\\{session}\\VirtualDesktops"
        ));
    }
    paths
}

struct DesktopRegistry {
    ids: Vec<Uuid>,
    active: Option<Uuid>,
}

fn read_desktops(
    paths: &[String],
    mut read: impl FnMut(&str, &str) -> Option<Vec<u8>>,
) -> DesktopRegistry {
    let mut ids = Vec::new();
    for path in paths {
        if let Some(bytes) = read(path, "VirtualDesktopIDs") {
            for id in decode_ids(&bytes) {
                if !id.is_nil() && !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        if !ids.is_empty() {
            break;
        }
    }
    let active = paths.iter().find_map(|path| {
        let bytes = read(path, "CurrentVirtualDesktop")?;
        let values = decode_ids(&bytes);
        (values.len() == 1 && ids.contains(&values[0])).then(|| values[0])
    });
    DesktopRegistry { ids, active }
}

fn desktop_registry() -> DesktopRegistry {
    read_desktops(&registry_paths(), |path, name| {
        registry_value(path, name, false).ok()
    })
}

fn window_desktop(window: &WebviewWindow) -> Result<Option<Uuid>, String> {
    desktop_at(window.hwnd().map_err(|e| e.to_string())?)
}

fn desktop_at(hwnd: windows::Win32::Foundation::HWND) -> Result<Option<Uuid>, String> {
    let _apartment = Apartment::enter()?;
    let manager: IVirtualDesktopManager =
        unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL) }
            .map_err(|e| e.to_string())?;
    let id = unsafe { manager.GetWindowDesktopId(hwnd) }.map_err(|e| e.to_string())?;
    if id != GUID::zeroed() {
        return Ok(Some(Uuid::from_u128(id.to_u128())));
    }
    // Shell registration can lag behind a newly shown floating window.
    if unsafe { manager.IsWindowOnCurrentVirtualDesktop(hwnd) }
        .map_err(|e| e.to_string())?
        .as_bool()
    {
        return Ok(desktop_registry().active);
    }
    Ok(None)
}

fn decode_ids(bytes: &[u8]) -> Vec<Uuid> {
    if bytes.len() % 16 != 0 {
        return Vec::new();
    }
    bytes
        .chunks_exact(16)
        .map(|raw| Uuid::from_bytes_le(raw.try_into().unwrap()))
        .collect()
}

fn active_desktop(registry: &DesktopRegistry) -> Option<Uuid> {
    registry
        .active
        .or_else(|| {
            let _apartment = Apartment::enter().ok()?;
            let manager: IVirtualDesktopManager =
                unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL) }.ok()?;
            let id = unsafe { manager.GetWindowDesktopId(GetForegroundWindow()) }.ok()?;
            let id = Uuid::from_u128(id.to_u128());
            registry.ids.contains(&id).then_some(id)
        })
        .or_else(|| (registry.ids.len() == 1).then(|| registry.ids[0]))
}

fn menu_snapshot(
    mut registry: DesktopRegistry,
    all: Option<bool>,
    current: Option<Uuid>,
    mut name: impl FnMut(Uuid) -> Option<String>,
) -> Result<DesktopMenu, String> {
    if registry.ids.is_empty() {
        registry.ids.extend(current);
    }
    if registry.ids.is_empty() {
        return Err("无法读取虚拟桌面列表".into());
    }
    let active = registry
        .active
        .filter(|id| registry.ids.contains(id))
        .map(|id| id.to_string());
    let items = registry
        .ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let label = name(*id)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("桌面 {}", index + 1));
            DesktopEntry {
                id: id.to_string(),
                label: label.replace('&', "&&"),
                current: all == Some(false) && Some(*id) == current,
            }
        })
        .collect();
    Ok(DesktopMenu { all, active, items })
}

pub fn pin_desktops(window: &WebviewWindow) -> Result<DesktopMenu, String> {
    let mut registry = desktop_registry();
    registry.active = active_desktop(&registry);
    let all = Pinning::for_window(window.hwnd().map_err(|e| e.to_string())?)
        .and_then(|pinning| pinning.is_pinned())
        .ok();
    let current = window_desktop(window).ok().flatten();
    menu_snapshot(registry, all, current, |id| {
        let bytes = registry_value(&format!("{ROOT}\\Desktops\\{{{id}}}"), "Name", true).ok()?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|c| *c != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    })
}

fn assignment_target(value: &str, registry: &DesktopRegistry) -> Result<Option<Uuid>, String> {
    if value == "all" {
        return Ok(None);
    }
    let id = if value == "current" {
        registry.active.ok_or("无法确定当前活动桌面")?
    } else {
        Uuid::parse_str(value).map_err(|_| "桌面标识无效")?
    };
    if !registry.ids.contains(&id) {
        return Err("目标桌面已不存在".into());
    }
    Ok(Some(id))
}

fn assign(
    target: Option<Uuid>,
    was_pinned: bool,
    mut set_pinned: impl FnMut(bool) -> Result<(), String>,
    mut move_window: impl FnMut(Uuid) -> Result<(), String>,
) -> Result<(), String> {
    let Some(id) = target else {
        return set_pinned(true);
    };
    if was_pinned {
        set_pinned(false)?;
    }
    if let Err(error) = move_window(id) {
        if was_pinned {
            if let Err(restore) = set_pinned(true) {
                return Err(format!("{error}；恢复所有桌面显示失败：{restore}"));
            }
        }
        return Err(error);
    }
    Ok(())
}

pub fn assign_pin_desktop(window: &WebviewWindow, value: &str) -> Result<Option<Uuid>, String> {
    // Resolve the active desktop at click time and reject targets removed since the menu opened.
    let target = if value == "all" {
        None
    } else {
        let mut registry = desktop_registry();
        if value == "current" {
            registry.active = active_desktop(&registry);
        }
        assignment_target(value, &registry)?
    };
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let pinning = Pinning::for_window(hwnd)?;
    let was_pinned = pinning.is_pinned()?;
    assign(
        target,
        was_pinned,
        |pinned| pinning.set_pinned(pinned),
        |id| {
            let manager: IVirtualDesktopManager =
                unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL) }
                    .map_err(|e| e.to_string())?;
            unsafe { manager.MoveWindowToDesktop(hwnd, &GUID::from_u128(id.as_u128())) }
                .map_err(|e| e.to_string())?;
            let actual = unsafe { manager.GetWindowDesktopId(hwnd) }.map_err(|e| e.to_string())?;
            if actual != GUID::from_u128(id.as_u128()) {
                return Err("系统未确认贴图已分配到目标桌面".into());
            }
            Ok(())
        },
    )?;
    Ok(target)
}

pub fn initialize_pin_desktop(window: &WebviewWindow) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        // A newly visible window may not have a Shell view immediately.
        match assign_pin_desktop(window, "all") {
            Ok(_) => return Ok(()),
            Err(error) if std::time::Instant::now() >= deadline => {
                return Err(format!("无法将贴图显示在所有桌面：{error}"));
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
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
        let parsed = decode_ids(&bytes);
        assert_eq!(parsed, [first, second]);
        assert!(decode_ids(&bytes[..31]).is_empty());
        assert!(decode_ids(&[]).is_empty());
    }

    fn read_fixture(values: &[(&str, &str, Vec<u8>)]) -> DesktopRegistry {
        read_desktops(&["root".into(), "session".into()], |path, name| {
            values
                .iter()
                .find(|(p, n, _)| *p == path && *n == name)
                .map(|(_, _, bytes)| bytes.clone())
        })
    }

    fn encoded(ids: &[Uuid]) -> Vec<u8> {
        ids.iter().flat_map(|id| id.to_bytes_le()).collect()
    }

    #[test]
    fn desktop_list_survives_missing_current_desktop_and_removes_duplicates() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let registry = read_fixture(&[(
            "root",
            "VirtualDesktopIDs",
            encoded(&[first, Uuid::nil(), second, first]),
        )]);
        assert_eq!(registry.ids, [first, second]);
        assert_eq!(registry.active, None);
    }

    #[test]
    fn invalid_or_missing_root_list_falls_back_to_session() {
        let desktop = Uuid::from_u128(2);
        for root in [vec![], vec![1; 15], encoded(&[Uuid::nil()])] {
            let registry = read_fixture(&[
                ("root", "VirtualDesktopIDs", root),
                ("session", "VirtualDesktopIDs", encoded(&[desktop])),
                ("session", "CurrentVirtualDesktop", encoded(&[desktop])),
            ]);
            assert_eq!(registry.ids, [desktop]);
            assert_eq!(registry.active, Some(desktop));
        }
        let registry = read_fixture(&[("session", "VirtualDesktopIDs", encoded(&[desktop]))]);
        assert_eq!(registry.ids, [desktop]);
    }

    #[test]
    fn current_desktop_must_belong_to_the_list_and_can_use_session_path() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        for invalid in [
            vec![0; 15],
            encoded(&[Uuid::nil()]),
            encoded(&[Uuid::from_u128(3)]),
            encoded(&[first, second]),
        ] {
            let registry = read_fixture(&[
                ("root", "VirtualDesktopIDs", encoded(&[first, second])),
                ("root", "CurrentVirtualDesktop", invalid),
                ("session", "CurrentVirtualDesktop", encoded(&[second])),
            ]);
            assert_eq!(registry.ids, [first, second]);
            assert_eq!(registry.active, Some(second));
        }
        let empty = read_fixture(&[]);
        assert!(empty.ids.is_empty());
        assert_eq!(empty.active, None);
    }

    #[test]
    fn root_list_and_current_desktop_take_precedence() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let registry = read_fixture(&[
            ("root", "VirtualDesktopIDs", encoded(&[first, second])),
            ("session", "VirtualDesktopIDs", encoded(&[second])),
            ("root", "CurrentVirtualDesktop", encoded(&[first])),
            ("session", "CurrentVirtualDesktop", encoded(&[second])),
        ]);
        assert_eq!(registry.ids, [first, second]);
        assert_eq!(registry.active, Some(first));
    }

    fn three_desktops() -> DesktopRegistry {
        DesktopRegistry {
            ids: (1..=3).map(Uuid::from_u128).collect(),
            active: Some(Uuid::from_u128(3)),
        }
    }

    #[test]
    fn all_desktops_keeps_the_active_desktop_in_the_complete_named_list() {
        let menu = menu_snapshot(
            three_desktops(),
            Some(true),
            Some(Uuid::from_u128(3)),
            |_| None,
        )
        .unwrap();
        assert_eq!(menu.all, Some(true));
        assert_eq!(menu.active, Some(Uuid::from_u128(3).to_string()));
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["桌面 1", "桌面 2", "桌面 3"]
        );
        assert!(menu.items.iter().all(|item| !item.current));
    }

    #[test]
    fn assigned_desktop_check_is_independent_of_the_active_desktop() {
        let menu = menu_snapshot(
            three_desktops(),
            Some(false),
            Some(Uuid::from_u128(2)),
            |_| None,
        )
        .unwrap();
        assert_eq!(menu.active, Some(Uuid::from_u128(3).to_string()));
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.current)
                .collect::<Vec<_>>(),
            [false, true, false]
        );
    }

    #[test]
    fn unknown_window_state_does_not_invent_a_checked_assignment() {
        for all in [None, Some(false)] {
            let menu = menu_snapshot(three_desktops(), all, None, |_| None).unwrap();
            assert_eq!(menu.items.len(), 3);
            assert!(menu.items.iter().all(|item| !item.current));
        }
        let empty = DesktopRegistry {
            ids: vec![],
            active: None,
        };
        assert!(menu_snapshot(empty, None, None, |_| None).is_err());
    }

    #[test]
    fn desktop_names_are_preserved_and_menu_accelerators_are_escaped() {
        let menu = menu_snapshot(three_desktops(), Some(true), None, |id| {
            match id.as_u128() {
                1 => Some("工作 & 设计".into()),
                2 => Some(" ".into()),
                _ => None,
            }
        })
        .unwrap();
        assert_eq!(menu.items[0].label, "工作 && 设计");
        assert_eq!(menu.items[1].label, "桌面 2");
        assert_eq!(menu.items[2].label, "桌面 3");
    }

    #[test]
    fn current_target_is_resolved_from_the_latest_snapshot() {
        let mut registry = three_desktops();
        assert_eq!(assignment_target("all", &registry).unwrap(), None);
        assert_eq!(
            assignment_target("current", &registry).unwrap(),
            Some(Uuid::from_u128(3))
        );
        registry.active = Some(Uuid::from_u128(1));
        assert_eq!(
            assignment_target("current", &registry).unwrap(),
            Some(Uuid::from_u128(1))
        );
        assert_eq!(
            assignment_target(&Uuid::from_u128(2).to_string(), &registry).unwrap(),
            Some(Uuid::from_u128(2))
        );
    }

    #[test]
    fn assignment_rejects_invalid_removed_and_unknown_current_targets() {
        let mut registry = three_desktops();
        assert!(assignment_target("invalid", &registry).is_err());
        assert!(assignment_target(&Uuid::from_u128(4).to_string(), &registry).is_err());
        registry.active = None;
        assert!(assignment_target("current", &registry).is_err());
        registry.active = Some(Uuid::from_u128(4));
        assert!(assignment_target("current", &registry).is_err());
        registry.ids.clear();
        assert_eq!(assignment_target("all", &registry).unwrap(), None);
    }

    #[test]
    fn all_assignment_pins_only_this_window_without_moving_it() {
        let mut changes = vec![];
        assign(
            None,
            false,
            |pinned| {
                changes.push(pinned);
                Ok(())
            },
            |_| panic!("unexpected move"),
        )
        .unwrap();
        assert_eq!(changes, [true]);
    }

    #[test]
    fn single_assignment_unpins_before_moving_and_does_not_repin() {
        let events = std::cell::RefCell::new(vec![]);
        assign(
            Some(Uuid::from_u128(2)),
            true,
            |pinned| {
                events.borrow_mut().push(format!("pin:{pinned}"));
                Ok(())
            },
            |id| {
                events.borrow_mut().push(format!("move:{}", id.as_u128()));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*events.borrow(), ["pin:false", "move:2"]);
        assign(
            Some(Uuid::from_u128(2)),
            false,
            |_| panic!("single-desktop window must not be pinned"),
            |_| Ok(()),
        )
        .unwrap();
    }

    #[test]
    fn failed_unpin_aborts_before_moving() {
        let result = assign(
            Some(Uuid::from_u128(2)),
            true,
            |_| Err("unpin failed".into()),
            |_| panic!("unexpected move"),
        );
        assert_eq!(result.unwrap_err(), "unpin failed");
    }

    #[test]
    fn failed_move_restores_all_desktops_and_reports_failures() {
        let mut changes = vec![];
        let result = assign(
            Some(Uuid::from_u128(2)),
            true,
            |pinned| {
                changes.push(pinned);
                Ok(())
            },
            |_| Err("move failed".into()),
        );
        assert_eq!(changes, [false, true]);
        assert_eq!(result.unwrap_err(), "move failed");
        let result = assign(
            Some(Uuid::from_u128(2)),
            true,
            |pinned| {
                if pinned {
                    Err("restore failed".into())
                } else {
                    Ok(())
                }
            },
            |_| Err("move failed".into()),
        );
        let error = result.unwrap_err();
        assert!(error.contains("move failed") && error.contains("restore failed"));
        let result = assign(
            Some(Uuid::from_u128(2)),
            false,
            |_| panic!("unexpected pin"),
            |_| Err("move failed".into()),
        );
        assert_eq!(result.unwrap_err(), "move failed");
    }
}
