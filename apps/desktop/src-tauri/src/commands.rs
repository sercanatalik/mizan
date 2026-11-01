//! Tauri commands. Each is a thin async wrapper that runs a plain function
//! over [`AppState`] on a blocking thread, so SQLite never blocks the UI.

use std::path::PathBuf;

use mizan_core::portfolio::{self, AttentionItem, Contributions, Holdings, Overview};
use mizan_core::Decision;
use mizan_import::Date;
use mizan_storage::AccountId;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::session::{AppError, CommitView, ImportSession, ImportView};
use crate::state::AppState;

/// One committed import in the history list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BatchView {
    #[cfg_attr(test, ts(type = "number"))]
    pub id: i64,
    pub committed_at: String,
    pub adapter: String,
    pub adapter_version: String,
    pub file_sha256: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub activity_count: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub assertion_count: i64,
    pub all_checks_passed: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AccountView {
    #[cfg_attr(test, ts(type = "number"))]
    pub id: i64,
    pub name: String,
    pub hints: Vec<String>,
}

fn store_err<T>(_: T) -> AppError {
    AppError::Invalid("the database is unavailable".into())
}

// ---- plain functions (tested) ----

pub fn import_open(state: &AppState, path: PathBuf) -> Result<ImportView, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    let session = ImportSession::open(&store, &path)?;
    let view = session.view();
    *state.session.lock().map_err(store_err)? = Some(session);
    Ok(view)
}

fn with_session<T>(
    state: &AppState,
    f: impl FnOnce(&mut ImportSession, &mut mizan_storage::Store) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut store = state.store.lock().map_err(store_err)?;
    let mut guard = state.session.lock().map_err(store_err)?;
    let session = guard
        .as_mut()
        .ok_or_else(|| AppError::Invalid("no import review is open".into()))?;
    f(session, &mut store)
}

pub fn import_current(state: &AppState) -> Result<Option<ImportView>, AppError> {
    Ok(state
        .session
        .lock()
        .map_err(store_err)?
        .as_ref()
        .map(|s| s.view()))
}

pub fn import_decide(
    state: &AppState,
    index: usize,
    decision: Option<Decision>,
) -> Result<ImportView, AppError> {
    with_session(state, |s, store| {
        s.decide(store, index, decision)?;
        Ok(s.view())
    })
}

pub fn import_create_account(state: &AppState, hint: String) -> Result<ImportView, AppError> {
    with_session(state, |s, store| {
        s.create_account(store, &hint)?;
        Ok(s.view())
    })
}

pub fn import_map_account(
    state: &AppState,
    hint: String,
    account_id: AccountId,
) -> Result<ImportView, AppError> {
    with_session(state, |s, store| {
        s.map_account(store, &hint, account_id)?;
        Ok(s.view())
    })
}

pub fn import_create_asset(state: &AppState, key: String) -> Result<ImportView, AppError> {
    with_session(state, |s, store| {
        s.create_asset(store, &key)?;
        Ok(s.view())
    })
}

pub fn import_commit(state: &AppState) -> Result<CommitView, AppError> {
    let result = with_session(state, |s, store| s.commit(store))?;
    *state.session.lock().map_err(store_err)? = None;
    Ok(result)
}

pub fn import_close(state: &AppState) -> Result<(), AppError> {
    *state.session.lock().map_err(store_err)? = None;
    Ok(())
}

pub fn import_history(state: &AppState) -> Result<Vec<BatchView>, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    Ok(store
        .list_batches()?
        .into_iter()
        .map(|b| BatchView {
            id: b.id,
            committed_at: b.committed_at,
            adapter: b.adapter,
            adapter_version: b.adapter_version,
            file_sha256: b.file_sha256,
            activity_count: b.activity_count,
            assertion_count: b.assertion_count,
            all_checks_passed: b.all_checks_passed,
        })
        .collect())
}

pub fn import_undo(state: &AppState, batch_id: i64) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(store_err)?;
    store.undo_batch(batch_id)?;
    Ok(())
}

pub fn pending_decisions(state: &AppState) -> Result<usize, AppError> {
    Ok(state
        .session
        .lock()
        .map_err(store_err)?
        .as_ref()
        .map(|s| s.pending_decisions())
        .unwrap_or(0))
}

pub fn holdings(state: &AppState) -> Result<Holdings, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    Ok(portfolio::holdings(&store)?)
}

/// Overview plus the open review's pending decisions, which only the app knows about.
pub fn overview(state: &AppState) -> Result<Overview, AppError> {
    let mut o = {
        let store = state.store.lock().map_err(store_err)?;
        portfolio::overview(&store)?
    };
    let pending = pending_decisions(state)?;
    if pending > 0 {
        o.attention.insert(
            0,
            AttentionItem::PendingDecisions {
                count: pending as i64,
            },
        );
    }
    Ok(o)
}

pub fn contributions(state: &AppState, today: Date) -> Result<Contributions, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    Ok(portfolio::contributions(&store, today)?)
}

pub fn accounts(state: &AppState) -> Result<Vec<AccountView>, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    Ok(store
        .list_accounts()?
        .into_iter()
        .map(|a| AccountView {
            id: a.id,
            name: a.name,
            hints: a.hints,
        })
        .collect())
}

fn today() -> Date {
    Date(chrono::Local::now().date_naive())
}

// ---- Tauri wrappers ----

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&AppState) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<AppState>()))
        .await
        .map_err(|e| AppError::Invalid(format!("background task failed: {e}")))?
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_open(app: AppHandle, path: PathBuf) -> Result<ImportView, AppError> {
    blocking(app, move |s| import_open(s, path)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_current(app: AppHandle) -> Result<Option<ImportView>, AppError> {
    blocking(app, import_current).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_decide(
    app: AppHandle,
    index: usize,
    decision: Option<Decision>,
) -> Result<ImportView, AppError> {
    blocking(app, move |s| import_decide(s, index, decision)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_create_account(
    app: AppHandle,
    hint: String,
) -> Result<ImportView, AppError> {
    blocking(app, move |s| import_create_account(s, hint)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_map_account(
    app: AppHandle,
    hint: String,
    account_id: i64,
) -> Result<ImportView, AppError> {
    blocking(app, move |s| import_map_account(s, hint, account_id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_create_asset(app: AppHandle, key: String) -> Result<ImportView, AppError> {
    blocking(app, move |s| import_create_asset(s, key)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_commit(app: AppHandle) -> Result<CommitView, AppError> {
    blocking(app, import_commit).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_close(app: AppHandle) -> Result<(), AppError> {
    blocking(app, import_close).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_history(app: AppHandle) -> Result<Vec<BatchView>, AppError> {
    blocking(app, import_history).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_import_undo(app: AppHandle, batch_id: i64) -> Result<(), AppError> {
    blocking(app, move |s| import_undo(s, batch_id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_pending_decisions(app: AppHandle) -> Result<usize, AppError> {
    blocking(app, pending_decisions).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_holdings(app: AppHandle) -> Result<Holdings, AppError> {
    blocking(app, holdings).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_overview(app: AppHandle) -> Result<Overview, AppError> {
    blocking(app, overview).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_contributions(app: AppHandle) -> Result<Contributions, AppError> {
    blocking(app, |s| contributions(s, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_accounts(app: AppHandle) -> Result<Vec<AccountView>, AppError> {
    blocking(app, accounts).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_storage::Store;
    use std::path::Path;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/mizan-core/tests/data/e2e")
            .join(name)
    }

    fn seeded() -> AppState {
        let state = AppState::new(Store::open_in_memory().unwrap());
        import_open(&state, fixture("seed.json")).unwrap();
        import_create_account(&state, "DEG-4417".into()).unwrap();
        import_create_asset(&state, "isin:NL0010273215".into()).unwrap();
        import_create_asset(&state, "isin:IE00BK5BQT80".into()).unwrap();
        import_commit(&state).unwrap();
        state
    }

    #[test]
    fn read_commands_over_seeded_store() {
        let state = seeded();
        assert!(
            import_current(&state).unwrap().is_none(),
            "commit closes the review"
        );

        let h = holdings(&state).unwrap();
        assert_eq!(h.accounts[0].name, "Degiro");
        assert_eq!(h.accounts[0].positions[0].quantity, "2");

        let accts = accounts(&state).unwrap();
        assert_eq!(accts[0].hints, vec!["DEG-4417".to_string()]);

        let c = contributions(&state, Date::parse("2026-11-01").unwrap()).unwrap();
        assert_eq!(c.periods[0].total[0].amount, "+5000.00");

        let hist = import_history(&state).unwrap();
        assert_eq!((hist.len(), hist[0].activity_count), (1, 3));
    }

    #[test]
    fn overview_includes_pending_decisions_from_open_review() {
        let state = seeded();
        assert!(!overview(&state)
            .unwrap()
            .attention
            .iter()
            .any(|a| matches!(a, AttentionItem::PendingDecisions { .. })));
        import_open(&state, fixture("batch.json")).unwrap();
        assert_eq!(pending_decisions(&state).unwrap(), 1);
        let o = overview(&state).unwrap();
        assert!(matches!(
            o.attention[0],
            AttentionItem::PendingDecisions { count: 1 }
        ));
        import_close(&state).unwrap();
        assert_eq!(pending_decisions(&state).unwrap(), 0);
    }

    #[test]
    fn undo_command_and_errors_without_session() {
        let state = seeded();
        let id = import_history(&state).unwrap()[0].id;
        import_undo(&state, id).unwrap();
        assert!(import_history(&state).unwrap().is_empty());
        assert!(holdings(&state).unwrap().accounts[0].positions.is_empty());
        assert!(import_undo(&state, id).is_err());
        assert!(import_commit(&state).is_err(), "no review open");
    }
}
