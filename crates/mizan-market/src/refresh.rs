//! One refresh: ask each enabled provider for fresh data and store it.
//!
//! Failures are isolated per source and never delete or alter stored data.
//! Disabled providers, providers missing their API key, and assets without a
//! provider source cause no requests.

use std::collections::{BTreeMap, BTreeSet};

use mizan_import::Date;
use mizan_storage::{
    AssetId, CachedFundPrice, CachedListedPrice, CachedRate, FetchStatus, FetchedUnit, FxRow,
    HistoryCache, PriceRow, PriceSourceKind, StorageError, Store,
};
use serde::{Deserialize, Serialize};

use crate::http::Http;
use crate::listed::{listed_provider, Close, ListedError, ListedProvider};
use crate::{api_key, is_enabled, tcmb, tefas, twelvedata, yahoo, PROVIDERS};

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
    /// Enabled with assets to price, but no API key is saved.
    NeedsKey,
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
    cache: Option<&mut HistoryCache>,
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
    if let Some(cache) = cache {
        if let Err(e) = cache_tefas(cache, &fetched, now) {
            messages.push(format!("history cache not updated: {e}"));
        }
    }
    let status = match fetched.error {
        Some(e) => {
            messages.insert(0, e);
            SourceStatus::Failed
        }
        None => SourceStatus::Ok,
    };
    Ok(report(tefas::SOURCE, status, updated, messages))
}

/// Assets priced from one listed provider, grouped by symbol.
pub(crate) fn listed_assets(
    store: &Store,
    provider: &str,
) -> Result<BTreeMap<String, Vec<AssetId>>, StorageError> {
    let mut by_symbol: BTreeMap<String, Vec<AssetId>> = BTreeMap::new();
    for (asset, source) in store.price_sources()? {
        if let Some((p, symbol)) = source.listed() {
            let symbol = symbol.trim().to_uppercase();
            if p == provider && !symbol.is_empty() {
                by_symbol.entry(symbol).or_default().push(asset);
            }
        }
    }
    for v in by_symbol.values_mut() {
        v.sort_unstable();
    }
    Ok(by_symbol)
}

/// Fetch recent closes of one symbol from a listed provider.
pub(crate) fn fetch_listed_latest(
    provider: ListedProvider,
    http: &dyn Http,
    key: Option<&str>,
    symbol: &str,
) -> Result<Vec<Close>, ListedError> {
    match provider.id {
        yahoo::SOURCE => yahoo::fetch_latest(http, symbol),
        _ => twelvedata::fetch_latest(http, key.unwrap_or(""), symbol),
    }
}

pub(crate) fn cached_closes(
    provider: &str,
    symbol: &str,
    closes: &[Close],
) -> Vec<CachedListedPrice> {
    closes
        .iter()
        .map(|c| CachedListedPrice {
            source: provider.into(),
            symbol: symbol.into(),
            date: c.date.to_string(),
            close: c.value.to_string(),
            currency: c.currency.to_string(),
        })
        .collect()
}

fn refresh_listed(
    store: &mut Store,
    mut cache: Option<&mut HistoryCache>,
    http: &dyn Http,
    provider: ListedProvider,
    now: &str,
) -> Result<SourceReport, StorageError> {
    let by_symbol = listed_assets(store, provider.id)?;
    if by_symbol.is_empty() {
        return Ok(report(
            provider.id,
            SourceStatus::Idle,
            0,
            vec![format!("no assets use {}", provider.name)],
        ));
    }
    let key = api_key(store, provider.id);
    if crate::NEEDS_KEY.contains(&provider.id) && key.is_none() {
        return Ok(report(
            provider.id,
            SourceStatus::NeedsKey,
            0,
            vec!["needs an API key".into()],
        ));
    }
    let mut updated = 0;
    let mut messages = Vec::new();
    let mut errors = 0;
    let mut stopped = false;
    let mut cache_error = None;
    for (i, (symbol, assets)) in by_symbol.iter().enumerate() {
        if i > 0 {
            http.pause(provider.interval);
        }
        match fetch_listed_latest(provider, http, key.as_deref(), symbol) {
            Ok(closes) => {
                let Some(last) = closes.last() else {
                    messages.push(format!("{symbol}: no recent prices"));
                    continue;
                };
                for asset in assets {
                    store.upsert_provider_price(&PriceRow {
                        asset_id: *asset,
                        date: last.date.to_string(),
                        source: provider.id.into(),
                        value: last.value.to_string(),
                        currency: last.currency.to_string(),
                        fetched_at: now.into(),
                    })?;
                    updated += 1;
                }
                if let Some(cache) = cache.as_deref_mut() {
                    for p in cached_closes(provider.id, symbol, &closes) {
                        if let Err(e) = cache.upsert_listed_price(&p, now) {
                            cache_error = Some(e);
                            break;
                        }
                    }
                }
            }
            Err(e) if e.stops_provider() => {
                messages.insert(0, format!("{}: {e}", provider.name));
                stopped = true;
                break;
            }
            Err(e @ ListedError::NotFound(_)) => messages.push(e.to_string()),
            Err(e) => {
                errors += 1;
                messages.push(format!("{symbol}: {e}"));
            }
        }
    }
    if let Some(e) = cache_error {
        messages.push(format!("history cache not updated: {e}"));
    }
    let status = if stopped || (errors > 0 && errors == by_symbol.len()) {
        SourceStatus::Failed
    } else {
        SourceStatus::Ok
    };
    Ok(report(provider.id, status, updated, messages))
}

/// Write the latest fund prices and the fund types they were listed under to the cache.
fn cache_tefas(
    cache: &mut HistoryCache,
    fetched: &tefas::Fetched,
    now: &str,
) -> Result<(), StorageError> {
    for (code, p) in &fetched.latest {
        cache.upsert_fund_price(
            &CachedFundPrice {
                code: code.clone(),
                date: p.date.to_string(),
                price: p.price.normalize().to_string(),
            },
            now,
        )?;
    }
    for (code, fund_type) in &fetched.fund_types {
        cache.set_fund_type(code, fund_type)?;
    }
    Ok(())
}

/// Write one TCMB day to the cache and mark that day fetched.
pub(crate) fn cache_tcmb_day(
    cache: &mut HistoryCache,
    date: Date,
    rates: &[tcmb::Rate],
    now: &str,
) -> Result<(), StorageError> {
    cache.commit_unit(&FetchedUnit {
        source: tcmb::SOURCE.into(),
        key: crate::backfill::TCMB_KEY.into(),
        period: date.to_string(),
        status: if rates.is_empty() {
            FetchStatus::None
        } else {
            FetchStatus::Ok
        },
        fund_prices: vec![],
        rates: rates
            .iter()
            .map(|r| CachedRate {
                currency: r.currency.to_string(),
                date: r.date.to_string(),
                try_per_unit: r.try_per_unit.to_string(),
                source: tcmb::SOURCE.into(),
            })
            .collect(),
        fetched_at: now.into(),
    })
}

fn refresh_tcmb(
    store: &mut Store,
    cache: Option<&mut HistoryCache>,
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
            let mut messages: Vec<String> = rates
                .first()
                .map(|r| format!("rates dated {}", r.date))
                .into_iter()
                .collect();
            if let (Some(cache), Some(first)) = (cache, rates.first()) {
                if let Err(e) = cache_tcmb_day(cache, first.date, &rates, now) {
                    messages.push(format!("history cache not updated: {e}"));
                }
            }
            Ok(report(
                tcmb::SOURCE,
                SourceStatus::Ok,
                rates.len(),
                messages,
            ))
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
/// The clock is passed in to keep this testable. When a history cache is given,
/// fetched prices and rates are also written to it; a cache failure is reported
/// in the source's messages and never fails the refresh.
pub fn refresh(
    store: &mut Store,
    mut cache: Option<&mut HistoryCache>,
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
            tefas::SOURCE => refresh_tefas(store, cache.as_deref_mut(), http, today, now)?,
            tcmb::SOURCE => refresh_tcmb(store, cache.as_deref_mut(), http, now)?,
            id => match listed_provider(id) {
                Some(lp) => refresh_listed(store, cache.as_deref_mut(), http, lp, now)?,
                None => continue,
            },
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
