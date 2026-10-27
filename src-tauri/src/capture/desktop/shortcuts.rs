//! Transactional capture shortcut registration and rollback.
use super::{launch, manager};
use crate::{models::CaptureSettings, runtime::RuntimeState};
use tauri::{AppHandle, Manager};

pub struct ShortcutChange {
    app: AppHandle,
    old: Vec<String>,
    next: Vec<String>,
    added: Vec<String>,
    committed: bool,
}
impl ShortcutChange {
    pub fn prepare(app: &AppHandle, next: &CaptureSettings) -> Result<Self, String> {
        use tauri_plugin_global_shortcut::{GlobalShortcut, ShortcutState};
        let shortcuts = app
            .try_state::<GlobalShortcut<tauri::Wry>>()
            .ok_or("全局快捷键服务未启动，请重启 Popper")?;
        let old = manager(app).shortcuts.lock().clone();
        let next = next
            .shortcuts()
            .iter()
            .filter(|value| !value.is_empty())
            .map(|v| (*v).to_owned())
            .collect::<Vec<_>>();
        let mut change = Self {
            app: app.clone(),
            old,
            next,
            added: Vec::new(),
            committed: false,
        };
        for value in &change.next {
            if change.old.contains(value) {
                continue;
            }
            let expected = value.clone();
            shortcuts
                .on_shortcut(value.as_str(), move |app, _, event| {
                    if event.state() != ShortcutState::Pressed
                        || app.state::<RuntimeState>().global_shortcuts_suspended()
                    {
                        return;
                    }
                    let preferences = app.state::<RuntimeState>().settings.get_settings().capture;
                    let operation = if preferences.shortcut == expected {
                        "capture"
                    } else if preferences.paste_shortcut == expected {
                        "paste"
                    } else {
                        return;
                    };
                    launch(app, operation);
                })
                .map_err(|e| format!("无法注册屏幕捕捉快捷键 {value}，可能已被占用：{e}"))?;
            change.added.push(value.clone());
        }
        Ok(change)
    }
    pub fn commit(mut self) {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        for value in &self.old {
            if !self.next.contains(value) {
                let _ = self.app.global_shortcut().unregister(value.as_str());
            }
        }
        *manager(&self.app).shortcuts.lock() = self.next.clone();
        self.app
            .state::<RuntimeState>()
            .set_capture_shortcut_error(None);
        self.committed = true;
    }
}
impl Drop for ShortcutChange {
    fn drop(&mut self) {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        if !self.committed {
            for value in &self.added {
                let _ = self.app.global_shortcut().unregister(value.as_str());
            }
        }
    }
}
