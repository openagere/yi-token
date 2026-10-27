mod commands;
mod state;

use state::AppState;
use tauri::Manager;
use yi_core::AppPaths;
use yi_core::services::Services;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .runtime(tauri_runtime_wry::Wry::default())
        .setup(|app| {
            let services = Services::start(AppPaths::from_env()?)?;
            app.manage(AppState(services));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::export_provider_config,
            commands::import_provider_config,
            commands::list_providers,
            commands::save_provider,
            commands::delete_provider,
            commands::test_provider,
            commands::get_proxy_status,
            commands::get_proxy_runtime,
            commands::get_proxy_monitor,
            commands::control_proxy,
            commands::list_standard_models,
            commands::get_model_catalog_info,
            commands::save_standard_model,
            commands::delete_standard_model,
            commands::get_settings,
            commands::save_settings,
            commands::get_logs,
            commands::get_usage,
            commands::get_terminal_profiles,
            commands::preview_terminal_config,
            commands::apply_terminal_config,
            commands::preview_terminal_direct_config,
            commands::apply_terminal_direct_config,
            commands::disconnect_terminal,
            commands::set_active_engine,
            commands::get_active_engines,
            commands::list_provider_models,
            commands::select_terminal_model,
            commands::discover_provider_models,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
