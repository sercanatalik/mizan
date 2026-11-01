//! One refresh: ask each enabled provider for fresh data and store it.
//!
//! Failures are isolated per source and never delete or alter stored data.
//! Disabled providers and assets without a provider source cause no requests.

use std::collections::{BTreeMap, BTreeSet};

use mizan_import::Date;
use mizan_storage::{AssetId, FxRow, PriceRow, PriceSourceKind, StorageError, Store};
use serde::{Deserialize, Serialize};

use crate::http::Http;
use crate::{is_enabled, tcmb, tefas, PROVIDERS};

pub const LAST_REFRESH_KEY: &str = "last_refresh_at";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Ok,
    Failed,
    Disabled,
    /// Enabled, but nothing to fetch (for example no asset uses TEFAS).
    Idle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SourceReport {
    pub id: String,
    pub name: String,
    pub status: SourceStatus,
    /// Prices or rates stored by this refresh.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub updated: usize,
    pub messages: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RefreshReport {
    pub started_at: String,
    pub finished_at: String,
    pub sources: Vec<SourceReport>,
}

impl RefreshReport {
    pub fn any_failed(&self) -> bool {
        self.sources
            .iter()
            .any(|s| s.status == SourceStatus::Failed)
    }
}

fn report(id: &str, status: SourceStatus, updated: usize, messages: Vec<String>) -> SourceReport {
    let name = PROVIDERS
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.name)
        .unwrap_or(id);
    SourceReport {
        id: id.into(),
        name: name.into(),
        status,
        updated,
        messages,
    }
}

fn refresh_tefas(
    store: &mut Store,
    http: &dyn Http,
    today: Date,
    now: &str,
) -> Result<SourceReport, StorageError> {
    let mut by_code: BTreeMap<String, Vec<AssetId>> = BTreeMap::new();
    for (asset, source) in store.price_sources()? {
        if let PriceSourceKind::Tefas { fund_code } = source {
            let code = fund_code.trim().to_uppercase();
            if !code.is_empty() {
                by_code.entry(code).or_default().push(asset);
            }
        }
    }
    if by_code.is_empty() {
        return Ok(report(
            tefas::SOURCE,
            SourceStatus::Idle,
            0,
            vec!["no assets use TEFAS".into()],
        ));
    }
    let codes: BTreeSet<String> = by_code.keys().cloned().collect();
    let fetched = tefas::fetch_latest(http, &codes, today);
    let mut updated = 0;
    for (code, p) in &fetched.latest {
        for asset in by_code.get(code).into_iter().flatten() {
            store.upsert_provider_price(&PriceRow {
                asset_id: *asset,
                date: p.date.to_string(),
                source: tefas::SOURCE.into(),
                value: p.price.normalize().to_string(),
                currency: "TRY".into(),
                fetched_at: now.into(),
            })?;
            updated += 1;
        }
    }
    let mut messages: Vec<String> = fetched
        .not_found
        .iter()
        .map(|c| format!("{c}: fund not found"))
        .collect();
    let status = match fetched.error {
        Some(e) => {
            messages.insert(0, e);
            SourceStatus::Failed
        }
        None => SourceStatus::Ok,
    };
    Ok(report(tefas::SOURCE, status, updated, messages))
}

fn refresh_tcmb(
    store: &mut Store,
    http: &dyn Http,
    now: &str,
) -> Result<SourceReport, StorageError> {
    match tcmb::fetch(http) {
        Ok(None) => Ok(report(
            tcmb::SOURCE,
            SourceStatus::Ok,
            0,
            vec!["no rates published today; stored rates kept".into()],
        )),
        Ok(Some(rates)) => {
            for r in &rates {
                store.upsert_fx_rate(
                    &FxRow {
                        currency: r.currency.to_string(),
                        date: r.date.to_string(),
                        source: tcmb::SOURCE.into(),
                        try_per_unit: r.try_per_unit.to_string(),
                    },
                    now,
                )?;
            }
            let date = rates
                .first()
                .map(|r| format!("rates dated {}", r.date))
                .into_iter()
                .collect();
            Ok(report(tcmb::SOURCE, SourceStatus::Ok, rates.len(), date))
        }
        Err(e) => Ok(report(
            tcmb::SOURCE,
            SourceStatus::Failed,
            0,
            vec![e.to_string()],
        )),
    }
}

/// Run one refresh. `now` is an RFC 3339 timestamp used for fetch times and the log.
/// The clock is passed in to keep this testable.
pub fn refresh(
    store: &mut Store,
    http: &dyn Http,
    today: Date,
    now: &str,
    finished: impl Fn() -> String,
) -> Result<RefreshReport, StorageError> {
    let mut sources = Vec::new();
    for p in PROVIDERS {
        if !is_enabled(store, p.id) {
            sources.push(report(p.id, SourceStatus::Disabled, 0, vec![]));
            continue;
        }
        let r = match p.id {
            tefas::SOURCE => refresh_tefas(store, http, today, now)?,
            tcmb::SOURCE => refresh_tcmb(store, http, now)?,
            _ => continue,
        };
        sources.push(r);
    }
    let rep = RefreshReport {
        started_at: now.into(),
        finished_at: finished(),
        sources,
    };
    store.log_refresh(
        &rep.started_at,
        &rep.finished_at,
        &serde_json::to_string(&rep).expect("serializable"),
    )?;
    if !rep.any_failed() {
        store.set_setting(LAST_REFRESH_KEY, &rep.finished_at)?;
    }
    Ok(rep)
}
