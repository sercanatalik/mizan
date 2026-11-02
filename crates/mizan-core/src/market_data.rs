//! The Market data screen's read model: every stored series (prices, rates,
//! indices) with its quality, one series in detail, CSV export, and the
//! per-database edits behind them.
//!
//! The shared history cache is only read here. Edits go to the user database
//! (`series_edits`); prices that exist only in the user database (`asset:<id>`
//! series) are edited through manual prices on the asset.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::str::FromStr;

use chrono::{Datelike, Duration, Weekday};
use mizan_import::{Code, Date};
use mizan_storage::inflation::{YearMonth, PPI_TR, SERIES};
use mizan_storage::{
    CachedSeries, HistoryCache, SeriesEdit, SeriesKind, StorageError, Store, MANUAL,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::returns::pricebook::series_key;
use crate::valuation::{base_currency, STALE_DAYS};

/// Table rows per page.
pub const PAGE_SIZE: usize = 200;
/// Most points drawn in a chart.
pub const CHART_POINTS: usize = 1000;

// ---------------------------------------------------------------- views

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SeriesFilter {
    pub source: Option<String>,
    pub currency: Option<String>,
    pub text: Option<String>,
    /// Only series the current database uses.
    pub used: bool,
    /// Only series with edits.
    pub edited: bool,
    /// Only stale series or series with weekday gaps.
    pub attention: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SeriesRow {
    /// `price`, `rate` or `index`.
    pub kind: String,
    /// For example `tefas:AFT`, `yahoo:VWCE.DE`, `USD`, `eurostat:TR:I15`, `asset:12`.
    pub key: String,
    pub name: String,
    /// Fund code, symbol, currency or series id.
    pub code: String,
    /// `tefas`, `yahoo`, `twelvedata`, `tcmb`, an index source, or `manual` for asset series.
    pub source: String,
    pub currency: Option<String>,
    pub latest: Option<String>,
    pub latest_date: Option<String>,
    pub first_date: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub count: i64,
    /// Daily series whose latest value is more than 7 days old.
    pub stale: bool,
    /// Weekdays between the series' first and last value without one (holidays included).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub series_gaps: i64,
    /// Weekdays this database held the instrument (or currency), up to the
    /// latest value, without a value in use; `None` when nothing holds it.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub held_gaps: Option<i64>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub edits: i64,
    /// Used by this database (an asset's price source, a currency in use, the base index).
    pub used: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SeriesPage {
    pub rows: Vec<SeriesRow>,
    /// Series of this kind before filtering.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total: i64,
    pub sources: Vec<String>,
    pub currencies: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SeriesPoint {
    /// `YYYY-MM-DD`, or `YYYY-MM` for indices.
    pub period: String,
    /// The value shown: the edit when edited, the stored value when ignored.
    pub value: String,
    pub currency: Option<String>,
    /// `cache`, `provider`, `manual` or `edited`.
    pub source: String,
    pub fetched_at: Option<String>,
    /// The provider's value when an edit replaced it.
    pub original: Option<String>,
    /// `edited` or `ignored`.
    pub tag: Option<String>,
    pub edited_at: Option<String>,
    /// Whether calculations use this value (false when ignored).
    pub in_use: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChartPoint {
    pub period: String,
    pub value: String,
    pub edited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SeriesDetail {
    pub row: SeriesRow,
    /// One page, newest first.
    pub points: Vec<SeriesPoint>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub offset: i64,
    /// The values in use, oldest first, thinned to at most 1,000 points.
    pub chart: Vec<ChartPoint>,
    /// Whether edits are made as manual prices on an asset.
    pub manual_only: bool,
}

// ---------------------------------------------------------------- series list

fn kind_of(s: &str) -> Result<SeriesKind, StorageError> {
    SeriesKind::parse(s).ok_or_else(|| StorageError::Invalid(format!("unknown series kind {s:?}")))
}

fn weekdays_between(first: Date, last: Date) -> i64 {
    let mut n = 0;
    let mut d = first.0;
    while d <= last.0 {
        if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
            n += 1;
        }
        d += Duration::days(1);
    }
    n
}

fn is_weekday(d: Date) -> bool {
    !matches!(d.0.weekday(), Weekday::Sat | Weekday::Sun)
}

/// What the current database uses, for "used by my assets".
struct Usage {
    price_series: HashMap<String, Vec<String>>,
    currencies: BTreeSet<String>,
    index_keys: BTreeSet<String>,
    /// Days each price series (by key) was held by any account.
    held_series: HashMap<String, BTreeSet<Date>>,
    /// Days each currency was held as cash or as an asset quoted in it.
    held_currency: HashMap<String, BTreeSet<Date>>,
}

/// Days on which a quantity series (value after each change day) is non-zero, up to `today`.
fn held_days(changes: &[(Date, Decimal)], today: Date, out: &mut BTreeSet<Date>) {
    for (i, (from, q)) in changes.iter().enumerate() {
        if q.is_zero() {
            continue;
        }
        let until = changes
            .get(i + 1)
            .map(|(d, _)| Date(d.0 - Duration::days(1)))
            .unwrap_or(today);
        let mut d = from.0;
        while d <= until.0 && d <= today.0 {
            out.insert(Date(d));
            d += Duration::days(1);
        }
    }
}

fn usage(store: &Store, today: Date) -> Result<Usage, StorageError> {
    let assets = store.list_assets()?;
    let names: HashMap<i64, String> = assets
        .iter()
        .map(|a| {
            let n = a
                .name
                .clone()
                .or(a.hint.name.clone())
                .or(a.hint.symbol.clone())
                .unwrap_or_else(|| a.instrument_key.clone());
            (a.id, n)
        })
        .collect();
    let mut price_series: HashMap<String, Vec<String>> = HashMap::new();
    for (asset, src) in store.price_sources()? {
        if let Some(k) = series_key(&src) {
            price_series
                .entry(k)
                .or_default()
                .push(names.get(&asset).cloned().unwrap_or_default());
        }
    }
    let mut currencies: BTreeSet<String> = store.fx_currencies()?.into_iter().collect();
    for a in &assets {
        if let Some(c) = a
            .currency
            .clone()
            .or(a.hint.currency.as_ref().map(|c| c.to_string()))
        {
            currencies.insert(c);
        }
    }
    let base = base_currency(store)?.to_string();
    currencies.insert(base.clone());
    currencies.remove("TRY");
    let mut index_keys = BTreeSet::new();
    if let Some(s) = mizan_storage::inflation::series_for(&base) {
        index_keys.insert(format!("{}:{}", s.source, s.id));
    }
    index_keys.insert(format!("{}:{}", PPI_TR.source, PPI_TR.id));

    // Held days, from each account's replayed quantities and cash.
    let keys = store.key_map()?;
    let canon = |k: &str| keys.get(k).cloned().unwrap_or_else(|| k.to_owned());
    let sources = store.price_sources()?;
    let mut series_of_key: HashMap<String, String> = HashMap::new();
    let mut currency_of_key: HashMap<String, String> = HashMap::new();
    for a in &assets {
        let key = canon(&a.instrument_key);
        let id = store.canonical_asset(a.id)?;
        if let Some(sk) = sources.get(&id).and_then(series_key) {
            series_of_key.insert(key.clone(), sk);
        }
        if let Some(c) = a
            .currency
            .clone()
            .or(a.hint.currency.as_ref().map(|c| c.to_string()))
        {
            currency_of_key.insert(key, c);
        }
    }
    let mut held_series: HashMap<String, BTreeSet<Date>> = HashMap::new();
    let mut held_currency: HashMap<String, BTreeSet<Date>> = HashMap::new();
    for acct in store.list_accounts()? {
        let stored = store.all_activities(acct.id)?;
        let rep = crate::returns::replay_account(acct.id, &acct.name, &stored, &keys);
        for (key, changes) in &rep.qty {
            let mut days = BTreeSet::new();
            held_days(changes, today, &mut days);
            if let Some(sk) = series_of_key.get(key) {
                held_series
                    .entry(sk.clone())
                    .or_default()
                    .extend(days.iter().copied());
            }
            if let Some(c) = currency_of_key.get(key).filter(|c| c.as_str() != "TRY") {
                held_currency.entry(c.clone()).or_default().extend(days);
            }
        }
        for (ccy, changes) in &rep.cash {
            if ccy.as_str() == "TRY" {
                continue;
            }
            held_days(
                changes,
                today,
                held_currency.entry(ccy.to_string()).or_default(),
            );
        }
    }
    Ok(Usage {
        price_series,
        currencies,
        index_keys,
        held_series,
        held_currency,
    })
}

fn index_label(source: &str, id: &str) -> String {
    SERIES
        .iter()
        .chain(std::iter::once(&PPI_TR))
        .find(|s| s.source == source && s.id == id)
        .map(|s| s.full_label())
        .unwrap_or_else(|| format!("{source} {id}"))
}

/// Edits grouped by (kind, series).
fn edits_by_series(
    store: &Store,
) -> Result<HashMap<(SeriesKind, String), Vec<SeriesEdit>>, StorageError> {
    let mut m: HashMap<(SeriesKind, String), Vec<SeriesEdit>> = HashMap::new();
    for e in store.all_series_edits()? {
        m.entry((e.kind, e.series.clone())).or_default().push(e);
    }
    Ok(m)
}

fn row_from_cached(
    kind: SeriesKind,
    key: String,
    name: String,
    s: &CachedSeries,
    today: Date,
) -> SeriesRow {
    let daily = kind != SeriesKind::Index;
    let last = Date::parse(&s.last).ok();
    let first = Date::parse(&s.first).ok();
    let gaps = match (daily, first, last) {
        (true, Some(f), Some(l)) => (weekdays_between(f, l) - s.weekday_values).max(0),
        _ => 0,
    };
    SeriesRow {
        kind: kind.as_str().into(),
        key,
        name,
        code: s.id.clone(),
        source: s.source.clone(),
        currency: s.currency.clone(),
        latest: Some(s.latest.clone()),
        latest_date: Some(s.last.clone()),
        first_date: Some(s.first.clone()),
        count: s.count,
        stale: daily && last.is_some_and(|l| (today.0 - l.0).num_days() > STALE_DAYS),
        series_gaps: gaps,
        held_gaps: None,
        edits: 0,
        used: false,
    }
}

/// Every series of one kind, before filtering.
fn all_rows(
    store: &Store,
    cache: Option<&HistoryCache>,
    kind: SeriesKind,
    today: Date,
) -> Result<Vec<SeriesRow>, StorageError> {
    let use_ = usage(store, today)?;
    let edits = edits_by_series(store)?;
    let mut rows: Vec<SeriesRow> = Vec::new();
    match kind {
        SeriesKind::Price => {
            if let Some(c) = cache {
                for s in c.fund_series()?.iter().chain(c.listed_series()?.iter()) {
                    let key = format!("{}:{}", s.source, s.id);
                    let name = use_
                        .price_series
                        .get(&key)
                        .and_then(|n| n.first().cloned())
                        .unwrap_or_else(|| s.id.clone());
                    rows.push(row_from_cached(kind, key, name, s, today));
                }
            }
            // Prices kept only in the user database.
            let cached: HashSet<String> = rows.iter().map(|r| r.key.clone()).collect();
            let sources = store.price_sources()?;
            for a in store.list_assets()? {
                if store.canonical_asset(a.id)? != a.id {
                    continue;
                }
                if sources
                    .get(&a.id)
                    .and_then(series_key)
                    .is_some_and(|k| cached.contains(&k))
                {
                    continue;
                }
                let prices = store.prices_for(a.id)?;
                if prices.is_empty() {
                    continue;
                }
                let newest = &prices[0];
                let oldest = &prices[prices.len() - 1];
                let manual = prices.iter().filter(|p| p.source == MANUAL).count() as i64;
                let name = a
                    .name
                    .clone()
                    .or(a.hint.name.clone())
                    .or(a.hint.symbol.clone())
                    .unwrap_or(a.instrument_key.clone());
                let last = Date::parse(&newest.date).ok();
                rows.push(SeriesRow {
                    kind: kind.as_str().into(),
                    key: format!("asset:{}", a.id),
                    name,
                    code: a.hint.symbol.clone().unwrap_or_default(),
                    source: if manual > 0 {
                        MANUAL.into()
                    } else {
                        newest.source.clone()
                    },
                    currency: Some(newest.currency.clone()),
                    latest: Some(newest.value.clone()),
                    latest_date: Some(newest.date.clone()),
                    first_date: Some(oldest.date.clone()),
                    count: prices
                        .iter()
                        .map(|p| p.date.as_str())
                        .collect::<BTreeSet<_>>()
                        .len() as i64,
                    stale: last.is_some_and(|l| (today.0 - l.0).num_days() > STALE_DAYS),
                    series_gaps: 0,
                    held_gaps: None,
                    edits: manual,
                    used: true,
                });
            }
        }
        SeriesKind::Rate => {
            if let Some(c) = cache {
                for s in c.rate_series()? {
                    rows.push(row_from_cached(kind, s.id.clone(), s.id.clone(), &s, today));
                }
            }
            let cached: HashSet<String> = rows.iter().map(|r| r.key.clone()).collect();
            for ccy in store.fx_currencies()? {
                if cached.contains(&ccy) {
                    continue;
                }
                let points = store.fx_rates_for(&ccy)?;
                if let (Some(first), Some(last)) = (points.first(), points.last()) {
                    let s = CachedSeries {
                        source: "tcmb".into(),
                        id: ccy.clone(),
                        currency: Some("TRY".into()),
                        count: points.len() as i64,
                        first: first.date.clone(),
                        last: last.date.clone(),
                        latest: last.try_per_unit.clone(),
                        weekday_values: points
                            .iter()
                            .filter(|p| Date::parse(&p.date).is_ok_and(is_weekday))
                            .count() as i64,
                    };
                    rows.push(row_from_cached(kind, ccy.clone(), ccy.clone(), &s, today));
                }
            }
        }
        SeriesKind::Index => {
            if let Some(c) = cache {
                for s in c.index_series()? {
                    let key = format!("{}:{}", s.source, s.id);
                    rows.push(row_from_cached(
                        kind,
                        key,
                        index_label(&s.source, &s.id),
                        &s,
                        today,
                    ));
                }
            }
        }
    }
    // Series that exist only through edits (values added by the user).
    let known: HashSet<String> = rows.iter().map(|r| r.key.clone()).collect();
    for ((k, series), es) in &edits {
        if *k != kind || known.contains(series) {
            continue;
        }
        let (source, code) = series.split_once(':').unwrap_or(("tcmb", series.as_str()));
        rows.push(SeriesRow {
            kind: kind.as_str().into(),
            key: series.clone(),
            name: match kind {
                SeriesKind::Index => index_label(source, code),
                _ => code.into(),
            },
            code: code.into(),
            source: if kind == SeriesKind::Rate {
                "tcmb".into()
            } else {
                source.into()
            },
            currency: es
                .iter()
                .find_map(|e| e.currency.clone())
                .or((kind == SeriesKind::Rate).then(|| "TRY".into())),
            latest: None,
            latest_date: None,
            first_date: None,
            count: 0,
            stale: false,
            series_gaps: 0,
            held_gaps: None,
            edits: 0,
            used: false,
        });
    }
    // Edits, usage and the latest value in use for edited series.
    for r in rows.iter_mut() {
        if let Some(es) = edits.get(&(kind, r.key.clone())) {
            r.edits = es.len() as i64;
            let detail = merged_points(store, cache, kind, &r.key)?;
            if let Some(p) = detail.iter().rev().find(|p| p.in_use) {
                r.latest = Some(p.value.clone());
                r.latest_date = Some(p.period.clone());
            }
            r.count = detail.len() as i64;
            r.first_date = detail.first().map(|p| p.period.clone());
        }
        let held = match kind {
            SeriesKind::Price => use_.held_series.get(&r.key),
            SeriesKind::Rate => use_.held_currency.get(&r.key),
            SeriesKind::Index => None,
        }
        .filter(|d| !d.is_empty());
        if let Some(held) = held {
            let values: HashSet<String> = merged_points(store, cache, kind, &r.key)?
                .into_iter()
                .filter(|p| p.in_use)
                .map(|p| p.period)
                .collect();
            let last = r
                .latest_date
                .as_deref()
                .and_then(|d| Date::parse(d).ok())
                .unwrap_or(today);
            r.held_gaps = Some(
                held.iter()
                    .filter(|d| **d <= last && is_weekday(**d) && !values.contains(&d.to_string()))
                    .count() as i64,
            );
        }
        r.used = r.used
            || match kind {
                SeriesKind::Price => use_.price_series.contains_key(&r.key),
                SeriesKind::Rate => use_.currencies.contains(&r.key),
                SeriesKind::Index => use_.index_keys.contains(&r.key),
            };
    }
    rows.sort_by(|a, b| {
        (!a.used, a.name.to_lowercase(), &a.key).cmp(&(!b.used, b.name.to_lowercase(), &b.key))
    });
    Ok(rows)
}

/// One tab of series, filtered.
pub fn series_page(
    store: &Store,
    cache: Option<&HistoryCache>,
    kind: &str,
    f: &SeriesFilter,
    today: Date,
) -> Result<SeriesPage, StorageError> {
    let kind = kind_of(kind)?;
    let all = all_rows(store, cache, kind, today)?;
    let sources: BTreeSet<String> = all.iter().map(|r| r.source.clone()).collect();
    let currencies: BTreeSet<String> = all.iter().filter_map(|r| r.currency.clone()).collect();
    let text = f
        .text
        .as_deref()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty());
    let blank = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let (src, ccy) = (blank(&f.source), blank(&f.currency));
    let total = all.len() as i64;
    let rows = all
        .into_iter()
        .filter(|r| src.as_ref().is_none_or(|s| &r.source == s))
        .filter(|r| ccy.as_ref().is_none_or(|c| r.currency.as_ref() == Some(c)))
        .filter(|r| {
            text.as_ref().is_none_or(|t| {
                [&r.name, &r.code, &r.key]
                    .iter()
                    .any(|v| v.to_lowercase().contains(t.as_str()))
            })
        })
        .filter(|r| !f.used || r.used)
        .filter(|r| !f.edited || r.edits > 0)
        .filter(|r| !f.attention || r.stale || r.held_gaps.unwrap_or(0) > 0)
        .collect();
    Ok(SeriesPage {
        rows,
        total,
        sources: sources.into_iter().collect(),
        currencies: currencies.into_iter().collect(),
    })
}

// ---------------------------------------------------------------- one series

/// Every value of a series with this database's edits applied, oldest first.
fn merged_points(
    store: &Store,
    cache: Option<&HistoryCache>,
    kind: SeriesKind,
    key: &str,
) -> Result<Vec<SeriesPoint>, StorageError> {
    let mut by: BTreeMap<String, SeriesPoint> = BTreeMap::new();
    let mut put = |period: String,
                   value: String,
                   currency: Option<String>,
                   source: &str,
                   fetched: Option<String>| {
        by.insert(
            period.clone(),
            SeriesPoint {
                period,
                value,
                currency,
                source: source.into(),
                fetched_at: fetched,
                original: None,
                tag: None,
                edited_at: None,
                in_use: true,
            },
        );
    };
    if let Some(asset) = key.strip_prefix("asset:") {
        let id: i64 = asset
            .parse()
            .map_err(|_| StorageError::Invalid(format!("bad series {key}")))?;
        let mut rows = store.prices_for(id)?;
        // Provider rows first, so a manual price on the same date wins.
        rows.sort_by_key(|p| (p.date.clone(), p.source == MANUAL));
        for p in rows {
            let src = if p.source == MANUAL {
                "manual"
            } else {
                "provider"
            };
            put(p.date, p.value, Some(p.currency), src, Some(p.fetched_at));
        }
        return Ok(by.into_values().collect());
    }
    match kind {
        SeriesKind::Price => {
            let (source, id) = key
                .split_once(':')
                .ok_or_else(|| StorageError::Invalid(format!("bad series {key}")))?;
            if let Some(c) = cache {
                let pts = if source == "tefas" {
                    c.fund_points(id)?
                } else {
                    c.listed_points(source, id)?
                };
                for p in pts {
                    put(p.period, p.value, p.currency, "cache", Some(p.fetched_at));
                }
            }
            // The latest provider prices of assets using this series.
            for (asset, src) in store.price_sources()? {
                if series_key(&src).as_deref() == Some(key) {
                    for p in store
                        .prices_for(asset)?
                        .into_iter()
                        .filter(|p| p.source != MANUAL)
                    {
                        put(
                            p.date,
                            p.value,
                            Some(p.currency),
                            "provider",
                            Some(p.fetched_at),
                        );
                    }
                }
            }
        }
        SeriesKind::Rate => {
            if let Some(c) = cache {
                for p in c.rate_points(key)? {
                    put(
                        p.period,
                        p.value,
                        Some("TRY".into()),
                        "cache",
                        Some(p.fetched_at),
                    );
                }
            }
            for r in store.fx_rates_for(key)? {
                put(r.date, r.try_per_unit, Some("TRY".into()), "provider", None);
            }
        }
        SeriesKind::Index => {
            if let (Some(c), Some((source, id))) = (cache, key.split_once(':')) {
                for p in c.index_points(source, id)? {
                    put(p.period, p.value, None, "cache", Some(p.fetched_at));
                }
            }
        }
    }
    for e in store.series_edits(kind, key)? {
        match by.get_mut(&e.period) {
            Some(p) if e.ignored => {
                p.tag = Some("ignored".into());
                p.in_use = false;
                p.edited_at = Some(e.edited_at.clone());
            }
            Some(p) => {
                p.original = Some(p.value.clone());
                p.value = e.value.clone().unwrap_or_default();
                p.currency = e.currency.clone().or(p.currency.take());
                p.source = "edited".into();
                p.tag = Some("edited".into());
                p.edited_at = Some(e.edited_at.clone());
            }
            None if !e.ignored => {
                by.insert(
                    e.period.clone(),
                    SeriesPoint {
                        period: e.period.clone(),
                        value: e.value.clone().unwrap_or_default(),
                        currency: e
                            .currency
                            .clone()
                            .or((kind == SeriesKind::Rate).then(|| "TRY".into())),
                        source: "edited".into(),
                        fetched_at: None,
                        original: None,
                        tag: Some("edited".into()),
                        edited_at: Some(e.edited_at.clone()),
                        in_use: true,
                    },
                );
            }
            None => {}
        }
    }
    Ok(by.into_values().collect())
}

/// Thin to at most `max` points: the last value of each equal-width bucket,
/// keeping every edited point.
fn thin(points: &[SeriesPoint], max: usize) -> Vec<ChartPoint> {
    let in_use: Vec<&SeriesPoint> = points.iter().filter(|p| p.in_use).collect();
    let chart = |p: &SeriesPoint| ChartPoint {
        period: p.period.clone(),
        value: p.value.clone(),
        edited: p.tag.is_some(),
    };
    if in_use.len() <= max {
        return in_use.into_iter().map(chart).collect();
    }
    let bucket = in_use.len().div_ceil(max);
    let mut out = Vec::new();
    for chunk in in_use.chunks(bucket) {
        for p in chunk.iter().filter(|p| p.tag.is_some()) {
            out.push(chart(p));
        }
        let last = chunk[chunk.len() - 1];
        if last.tag.is_none() {
            out.push(chart(last));
        }
    }
    out
}

/// One series: a page of values newest first, and its chart.
pub fn series_detail(
    store: &Store,
    cache: Option<&HistoryCache>,
    kind: &str,
    key: &str,
    offset: i64,
    today: Date,
) -> Result<SeriesDetail, StorageError> {
    let k = kind_of(kind)?;
    let row = all_rows(store, cache, k, today)?
        .into_iter()
        .find(|r| r.key == key)
        .ok_or_else(|| StorageError::NotFound(format!("series {key}")))?;
    let points = merged_points(store, cache, k, key)?;
    let total = points.len() as i64;
    let start = offset.max(0) as usize;
    let page: Vec<SeriesPoint> = points
        .iter()
        .rev()
        .skip(start)
        .take(PAGE_SIZE)
        .cloned()
        .collect();
    Ok(SeriesDetail {
        row,
        chart: thin(&points, CHART_POINTS),
        points: page,
        total,
        offset,
        manual_only: key.starts_with("asset:"),
    })
}

// ---------------------------------------------------------------- edits

fn check_period(kind: SeriesKind, period: &str) -> Result<(), StorageError> {
    let ok = match kind {
        SeriesKind::Index => YearMonth::parse(period).is_some(),
        _ => Date::parse(period).is_ok(),
    };
    if ok {
        Ok(())
    } else {
        Err(StorageError::Invalid(match kind {
            SeriesKind::Index => format!("{period:?} is not a month (YYYY-MM)"),
            _ => format!("{period:?} is not a date (YYYY-MM-DD)"),
        }))
    }
}

fn asset_of(key: &str) -> Option<i64> {
    key.strip_prefix("asset:").and_then(|a| a.parse().ok())
}

/// Edit or add a value. `currency` is required for prices; rates are TRY
/// per unit and indices have none.
#[allow(clippy::too_many_arguments)]
pub fn set_value(
    store: &mut Store,
    kind: &str,
    key: &str,
    period: &str,
    value: &str,
    currency: Option<&str>,
    now: &str,
) -> Result<(), StorageError> {
    let k = kind_of(kind)?;
    let period = period.trim();
    check_period(k, period)?;
    let v = Decimal::from_str(value.trim().replace(',', "").as_str())
        .map_err(|_| StorageError::Invalid(format!("{value:?} is not a number")))?;
    if v <= Decimal::ZERO {
        return Err(StorageError::Invalid("a value must be positive".into()));
    }
    let ccy = match (k, currency.map(str::trim).filter(|c| !c.is_empty())) {
        (SeriesKind::Price, Some(c)) => Some(
            Code::parse(&c.to_uppercase())
                .map_err(|_| StorageError::Invalid(format!("{c:?} is not a currency code")))?
                .to_string(),
        ),
        (SeriesKind::Price, None) => {
            return Err(StorageError::Invalid("a price needs a currency".into()));
        }
        _ => None,
    };
    let value = v.normalize().to_string();
    if let Some(asset) = asset_of(key) {
        return store.add_manual_price(asset, period, &value, ccy.as_deref().unwrap_or("TRY"), now);
    }
    store.set_series_value(k, key, period, &value, ccy.as_deref(), now)
}

/// Ignore a stored value.
pub fn ignore_value(
    store: &mut Store,
    kind: &str,
    key: &str,
    period: &str,
    now: &str,
) -> Result<(), StorageError> {
    let k = kind_of(kind)?;
    check_period(k, period.trim())?;
    if asset_of(key).is_some() {
        return Err(StorageError::Invalid(
            "prices kept only in this database are removed by deleting the manual price".into(),
        ));
    }
    store.ignore_series_value(k, key, period.trim(), now)
}

/// Drop an edit (or, for an asset series, the manual price).
pub fn revert_value(
    store: &mut Store,
    kind: &str,
    key: &str,
    period: &str,
) -> Result<(), StorageError> {
    let k = kind_of(kind)?;
    if let Some(asset) = asset_of(key) {
        return store.delete_manual_price(asset, period.trim());
    }
    if store.revert_series_value(k, key, period.trim())? {
        Ok(())
    } else {
        Err(StorageError::NotFound(format!(
            "an edit of {key} on {period}"
        )))
    }
}

// ---------------------------------------------------------------- CSV

fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// Every value of a series as CSV, oldest first, ignored values included.
pub fn series_csv(
    store: &Store,
    cache: Option<&HistoryCache>,
    kind: &str,
    key: &str,
) -> Result<String, StorageError> {
    let k = kind_of(kind)?;
    let points = merged_points(store, cache, k, key)?;
    let mut out = String::from("period,value,currency,source,tag,original,fetched_at\r\n");
    for p in points {
        let cells = [
            p.period.as_str(),
            p.value.as_str(),
            p.currency.as_deref().unwrap_or(""),
            p.source.as_str(),
            p.tag.as_deref().unwrap_or(""),
            p.original.as_deref().unwrap_or(""),
            p.fetched_at.as_deref().unwrap_or(""),
        ];
        out.push_str(&cells.iter().map(|c| field(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Ok(out)
}

/// A file name for a series, for example `mizan-price-yahoo-VWCE.DE.csv`.
pub fn csv_file_name(kind: &str, key: &str) -> String {
    let safe: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("mizan-{kind}-{safe}.csv")
}
