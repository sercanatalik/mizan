//! Activity screen commands: the list, a record's detail, and manual entries.

use mizan_core::activity::{
    check_form, delete_entry, detail, page, revert_entry, save_form, ActivityDetail, ActivityForm,
    ActivityPage, ActivityQuery, FormCheck, ManualContext, SaveError, MANUAL_KINDS,
};
use serde::Serialize;
use tauri::AppHandle;

use crate::commands::{blocking, store_err};
use crate::market::bump_generation;
use crate::session::AppError;
use crate::state::AppState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Choice {
    #[cfg_attr(test, ts(type = "number"))]
    pub id: i64,
    pub label: String,
    /// Assets: the quoted currency, when known.
    pub currency: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActivityOptions {
    pub accounts: Vec<Choice>,
    /// Surviving assets only; merged ones are offered as the asset they joined.
    pub assets: Vec<Choice>,
    /// Kinds stored so far, for the filter.
    pub kinds: Vec<String>,
    /// Kinds the manual form offers.
    pub manual_kinds: Vec<String>,
}

/// The result of saving: the new activity's id, or why nothing was saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SaveOutcome {
    #[cfg_attr(test, ts(type = "number | null"))]
    pub saved_id: Option<i64>,
    pub check: FormCheck,
    /// The entry may be a duplicate and was not saved; confirm to save it.
    pub needs_confirmation: bool,
}

fn ctx(now: &str) -> ManualContext {
    ManualContext {
        app_version: env!("CARGO_PKG_VERSION").into(),
        now: now.into(),
    }
}

pub fn activities(
    state: &AppState,
    q: &ActivityQuery,
    offset: i64,
) -> Result<ActivityPage, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    Ok(page(&store, q, offset)?)
}

pub fn activity_detail(state: &AppState, id: i64) -> Result<ActivityDetail, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    Ok(detail(&store, id)?)
}

pub fn activity_options(state: &AppState) -> Result<ActivityOptions, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    let accounts = store
        .list_accounts()?
        .into_iter()
        .map(|a| Choice {
            id: a.id,
            label: a.name,
            currency: None,
        })
        .collect();
    let mut assets = Vec::new();
    for a in store.list_assets()? {
        if store.canonical_asset(a.id)? != a.id {
            continue;
        }
        let label = match (&a.hint.symbol, a.name.as_ref().or(a.hint.name.as_ref())) {
            (Some(s), Some(n)) if s != n => format!("{s} · {n}"),
            (Some(s), _) => s.clone(),
            (None, Some(n)) => n.clone(),
            (None, None) => a.instrument_key.clone(),
        };
        assets.push(Choice {
            id: a.id,
            label,
            currency: a
                .currency
                .clone()
                .or(a.hint.currency.as_ref().map(|c| c.to_string())),
        });
    }
    assets.sort_by_key(|a| a.label.to_lowercase());
    let mut kinds: Vec<String> = Vec::new();
    for a in store.list_accounts()? {
        for s in store.all_activities(a.id)? {
            let k = s.record.kind.as_str().to_string();
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
    }
    kinds.sort();
    Ok(ActivityOptions {
        accounts,
        assets,
        kinds,
        manual_kinds: MANUAL_KINDS.iter().map(|k| k.to_string()).collect(),
    })
}

pub fn activity_check(
    state: &AppState,
    form: &ActivityForm,
    replace_id: Option<i64>,
    now: &str,
) -> Result<FormCheck, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    Ok(check_form(&store, form, &ctx(now), replace_id)?)
}

pub fn activity_save(
    state: &AppState,
    form: &ActivityForm,
    confirm_duplicate: bool,
    replace_id: Option<i64>,
    now: &str,
) -> Result<SaveOutcome, AppError> {
    let result = {
        let mut store = state.store.lock().map_err(store_err)?;
        let saved = save_form(&mut store, form, &ctx(now), confirm_duplicate, replace_id);
        if saved.is_ok() {
            mizan_core::assets::apply_account_sources(&mut store)?;
        }
        saved
    };
    match result {
        Ok(id) => {
            bump_generation(state);
            Ok(SaveOutcome {
                saved_id: Some(id),
                check: FormCheck {
                    errors: vec![],
                    warnings: vec![],
                    duplicate: None,
                },
                needs_confirmation: false,
            })
        }
        Err(SaveError::Invalid(check)) => Ok(SaveOutcome {
            saved_id: None,
            check,
            needs_confirmation: false,
        }),
        Err(SaveError::NeedsConfirmation(check)) => Ok(SaveOutcome {
            saved_id: None,
            check,
            needs_confirmation: true,
        }),
        Err(SaveError::Storage(e)) => Err(e.into()),
        Err(SaveError::Commit(m)) => Err(AppError::Commit(m)),
    }
}

pub fn activity_delete(state: &AppState, id: i64) -> Result<(), AppError> {
    {
        let mut store = state.store.lock().map_err(store_err)?;
        delete_entry(&mut store, id)?;
    }
    bump_generation(state);
    Ok(())
}

pub fn activity_revert(state: &AppState, id: i64) -> Result<(), AppError> {
    {
        let mut store = state.store.lock().map_err(store_err)?;
        revert_entry(&mut store, id)?;
    }
    bump_generation(state);
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_activities(
    app: AppHandle,
    query: ActivityQuery,
    offset: i64,
) -> Result<ActivityPage, AppError> {
    blocking(app, move |s| activities(s, &query, offset)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_activity_detail(app: AppHandle, id: i64) -> Result<ActivityDetail, AppError> {
    blocking(app, move |s| activity_detail(s, id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_activity_options(app: AppHandle) -> Result<ActivityOptions, AppError> {
    blocking(app, activity_options).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_activity_check(
    app: AppHandle,
    form: ActivityForm,
    replace_id: Option<i64>,
) -> Result<FormCheck, AppError> {
    blocking(app, move |s| activity_check(s, &form, replace_id, &now())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_activity_save(
    app: AppHandle,
    form: ActivityForm,
    confirm_duplicate: bool,
    replace_id: Option<i64>,
) -> Result<SaveOutcome, AppError> {
    blocking(app, move |s| {
        activity_save(s, &form, confirm_duplicate, replace_id, &now())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_activity_delete(app: AppHandle, id: i64) -> Result<(), AppError> {
    blocking(app, move |s| activity_delete(s, id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_activity_revert(app: AppHandle, id: i64) -> Result<(), AppError> {
    blocking(app, move |s| activity_revert(s, id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_core::activity::NewAssetForm;
    use mizan_core::{commit, prepare, resolve_batch, CreatePolicy, Decisions, MappingResolver};
    use mizan_import::Date;
    use mizan_storage::Store;

    fn state() -> AppState {
        let mut store = Store::open_in_memory().unwrap();
        let recs = serde_json::json!([
            {"kind": "buy", "date": "2025-01-03", "instrument": {"symbol": "VWCE", "isin": "IE00BK5BQT80", "currency": "EUR"}, "quantity": "5", "net_cash": {"amount": "550.00", "currency": "EUR"}}
        ]);
        let mut recs = recs.as_array().unwrap().clone();
        for (i, r) in recs.iter_mut().enumerate() {
            r["source"] = serde_json::json!({"adapter": "t", "adapter_version": "1", "file_sha256": "a".repeat(64), "locator": format!("row {}", i + 1)});
            r["account"] = serde_json::json!({"hint": "D1", "name_hint": "Degiro"});
        }
        let parsed = mizan_import::parse_batch_value(
            serde_json::json!({"schema": "mizan.import/1", "records": recs}),
        )
        .unwrap();
        let rb = {
            let mut r = MappingResolver {
                store: &mut store,
                policy: CreatePolicy {
                    create_accounts: true,
                    create_assets: true,
                },
            };
            resolve_batch(parsed, &mut r).unwrap()
        };
        let p = prepare(&store, rb).unwrap();
        commit(&mut store, &p, &Decisions::new()).unwrap();
        AppState::new(store)
    }

    fn held(st: &AppState) -> String {
        let h = mizan_core::portfolio::holdings(
            &st.store.lock().unwrap(),
            Date::parse("2026-11-02").unwrap(),
        )
        .unwrap();
        h.accounts[0].positions[0].quantity.clone()
    }

    #[test]
    fn add_edit_delete_round_trip() {
        let st = state();
        let opts = activity_options(&st).unwrap();
        assert_eq!(opts.accounts[0].label, "Degiro");
        assert_eq!(opts.assets[0].currency.as_deref(), Some("EUR"));
        assert_eq!(opts.kinds, ["buy"]);
        assert_eq!(opts.manual_kinds.len(), 11);

        let generation = st.generation.load(std::sync::atomic::Ordering::SeqCst);
        let mut f = ActivityForm {
            kind: "sell".into(),
            date: "2025-06-02".into(),
            account_id: Some(opts.accounts[0].id),
            asset_id: Some(opts.assets[0].id),
            currency: "EUR".into(),
            quantity: Some("2".into()),
            price: Some("120".into()),
            ..Default::default()
        };
        let out = activity_save(&st, &f, false, None, "2026-11-02T10:00:00Z").unwrap();
        let id = out.saved_id.unwrap();
        assert!(st.generation.load(std::sync::atomic::Ordering::SeqCst) > generation);
        assert_eq!(held(&st), "3", "Holdings sees the manual sell");

        f.quantity = Some("1".into());
        let out = activity_save(&st, &f, false, Some(id), "t").unwrap();
        let new_id = out.saved_id.unwrap();
        assert_eq!(held(&st), "4");
        assert_eq!(
            activity_detail(&st, new_id)
                .unwrap()
                .row
                .quantity
                .as_deref(),
            Some("1")
        );

        activity_delete(&st, new_id).unwrap();
        assert_eq!(held(&st), "5");
        let p = activities(&st, &ActivityQuery::default(), 0).unwrap();
        assert_eq!(p.total, 1);
    }

    #[test]
    fn invalid_entry_and_imported_rows() {
        let st = state();
        let f = ActivityForm {
            kind: "buy".into(),
            date: "2025-06-02".into(),
            new_account: Some("Old broker".into()),
            new_asset: Some(NewAssetForm {
                symbol: Some("ASML".into()),
                ..Default::default()
            }),
            currency: "EUR".into(),
            ..Default::default()
        };
        let out = activity_save(&st, &f, false, None, "t").unwrap();
        assert!(out.saved_id.is_none());
        assert!(out.check.errors.iter().any(|e| e.field == "quantity"));
        assert_eq!(
            activity_options(&st).unwrap().accounts.len(),
            1,
            "nothing was created"
        );

        let imported = activities(&st, &ActivityQuery::default(), 0).unwrap().rows[0].id;
        assert!(activity_delete(&st, imported).is_err());
        assert!(activity_detail(&st, imported).unwrap().import.is_some());

        // Imported activities are amended and can be reverted.
        let mut form = activity_detail(&st, imported).unwrap().form;
        form.quantity = Some("6".into());
        let out = activity_save(&st, &form, false, Some(imported), "t").unwrap();
        assert_eq!(out.saved_id, Some(imported));
        assert_eq!(held(&st), "6");
        assert!(activity_detail(&st, imported).unwrap().row.source.amended);
        activity_revert(&st, imported).unwrap();
        assert_eq!(held(&st), "5");
    }
}
