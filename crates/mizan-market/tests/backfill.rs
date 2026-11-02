use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use mizan_import::Date;
use mizan_market::backfill::{self, BackfillState, Plan, PlanInput};
use mizan_market::*;
use mizan_storage::{FetchStatus, HistoryCache};

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

type Handler = Box<dyn Fn(&str, Option<&str>) -> Result<Response, HttpError> + Send + Sync>;

/// Answers every request with a handler and records (virtual time, url, body).
struct FakeHttp {
    handler: Handler,
    calls: Mutex<Vec<(Duration, String, Option<String>)>>,
    clock: Mutex<Duration>,
}

impl FakeHttp {
    fn new(
        f: impl Fn(&str, Option<&str>) -> Result<Response, HttpError> + Send + Sync + 'static,
    ) -> Self {
        FakeHttp {
            handler: Box::new(f),
            calls: Mutex::new(vec![]),
            clock: Mutex::new(Duration::ZERO),
        }
    }
    fn call(&self, url: &str, body: Option<&str>) -> Result<Response, HttpError> {
        let t = *self.clock.lock().unwrap();
        self.calls
            .lock()
            .unwrap()
            .push((t, url.into(), body.map(str::to_owned)));
        (self.handler)(url, body)
    }
    fn times(&self, prefix: &str) -> Vec<Duration> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, u, _)| u.starts_with(prefix))
            .map(|(t, _, _)| *t)
            .collect()
    }
    fn count(&self, prefix: &str) -> usize {
        self.times(prefix).len()
    }
}

impl Http for FakeHttp {
    fn get(&self, url: &str) -> Result<Response, HttpError> {
        self.call(url, None)
    }
    fn post_json(&self, url: &str, body: &str) -> Result<Response, HttpError> {
        self.call(url, Some(body))
    }
    fn pause(&self, d: Duration) {
        *self.clock.lock().unwrap() += d;
    }
}

fn ok(body: String) -> Result<Response, HttpError> {
    Ok(Response { status: 200, body })
}

/// One TEFAS row per request: the fund's price on the range's first day.
fn tefas_answer(body: &str) -> Result<Response, HttpError> {
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    let code = v["fonKodu"].as_str().unwrap().to_string();
    let from = v["basTarih"].as_str().unwrap();
    let date = format!("{}-{}-{}", &from[..4], &from[4..6], &from[6..8]);
    ok(format!(
        r#"{{"errorCode":null,"errorMessage":null,"resultList":[{{"fonKodu":"{code}","tarih":"{date}","fiyat":1.5}}]}}"#
    ))
}

fn tcmb_answer(url: &str) -> Result<Response, HttpError> {
    // .../kurlar/YYYYMM/DDMMYYYY.xml
    let file = url.rsplit('/').next().unwrap().trim_end_matches(".xml");
    let (d, m, y) = (&file[..2], &file[2..4], &file[4..8]);
    ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Tarih_Date Tarih="{d}.{m}.{y}"><Currency CurrencyCode="USD"><Unit>1</Unit><ForexBuying>34.5</ForexBuying></Currency></Tarih_Date>"#
    ))
}

fn healthy() -> FakeHttp {
    FakeHttp::new(|url, body| match body {
        Some(b) => tefas_answer(b),
        None => tcmb_answer(url),
    })
}

fn input(tefas: &[(&str, &str)], fx_from: Option<&str>) -> PlanInput {
    PlanInput {
        tefas: tefas
            .iter()
            .map(|(c, d)| (c.to_string(), day(d)))
            .collect::<BTreeMap<_, _>>(),
        fx_from: fx_from.map(day),
        tefas_enabled: true,
        tcmb_enabled: true,
        ..Default::default()
    }
}

fn run(
    plan: &Plan,
    cache: &mut HistoryCache,
    http: &FakeHttp,
    cancel: &AtomicBool,
) -> backfill::BackfillProgress {
    backfill::run(
        plan,
        cache,
        http,
        cancel,
        &|| "2026-11-20T10:00:00Z".into(),
        &mut |_| {},
    )
}

// ---- planner ----

#[test]
fn try_only_database_plans_no_exchange_rates() {
    let cache = HistoryCache::open_in_memory().unwrap();
    let plan = backfill::plan(
        &input(&[("AFT", "2026-09-15")], None),
        &cache,
        day("2026-11-20"),
    )
    .unwrap();
    assert!(plan.tcmb.is_empty());
    assert!(!plan.tcmb_needed);
    let periods: Vec<&str> = plan.tefas.iter().map(|u| u.period.as_str()).collect();
    assert_eq!(periods, ["2026-11", "2026-10", "2026-09"]);
    assert_eq!(plan.tefas[2].from, day("2026-09-15"));
    assert_eq!(plan.tefas[0].to, day("2026-11-20"));
}

#[test]
fn fully_cached_fund_plans_only_the_current_month() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = healthy();
    let inp = input(&[("AFT", "2024-01-10")], None);
    let first = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert_eq!(first.tefas.len(), 35);
    run(&first, &mut cache, &http, &AtomicBool::new(false));
    // A second database with the same fund, same machine.
    let again = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    let periods: Vec<&str> = again.tefas.iter().map(|u| u.period.as_str()).collect();
    assert_eq!(periods, ["2026-11"]);
}

#[test]
fn early_month_also_rechecks_the_previous_month() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let inp = input(&[("AFT", "2026-09-01")], None);
    run(
        &backfill::plan(&inp, &cache, day("2026-11-02")).unwrap(),
        &mut cache,
        &healthy(),
        &AtomicBool::new(false),
    );
    let again = backfill::plan(&inp, &cache, day("2026-11-02")).unwrap();
    let periods: Vec<&str> = again.tefas.iter().map(|u| u.period.as_str()).collect();
    assert_eq!(periods, ["2026-11", "2026-10"]);
}

#[test]
fn five_year_clamp_and_ranges_within_one_month() {
    let cache = HistoryCache::open_in_memory().unwrap();
    let plan = backfill::plan(
        &input(&[("AFT", "2015-06-01")], None),
        &cache,
        day("2026-11-20"),
    )
    .unwrap();
    assert!(plan.tefas_limited);
    let oldest = plan.tefas.last().unwrap();
    assert_eq!(oldest.period, "2021-11");
    assert_eq!(oldest.from, day("2021-11-21"));
    for u in &plan.tefas {
        assert_eq!(u.from.0.format("%Y-%m").to_string(), u.period);
        assert_eq!(
            u.to.0.format("%Y-%m").to_string(),
            u.period,
            "range stays within one month"
        );
        assert!(u.from <= u.to);
    }
    let p = backfill::BackfillProgress::for_plan(&plan, None);
    assert_eq!(
        p.source("tefas").unwrap().note.as_deref(),
        Some(backfill::LIMIT_NOTE)
    );
}

#[test]
fn tcmb_days_are_business_days_newest_first_and_skip_fetched_ones() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let inp = input(&[], Some("2026-10-01"));
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert_eq!(plan.tcmb.first(), Some(&day("2026-11-20")));
    assert_eq!(plan.tcmb.last(), Some(&day("2026-10-01")));
    assert!(plan.tcmb.iter().all(|d| !matches!(
        chrono::Datelike::weekday(&d.0),
        chrono::Weekday::Sat | chrono::Weekday::Sun
    )));
    // 2026-10-29 is a holiday: TCMB publishes nothing.
    let http = FakeHttp::new(|url, _| {
        if url.ends_with("29102026.xml") {
            Ok(Response {
                status: 404,
                body: String::new(),
            })
        } else {
            tcmb_answer(url)
        }
    });
    run(&plan, &mut cache, &http, &AtomicBool::new(false));
    assert_eq!(
        cache.fetched("tcmb", "*").unwrap().get("2026-10-29"),
        Some(&FetchStatus::None)
    );
    let again = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert!(
        !again.tcmb.contains(&day("2026-10-29")),
        "holiday not asked for again"
    );
    assert!(
        again.tcmb.iter().all(|d| d.0 >= day("2026-11-14").0),
        "only the last 7 days are rechecked"
    );
}

// ---- runner ----

#[test]
fn resumes_after_cancel_at_unit_20_of_48() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = healthy();
    let inp = input(&[("AFT", "2022-12-01")], None);
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert_eq!(plan.tefas.len(), 48);
    let cancel = AtomicBool::new(false);
    let p = backfill::run(
        &plan,
        &mut cache,
        &http,
        &cancel,
        &|| "t".into(),
        &mut |p| {
            if p.source("tefas").unwrap().done == 20 {
                cancel.store(true, Ordering::SeqCst);
            }
        },
    );
    assert_eq!(p.source("tefas").unwrap().done, 20);
    assert_eq!(p.source("tefas").unwrap().state, BackfillState::Waiting);
    // The 20 newest months are cached and usable.
    assert_eq!(cache.fetched("tefas", "AFT").unwrap().len(), 20);
    let next = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    // The current month is rechecked; the rest starts at month 21.
    assert_eq!(next.tefas.len(), 29);
    assert_eq!(next.tefas[0].period, "2026-11");
    assert_eq!(next.tefas[1].period, plan.tefas[20].period);
}

#[test]
fn tefas_failure_leaves_tcmb_running_and_cache_untouched() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    cache
        .upsert_fund_price(
            &mizan_storage::CachedFundPrice {
                code: "AFT".into(),
                date: "2026-01-05".into(),
                price: "1.10".into(),
            },
            "old",
        )
        .unwrap();
    cache.set_fund_type("AFT", "YAT").unwrap();
    let http = FakeHttp::new(|url, body| match body {
        Some(_) => Err(HttpError("connection closed".into())),
        None => tcmb_answer(url),
    });
    let plan = backfill::plan(
        &input(&[("AFT", "2026-01-02")], Some("2026-11-02")),
        &cache,
        day("2026-11-20"),
    )
    .unwrap();
    let p = run(&plan, &mut cache, &http, &AtomicBool::new(false));
    let t = p.source("tefas").unwrap();
    assert_eq!(t.state, BackfillState::Failed);
    assert!(t.error.as_deref().unwrap().contains("connection closed"));
    assert_eq!(http.count(tefas::URL), 1, "TEFAS stops at its first error");
    let c = p.source("tcmb").unwrap();
    assert_eq!((c.state, c.done), (BackfillState::Done, 15));
    assert_eq!(cache.fund_prices("AFT").unwrap()[0].value, "1.10");
    assert!(cache.fetched("tefas", "AFT").unwrap().is_empty());
}

#[test]
fn disabled_tefas_makes_no_requests() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = healthy();
    let mut inp = input(&[("AFT", "2026-01-02")], Some("2026-11-16"));
    inp.tefas_enabled = false;
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    let p = run(&plan, &mut cache, &http, &AtomicBool::new(false));
    assert_eq!(http.count(tefas::URL), 0);
    assert_eq!(p.source("tefas").unwrap().state, BackfillState::Disabled);
    assert_eq!(p.source("tcmb").unwrap().state, BackfillState::Done);
}

#[test]
fn requests_are_paced_per_provider() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    cache.set_fund_type("AFT", "YAT").unwrap();
    let http = healthy();
    let plan = backfill::plan(
        &input(&[("AFT", "2026-06-01")], Some("2026-11-02")),
        &cache,
        day("2026-11-20"),
    )
    .unwrap();
    run(&plan, &mut cache, &http, &AtomicBool::new(false));
    let gaps = |ts: Vec<Duration>| ts.windows(2).map(|w| w[1] - w[0]).min().unwrap();
    assert!(gaps(http.times(tefas::URL)) >= tefas::MIN_INTERVAL);
    assert!(gaps(http.times("https://www.tcmb.gov.tr/")) >= backfill::TCMB_INTERVAL);
    // Interleaved: TCMB does not wait for TEFAS to finish.
    let first_tcmb = http.times("https://www.tcmb.gov.tr/")[0];
    let last_tefas = *http.times(tefas::URL).last().unwrap();
    assert!(first_tcmb < last_tefas);
}

#[test]
fn unknown_fund_type_tries_yat_then_emk_and_remembers() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = FakeHttp::new(|_, body| {
        let b = body.unwrap();
        if b.contains("\"fonTipi\":\"EMK\"") {
            tefas_answer(b)
        } else {
            ok(r#"{"errorCode":null,"errorMessage":"Index 0 out of bounds for length 0","resultList":null}"#.into())
        }
    });
    let plan = backfill::plan(
        &input(&[("AH2", "2026-10-05")], None),
        &cache,
        day("2026-11-20"),
    )
    .unwrap();
    run(&plan, &mut cache, &http, &AtomicBool::new(false));
    assert_eq!(cache.fund_type("AH2").unwrap().as_deref(), Some("EMK"));
    // Two requests for the first month, then one per month with the known type.
    assert_eq!(http.count(tefas::URL), 3);
    assert_eq!(cache.fund_prices("AH2").unwrap().len(), 2);
}

#[test]
fn at_rest_counts_only_never_fetched_units() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let inp = input(&[("AFT", "2026-08-03")], Some("2026-11-02"));
    let before = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    let rest = backfill::BackfillProgress::at_rest(&before, None);
    assert_eq!(rest.source("tefas").unwrap().state, BackfillState::Waiting);
    assert_eq!(rest.source("tefas").unwrap().planned, 4);
    run(&before, &mut cache, &healthy(), &AtomicBool::new(false));
    let after = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert!(!after.tefas.is_empty(), "current month is rechecked");
    let rest = backfill::BackfillProgress::at_rest(&after, None);
    assert_eq!(rest.source("tefas").unwrap().state, BackfillState::Done);
    assert_eq!(rest.source("tcmb").unwrap().state, BackfillState::Done);
}

// ---- listed (Yahoo Finance, Twelve Data) ----

use mizan_market::listed::ApiKey;

/// A Yahoo chart body with one bar per weekday from `from` to `to` (close 100).
fn yahoo_span(url: &str) -> Result<Response, HttpError> {
    let q = |name: &str| {
        url.split(&format!("{name}="))
            .nth(1)
            .and_then(|r| r.split('&').next())
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap()
    };
    let (mut t, end) = (q("period1") + 14 * 3600, q("period2"));
    let mut ts = Vec::new();
    while t < end {
        let wd = chrono::DateTime::from_timestamp(t, 0)
            .unwrap()
            .format("%a")
            .to_string();
        if wd != "Sat" && wd != "Sun" {
            ts.push(t);
        }
        t += 86_400;
    }
    let closes = vec!["100.0"; ts.len()].join(",");
    let ts: Vec<String> = ts.iter().map(|t| t.to_string()).collect();
    ok(format!(
        r#"{{"chart":{{"result":[{{"meta":{{"currency":"EUR","priceHint":2,"gmtoffset":3600}},"timestamp":[{}],"indicators":{{"quote":[{{"close":[{closes}]}}]}}}}],"error":null}}}}"#,
        ts.join(",")
    ))
}

fn listed_http(fail_yahoo: bool) -> FakeHttp {
    FakeHttp::new(move |url, body| {
        if url.starts_with(yahoo::BASE) {
            if url.contains("NOSUCHXYZ") {
                return Ok(Response {
                    status: 404,
                    body: r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#.into(),
                });
            }
            if fail_yahoo {
                return Err(HttpError("connection reset".into()));
            }
            return yahoo_span(url);
        }
        match body {
            Some(b) => tefas_answer(b),
            None => tcmb_answer(url),
        }
    })
}

fn listed_input(rows: &[(&str, &str, &str)]) -> PlanInput {
    PlanInput {
        listed: rows
            .iter()
            .map(|(p, s, d)| ((p.to_string(), s.to_string()), day(d)))
            .collect(),
        tefas_enabled: true,
        tcmb_enabled: true,
        ..Default::default()
    }
}

fn years(u: &backfill::ListedUnit) -> Vec<(&str, bool)> {
    u.years.iter().map(|(y, f)| (y.as_str(), *f)).collect()
}

#[test]
fn listed_plan_one_span_per_symbol_from_first_activity() {
    let cache = HistoryCache::open_in_memory().unwrap();
    let plan = backfill::plan(
        &listed_input(&[("yahoo", "VWCE.DE", "2021-06-14")]),
        &cache,
        day("2026-11-20"),
    )
    .unwrap();
    assert_eq!(plan.listed.len(), 1);
    let u = &plan.listed[0];
    assert_eq!((u.from, u.to), (day("2021-06-14"), day("2026-11-20")));
    assert_eq!(
        years(u),
        [
            ("2021", false),
            ("2022", false),
            ("2023", false),
            ("2024", false),
            ("2025", false),
            ("2026", false)
        ]
    );
    assert_eq!(plan.listed_missing("yahoo"), 6);
    let p = backfill::BackfillProgress::for_plan(&plan, None);
    let y = p.source("yahoo").unwrap();
    assert_eq!(
        (y.state, y.planned, y.unit.as_str()),
        (BackfillState::Running, 6, "years")
    );
    assert_eq!(p.source("twelvedata").unwrap().state, BackfillState::Idle);
}

#[test]
fn listed_cached_years_are_skipped_and_the_current_year_rechecked() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let ok = |y: &str| (y.to_string(), FetchStatus::Ok);
    cache
        .commit_listed(
            "yahoo",
            "VWCE.DE",
            &[
                ok("2021"),
                ok("2022"),
                ok("2023"),
                ok("2024"),
                ok("2025"),
                ok("2026"),
            ],
            &[],
            "t",
        )
        .unwrap();
    let inp = listed_input(&[("yahoo", "VWCE.DE", "2021-06-14")]);
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    let u = &plan.listed[0];
    assert_eq!(years(u), [("2026", true)]);
    assert_eq!(u.from, day("2026-01-01"));
    assert_eq!(plan.listed_missing("yahoo"), 0);
    assert_eq!(
        backfill::BackfillProgress::at_rest(&plan, None)
            .source("yahoo")
            .unwrap()
            .state,
        BackfillState::Done
    );

    // An older first activity (an earlier statement imported) adds the gap only once.
    let inp = listed_input(&[("yahoo", "VWCE.DE", "2019-03-01")]);
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    let u = &plan.listed[0];
    assert_eq!(u.from, day("2019-03-01"));
    assert_eq!(
        &years(u)[..3],
        [("2019", false), ("2020", false), ("2021", true)]
    );
    assert_eq!(plan.listed_missing("yahoo"), 2);
}

#[test]
fn twelvedata_without_a_key_plans_nothing_and_waits_for_a_key() {
    let cache = HistoryCache::open_in_memory().unwrap();
    let mut inp = listed_input(&[("twelvedata", "MSFT", "2024-02-01")]);
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert!(plan.listed.is_empty());
    let p = backfill::BackfillProgress::for_plan(&plan, None);
    assert_eq!(
        p.source("twelvedata").unwrap().state,
        BackfillState::NeedsKey
    );
    assert_eq!(
        backfill::BackfillProgress::at_rest(&plan, None)
            .source("twelvedata")
            .unwrap()
            .state,
        BackfillState::NeedsKey
    );

    inp.keys
        .insert("twelvedata".into(), ApiKey("k-3f9a".into()));
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert_eq!(plan.listed.len(), 1);
    assert!(
        !format!("{plan:?}").contains("k-3f9a"),
        "key hidden in debug output"
    );

    inp.listed_disabled.insert("twelvedata".into());
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert!(plan.listed.is_empty());
    assert_eq!(
        backfill::BackfillProgress::for_plan(&plan, None)
            .source("twelvedata")
            .unwrap()
            .state,
        BackfillState::Disabled
    );
}

#[test]
fn listed_run_records_years_spaces_requests_and_counts_progress() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = listed_http(false);
    let inp = listed_input(&[
        ("yahoo", "SPY", "2020-01-02"),
        ("yahoo", "VWCE.DE", "2020-05-04"),
    ]);
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert_eq!(plan.listed_planned("yahoo"), 14);
    let mut seen = Vec::new();
    let p = backfill::run(
        &plan,
        &mut cache,
        &http,
        &AtomicBool::new(false),
        &|| "2026-11-20T10:00:00Z".into(),
        &mut |p| {
            let y = p.source("yahoo").unwrap();
            seen.push(format!("{} of {}", y.done, y.planned));
        },
    );
    assert_eq!(seen, ["0 of 14", "7 of 14", "14 of 14"]);
    assert_eq!(p.source("yahoo").unwrap().state, BackfillState::Done);
    assert_eq!(http.count(yahoo::BASE), 2, "one request per symbol");
    assert_eq!(
        http.times(yahoo::BASE),
        [Duration::ZERO, Duration::from_secs(2)]
    );
    let done = cache.fetched("yahoo", "VWCE.DE").unwrap();
    assert_eq!(done.len(), 7);
    assert!(done.values().all(|s| *s == FetchStatus::Ok));
    let closes = cache.listed_prices("yahoo", "VWCE.DE").unwrap();
    assert_eq!(closes.first().unwrap().date, "2020-05-04");
    assert_eq!(closes.last().unwrap().date, "2026-11-20");
    assert!(closes
        .iter()
        .all(|c| c.currency == "EUR" && c.close == "100"));
}

#[test]
fn listed_resumes_after_interruption() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = listed_http(false);
    let inp = listed_input(&[
        ("yahoo", "SPY", "2023-01-03"),
        ("yahoo", "VWCE.DE", "2023-01-03"),
    ]);
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    let cancel = AtomicBool::new(false);
    let p = backfill::run(
        &plan,
        &mut cache,
        &http,
        &cancel,
        &|| "t".into(),
        &mut |p| {
            if p.source("yahoo").unwrap().done > 0 {
                cancel.store(true, Ordering::SeqCst);
            }
        },
    );
    assert_eq!(p.source("yahoo").unwrap().state, BackfillState::Waiting);
    assert_eq!(cache.fetched("yahoo", "SPY").unwrap().len(), 4);
    assert!(cache.fetched("yahoo", "VWCE.DE").unwrap().is_empty());
    let again = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    assert_eq!(
        years(&again.listed[0]),
        [("2026", true)],
        "SPY: recheck only"
    );
    assert_eq!(again.listed[1].symbol, "VWCE.DE");
    assert_eq!(again.listed[1].years.len(), 4);
}

#[test]
fn yahoo_failure_does_not_stop_tefas_and_unknown_symbols_are_noted() {
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let mut inp = listed_input(&[("yahoo", "VWCE.DE", "2026-09-01")]);
    inp.tefas.insert("AFT".into(), day("2026-09-15"));
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    let p = run(
        &plan,
        &mut cache,
        &listed_http(true),
        &AtomicBool::new(false),
    );
    let y = p.source("yahoo").unwrap();
    assert_eq!(y.state, BackfillState::Failed);
    assert!(y.error.as_deref().unwrap().contains("connection reset"));
    assert_eq!(p.source("tefas").unwrap().state, BackfillState::Done);
    assert!(
        cache.fetched("yahoo", "VWCE.DE").unwrap().is_empty(),
        "failure caches nothing"
    );

    let inp = listed_input(&[
        ("yahoo", "NOSUCHXYZ", "2026-02-02"),
        ("yahoo", "VWCE.DE", "2026-02-02"),
    ]);
    let plan = backfill::plan(&inp, &cache, day("2026-11-20")).unwrap();
    let p = run(
        &plan,
        &mut cache,
        &listed_http(false),
        &AtomicBool::new(false),
    );
    let y = p.source("yahoo").unwrap();
    assert_eq!(y.state, BackfillState::Done);
    assert_eq!(y.note.as_deref(), Some("NOSUCHXYZ: symbol not found"));
    assert_eq!(
        cache.fetched("yahoo", "NOSUCHXYZ").unwrap().get("2026"),
        Some(&FetchStatus::None)
    );
    assert!(!cache.listed_prices("yahoo", "VWCE.DE").unwrap().is_empty());
}
