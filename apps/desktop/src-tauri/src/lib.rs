//! Mizan desktop app: Tauri host for the import pipeline and read models.

pub mod commands;
pub mod mapper;
pub mod market;
pub mod menu;
pub mod returns;
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
            commands::cmd_import_choose_adapter,
            mapper::cmd_mapper_inspect,
            mapper::cmd_mapper_preview,
            mapper::cmd_mapper_save,
            mapper::cmd_mapper_export,
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
            returns::cmd_performance,
            returns::cmd_value_history,
            returns::cmd_set_benchmark,
            market::cmd_refresh_prices,
            market::cmd_backfill_status,
            market::cmd_settings,
            market::cmd_set_provider_enabled,
            market::cmd_set_api_key,
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
            let st = state::AppState::for_app_dir(store, &dir);
            if let Ok(reg) = st.registry.read() {
                for e in &reg.errors {
                    eprintln!("mizan: adapter {} not loaded: {}", e.location, e.message);
                }
            }
            app.manage(st);
            menu::build(app)?;
            market::spawn_start_refresh(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Mizan")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Stop the history backfill between units; completed units stay cached.
                if let Some(st) = app.try_state::<state::AppState>() {
                    st.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
}
