//! The Income screen and its CSV export.

use std::path::{Path, PathBuf};

use mizan_core::income::{income_csv, income_file_name, income_view, IncomeView};
use mizan_core::portfolio::{AccountScope, Period};
use mizan_import::Date;
use tauri::AppHandle;

use crate::commands::{blocking, price_book, store_err};
use crate::session::AppError;
use crate::state::AppState;

pub fn income(
    state: &AppState,
    scope: AccountScope,
    period: Period,
    today: Date,
) -> Result<IncomeView, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    let book = price_book(state, &store)?;
    Ok(income_view(&store, &book, scope, period, today)?)
}

fn scope_name(state: &AppState, scope: AccountScope) -> Result<String, AppError> {
    Ok(match scope {
        AccountScope::All => "All accounts".into(),
        AccountScope::One(id) => state
            .store
            .lock()
            .map_err(store_err)?
            .list_accounts()?
            .into_iter()
            .find(|a| a.id == id)
            .map(|a| a.name)
            .unwrap_or_else(|| format!("account {id}")),
    })
}

/// Write the payments behind the Income view to `dir`; returns the file path.
pub fn export_income_csv(
    state: &AppState,
    scope: AccountScope,
    period: Period,
    dir: &Path,
    today: Date,
) -> Result<String, AppError> {
    let v = income(state, scope, period, today)?;
    let path: PathBuf = dir.join(income_file_name(&v, &scope_name(state, scope)?));
    std::fs::write(&path, income_csv(&v)).map_err(|e| AppError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    Ok(path.display().to_string())
}

fn today() -> Date {
    Date(chrono::Local::now().date_naive())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_income(
    app: AppHandle,
    period: Period,
    account_id: Option<i64>,
) -> Result<IncomeView, AppError> {
    blocking(app, move |s| {
        income(s, AccountScope::from_option(account_id), period, today())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_export_income_csv(
    app: AppHandle,
    period: Period,
    account_id: Option<i64>,
    dir: PathBuf,
) -> Result<String, AppError> {
    blocking(app, move |s| {
        export_income_csv(
            s,
            AccountScope::from_option(account_id),
            period,
            &dir,
            today(),
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_storage::Store;

    #[test]
    fn export_writes_a_csv_named_for_the_scope() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.create_account("My Bank", "B1").unwrap();
        let st = AppState::new(store);
        let dir = std::env::temp_dir().join(format!("mizan-income-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let today = Date::parse("2026-11-02").unwrap();
        let path = export_income_csv(&st, AccountScope::One(id), Period::Ytd, &dir, today).unwrap();
        assert!(
            path.ends_with("mizan-income-my-bank-2026-01-01-2026-11-02.csv"),
            "{path}"
        );
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("date,account,instrument,type"));
        assert_eq!(body.lines().count(), 1, "header only without income");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(export_income_csv(
            &st,
            AccountScope::All,
            Period::Ytd,
            Path::new("/nonexistent/x"),
            today
        )
        .is_err());
    }
}
