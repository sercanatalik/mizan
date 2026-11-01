//! Mizan desktop app: Tauri host for the import pipeline and read models.

pub mod commands;
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
        ])
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let store = state::open_store(&dir).map_err(std::io::Error::other)?;
            app.manage(state::AppState::new(store));
            menu::build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Mizan");
}
