//! Returns and value history for Performance and Overview, memoized per data generation.

use std::sync::atomic::Ordering;

use mizan_core::portfolio::Period;
use mizan_core::returns::view::InflationView;
use mizan_core::returns::{
    returns_view, value_history, Benchmark, Inputs, PeriodReturns, ReturnsView, ValueHistory,
};
use mizan_import::Date;
use serde::Serialize;
use tauri::AppHandle;

use crate::commands::{blocking, store_err};
use crate::market::{bump_generation, with_history};
use crate::session::AppError;
use crate::state::AppState;

pub const BENCHMARK_KEY: &str = mizan_market::backfill::BENCHMARK_KEY;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BenchmarkOption {
    #[cfg_attr(test, ts(type = "number"))]
    pub asset_id: i64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PerformanceView {
    pub base_currency: String,
    /// The inflation index behind the real figures.
    pub inflation: InflationView,
    pub returns: PeriodReturns,
    /// Assets with stored prices that can be the benchmark.
    pub benchmark_options: Vec<BenchmarkOption>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub benchmark_asset_id: Option<i64>,
}

/// Everything computed from one data generation.
#[derive(Clone)]
pub struct Computed {
    pub generation: u64,
    pub today: Date,
    pub returns: ReturnsView,
    pub history: ValueHistory,
    pub options: Vec<BenchmarkOption>,
    pub benchmark: Option<i64>,
}

fn label(a: &mizan_storage::AssetRow) -> String {
    match (&a.hint.symbol, &a.hint.name) {
        (Some(s), Some(n)) => format!("{s} · {n}"),
        (Some(s), None) => s.clone(),
        (None, Some(n)) => n.clone(),
        (None, None) => a.instrument_key.clone(),
    }
}

fn compute(state: &AppState, today: Date, generation: u64) -> Result<Computed, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    let inputs = with_history(state, |cache| {
        Ok(Inputs::load(&store, cache.ok().map(|c| &*c))?)
    })?;
    let keys = store.key_map()?;
    let canon = |k: &str| keys.get(k).cloned().unwrap_or_else(|| k.to_string());
    let assets = store.list_assets()?;
    let priced: std::collections::HashSet<&str> = inputs.book.priced_keys().collect();
    let mut options: Vec<BenchmarkOption> = assets
        .iter()
        .filter(|a| {
            !keys.contains_key(&a.instrument_key) && priced.contains(a.instrument_key.as_str())
        })
        .map(|a| BenchmarkOption {
            asset_id: a.id,
            name: label(a),
        })
        .collect();
    options.sort_by(|a, b| a.name.cmp(&b.name));
    let chosen = store
        .setting(BENCHMARK_KEY)?
        .and_then(|v| v.parse::<i64>().ok())
        .map(|id| store.canonical_asset(id))
        .transpose()?;
    let benchmark = chosen.and_then(|id| {
        assets.iter().find(|a| a.id == id).map(|a| Benchmark {
            key: canon(&a.instrument_key),
            asset_id: a.id,
            name: label(a),
        })
    });
    let series = inputs.series(today);
    Ok(Computed {
        generation,
        today,
        returns: returns_view(
            &series,
            &inputs.book,
            &inputs.base,
            benchmark.as_ref(),
            &inputs.inflation,
            today,
        ),
        history: value_history(&series, &inputs.base, today),
        options,
        benchmark: benchmark.map(|b| b.asset_id),
    })
}

/// The memoized computation for the current data generation, recomputed when stale.
pub fn computed(state: &AppState, today: Date) -> Result<Computed, AppError> {
    let generation = state.generation.load(Ordering::SeqCst);
    if let Some(c) = state.returns_memo.lock().map_err(store_err)?.as_ref() {
        if c.generation == generation && c.today == today {
            return Ok(c.clone());
        }
    }
    let c = compute(state, today, generation)?;
    *state.returns_memo.lock().map_err(store_err)? = Some(c.clone());
    Ok(c)
}

pub fn performance(
    state: &AppState,
    period: Period,
    today: Date,
) -> Result<PerformanceView, AppError> {
    let c = computed(state, today)?;
    let returns = c
        .returns
        .periods
        .iter()
        .find(|p| p.period == period)
        .cloned()
        .expect("every period is computed");
    Ok(PerformanceView {
        base_currency: c.returns.base_currency.clone(),
        inflation: c.returns.inflation.clone(),
        returns,
        benchmark_options: c.options.clone(),
        benchmark_asset_id: c.benchmark,
    })
}

pub fn history(state: &AppState, today: Date) -> Result<ValueHistory, AppError> {
    Ok(computed(state, today)?.history)
}

/// Choose the benchmark (an asset with stored prices) or none.
pub fn set_benchmark(state: &AppState, asset_id: Option<i64>) -> Result<(), AppError> {
    {
        let mut store = state.store.lock().map_err(store_err)?;
        match asset_id {
            Some(id) => {
                if !store.list_assets()?.iter().any(|a| a.id == id) {
                    return Err(AppError::Invalid(format!("asset {id} does not exist")));
                }
                store.set_setting(BENCHMARK_KEY, &id.to_string())?;
            }
            None => store.set_setting(BENCHMARK_KEY, "")?,
        }
    }
    bump_generation(state);
    Ok(())
}

fn today() -> Date {
    Date(chrono::Local::now().date_naive())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_performance(app: AppHandle, period: Period) -> Result<PerformanceView, AppError> {
    blocking(app, move |s| performance(s, period, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_value_history(app: AppHandle) -> Result<ValueHistory, AppError> {
    blocking(app, |s| history(s, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_set_benchmark(app: AppHandle, asset_id: Option<i64>) -> Result<(), AppError> {
    blocking(app, move |s| set_benchmark(s, asset_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_core::{commit, prepare, resolve_batch, CreatePolicy, Decisions, MappingResolver};
    use mizan_storage::Store;

    fn day(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    fn import(store: &mut Store, file: char, records: serde_json::Value) {
        let mut recs = records.as_array().unwrap().clone();
        for (i, r) in recs.iter_mut().enumerate() {
            r["source"] = serde_json::json!({"adapter": "t", "adapter_version": "1", "file_sha256": file.to_string().repeat(64), "locator": format!("row {}", i + 1)});
            r["account"] = serde_json::json!({"hint": "P1", "name_hint": "Pension"});
        }
        let parsed = mizan_import::parse_batch_value(
            serde_json::json!({"schema": "mizan.import/1", "records": recs}),
        )
        .unwrap();
        let rb = {
            let mut r = MappingResolver {
                store,
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

    /// A pension with one fund priced by TEFAS history in the cache.
    fn state() -> (AppState, i64) {
        let mut store = Store::open_in_memory().unwrap();
        import(
            &mut store,
            'a',
            serde_json::json!([
                {"kind": "deposit", "date": "2026-01-05", "net_cash": {"amount": "1000.00", "currency": "TRY"}},
                {"kind": "buy", "date": "2026-01-05", "instrument": {"symbol": "AH2", "name": "Para Piyasası", "class_hint": "fund", "currency": "TRY"}, "quantity": "1000", "net_cash": {"amount": "1000.00", "currency": "TRY"}}
            ]),
        );
        let asset = store.list_assets().unwrap()[0].id;
        store
            .set_price_source(
                asset,
                Some(&mizan_storage::PriceSourceKind::Tefas {
                    fund_code: "AH2".into(),
                }),
            )
            .unwrap();
        let st = AppState::new(store);
        {
            let mut c = st.history_mem.lock().unwrap();
            for (d, p) in [("2026-01-05", "1.00"), ("2026-06-01", "1.10")] {
                c.upsert_fund_price(
                    &mizan_storage::CachedFundPrice {
                        code: "AH2".into(),
                        date: d.into(),
                        price: p.into(),
                    },
                    "t",
                )
                .unwrap();
            }
        }
        (st, asset)
    }

    #[test]
    fn performance_on_the_synthetic_pension() {
        let (st, asset) = state();
        let v = performance(&st, Period::SinceStart, day("2026-06-02")).unwrap();
        assert_eq!(v.base_currency, "TRY");
        assert_eq!(v.returns.twr.value.as_deref(), Some("+10.00%"));
        assert_eq!(v.returns.mwr.value.as_deref(), Some("+10.00%"));
        assert_eq!(v.returns.accounts[0].name, "Pension");
        assert_eq!(v.benchmark_asset_id, None);
        assert_eq!(v.benchmark_options.len(), 1);
        assert_eq!(v.benchmark_options[0].asset_id, asset);
        assert_eq!(v.benchmark_options[0].name, "AH2 · Para Piyasası");
        let h = history(&st, day("2026-06-02")).unwrap();
        assert_eq!(h.points.first().unwrap().value.as_deref(), Some("1000.00"));
        assert_eq!(h.points.last().unwrap().value.as_deref(), Some("1100.00"));
        assert_eq!(h.points.last().unwrap().contributions, "1000.00");
    }

    #[test]
    fn benchmark_is_saved_and_used() {
        let (st, asset) = state();
        set_benchmark(&st, Some(asset)).unwrap();
        let v = performance(&st, Period::SinceStart, day("2026-06-02")).unwrap();
        assert_eq!(v.benchmark_asset_id, Some(asset));
        let b = v.returns.benchmark.unwrap();
        assert_eq!(b.figure.value.as_deref(), Some("+10.00%"));
        set_benchmark(&st, None).unwrap();
        let v = performance(&st, Period::SinceStart, day("2026-06-02")).unwrap();
        assert!(v.returns.benchmark.is_none());
        assert!(set_benchmark(&st, Some(9999)).is_err());
    }

    #[test]
    fn memo_is_reused_until_data_changes() {
        let (st, asset) = state();
        let today = day("2026-06-02");
        let a = computed(&st, today).unwrap();
        // A price written behind the app's back is not seen while the memo is valid...
        st.store
            .lock()
            .unwrap()
            .add_manual_price(asset, "2026-06-02", "2.00", "TRY", "t")
            .unwrap();
        let b = computed(&st, today).unwrap();
        assert_eq!(a.generation, b.generation);
        assert_eq!(
            b.history.points.last().unwrap().value.as_deref(),
            Some("1100.00")
        );
        // ...and is after any change that bumps the generation, such as a commit.
        crate::market::bump_generation(&st);
        let c = computed(&st, today).unwrap();
        assert_eq!(
            c.history.points.last().unwrap().value.as_deref(),
            Some("2000.00")
        );
    }

    #[test]
    fn manual_price_edit_invalidates_the_memo() {
        let (st, asset) = state();
        let today = day("2026-06-02");
        computed(&st, today).unwrap();
        crate::market::add_manual_price(&st, asset, "2026-06-02", "1.50", "TRY", "t").unwrap();
        let c = computed(&st, today).unwrap();
        assert_eq!(
            c.history.points.last().unwrap().value.as_deref(),
            Some("1500.00")
        );
    }
}
