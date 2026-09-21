#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "windows")]
    if popper_lib::selection::run_windows_selection_helper_if_requested() {
        return;
    }
    popper_lib::run();
}
