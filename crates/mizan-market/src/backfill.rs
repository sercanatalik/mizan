//! Background backfill of price and exchange-rate history into the shared cache.
//!
//! [`plan`] works out which fund-months and TCMB days are missing, using the
//! user database only for dates and fund codes. [`run`] fetches them newest
//! first within provider limits and commits each unit to the cache on its own,
//! so an interruption loses at most one unit and the next run resumes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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
/// Key under which TCMB days are recorded in the cache (rates cover every currency).
pub const TCMB_KEY: &str = "*";
/// TCMB publishes no limit; static files, fetched politely one a second.
pub const TCMB_INTERVAL: Duration = Duration::from_secs(1);
/// Days before today that are fetched again to pick up late publications.
pub const RECHECK_DAYS: i64 = 7;
/// Fund types tried, in order, when a fund's type is not known yet.
const TRY_TYPES: [&str; 2] = ["YAT", "EMK"];

// ---------------------------------------------------------------- inputs

/// What the planner needs from a user database: dates and fund codes only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanInput {
    /// First activity date per TEFAS fund code.
    pub tefas: BTreeMap<String, Date>,
    /// First date exchange rates are needed, if ever.
    pub fx_from: Option<Date>,
    pub tefas_enabled: bool,
    pub tcmb_enabled: bool,
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
        let mut first_any: Option<Date> = None;
        let mut first_foreign: Option<Date> = None;
        for a in store.list_accounts()? {
            for s in store.all_activities(a.id)? {
                let d = s.record.date;
                first_any = Some(first_any.map_or(d, |f| f.min(d)));
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
        // The benchmark is compared from the portfolio's first activity, held or not.
        let benchmark = store
            .setting(BENCHMARK_KEY)?
            .and_then(|v| v.trim().parse::<i64>().ok())
            .map(|id| store.canonical_asset(id))
            .transpose()?;
        let mut tefas: BTreeMap<String, Date> = BTreeMap::new();
        for (asset, source) in store.price_sources()? {
            let PriceSourceKind::Tefas { fund_code } = source else {
                continue;
            };
            let code = fund_code.trim().to_uppercase();
            if code.is_empty() {
                continue;
            }
            let held = first_by_asset.get(&asset).copied();
            let as_benchmark = (benchmark == Some(asset)).then_some(first_any).flatten();
            if let Some(d) = [held, as_benchmark].into_iter().flatten().min() {
                let e = tefas.entry(code).or_insert(d);
                *e = (*e).min(d);
            }
        }
        let fx_from = if base != "TRY" {
            first_any
        } else {
            first_foreign
        };
        Ok(PlanInput {
            tefas,
            fx_from,
            tefas_enabled: is_enabled(store, tefas::SOURCE),
            tcmb_enabled: is_enabled(store, tcmb::SOURCE),
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
}

impl Plan {
    /// Fund-months never fetched.
    pub fn tefas_missing(&self) -> usize {
        self.tefas.iter().filter(|u| !u.recheck).count()
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
    /// "months" (fund-months) or "days".
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
            ],
            cache_path,
        }
    }

    /// Progress at the start of running `plan`.
    pub fn for_plan(plan: &Plan, cache_path: Option<String>) -> Self {
        let mut p = Self::waiting(cache_path);
        let rows = [
            (plan.tefas_enabled, plan.tefas_needed, plan.tefas.len()),
            (plan.tcmb_enabled, plan.tcmb_needed, plan.tcmb.len()),
        ];
        for (s, (enabled, needed, n)) in p.sources.iter_mut().zip(rows) {
            s.planned = n;
            s.state = if !enabled {
                BackfillState::Disabled
            } else if !needed {
                BackfillState::Idle
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
        let missing = [plan.tefas_missing(), plan.tcmb_missing()];
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

enum Next {
    Tefas,
    Tcmb,
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

/// Run a plan. Requests are spaced by provider (TEFAS 10 s, TCMB 1 s) and the
/// two providers are interleaved, newest units first. A source stops at its
/// first error; `cancel` stops everything between units. `notify` is called
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
    let mut tefas_q = plan.tefas.iter().peekable();
    let mut tcmb_q = plan.tcmb.iter().peekable();
    // Virtual clock: only pauses count, so real spacing is never shorter.
    let mut clock = Duration::ZERO;
    let mut tefas_at = Duration::ZERO;
    let mut tcmb_at = Duration::ZERO;
    let mut tefas_on =
        progress.source(tefas::SOURCE).map(|s| s.state) == Some(BackfillState::Running);
    let mut tcmb_on =
        progress.source(tcmb::SOURCE).map(|s| s.state) == Some(BackfillState::Running);

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
        let tefas_ready = tefas_on && tefas_q.peek().is_some();
        let tcmb_ready = tcmb_on && tcmb_q.peek().is_some();
        let next = match (tefas_ready, tcmb_ready) {
            (false, false) => break,
            (true, false) => Next::Tefas,
            (false, true) => Next::Tcmb,
            (true, true) if tefas_at <= tcmb_at => Next::Tefas,
            (true, true) => Next::Tcmb,
        };
        let at = match next {
            Next::Tefas => tefas_at,
            Next::Tcmb => tcmb_at,
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
        }
        notify(&progress);
    }
    progress
}

/// Every fund code a plan touches (for reporting).
pub fn plan_codes(plan: &Plan) -> BTreeSet<&str> {
    plan.tefas.iter().map(|u| u.code.as_str()).collect()
}
