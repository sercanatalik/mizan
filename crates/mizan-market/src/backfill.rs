//! Background backfill of price and exchange-rate history into the shared cache.
//!
//! [`plan`] works out which fund-months, listed symbol-years and TCMB days are
//! missing, using the user database only for dates, fund codes and symbols. [`run`] fetches them newest
//! first within provider limits and commits each unit to the cache on its own,
//! so an interruption loses at most one unit and the next run resumes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::listed::{ApiKey, Close, ListedError, ListedProvider, LISTED};
use crate::{twelvedata, yahoo, NEEDS_KEY};
use chrono::{Datelike, Duration as Days, NaiveDate, Weekday};
use mizan_import::{Date, ImportRecord};
use mizan_storage::{
    CachedFundPrice, FetchStatus, FetchedUnit, HistoryCache, PriceSourceKind, StorageError, Store,
};
use serde::{Deserialize, Serialize};

use crate::http::Http;
use crate::{is_enabled, tcmb, tefas};

/// Setting holding the benchmark asset id (shared with the returns screen).
pub const BENCHMARK_KEY: &str = "benchmark.asset";
/// Setting set once a UK or US tax report has been opened: their currencies
/// need exchange rates from the first activity even for a TRY-only history.
pub const TAX_RATES_KEY: &str = "tax.rates_needed";
/// Key under which TCMB days are recorded in the cache (rates cover every currency).
pub const TCMB_KEY: &str = "*";
/// TCMB publishes no limit; static files, fetched politely one a second.
pub const TCMB_INTERVAL: Duration = Duration::from_secs(1);
/// Days before today that are fetched again to pick up late publications.
pub const RECHECK_DAYS: i64 = 7;
/// Fund types tried, in order, when a fund's type is not known yet.
const TRY_TYPES: [&str; 2] = ["YAT", "EMK"];

// ---------------------------------------------------------------- inputs

/// What the planner needs from a user database: dates, fund codes, provider
/// symbols and the API keys of providers that need one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanInput {
    /// First activity date per TEFAS fund code.
    pub tefas: BTreeMap<String, Date>,
    /// First date exchange rates are needed, if ever.
    pub fx_from: Option<Date>,
    pub tefas_enabled: bool,
    pub tcmb_enabled: bool,
    /// First activity date per (listed provider, symbol).
    pub listed: BTreeMap<(String, String), Date>,
    /// Listed providers turned off in Settings.
    pub listed_disabled: BTreeSet<String>,
    /// Saved API keys, by provider.
    pub keys: BTreeMap<String, ApiKey>,
}

fn record_currencies(r: &ImportRecord) -> impl Iterator<Item = &str> {
    let money = [r.net_cash.as_ref(), r.gross.as_ref(), r.price.as_ref()]
        .into_iter()
        .flatten()
        .map(|m| m.currency.as_str());
    let inst = r
        .instrument
        .as_ref()
        .and_then(|i| i.currency.as_ref())
        .map(|c| c.as_str());
    let fx =
        r.fx.as_ref()
            .into_iter()
            .flat_map(|f| [f.from.as_str(), f.to.as_str()]);
    let charges = r.charges.iter().map(|c| c.amount.currency.as_str());
    money.chain(inst).chain(fx).chain(charges)
}

impl PlanInput {
    /// Read fund codes, first activity dates and currencies from a user database.
    pub fn from_store(store: &Store) -> Result<Self, StorageError> {
        let base = crate::base_currency_code(store)?;
        let mut first_by_asset: BTreeMap<i64, Date> = BTreeMap::new();
        // First activity of each account, and the accounts each asset appears in.
        let mut first_by_account: BTreeMap<i64, Date> = BTreeMap::new();
        let mut accounts_by_asset: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
        let mut first_any: Option<Date> = None;
        let mut first_foreign: Option<Date> = None;
        for a in store.list_accounts()? {
            for s in store.all_activities(a.id)? {
                let d = s.record.date;
                first_any = Some(first_any.map_or(d, |f| f.min(d)));
                let e = first_by_account.entry(a.id).or_insert(d);
                *e = (*e).min(d);
                if let Some(asset) = s.asset_id {
                    accounts_by_asset
                        .entry(store.canonical_asset(asset)?)
                        .or_default()
                        .insert(a.id);
                }
                if record_currencies(&s.record).any(|c| c != "TRY") {
                    first_foreign = Some(first_foreign.map_or(d, |f| f.min(d)));
                }
                if let Some(asset) = s.asset_id {
                    let asset = store.canonical_asset(asset)?;
                    let e = first_by_asset.entry(asset).or_insert(d);
                    *e = (*e).min(d);
                }
            }
        }
        let mut tefas: BTreeMap<String, Date> = BTreeMap::new();
        let mut listed: BTreeMap<(String, String), Date> = BTreeMap::new();
        // The benchmark and every asset marked as a benchmark on the Assets
        // screen: when held, from the first activity of the (earliest) account
        // holding it; when not held, from the portfolio's first activity.
        let mut benchmarks = store.benchmark_candidates()?;
        if let Some(b) = store
            .setting(BENCHMARK_KEY)?
            .and_then(|v| v.trim().parse::<i64>().ok())
        {
            benchmarks.insert(store.canonical_asset(b)?);
        }
        for (asset, source) in store.price_sources()? {
            let held = first_by_asset.get(&asset).copied();
            let holding_accounts = accounts_by_asset.get(&asset).and_then(|accts| {
                accts
                    .iter()
                    .filter_map(|a| first_by_account.get(a))
                    .min()
                    .copied()
            });
            let as_benchmark = benchmarks
                .contains(&asset)
                .then(|| holding_accounts.or(first_any))
                .flatten();
            let Some(d) = [held, as_benchmark].into_iter().flatten().min() else {
                continue;
            };
            let slot = match &source {
                PriceSourceKind::Tefas { fund_code } => {
                    let code = fund_code.trim().to_uppercase();
                    if code.is_empty() {
                        continue;
                    }
                    tefas.entry(code).or_insert(d)
                }
                other => match other.listed() {
                    Some((provider, symbol)) => {
                        let symbol = symbol.trim().to_uppercase();
                        if symbol.is_empty() {
                            continue;
                        }
                        listed.entry((provider.to_owned(), symbol)).or_insert(d)
                    }
                    None => continue,
                },
            };
            *slot = (*slot).min(d);
        }
        let listed_disabled = LISTED
            .iter()
            .filter(|p| !is_enabled(store, p.id))
            .map(|p| p.id.to_owned())
            .collect();
        let keys = NEEDS_KEY
            .iter()
            .filter_map(|id| crate::api_key(store, id).map(|k| (id.to_string(), ApiKey(k))))
            .collect();
        let tax_rates = store.setting(TAX_RATES_KEY)?.as_deref() == Some("true");
        let fx_from = if base != "TRY" || tax_rates {
            first_any
        } else {
            first_foreign
        };
        Ok(PlanInput {
            tefas,
            fx_from,
            tefas_enabled: is_enabled(store, tefas::SOURCE),
            tcmb_enabled: is_enabled(store, tcmb::SOURCE),
            listed,
            listed_disabled,
            keys,
        })
    }
}

// ---------------------------------------------------------------- plan

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TefasUnit {
    pub code: String,
    /// `YYYY-MM`
    pub period: String,
    pub from: Date,
    pub to: Date,
    /// Already fetched, but asked for again for late publications.
    pub recheck: bool,
}

/// One request's worth of listed history: every missing year of one symbol,
/// from the earliest missing year through today.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedUnit {
    pub source: String,
    pub symbol: String,
    /// `YYYY` and whether it was already fetched (asked again as a recheck).
    pub years: Vec<(String, bool)>,
    pub from: Date,
    pub to: Date,
}

/// Plan state of one listed provider.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListedPlan {
    pub enabled: bool,
    /// Some asset needs history from this provider.
    pub needed: bool,
    /// The provider needs a key and none is saved.
    pub needs_key: bool,
    pub key: Option<ApiKey>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Missing fund-months, newest first.
    pub tefas: Vec<TefasUnit>,
    /// Missing TCMB business days, newest first.
    pub tcmb: Vec<Date>,
    /// Some fund history is older than TEFAS serves.
    pub tefas_limited: bool,
    /// TEFAS has funds to backfill (even if nothing is missing).
    pub tefas_needed: bool,
    pub tcmb_needed: bool,
    pub tefas_enabled: bool,
    pub tcmb_enabled: bool,
    /// Planned TCMB days that were already fetched (rechecks).
    pub tcmb_done: BTreeSet<Date>,
    /// Missing listed history, one unit per symbol.
    pub listed: Vec<ListedUnit>,
    /// Per listed provider id.
    pub listed_state: BTreeMap<String, ListedPlan>,
}

impl Plan {
    /// Fund-months never fetched.
    pub fn tefas_missing(&self) -> usize {
        self.tefas.iter().filter(|u| !u.recheck).count()
    }

    /// Symbol-years planned for a listed provider.
    pub fn listed_planned(&self, source: &str) -> usize {
        self.listed
            .iter()
            .filter(|u| u.source == source)
            .map(|u| u.years.len())
            .sum()
    }

    /// Symbol-years never fetched for a listed provider.
    pub fn listed_missing(&self, source: &str) -> usize {
        self.listed
            .iter()
            .filter(|u| u.source == source)
            .flat_map(|u| &u.years)
            .filter(|(_, fetched)| !fetched)
            .count()
    }

    /// TCMB days never fetched.
    pub fn tcmb_missing(&self) -> usize {
        self.tcmb
            .iter()
            .filter(|d| !self.tcmb_done.contains(d))
            .count()
    }
}

fn month_start(d: NaiveDate) -> NaiveDate {
    d.with_day(1).expect("day 1")
}

fn month_end(d: NaiveDate) -> NaiveDate {
    let next = if d.month() == 12 {
        NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)
    }
    .expect("valid month");
    next.pred_opt().expect("valid day")
}

/// Work out the missing units. `today` is the local date.
pub fn plan(input: &PlanInput, cache: &HistoryCache, today: Date) -> Result<Plan, StorageError> {
    let t = today.0;
    let recheck_from = t - Days::days(RECHECK_DAYS - 1);
    let mut out = Plan {
        tefas_enabled: input.tefas_enabled,
        tcmb_enabled: input.tcmb_enabled,
        tefas_needed: !input.tefas.is_empty(),
        tcmb_needed: input.fx_from.is_some(),
        ..Default::default()
    };

    if input.tefas_enabled {
        let earliest = tefas::earliest_history_date(today).0;
        for (code, first) in &input.tefas {
            if first.0 < earliest {
                out.tefas_limited = true;
            }
            let start = first.0.max(earliest);
            if start > t {
                continue;
            }
            let done = cache.fetched(tefas::SOURCE, code)?;
            let mut m = month_start(t);
            loop {
                let from = m.max(start);
                let to = month_end(m).min(t);
                let period = m.format("%Y-%m").to_string();
                let fetched = done.contains_key(&period);
                if to >= recheck_from || !fetched {
                    out.tefas.push(TefasUnit {
                        code: code.clone(),
                        period,
                        from: Date(from),
                        to: Date(to),
                        recheck: fetched,
                    });
                }
                if m <= month_start(start) {
                    break;
                }
                m = month_start(m.pred_opt().expect("valid day"));
            }
        }
        // Newest first across funds.
        out.tefas
            .sort_by(|a, b| b.period.cmp(&a.period).then(a.code.cmp(&b.code)));
    }

    for p in LISTED {
        let enabled = !input.listed_disabled.contains(p.id);
        let key = input.keys.get(p.id).cloned();
        let needs_key = NEEDS_KEY.contains(&p.id) && key.is_none();
        let needed = input.listed.keys().any(|(src, _)| src == p.id);
        out.listed_state.insert(
            p.id.to_owned(),
            ListedPlan {
                enabled,
                needed,
                needs_key,
                key,
            },
        );
        if !enabled || needs_key {
            continue;
        }
        for ((src, symbol), first) in input.listed.iter().filter(|((src, _), _)| src == p.id) {
            if first.0 > t {
                continue;
            }
            let done = cache.fetched(src, symbol)?;
            let years: Vec<i32> = (first.0.year()..=t.year()).collect();
            // The earliest year not yet fetched; the current year is always rechecked.
            let earliest = years
                .iter()
                .copied()
                .find(|y| !done.contains_key(&y.to_string()))
                .unwrap_or(t.year());
            let span: Vec<(String, bool)> = (earliest..=t.year())
                .map(|y| (y.to_string(), done.contains_key(&y.to_string())))
                .collect();
            let from = NaiveDate::from_ymd_opt(earliest, 1, 1)
                .expect("valid date")
                .max(first.0);
            out.listed.push(ListedUnit {
                source: src.clone(),
                symbol: symbol.clone(),
                years: span,
                from: Date(from),
                to: today,
            });
        }
    }

    if let (true, Some(from)) = (input.tcmb_enabled, input.fx_from) {
        let done = cache.fetched(tcmb::SOURCE, TCMB_KEY)?;
        let mut d = t;
        while d >= from.0 {
            let weekday = !matches!(d.weekday(), Weekday::Sat | Weekday::Sun);
            let fetched = done.contains_key(&d.to_string());
            if weekday && (d >= recheck_from || !fetched) {
                out.tcmb.push(Date(d));
                if fetched {
                    out.tcmb_done.insert(Date(d));
                }
            }
            d = d.pred_opt().expect("valid day");
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- progress

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum BackfillState {
    /// Nothing needs history from this provider.
    Idle,
    /// Waiting for the next refresh to start the backfill.
    Waiting,
    Running,
    Done,
    Failed,
    Disabled,
    /// The provider needs an API key and none is saved.
    NeedsKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SourceProgress {
    pub id: String,
    pub name: String,
    pub state: BackfillState,
    /// Units fetched in this run.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub done: usize,
    /// Units this run set out to fetch.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub planned: usize,
    /// "months" (fund-months), "days" or "years" (symbol-years).
    pub unit: String,
    pub error: Option<String>,
    /// For example that prices older than five years are not available.
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackfillProgress {
    pub sources: Vec<SourceProgress>,
    /// Where the history cache lives, when known.
    pub cache_path: Option<String>,
}

pub const LIMIT_NOTE: &str = "Fund prices older than five years are not available from TEFAS.";

impl BackfillProgress {
    /// The state before any backfill ran in this session.
    pub fn waiting(cache_path: Option<String>) -> Self {
        let src = |id: &str, name: &str, unit: &str| SourceProgress {
            id: id.into(),
            name: name.into(),
            state: BackfillState::Waiting,
            done: 0,
            planned: 0,
            unit: unit.into(),
            error: None,
            note: None,
        };
        BackfillProgress {
            sources: vec![
                src(tefas::SOURCE, "TEFAS", "months"),
                src(tcmb::SOURCE, "TCMB", "days"),
                src(yahoo::SOURCE, "Yahoo Finance", "years"),
                src(twelvedata::SOURCE, "Twelve Data", "years"),
            ],
            cache_path,
        }
    }

    /// Progress at the start of running `plan`.
    pub fn for_plan(plan: &Plan, cache_path: Option<String>) -> Self {
        let mut p = Self::waiting(cache_path);
        let listed = |id: &str| {
            let st = plan.listed_state.get(id).cloned().unwrap_or_default();
            (st.enabled, st.needed, st.needs_key, plan.listed_planned(id))
        };
        let rows = [
            (
                plan.tefas_enabled,
                plan.tefas_needed,
                false,
                plan.tefas.len(),
            ),
            (plan.tcmb_enabled, plan.tcmb_needed, false, plan.tcmb.len()),
            listed(yahoo::SOURCE),
            listed(twelvedata::SOURCE),
        ];
        for (s, (enabled, needed, needs_key, n)) in p.sources.iter_mut().zip(rows) {
            s.planned = n;
            s.state = if !enabled {
                BackfillState::Disabled
            } else if !needed {
                BackfillState::Idle
            } else if needs_key {
                BackfillState::NeedsKey
            } else if n == 0 {
                BackfillState::Done
            } else {
                BackfillState::Running
            };
        }
        if plan.tefas_limited {
            p.sources[0].note = Some(LIMIT_NOTE.into());
        }
        p
    }

    /// The state between runs: what is still missing, without fetching anything.
    pub fn at_rest(plan: &Plan, cache_path: Option<String>) -> Self {
        let mut p = Self::for_plan(plan, cache_path);
        let missing = [
            plan.tefas_missing(),
            plan.tcmb_missing(),
            plan.listed_missing(yahoo::SOURCE),
            plan.listed_missing(twelvedata::SOURCE),
        ];
        for (s, n) in p.sources.iter_mut().zip(missing) {
            if s.state == BackfillState::Running || s.state == BackfillState::Done {
                s.done = 0;
                s.planned = n;
                s.state = if n == 0 {
                    BackfillState::Done
                } else {
                    BackfillState::Waiting
                };
            }
        }
        p
    }

    pub fn source(&self, id: &str) -> Option<&SourceProgress> {
        self.sources.iter().find(|s| s.id == id)
    }

    fn source_mut(&mut self, id: &str) -> &mut SourceProgress {
        self.sources
            .iter_mut()
            .find(|s| s.id == id)
            .expect("known source")
    }

    pub fn is_running(&self) -> bool {
        self.sources
            .iter()
            .any(|s| s.state == BackfillState::Running)
    }
}

// ---------------------------------------------------------------- run

#[derive(Clone, Copy)]
enum Next {
    Tefas,
    Tcmb,
    /// Index into [`LISTED`].
    Listed(usize),
}

/// Fetch one fund-month, trying fund types when the code's type is unknown.
/// Returns the unit to commit, or an error message.
fn fetch_tefas_unit(
    http: &dyn Http,
    cache: &mut HistoryCache,
    u: &TefasUnit,
    clock: &mut Duration,
    now: &str,
) -> Result<FetchedUnit, String> {
    let known = cache.fund_type(&u.code).map_err(|e| e.to_string())?;
    let types: Vec<String> = match known {
        Some(t) => vec![t],
        None => TRY_TYPES.iter().map(|t| t.to_string()).collect(),
    };
    let mut status = FetchStatus::None;
    let mut prices = Vec::new();
    for (i, fund_type) in types.iter().enumerate() {
        if i > 0 {
            http.pause(tefas::MIN_INTERVAL);
            *clock += tefas::MIN_INTERVAL;
        }
        match tefas::fetch_range(http, fund_type, &u.code, u.from, u.to) {
            Ok(tefas::MonthFetch::TooOld) => {
                status = FetchStatus::TooOld;
                break;
            }
            Ok(tefas::MonthFetch::Prices(rows)) if !rows.is_empty() => {
                if types.len() > 1 {
                    cache
                        .set_fund_type(&u.code, fund_type)
                        .map_err(|e| e.to_string())?;
                }
                status = FetchStatus::Ok;
                prices = rows;
                break;
            }
            Ok(tefas::MonthFetch::Prices(_)) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(FetchedUnit {
        source: tefas::SOURCE.into(),
        key: u.code.clone(),
        period: u.period.clone(),
        status,
        fund_prices: prices
            .into_iter()
            .map(|p| CachedFundPrice {
                code: p.code,
                date: p.date.to_string(),
                price: p.price.normalize().to_string(),
            })
            .collect(),
        rates: vec![],
        fetched_at: now.into(),
    })
}

/// Fetch one listed unit (all its years in one span). Twelve Data spans longer
/// than its point limit take several requests; their spacing advances `clock`.
fn fetch_listed_unit(
    http: &dyn Http,
    provider: ListedProvider,
    key: Option<&ApiKey>,
    u: &ListedUnit,
    clock: &mut Duration,
) -> Result<Vec<Close>, ListedError> {
    match provider.id {
        yahoo::SOURCE => yahoo::fetch_span(http, &u.symbol, u.from, u.to),
        _ => {
            let key = key.map(|k| k.0.as_str()).unwrap_or("");
            let extra = twelvedata::spans(u.from, u.to).len().saturating_sub(1) as u32;
            let r = twelvedata::fetch_span(http, key, &u.symbol, u.from, u.to);
            *clock += provider.interval * extra;
            r
        }
    }
}

/// Commit a listed unit: its closes, and each year as ok (has closes) or none.
fn commit_listed_unit(
    cache: &mut HistoryCache,
    u: &ListedUnit,
    closes: &[Close],
    now: &str,
) -> Result<(), StorageError> {
    let with_data: BTreeSet<String> = closes.iter().map(|c| c.date.0.year().to_string()).collect();
    let years: Vec<(String, FetchStatus)> = u
        .years
        .iter()
        .map(|(y, _)| {
            let status = if with_data.contains(y) {
                FetchStatus::Ok
            } else {
                FetchStatus::None
            };
            (y.clone(), status)
        })
        .collect();
    let prices = crate::refresh::cached_closes(&u.source, &u.symbol, closes);
    cache.commit_listed(&u.source, &u.symbol, &years, &prices, now)
}

/// Run a plan. Requests are spaced by provider (TEFAS 10 s, TCMB 1 s, Yahoo
/// 2 s, Twelve Data 8 s) and the providers are interleaved, newest units
/// first. A source stops at its first error (an unknown listed symbol is noted
/// and skipped); `cancel` stops everything between units. `notify` is called
/// with the progress after every unit.
pub fn run(
    plan: &Plan,
    cache: &mut HistoryCache,
    http: &dyn Http,
    cancel: &AtomicBool,
    now: &dyn Fn() -> String,
    notify: &mut dyn FnMut(&BackfillProgress),
) -> BackfillProgress {
    let mut progress =
        BackfillProgress::for_plan(plan, cache.path().map(|p| p.display().to_string()));
    notify(&progress);
    let running = |p: &BackfillProgress, id: &str| {
        p.source(id).map(|s| s.state) == Some(BackfillState::Running)
    };
    let mut tefas_q = plan.tefas.iter().peekable();
    let mut tcmb_q = plan.tcmb.iter().peekable();
    let mut listed_q: Vec<_> = LISTED
        .iter()
        .map(|p| {
            plan.listed
                .iter()
                .filter(move |u| u.source == p.id)
                .peekable()
        })
        .collect();
    // Virtual clock: only pauses count, so real spacing is never shorter.
    let mut clock = Duration::ZERO;
    let mut tefas_at = Duration::ZERO;
    let mut tcmb_at = Duration::ZERO;
    let mut listed_at = [Duration::ZERO; LISTED.len()];
    let mut tefas_on = running(&progress, tefas::SOURCE);
    let mut tcmb_on = running(&progress, tcmb::SOURCE);
    let mut listed_on: Vec<bool> = LISTED.iter().map(|p| running(&progress, p.id)).collect();

    loop {
        if cancel.load(Ordering::SeqCst) {
            for s in progress.sources.iter_mut() {
                if s.state == BackfillState::Running {
                    s.state = BackfillState::Waiting;
                }
            }
            notify(&progress);
            break;
        }
        // The ready source whose next request is due first; ties keep this order.
        let mut ready: Vec<(Duration, Next)> = Vec::new();
        if tefas_on && tefas_q.peek().is_some() {
            ready.push((tefas_at, Next::Tefas));
        }
        if tcmb_on && tcmb_q.peek().is_some() {
            ready.push((tcmb_at, Next::Tcmb));
        }
        for (i, q) in listed_q.iter_mut().enumerate() {
            if listed_on[i] && q.peek().is_some() {
                ready.push((listed_at[i], Next::Listed(i)));
            }
        }
        let Some(&(at, next)) = ready.iter().min_by_key(|(at, _)| *at) else {
            break;
        };
        if at > clock {
            http.pause(at - clock);
            clock = at;
        }
        let stamp = now();
        match next {
            Next::Tefas => {
                let u = tefas_q.next().expect("peeked");
                let result = fetch_tefas_unit(http, cache, u, &mut clock, &stamp)
                    .and_then(|unit| cache.commit_unit(&unit).map_err(|e| e.to_string()));
                tefas_at = clock + tefas::MIN_INTERVAL;
                let s = progress.source_mut(tefas::SOURCE);
                match result {
                    Ok(()) => {
                        s.done += 1;
                        if tefas_q.peek().is_none() {
                            s.state = BackfillState::Done;
                        }
                    }
                    Err(e) => {
                        s.state = BackfillState::Failed;
                        s.error = Some(e);
                        tefas_on = false;
                    }
                }
            }
            Next::Tcmb => {
                let d = *tcmb_q.next().expect("peeked");
                let result = tcmb::fetch_day(http, d)
                    .map_err(|e| e.to_string())
                    .and_then(|rates| {
                        crate::refresh::cache_tcmb_day(cache, d, &rates.unwrap_or_default(), &stamp)
                            .map_err(|e| e.to_string())
                    });
                tcmb_at = clock + TCMB_INTERVAL;
                let s = progress.source_mut(tcmb::SOURCE);
                match result {
                    Ok(()) => {
                        s.done += 1;
                        if tcmb_q.peek().is_none() {
                            s.state = BackfillState::Done;
                        }
                    }
                    Err(e) => {
                        s.state = BackfillState::Failed;
                        s.error = Some(e);
                        tcmb_on = false;
                    }
                }
            }
            Next::Listed(i) => {
                let provider = LISTED[i];
                let u = listed_q[i].next().expect("peeked");
                let key = plan
                    .listed_state
                    .get(provider.id)
                    .and_then(|st| st.key.as_ref());
                let mut note = None;
                let result = match fetch_listed_unit(http, provider, key, u, &mut clock) {
                    Ok(closes) => {
                        commit_listed_unit(cache, u, &closes, &stamp).map_err(|e| e.to_string())
                    }
                    Err(e @ ListedError::NotFound(_)) => {
                        note = Some(e.to_string());
                        commit_listed_unit(cache, u, &[], &stamp).map_err(|e| e.to_string())
                    }
                    Err(e) => Err(match key {
                        Some(k) => twelvedata::scrub(&e.to_string(), &k.0),
                        None => e.to_string(),
                    }),
                };
                listed_at[i] = clock + provider.interval;
                let s = progress.source_mut(provider.id);
                if let Some(n) = note {
                    s.note = Some(match s.note.take() {
                        Some(prev) => format!("{prev}; {n}"),
                        None => n,
                    });
                }
                match result {
                    Ok(()) => {
                        s.done += u.years.len();
                        if listed_q[i].peek().is_none() {
                            s.state = BackfillState::Done;
                        }
                    }
                    Err(e) => {
                        s.state = BackfillState::Failed;
                        s.error = Some(e);
                        listed_on[i] = false;
                    }
                }
            }
        }
        notify(&progress);
    }
    progress
}

/// Every fund code a plan touches (for reporting).
pub fn plan_codes(plan: &Plan) -> BTreeSet<&str> {
    plan.tefas.iter().map(|u| u.code.as_str()).collect()
}
