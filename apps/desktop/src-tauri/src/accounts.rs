//! Accounts screen and account page commands: list, detail, the current-units
//! chart, and management (rename, identifiers, delete, merge and undo).

use mizan_core::accounts::{
    account_detail, accounts_list, current_units_chart, set_second_currency, AccountDetail,
    AccountsList, ChartPeriod, CurrentUnitsChart, DetailInputs,
};
use mizan_core::returns::{returns_view, Inputs};
use mizan_core::valuation::BookMarket;
use mizan_import::Date;
use serde::Serialize;
use tauri::AppHandle;

use crate::commands::{blocking, price_book, store_err};
use crate::market::{bump_generation, with_history};
use crate::session::AppError;
use crate::state::AppState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MergePreview {
    #[cfg_attr(test, ts(type = "number"))]
    pub activities: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub balance_checks: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub identifiers: i64,
}

fn today() -> Date {
    Date(chrono::Local::now().date_naive())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn list(state: &AppState, today: Date) -> Result<AccountsList, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    let book = price_book(state, &store)?;
    let market = BookMarket {
        store: &store,
        book: &book,
    };
    Ok(accounts_list(&store, &market, today)?)
}

pub fn detail(state: &AppState, id: i64, today: Date) -> Result<AccountDetail, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    let inputs = with_history(state, |cache| {
        Ok(Inputs::load(&store, cache.ok().map(|c| &*c))?)
    })?;
    let series = inputs.series(today);
    let view = returns_view(
        &series,
        &inputs.book,
        &inputs.base,
        None,
        &inputs.inflation,
        today,
    );
    let market = BookMarket {
        store: &store,
        book: &inputs.book,
    };
    let i = DetailInputs {
        store: &store,
        market: &market,
        book: &inputs.book,
        series: &series,
        returns: &view,
        today,
    };
    Ok(account_detail(&i, id)?)
}

pub fn chart(
    state: &AppState,
    id: i64,
    period: ChartPeriod,
    currency: &str,
    today: Date,
) -> Result<CurrentUnitsChart, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    let book = price_book(state, &store)?;
    let market = BookMarket {
        store: &store,
        book: &book,
    };
    Ok(current_units_chart(
        &store, &market, &book, id, period, currency, today,
    )?)
}

/// Run a write on the store and refresh every read model.
fn write<T>(
    state: &AppState,
    f: impl FnOnce(&mut mizan_storage::Store) -> Result<T, mizan_storage::StorageError>,
) -> Result<T, AppError> {
    let out = {
        let mut store = state.store.lock().map_err(store_err)?;
        f(&mut store)?
    };
    bump_generation(state);
    Ok(out)
}

pub fn rename(state: &AppState, id: i64, name: &str) -> Result<(), AppError> {
    write(state, |s| s.rename_account(id, name))
}

pub fn add_identifier(state: &AppState, id: i64, hint: &str) -> Result<(), AppError> {
    write(state, |s| s.add_account_hint(id, hint))
}

pub fn move_identifier(state: &AppState, hint: &str, to: i64) -> Result<(), AppError> {
    write(state, |s| s.move_account_hint(hint, to))
}

pub fn delete(state: &AppState, id: i64) -> Result<(), AppError> {
    write(state, |s| s.delete_account(id))
}

pub fn merge_preview(state: &AppState, from: i64) -> Result<MergePreview, AppError> {
    let c = state.store.lock().map_err(store_err)?.merge_preview(from)?;
    Ok(MergePreview {
        activities: c.activities,
        balance_checks: c.assertions,
        identifiers: c.hints,
    })
}

pub fn merge(state: &AppState, from: i64, into: i64, now: &str) -> Result<i64, AppError> {
    write(state, |s| {
        let id = s.merge_accounts(from, into, now)?;
        mizan_core::assets::apply_account_sources(s)?;
        Ok(id)
    })
}

/// What setting an account's price provider did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PriceSourceOutcome {
    /// Assets that got a price source now.
    #[cfg_attr(test, ts(type = "number"))]
    pub set: usize,
    /// Assets left as they were: no usable symbol for the provider.
    pub skipped: Vec<String>,
}

/// Set (or clear) the account's price provider and give its assets without a
/// source one now. Sources chosen per asset are kept.
pub fn set_price_source(
    state: &AppState,
    id: i64,
    provider: Option<&str>,
) -> Result<PriceSourceOutcome, AppError> {
    write(state, |s| {
        s.set_account_price_source(id, provider)?;
        let applied = mizan_core::assets::apply_account_sources(s)?;
        let mine = s.account_asset_ids(id)?;
        Ok(PriceSourceOutcome {
            set: applied.set.iter().filter(|a| mine.contains(a)).count(),
            skipped: applied.skipped,
        })
    })
}

/// Undo a merge; returns identifiers that had moved elsewhere and stayed.
pub fn undo_merge(state: &AppState, merge_id: i64, now: &str) -> Result<Vec<String>, AppError> {
    write(state, |s| s.undo_account_merge(merge_id, now))
}

pub fn second_currency(state: &AppState, code: &str) -> Result<(), AppError> {
    write(state, |s| set_second_currency(s, code))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_accounts_list(app: AppHandle) -> Result<AccountsList, AppError> {
    blocking(app, |s| list(s, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_detail(app: AppHandle, id: i64) -> Result<AccountDetail, AppError> {
    blocking(app, move |s| detail(s, id, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_chart(
    app: AppHandle,
    id: i64,
    period: ChartPeriod,
    currency: String,
) -> Result<CurrentUnitsChart, AppError> {
    blocking(app, move |s| chart(s, id, period, &currency, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_rename(app: AppHandle, id: i64, name: String) -> Result<(), AppError> {
    blocking(app, move |s| rename(s, id, &name)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_add_identifier(
    app: AppHandle,
    id: i64,
    identifier: String,
) -> Result<(), AppError> {
    blocking(app, move |s| add_identifier(s, id, &identifier)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_move_identifier(
    app: AppHandle,
    identifier: String,
    to: i64,
) -> Result<(), AppError> {
    blocking(app, move |s| move_identifier(s, &identifier, to)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_delete(app: AppHandle, id: i64) -> Result<(), AppError> {
    blocking(app, move |s| delete(s, id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_merge_preview(
    app: AppHandle,
    from: i64,
) -> Result<MergePreview, AppError> {
    blocking(app, move |s| merge_preview(s, from)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_merge(app: AppHandle, from: i64, into: i64) -> Result<i64, AppError> {
    blocking(app, move |s| merge(s, from, into, &now())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_undo_merge(
    app: AppHandle,
    merge_id: i64,
) -> Result<Vec<String>, AppError> {
    blocking(app, move |s| undo_merge(s, merge_id, &now())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_set_second_currency(app: AppHandle, currency: String) -> Result<(), AppError> {
    blocking(app, move |s| second_currency(s, &currency)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_account_set_price_source(
    app: AppHandle,
    id: i64,
    provider: Option<String>,
) -> Result<PriceSourceOutcome, AppError> {
    blocking(app, move |s| set_price_source(s, id, provider.as_deref())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_core::{commit, prepare, resolve_batch, CreatePolicy, Decisions, MappingResolver};
    use mizan_storage::{CachedFundPrice, PriceSourceKind, Store};

    fn import(store: &mut Store, file: char, hint: &str, sym: &str) {
        let recs = serde_json::json!([
            {"kind": "deposit", "date": "2026-01-05", "net_cash": {"amount": "1000.00", "currency": "TRY"}},
            {"kind": "buy", "date": "2026-01-05", "instrument": {"symbol": sym, "class_hint": "fund", "currency": "TRY"}, "quantity": "1000", "net_cash": {"amount": "1000.00", "currency": "TRY"}}
        ]);
        let mut recs = recs.as_array().unwrap().clone();
        for (i, r) in recs.iter_mut().enumerate() {
            r["source"] = serde_json::json!({"adapter": "t", "adapter_version": "1", "file_sha256": file.to_string().repeat(64), "locator": format!("row {}", i + 1)});
            r["account"] = serde_json::json!({"hint": hint, "name_hint": hint});
        }
        let parsed = mizan_import::parse_batch_value(
            serde_json::json!({"schema": "mizan.import/1", "records": recs}),
        )
        .unwrap();
        let rb = {
            let mut r = MappingResolver {
                store: &mut *store,
                policy: CreatePolicy {
                    create_accounts: true,
                    create_assets: true,
                },
            };
            resolve_batch(parsed, &mut r).unwrap()
        };
        let p = prepare(store, rb).unwrap();
        commit(store, &p, &Decisions::new()).unwrap();
    }

    fn state() -> AppState {
        let mut store = Store::open_in_memory().unwrap();
        import(&mut store, 'a', "P1", "AH2");
        import(&mut store, 'b', "P1-DK", "AET");
        store.create_account("TR Bank", "TB1").unwrap();
        for a in store.list_assets().unwrap() {
            let code = a.hint.symbol.clone().unwrap();
            store
                .set_price_source(a.id, Some(&PriceSourceKind::Tefas { fund_code: code }))
                .unwrap();
        }
        let st = AppState::new(store);
        {
            let mut c = st.history_mem.lock().unwrap();
            for code in ["AH2", "AET"] {
                for (d, p) in [("2026-01-05", "1.00"), ("2026-06-01", "1.10")] {
                    c.upsert_fund_price(
                        &CachedFundPrice {
                            code: code.into(),
                            date: d.into(),
                            price: p.into(),
                        },
                        "t",
                    )
                    .unwrap();
                }
            }
        }
        st
    }

    fn day() -> Date {
        Date::parse("2026-06-02").unwrap()
    }

    fn id_of(st: &AppState, name: &str) -> i64 {
        list(st, day())
            .unwrap()
            .accounts
            .into_iter()
            .chain(list(st, day()).unwrap().merged)
            .find(|a| a.name == name)
            .unwrap()
            .id
    }

    #[test]
    fn rename_shows_everywhere_and_delete_refuses_non_empty() {
        let st = state();
        let p1 = id_of(&st, "P1");
        rename(&st, p1, "Anadolu Hayat").unwrap();
        assert_eq!(
            crate::commands::holdings(&st, day()).unwrap().accounts[0].name,
            "Anadolu Hayat"
        );
        assert_eq!(detail(&st, p1, day()).unwrap().name, "Anadolu Hayat");
        assert!(delete(&st, p1).is_err());
        let tb = id_of(&st, "TR Bank");
        assert_eq!(detail(&st, tb, day()).unwrap().delete_blocker, None);
        delete(&st, tb).unwrap();
        assert_eq!(list(&st, day()).unwrap().accounts.len(), 2);
    }

    #[test]
    fn merge_combines_holdings_hides_the_account_and_undo_restores() {
        let st = state();
        let (p1, dk) = (id_of(&st, "P1"), id_of(&st, "P1-DK"));
        let pv = merge_preview(&st, dk).unwrap();
        assert_eq!(
            (pv.activities, pv.balance_checks, pv.identifiers),
            (2, 0, 1)
        );
        let m = merge(&st, dk, p1, "t").unwrap();
        let h = crate::commands::holdings(&st, day()).unwrap();
        assert_eq!(h.accounts.len(), 2, "P1 and TR Bank");
        let p1h = h.accounts.iter().find(|a| a.account_id == p1).unwrap();
        assert_eq!(p1h.positions.len(), 2, "both funds in one account");
        assert!(crate::commands::overview(&st, day())
            .unwrap()
            .accounts
            .iter()
            .all(|a| a.account_id != dk));
        assert_eq!(crate::tax::report_accounts_for_tests(&st), 2);
        let l = list(&st, day()).unwrap();
        assert_eq!(l.merged.len(), 1);
        assert_eq!(l.merged[0].merge.as_ref().unwrap().into_name, "P1");
        assert!(undo_merge(&st, m, "t").unwrap().is_empty());
        assert_eq!(
            crate::commands::holdings(&st, day())
                .unwrap()
                .accounts
                .len(),
            3
        );
    }

    #[test]
    fn identifiers_chart_and_second_currency() {
        let st = state();
        let (p1, tb) = (id_of(&st, "P1"), id_of(&st, "TR Bank"));
        add_identifier(&st, p1, "U1234567").unwrap();
        assert!(add_identifier(&st, tb, "U1234567")
            .unwrap_err()
            .to_string()
            .contains("already belongs to P1"));
        move_identifier(&st, "U1234567", tb).unwrap();
        assert!(detail(&st, tb, day())
            .unwrap()
            .identifiers
            .contains(&"U1234567".to_string()));
        let c = chart(&st, p1, ChartPeriod::SixMonths, "TRY", day()).unwrap();
        assert_eq!(c.bands.len(), 1);
        assert_eq!(
            c.bands[0].values.last().unwrap().as_deref(),
            Some("1100.00")
        );
        second_currency(&st, "EUR").unwrap();
        assert_eq!(
            detail(&st, p1, day()).unwrap().summary.second_currency,
            "EUR"
        );
        assert!(second_currency(&st, "JPY").is_err());
    }

    #[test]
    fn account_price_source_fills_assets_without_one() {
        let mut store = Store::open_in_memory().unwrap();
        import(&mut store, 'a', "P1", "AH1");
        import(&mut store, 'b', "P1", "AET");
        let aet = store
            .list_assets()
            .unwrap()
            .into_iter()
            .find(|a| a.hint.symbol.as_deref() == Some("AET"))
            .unwrap();
        let chosen = PriceSourceKind::Yahoo {
            symbol: "AET.IS".into(),
        };
        store.set_price_source(aet.id, Some(&chosen)).unwrap();
        let account = store.list_accounts().unwrap()[0].id;
        let st = AppState::new(store);

        let out = set_price_source(&st, account, Some("tefas")).unwrap();
        assert_eq!(out.set, 1);
        let mut store = st.store.lock().unwrap();
        let sources = store.price_sources().unwrap();
        assert!(sources.values().any(|s| *s
            == PriceSourceKind::Tefas {
                fund_code: "AH1".into()
            }));
        assert_eq!(
            sources[&aet.id], chosen,
            "a source chosen per asset is kept"
        );

        // A later import into the same account picks up the default.
        import(&mut store, 'c', "P1", "AH5");
        mizan_core::assets::apply_account_sources(&mut store).unwrap();
        assert!(store.price_sources().unwrap().values().any(|s| *s
            == PriceSourceKind::Tefas {
                fund_code: "AH5".into()
            }));
        assert_eq!(
            store.account_price_source(account).unwrap().as_deref(),
            Some("tefas")
        );
        drop(store);
        assert!(set_price_source(&st, account, Some("bogus")).is_err());
        set_price_source(&st, account, None).unwrap();
    }
}
