pub mod app;
pub mod bot;
pub mod commands;
pub mod config;
pub mod core;
pub mod events;
pub mod hotkeys;
pub mod tray;
pub mod webhook;
pub mod windows;
pub mod discord_rpc;
pub mod vpn;
pub mod multi_roblox;
pub mod laptop_light;
pub mod laptop_fan;

use tauri::Manager;

pub fn run() {
    let state = app::build_state();
    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            windows::show_panel(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let st = app.state::<app::AppState>();
            app::setup(&handle, &st)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "panel" {
                    api.prevent_close();
                    windows::hide_panel(window.app_handle());
                }
                if window.label() == "guide" {
                    api.prevent_close();
                    windows::hide_guide(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::snapshot,
            commands::bot_start,
            commands::bot_pause,
            commands::bot_stop,
            commands::bot_toggle,
            commands::settings_get,
            commands::settings_set,
            commands::settings_reset,
            commands::settings_load_error,
            commands::web_dashboard_url,
            commands::web_set_allow_lan,
            commands::web_regenerate_token,
            commands::health_check,
            commands::preset_list,
            commands::preset_save,
            commands::preset_load,
            commands::preset_delete,
            commands::legacy_import,
            commands::overlay_open,
            commands::overlay_commit,
            commands::overlay_cancel,
            commands::overlay_open_regions,
            commands::overlay_pending,
            commands::overlay_commit_regions,
            commands::panel_placement_changed,
            commands::region_preview,
            commands::ocr_test,
            commands::webhook_test,
            commands::detect_bar_region,
            commands::hud_set_offset,
            commands::hud_toggle,
            commands::panel_show,
            commands::panel_hide,
            commands::panel_toggle,
            commands::panel_visible,
            commands::guide_open,
            commands::guide_hide,
            commands::app_quit,
            commands::open_url,
            commands::data_dir,
            commands::catches_list,
            commands::catches_clear,
            commands::catches_open,
            commands::boss_tracker_scan_server_age,
            commands::boss_tracker_sync_server_age,
            commands::scan_bait_stock,
            commands::test_gemini,
            commands::macro_list,
            commands::macro_status,
            commands::macro_record,
            commands::macro_play,
            commands::macro_rename,
            commands::macro_append_vpn_step,
            commands::macro_delete,
            commands::dataset_list,
            commands::dataset_label,
            commands::dataset_open,
            commands::knowledge_stats,
            commands::knowledge_list,
            commands::knowledge_sync_wiki,
            commands::ml_samples,
            commands::ml_annotate,
            commands::ml_validate,
            commands::ml_baseline,
            commands::ml_model_status,
            commands::ml_readiness,
            commands::vpn_get_status,
            commands::vpn_connect,
            commands::vpn_disconnect,
            commands::vpn_test_ping,
            commands::vpn_reset_network,
            commands::vpn_get_logs,
            commands::vpn_set_auto_reconnect,
            commands::multi_roblox_get_status,
            commands::multi_roblox_set_enabled,
            commands::multi_roblox_list_instances,
            commands::multi_roblox_focus_instance,
            commands::multi_roblox_kill_instance,
            commands::multi_roblox_kill_all,
            commands::multi_roblox_set_target,
            commands::multi_roblox_launch,
            commands::multi_roblox_list_accounts,
            commands::multi_roblox_add_account,
            commands::multi_roblox_remove_account,
            commands::multi_roblox_launch_account,
            commands::laptop_keyboard_light_get,
            commands::laptop_keyboard_light_set,
            commands::laptop_keyboard_light_setup,
            commands::laptop_fan_get,
            commands::laptop_fan_set,
            commands::laptop_fan_set_auto_turbo,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                    windows::hide_panel(app);
                }
            }
        });
}
