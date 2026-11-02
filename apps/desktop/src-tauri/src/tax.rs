//! Tax screen commands: tax years, reports, account tax treatment and CSV
//! export. Reports are computed from stored data only and never use the network.

use std::path::{Path, PathBuf};

use mizan_core::tax::csv::{file_names, gains_csv, income_csv};
use mizan_core::tax::report::{report, years, TaxInputs, TaxReport, TaxYears};
use mizan_core::tax::{Country, TaxTreatment, TaxYear};
use mizan_import::Date;
use serde::Serialize;
use tauri::AppHandle;

use crate::commands::{blocking, store_err};
use crate::market::{bump_generation, with_history};
use crate::session::AppError;
use crate::state::AppState;

/// Paths of the two files an export wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TaxExport {
    pub gains: String,
    pub income: String,
}

fn country(s: &str) -> Result<Country, AppError> {
    Country::parse(s).ok_or_else(|| AppError::Invalid(format!("unknown country {s:?}")))
}

fn inputs(state: &AppState) -> Result<TaxInputs, AppError> {
    let store = state.store.lock().map_err(store_err)?;
    with_history(state, |cache| {
        Ok(TaxInputs::load(&store, cache.ok().map(|c| &*c))?)
    })
}

fn year(c: Country, label: &str) -> Result<TaxYear, AppError> {
    TaxYear::parse(c, label).ok_or_else(|| AppError::Invalid(format!("not a tax year: {label:?}")))
}

pub fn tax_years(state: &AppState, c: &str, today: Date) -> Result<TaxYears, AppError> {
    let c = country(c)?;
    Ok(years(&inputs(state)?, c, today))
}

pub fn tax_report(
    state: &AppState,
    c: &str,
    label: &str,
    now: &str,
) -> Result<TaxReport, AppError> {
    let c = country(c)?;
    let y = year(c, label)?;
    if c != Country::Tr {
        // From now on the backfill fetches rates for this report's currency.
        let mut store = state.store.lock().map_err(store_err)?;
        if store
            .setting(mizan_market::backfill::TAX_RATES_KEY)?
            .as_deref()
            != Some("true")
        {
            store.set_setting(mizan_market::backfill::TAX_RATES_KEY, "true")?;
        }
    }
    Ok(report(&inputs(state)?, c, y, now))
}

pub fn set_tax_treatment(
    state: &AppState,
    account_id: i64,
    treatment: &str,
) -> Result<(), AppError> {
    let t = TaxTreatment::parse(treatment)
        .ok_or_else(|| AppError::Invalid(format!("unknown tax treatment {treatment:?}")))?;
    state
        .store
        .lock()
        .map_err(store_err)?
        .set_tax_treatment(account_id, t)?;
    bump_generation(state);
    Ok(())
}

fn write(path: &Path, body: &str) -> Result<(), AppError> {
    std::fs::write(path, body).map_err(|e| AppError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })
}

pub fn export_tax_csv(
    state: &AppState,
    c: &str,
    label: &str,
    dir: &Path,
    now: &str,
) -> Result<TaxExport, AppError> {
    let r = tax_report(state, c, label, now)?;
    let (g, i) = file_names(&r);
    let (gp, ip) = (dir.join(g), dir.join(i));
    write(&gp, &gains_csv(&r))?;
    write(&ip, &income_csv(&r))?;
    Ok(TaxExport {
        gains: gp.display().to_string(),
        income: ip.display().to_string(),
    })
}

fn today() -> Date {
    Date(chrono::Local::now().date_naive())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_tax_years(app: AppHandle, country: String) -> Result<TaxYears, AppError> {
    blocking(app, move |s| tax_years(s, &country, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_tax_report(
    app: AppHandle,
    country: String,
    year: String,
) -> Result<TaxReport, AppError> {
    blocking(app, move |s| tax_report(s, &country, &year, &now())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_set_tax_treatment(
    app: AppHandle,
    account_id: i64,
    treatment: String,
) -> Result<(), AppError> {
    blocking(app, move |s| set_tax_treatment(s, account_id, &treatment)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_export_tax_csv(
    app: AppHandle,
    country: String,
    year: String,
    dir: PathBuf,
) -> Result<TaxExport, AppError> {
    blocking(app, move |s| {
        export_tax_csv(s, &country, &year, &dir, &now())
    })
    .await
}

/// Accounts the Tax screen lists (tests).
#[cfg(test)]
pub fn report_accounts_for_tests(state: &AppState) -> usize {
    tax_report(state, "tr", "2026", "t").unwrap().accounts.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_core::{commit, prepare, resolve_batch, CreatePolicy, Decisions, MappingResolver};
    use mizan_storage::Store;

    fn state() -> AppState {
        let mut store = Store::open_in_memory().unwrap();
        let recs = serde_json::json!([
            {"kind": "buy", "date": "2024-03-01", "instrument": {"symbol": "AFT", "class_hint": "fund", "currency": "TRY"}, "quantity": "100", "net_cash": {"amount": "100.00", "currency": "TRY"}},
            {"kind": "sell", "date": "2025-03-03", "instrument": {"symbol": "AFT", "class_hint": "fund", "currency": "TRY"}, "quantity": "100", "net_cash": {"amount": "150.00", "currency": "TRY"}}
        ]);
        let mut recs = recs.as_array().unwrap().clone();
        for (i, r) in recs.iter_mut().enumerate() {
            r["source"] = serde_json::json!({"adapter": "t", "adapter_version": "1", "file_sha256": "a".repeat(64), "locator": format!("row {}", i + 1)});
            r["account"] = serde_json::json!({"hint": "B1", "name_hint": "Broker"});
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

    fn day(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    #[test]
    fn years_report_treatment_and_export() {
        let st = state();
        let y = tax_years(&st, "tr", day("2026-11-02")).unwrap();
        assert_eq!(y.default.as_deref(), Some("2025"));
        let r = tax_report(&st, "tr", "2025", "2026-11-02T10:00:00Z").unwrap();
        assert_eq!(r.gains.iter().map(|g| g.rows.len()).sum::<usize>(), 1);
        assert_eq!(r.computed_at, "2026-11-02T10:00:00Z");
        // A Turkish report needs no extra exchange rates.
        assert_eq!(
            st.store
                .lock()
                .unwrap()
                .setting(mizan_market::backfill::TAX_RATES_KEY)
                .unwrap(),
            None
        );

        let id = r.accounts[0].id;
        set_tax_treatment(&st, id, "pension").unwrap();
        let r = tax_report(&st, "uk", "2024-25", "t").unwrap();
        assert!(r.gains.iter().all(|g| g.rows.is_empty()));
        assert_eq!(r.excluded[0].treatment, "pension");
        assert!(set_tax_treatment(&st, id, "offshore").is_err());
        // Opening a UK report asks the backfill for rates.
        assert_eq!(
            st.store
                .lock()
                .unwrap()
                .setting(mizan_market::backfill::TAX_RATES_KEY)
                .unwrap()
                .as_deref(),
            Some("true")
        );

        set_tax_treatment(&st, id, "taxable").unwrap();
        let dir = std::env::temp_dir().join(format!("mizan-tax-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = export_tax_csv(&st, "tr", "2025", &dir, "t").unwrap();
        assert!(out.gains.ends_with("mizan-tr-2025-gains.csv"));
        let gains = std::fs::read_to_string(&out.gains).unwrap();
        assert!(gains.starts_with("section,description,isin"));
        assert_eq!(gains.lines().count(), 2);
        assert!(std::fs::read_to_string(&out.income)
            .unwrap()
            .starts_with("section,date"));
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(tax_report(&st, "de", "2025", "t").is_err());
        assert!(tax_report(&st, "uk", "2025", "t").is_err());
    }
}
