//! Mizan desktop app: Tauri host for the import pipeline and read models.

pub mod commands;
pub mod market;
pub mod menu;
pub mod session;
pub mod state;

#[cfg(test)]
mod bindings;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::cmd_import_open,
            commands::cmd_import_current,
            commands::cmd_import_decide,
            commands::cmd_import_create_account,
            commands::cmd_import_map_account,
            commands::cmd_import_create_asset,
            commands::cmd_import_commit,
            commands::cmd_import_close,
            commands::cmd_import_history,
            commands::cmd_import_undo,
            commands::cmd_pending_decisions,
            commands::cmd_holdings,
            commands::cmd_overview,
            commands::cmd_contributions,
            commands::cmd_accounts,
            market::cmd_refresh_prices,
            market::cmd_settings,
            market::cmd_set_provider_enabled,
            market::cmd_set_base_currency,
            market::cmd_set_refresh_on_start,
            market::cmd_assets,
            market::cmd_set_price_source,
            market::cmd_accept_suggestion,
            market::cmd_add_manual_price,
            market::cmd_delete_manual_price,
            market::cmd_merge_assets,
            market::cmd_undo_merge,
        ])
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let store = state::open_store(&dir).map_err(std::io::Error::other)?;
            app.manage(state::AppState::with_path(store, state::db_path(&dir)));
            menu::build(app)?;
            market::spawn_start_refresh(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Mizan");
}
