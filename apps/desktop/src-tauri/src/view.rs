//! The view bar's shared choices: account scope, period and Nominal | Real,
//! kept in the user database so they survive a restart.

use mizan_core::portfolio::{AccountScope, Period};
use mizan_storage::Store;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::commands::{blocking, store_err};
use crate::session::AppError;
use crate::state::AppState;

pub const SCOPE_KEY: &str = "view.scope";
pub const PERIOD_KEY: &str = "view.period";
pub const REAL_KEY: &str = "view.real";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ViewState {
    /// The account in scope; `None` for all accounts.
    #[cfg_attr(test, ts(type = "number | null"))]
    pub account_id: Option<i64>,
    pub period: Period,
    pub real: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            account_id: None,
            period: Period::Ytd,
            real: false,
        }
    }
}

impl ViewState {
    pub fn scope(&self) -> AccountScope {
        AccountScope::from_option(self.account_id)
    }
}

fn period_str(p: Period) -> String {
    serde_json::to_value(p)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "ytd".into())
}

/// The stored view. A scoped account that was merged away or deleted falls
/// back to all accounts, and the stored choice is rewritten.
pub fn load(store: &mut Store) -> Result<ViewState, AppError> {
    let mut v = ViewState::default();
    if let Some(p) = store.setting(PERIOD_KEY)? {
        if let Ok(p) = serde_json::from_value(serde_json::Value::String(p)) {
            v.period = p;
        }
    }
    v.real = store.setting(REAL_KEY)?.as_deref() == Some("true");
    if let Some(id) = store
        .setting(SCOPE_KEY)?
        .and_then(|s| s.parse::<i64>().ok())
    {
        if store.list_accounts()?.iter().any(|a| a.id == id) {
            v.account_id = Some(id);
        } else {
            store.set_setting(SCOPE_KEY, "all")?;
        }
    }
    Ok(v)
}

/// The scope to compute with right now.
pub fn current_scope(state: &AppState) -> Result<AccountScope, AppError> {
    let mut store = state.store.lock().map_err(store_err)?;
    Ok(load(&mut store)?.scope())
}

pub fn view_get(state: &AppState) -> Result<ViewState, AppError> {
    let mut store = state.store.lock().map_err(store_err)?;
    load(&mut store)
}

pub fn view_set(state: &AppState, view: ViewState) -> Result<ViewState, AppError> {
    let mut store = state.store.lock().map_err(store_err)?;
    if let Some(id) = view.account_id {
        if !store.list_accounts()?.iter().any(|a| a.id == id) {
            return Err(AppError::Invalid(format!("account {id} does not exist")));
        }
    }
    let scope = view
        .account_id
        .map_or_else(|| "all".to_string(), |id| id.to_string());
    store.set_setting(SCOPE_KEY, &scope)?;
    store.set_setting(PERIOD_KEY, &period_str(view.period))?;
    store.set_setting(REAL_KEY, if view.real { "true" } else { "false" })?;
    load(&mut store)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_view_get(app: AppHandle) -> Result<ViewState, AppError> {
    blocking(app, view_get).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_view_set(app: AppHandle, view: ViewState) -> Result<ViewState, AppError> {
    blocking(app, move |s| view_set(s, view)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_storage::Store;

    fn state_with_accounts(names: &[&str]) -> (AppState, Vec<i64>) {
        let mut store = Store::open_in_memory().unwrap();
        let ids = names
            .iter()
            .enumerate()
            .map(|(i, n)| store.create_account(n, &format!("H{i}")).unwrap())
            .collect();
        (AppState::new(store), ids)
    }

    #[test]
    fn defaults_to_all_accounts_ytd_nominal() {
        let (s, _) = state_with_accounts(&[]);
        assert_eq!(view_get(&s).unwrap(), ViewState::default());
    }

    #[test]
    fn choices_are_kept() {
        let (s, ids) = state_with_accounts(&["Pension", "IBKR"]);
        let v = ViewState {
            account_id: Some(ids[0]),
            period: Period::ThreeMonths,
            real: true,
        };
        assert_eq!(view_set(&s, v).unwrap(), v);
        assert_eq!(view_get(&s).unwrap(), v, "read back as a restart would");
        let stored = s.store.lock().unwrap().setting(PERIOD_KEY).unwrap();
        assert_eq!(stored.as_deref(), Some("three_months"));
    }

    #[test]
    fn unknown_account_is_refused() {
        let (s, _) = state_with_accounts(&["A"]);
        let v = ViewState {
            account_id: Some(999),
            ..ViewState::default()
        };
        assert!(view_set(&s, v).is_err());
    }

    #[test]
    fn deleted_scoped_account_falls_back_to_all() {
        let (s, ids) = state_with_accounts(&["A", "B"]);
        view_set(
            &s,
            ViewState {
                account_id: Some(ids[1]),
                ..ViewState::default()
            },
        )
        .unwrap();
        s.store.lock().unwrap().delete_account(ids[1]).unwrap();
        assert_eq!(view_get(&s).unwrap().account_id, None);
        let stored = s.store.lock().unwrap().setting(SCOPE_KEY).unwrap();
        assert_eq!(stored.as_deref(), Some("all"));
    }

    #[test]
    fn merged_scoped_account_falls_back_to_all() {
        let (s, ids) = state_with_accounts(&["A", "B"]);
        view_set(
            &s,
            ViewState {
                account_id: Some(ids[0]),
                ..ViewState::default()
            },
        )
        .unwrap();
        s.store
            .lock()
            .unwrap()
            .merge_accounts(ids[0], ids[1], "2026-11-02T10:00:00Z")
            .unwrap();
        assert_eq!(view_get(&s).unwrap().account_id, None);
    }
}
