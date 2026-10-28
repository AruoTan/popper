//! Native popup layout; the system measures text using its current menu font and DPI.
use tauri::menu::{ContextMenu, Menu};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMenuInfo, SetMenuInfo, HMENU, MENUINFO, MIM_STYLE, MNS_CHECKORBMP,
};

pub(super) fn fit(menu: &Menu<tauri::Wry>) -> tauri::Result<()> {
    let handle = menu.hpopupmenu()? as usize;
    let menu = menu.clone();
    // Keep the menu alive and touch its HMENU only on its owning UI thread.
    menu.app_handle().clone().run_on_main_thread(move || {
        let _menu = menu;
        let mut info = MENUINFO {
            cbSize: std::mem::size_of::<MENUINFO>() as u32,
            fMask: MIM_STYLE,
            ..Default::default()
        };
        unsafe {
            let handle = HMENU(handle as *mut _);
            if GetMenuInfo(handle, &mut info).is_ok() {
                // Share the bitmap/checkmark column instead of reserving two gutters.
                info.dwStyle |= MNS_CHECKORBMP;
                let _ = SetMenuInfo(handle, &info);
            }
        }
    })
}
