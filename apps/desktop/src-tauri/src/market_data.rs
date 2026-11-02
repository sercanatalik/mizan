//! Market data screen commands: series lists, one series in detail, edits and
//! CSV export. The shared history cache is only read; edits go to this database.

use std::path::{Path, PathBuf};

use mizan_core::market_data::{
    csv_file_name, ignore_value, revert_value, series_csv, series_detail, series_page, set_value,
    SeriesDetail, SeriesFilter, SeriesPage,
};
use mizan_import::Date;
use tauri::AppHandle;

use crate::commands::{blocking, store_err};
use crate::market::{bump_generation, with_history};
use crate::session::AppError;
use crate::state::AppState;

fn today() -> Date {
    Date(chrono::Local::now().date_naive())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn market_series(
    state: &AppState,
    kind: &str,
    filter: &SeriesFilter,
    today: Date,
) -> Result<SeriesPage, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    with_history(state, |cache| {
        Ok(series_page(
            &store,
            cache.ok().map(|c| &*c),
            kind,
            filter,
            today,
        )?)
    })
}

pub fn market_series_detail(
    state: &AppState,
    kind: &str,
    key: &str,
    offset: i64,
    today: Date,
) -> Result<SeriesDetail, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    with_history(state, |cache| {
        Ok(series_detail(
            &store,
            cache.ok().map(|c| &*c),
            kind,
            key,
            offset,
            today,
        )?)
    })
}

pub fn market_set_value(
    state: &AppState,
    kind: &str,
    key: &str,
    period: &str,
    value: &str,
    currency: Option<&str>,
    now: &str,
) -> Result<(), AppError> {
    {
        let mut store = state.store.lock().map_err(store_err)?;
        set_value(&mut store, kind, key, period, value, currency, now)?;
    }
    bump_generation(state);
    Ok(())
}

pub fn market_ignore(
    state: &AppState,
    kind: &str,
    key: &str,
    period: &str,
    now: &str,
) -> Result<(), AppError> {
    {
        let mut store = state.store.lock().map_err(store_err)?;
        ignore_value(&mut store, kind, key, period, now)?;
    }
    bump_generation(state);
    Ok(())
}

pub fn market_revert(
    state: &AppState,
    kind: &str,
    key: &str,
    period: &str,
) -> Result<(), AppError> {
    {
        let mut store = state.store.lock().map_err(store_err)?;
        revert_value(&mut store, kind, key, period)?;
    }
    bump_generation(state);
    Ok(())
}

/// Write the series as CSV into `dir`; returns the file's path.
pub fn market_export_csv(
    state: &AppState,
    kind: &str,
    key: &str,
    dir: &Path,
) -> Result<String, AppError> {
    let body = {
        let store = state.store.lock().map_err(store_err)?;
        with_history(state, |cache| {
            Ok(series_csv(&store, cache.ok().map(|c| &*c), kind, key)?)
        })?
    };
    let path = dir.join(csv_file_name(kind, key));
    std::fs::write(&path, body).map_err(|e| AppError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    Ok(path.display().to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_market_series(
    app: AppHandle,
    kind: String,
    filter: SeriesFilter,
) -> Result<SeriesPage, AppError> {
    blocking(app, move |s| market_series(s, &kind, &filter, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_market_series_detail(
    app: AppHandle,
    kind: String,
    key: String,
    offset: i64,
) -> Result<SeriesDetail, AppError> {
    blocking(app, move |s| {
        market_series_detail(s, &kind, &key, offset, today())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_market_set_value(
    app: AppHandle,
    kind: String,
    key: String,
    period: String,
    value: String,
    currency: Option<String>,
) -> Result<(), AppError> {
    blocking(app, move |s| {
        market_set_value(s, &kind, &key, &period, &value, currency.as_deref(), &now())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_market_ignore(
    app: AppHandle,
    kind: String,
    key: String,
    period: String,
) -> Result<(), AppError> {
    blocking(app, move |s| market_ignore(s, &kind, &key, &period, &now())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_market_revert(
    app: AppHandle,
    kind: String,
    key: String,
    period: String,
) -> Result<(), AppError> {
    blocking(app, move |s| market_revert(s, &kind, &key, &period)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_market_export_csv(
    app: AppHandle,
    kind: String,
    key: String,
    dir: PathBuf,
) -> Result<String, AppError> {
    blocking(app, move |s| market_export_csv(s, &kind, &key, &dir)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_core::{commit, prepare, resolve_batch, CreatePolicy, Decisions, MappingResolver};
    use mizan_storage::{CachedFundPrice, PriceSourceKind, Store};

    /// A pension holding fund AH2, with two TEFAS prices in the cache.
    fn state() -> AppState {
        let mut store = Store::open_in_memory().unwrap();
        let recs = serde_json::json!([
            {"kind": "deposit", "date": "2026-01-05", "net_cash": {"amount": "1000.00", "currency": "TRY"}},
            {"kind": "buy", "date": "2026-01-05", "instrument": {"symbol": "AH2", "class_hint": "fund", "currency": "TRY"}, "quantity": "1000", "net_cash": {"amount": "1000.00", "currency": "TRY"}}
        ]);
        let mut recs = recs.as_array().unwrap().clone();
        for (i, r) in recs.iter_mut().enumerate() {
            r["source"] = serde_json::json!({"adapter": "t", "adapter_version": "1", "file_sha256": "a".repeat(64), "locator": format!("row {}", i + 1)});
            r["account"] = serde_json::json!({"hint": "P1", "name_hint": "Pension"});
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
        let asset = store.list_assets().unwrap()[0].id;
        store
            .set_price_source(
                asset,
                Some(&PriceSourceKind::Tefas {
                    fund_code: "AH2".into(),
                }),
            )
            .unwrap();
        let st = AppState::new(store);
        {
            let mut c = st.history_mem.lock().unwrap();
            for (d, p) in [("2026-01-05", "1.00"), ("2026-06-01", "1.10")] {
                c.upsert_fund_price(
                    &CachedFundPrice {
                        code: "AH2".into(),
                        date: d.into(),
                        price: p.into(),
                    },
                    "t",
                )
                .unwrap();
            }
        }
        st
    }

    fn day(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    fn holding_value(st: &AppState) -> Option<String> {
        crate::commands::holdings(st, day("2026-06-02"))
            .unwrap()
            .accounts[0]
            .positions[0]
            .value
            .clone()
    }

    #[test]
    fn edits_reach_holdings_and_performance_and_revert_restores() {
        let st = state();
        let p = market_series(&st, "price", &SeriesFilter::default(), day("2026-06-02")).unwrap();
        assert_eq!(p.rows[0].key, "tefas:AH2");
        assert_eq!(holding_value(&st).as_deref(), Some("1100.00"));
        let g = st.generation.load(std::sync::atomic::Ordering::SeqCst);

        market_set_value(
            &st,
            "price",
            "tefas:AH2",
            "2026-06-01",
            "1.20",
            Some("TRY"),
            "t",
        )
        .unwrap();
        assert!(st.generation.load(std::sync::atomic::Ordering::SeqCst) > g);
        assert_eq!(holding_value(&st).as_deref(), Some("1200.00"));
        let perf = crate::returns::history(&st, day("2026-06-02")).unwrap();
        assert_eq!(
            perf.points.last().unwrap().value.as_deref(),
            Some("1200.00")
        );
        let d = market_series_detail(&st, "price", "tefas:AH2", 0, day("2026-06-02")).unwrap();
        assert_eq!(d.points[0].tag.as_deref(), Some("edited"));

        market_ignore(&st, "price", "tefas:AH2", "2026-06-01", "t").unwrap();
        assert_eq!(holding_value(&st).as_deref(), Some("1000.00"));

        market_revert(&st, "price", "tefas:AH2", "2026-06-01").unwrap();
        assert_eq!(holding_value(&st).as_deref(), Some("1100.00"));
    }

    #[test]
    fn export_writes_one_file() {
        let st = state();
        let dir = std::env::temp_dir().join(format!("mizan-market-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = market_export_csv(&st, "price", "tefas:AH2", &dir).unwrap();
        assert!(path.ends_with("mizan-price-tefas-AH2.csv"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(market_series(&st, "nope", &SeriesFilter::default(), day("2026-06-02")).is_err());
    }
}
