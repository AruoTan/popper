mod dictionary;
mod eudic;
pub mod actions;
mod clipboard;
mod quick_ask;
mod local_secrets;
pub mod models;
mod openai_protocol;
mod openai_transport;
mod dsh_transport;
pub mod runtime;
pub mod selection;
pub mod settings;
mod startup;
mod thinking;
pub mod windows;

#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use actions::ActionService;
use runtime::RuntimeState;
use selection::SelectionMonitor;
use settings::SettingsRepository;
use tauri::Manager;

#[cfg(target_os = "windows")]
static PENDING_SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "windows")]
static WINDOWS_RUNTIME_READY: AtomicBool = AtomicBool::new(false);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    startup::initialize();
    startup::stage("building application");
    let builder = tauri::Builder::default();
    // Windows allows launching the same executable repeatedly from the Start
    // menu. Register this before every other plugin so only the first process
    // owns the tray, hooks and settings repository; another launch opens the
    // existing process's settings even when its tray is disabled.
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        // The single-instance plugin initializes before configured webviews.
        // Remember an exceptionally early second launch and consume it at the
        // end of the primary process setup instead of losing the request.
        PENDING_SETTINGS_OPEN.store(true, Ordering::Release);
        if WINDOWS_RUNTIME_READY.load(Ordering::Acquire)
            && PENDING_SETTINGS_OPEN.swap(false, Ordering::AcqRel)
        {
            runtime::open_settings_window(app);
        }
    }));

    let app = builder
        // Keep Tauri's native macOS menu so the focused WebView receives the
        // standard responder actions for Cmd+X/C/V/A and undo/redo. Without
        // an Edit menu, WKWebView text fields accept typing but those keyboard
        // shortcuts do not reach the active input in packaged builds.
        .enable_macos_default_menu(true)
        .setup(|app| {
            startup::stage("initializing UI thread");
            #[cfg(target_os = "windows")]
            windows::initialize_windows_ui_thread();

            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            startup::stage("loading settings");
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings = Arc::new(SettingsRepository::new_with_legacy_migration(
                settings_path,
            )?);
            startup::stage("initializing HTTP clients");
            let actions = ActionService::new(settings.clone())?;
            startup::stage("initializing selection monitor");
            let mut monitor = SelectionMonitor::new(&app.config().identifier)?;
            let receiver = monitor
                .take_event_receiver()
                .ok_or("无法取得划词事件通道")?;

            app.manage(RuntimeState::new(settings, actions, monitor));
            let handle = app.handle().clone();
            // Create configured UI only after state exists: its first events
            // and renderer IPC are allowed to arrive during WebView creation.
            startup::stage("creating settings window");
            let settings_window = handle
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "settings")
                .ok_or("缺少设置窗口配置")?;
            tauri::WebviewWindowBuilder::from_config(&handle, settings_window)?.build()?;
            startup::stage("creating toolbar");
            handle
                .state::<RuntimeState>()
                .windows
                .ensure_toolbar(&handle)?;
            runtime::spawn_selection_loop(handle.clone(), receiver)?;
            startup::stage("creating tray");
            runtime::setup_tray(&handle)?;
            startup::stage("initializing quick ask shortcut");
            runtime::setup_quick_ask_shortcut(&handle);

            startup::stage("starting selection listener");
            let state = handle.state::<RuntimeState>();
            state.reconcile_capture(&handle);
            runtime::start_permission_poll(handle.clone());
            #[cfg(target_os = "macos")]
            if !SelectionMonitor::is_accessibility_trusted() {
                runtime::open_settings_window(&handle);
            }
            #[cfg(target_os = "windows")]
            {
                startup::stage("showing startup notice");
                if !PENDING_SETTINGS_OPEN.load(Ordering::Acquire) {
                    if let Err(error) = windows::show_startup_notice(
                        &handle,
                        windows::StartupNoticeKind::Started,
                    ) {
                        eprintln!("failed to show Windows startup notice: {error}");
                    }
                }
                WINDOWS_RUNTIME_READY.store(true, Ordering::Release);
                if PENDING_SETTINGS_OPEN.swap(false, Ordering::AcqRel) {
                    runtime::open_settings_window(&handle);
                }
            }
            startup::stage("setup complete");
            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();
            // Configured windows can emit events before setup manages the
            // runtime. Ignore those events rather than panicking at startup.
            if let Some(state) = app.try_state::<RuntimeState>() {
                state.handle_window_event(app, window.label(), event);
            }
        })
        .invoke_handler(tauri::generate_handler![
            runtime::get_settings,
            runtime::update_settings,
            runtime::reset_result_size,
            runtime::create_provider,
            runtime::update_provider,
            runtime::delete_provider,
            runtime::set_provider_api_key,
            runtime::clear_provider_api_key,
            runtime::get_provider_api_key,
            runtime::test_provider_connection,
            runtime::list_provider_models,
            runtime::sync_provider_models,
            runtime::get_accessibility_status,
            runtime::request_accessibility,
            runtime::open_settings,
            runtime::take_settings_guidance,
            runtime::toolbar_ready,
            runtime::present_toolbar,
            runtime::recover_toolbar,
            runtime::set_toolbar_input_mode,
            runtime::focus_toolbar_input,
            runtime::run_action,
            runtime::submit_quick_ask,
            runtime::set_quick_ask_shortcut_recording,
            runtime::close_quick_ask,
            runtime::quick_ask_ready,
            runtime::report_quick_ask_size,
            runtime::dictionary_state,
            runtime::dictionary_query,
            runtime::submit_translation,
            runtime::translation_input_suggestions,
            runtime::dictionary_suggest,
            runtime::dictionary_cancel_input,
            runtime::dictionary_audio,
            runtime::eudic_books,
            runtime::eudic_add,
            runtime::set_eudic_authorization,
            runtime::eudic_configured,
            runtime::hide_toolbar,
            runtime::report_toolbar_size,
            runtime::get_selection_detection_debug,
            runtime::get_selection_detection_debug_enabled,
            runtime::set_selection_detection_debug_enabled,
            runtime::begin_result_ready,
            runtime::ack_result_ready,
            runtime::prepare_result_reveal,
            runtime::commit_result_reveal,
            runtime::fail_result_reveal,
            runtime::set_result_pinned,
            runtime::set_result_pointer_inside,
            runtime::show_result_selection,
            runtime::hide_result_selection,
            runtime::cancel_action,
            runtime::retry_action,
            runtime::continue_action,
            runtime::copy_text,
            runtime::open_external,
            runtime::hide_result,
            runtime::close_result,
        ])
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            startup::failure(&format!("failed to build Popper: {error}"));
            return;
        }
    };
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Ready) {
            startup::ready();
        }
        #[cfg(target_os = "macos")]
        if matches!(event, tauri::RunEvent::Reopen { .. }) {
            runtime::open_settings_window(app);
        }
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            if let Some(state) = app.try_state::<RuntimeState>() {
                state.shutdown(app);
            }
        }
    });
}
