//! Inflation indices: Eurostat, BLS and ONS parsing, and the refresh step that
//! fetches only the base currency's series (inflation spec).

use std::sync::Mutex;
use std::time::Duration;

use mizan_core::*;
use mizan_import::{parse_batch_value, Date};
use mizan_market::inflation::{self, YearMonth};
use mizan_market::*;
use mizan_storage::{CachedIndexValue, HistoryCache, Store};

fn fixture(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn ym(s: &str) -> YearMonth {
    YearMonth::parse(s).unwrap()
}

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

fn values(v: &[inflation::IndexValue]) -> Vec<(String, String)> {
    v.iter()
        .map(|x| (x.month.to_string(), x.value.to_string()))
        .collect()
}

// ---------------------------------------------------------------- parsing

#[test]
fn eurostat_parses_euro_area_and_turkiye() {
    let body = fixture("eurostat/hicp_ea_tr.json");
    let ea = eurostat::parse(&body, "EA").unwrap();
    let tr = eurostat::parse(&body, "TR").unwrap();
    assert_eq!(ea.len(), 5);
    assert_eq!(
        values(&ea).last().unwrap(),
        &("2026-09".to_string(), "134.31".to_string())
    );
    assert_eq!(values(&ea)[3], ("2026-08".to_string(), "133.5".to_string()));
    assert_eq!(
        values(&tr).last().unwrap(),
        &("2026-09".to_string(), "1675.96".to_string())
    );
    assert_eq!(tr[0].month, ym("2026-05"));
    assert!(eurostat::parse(&fixture("eurostat/unknown_geo.json"), "XX")
        .unwrap()
        .is_empty());
    assert_eq!(
        eurostat::url("TR:I15", ym("2025-02")).unwrap(),
        format!(
            "{}?format=JSON&geo=TR&unit=I15&coicop18=TOTAL&sinceTimePeriod=2025-02",
            eurostat::BASE
        )
    );
}

#[test]
fn eurostat_error_body_is_reported() {
    let body = r#"{"error":[{"status":400,"id":150,"label":"INVALID_QUERY_DIMENSION"}]}"#;
    assert_eq!(
        eurostat::parse(body, "TR"),
        Err(inflation::InflationError::Api(
            "INVALID_QUERY_DIMENSION".into()
        ))
    );
}

#[test]
fn bls_parses_monthly_values_and_skips_the_annual_average() {
    let v = bls::parse(&fixture("bls/cpi_2017_2026.json")).unwrap();
    assert_eq!(v.first().unwrap().month, ym("2017-01"));
    assert_eq!(
        values(&v).last().unwrap(),
        &("2026-09".to_string(), "336.94".to_string())
    );
    assert!(v.windows(2).all(|w| w[0].month < w[1].month));
    // October 2025 was published as "-" (no data collected) and is left out.
    assert!(v.iter().all(|x| x.month != ym("2025-10")));
    assert!(v.iter().any(|x| x.month == ym("2025-11")));
    let with_annual = r#"{"status":"REQUEST_SUCCEEDED","message":[],"Results":{"series":[{"data":[
        {"year":"2024","period":"M13","value":"313.689"},
        {"year":"2024","period":"M12","value":"315.605"}]}]}}"#;
    assert_eq!(
        values(&bls::parse(with_annual).unwrap()),
        [("2024-12".to_string(), "315.605".to_string())]
    );
}

#[test]
fn bls_errors_quota_and_unknown_series() {
    let quota = bls::parse(&fixture("bls/quota.json")).unwrap_err();
    assert!(quota.to_string().contains("daily threshold"), "{quota}");
    let unknown = bls::parse(&fixture("bls/unknown_series.json")).unwrap_err();
    assert!(unknown.to_string().contains("does not exist"), "{unknown}");
}

#[test]
fn bls_requests_at_most_ten_years_each_newest_first() {
    assert_eq!(bls::windows(2012, 2026), vec![(2017, 2026), (2012, 2016)]);
    assert_eq!(bls::windows(2025, 2026), vec![(2025, 2026)]);
    let http = Fake::default().on(bls::URL, 200, &fixture("bls/cpi_2017_2026.json"));
    let s = mizan_storage::inflation::series_for("USD").unwrap();
    inflation::fetch(s, &http, ym("2012-01"), day("2026-11-02")).unwrap();
    let bodies: Vec<String> = http.calls().into_iter().map(|c| c.1.unwrap()).collect();
    assert_eq!(bodies.len(), 2);
    assert!(
        bodies[0].contains(r#""startyear":"2017""#) && bodies[0].contains(r#""endyear":"2026""#)
    );
    assert!(
        bodies[1].contains(r#""startyear":"2012""#) && bodies[1].contains(r#""endyear":"2016""#)
    );
    assert!(bodies[0].contains("CUUR0000SA0"));
}

#[test]
fn ons_parses_months() {
    let v = ons::parse(&fixture("ons/d7bt.json")).unwrap();
    assert_eq!(v.len(), 21);
    assert_eq!(v[0].month, ym("2025-01"));
    assert_eq!(
        values(&v).last().unwrap(),
        &("2026-09".to_string(), "144.1".to_string())
    );
    assert_eq!(
        ons::url("D7BT"),
        "https://www.ons.gov.uk/economy/inflationandpriceindices/timeseries/d7bt/mm23/data"
    );
    assert!(ons::parse(r#"{"months":[{"date":"2026 FOO","value":"1"}]}"#).is_err());
    assert!(ons::parse(r#"{"months":[]}"#).is_err());
}

// ---------------------------------------------------------------- refresh

/// Serves fixed responses by URL prefix and records (url, body) of every call.
/// Unknown URLs answer 404, so other providers fail quietly.
#[derive(Default)]
struct Fake {
    routes: Vec<(String, u16, String)>,
    calls: Mutex<Vec<(String, Option<String>)>>,
}

impl Fake {
    fn on(mut self, prefix: &str, status: u16, body: &str) -> Self {
        self.routes.push((prefix.into(), status, body.into()));
        self
    }
    fn calls(&self) -> Vec<(String, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }
    fn calls_to(&self, prefix: &str) -> usize {
        self.calls()
            .iter()
            .filter(|(u, _)| u.starts_with(prefix))
            .count()
    }
    fn serve(&self, url: &str, body: Option<&str>) -> Result<Response, HttpError> {
        self.calls
            .lock()
            .unwrap()
            .push((url.into(), body.map(String::from)));
        Ok(
            match self.routes.iter().find(|(p, _, _)| url.starts_with(p)) {
                Some((_, status, body)) => Response {
                    status: *status,
                    body: body.clone(),
                },
                None => Response {
                    status: 404,
                    body: String::new(),
                },
            },
        )
    }
}

impl Http for Fake {
    fn get(&self, url: &str) -> Result<Response, HttpError> {
        self.serve(url, None)
    }
    fn post_json(&self, url: &str, body: &str) -> Result<Response, HttpError> {
        self.serve(url, Some(body))
    }
    fn pause(&self, _d: Duration) {}
}

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

/// A database whose first activity is 2025-03-05.
fn pension_store() -> Store {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../adapters/anadolu-hayat/fixtures/basic/expected.json"
    );
    let batch: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let parsed = parse_batch_value(batch).unwrap();
    let rb = {
        let mut r = MappingResolver {
            store: &mut store,
            policy: CREATE,
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    let p = prepare(&store, rb).unwrap();
    commit(&mut store, &p, &Decisions::new()).unwrap();
    store
}

fn run(s: &mut Store, cache: &mut HistoryCache, http: &Fake, now: &str) -> RefreshReport {
    refresh(s, Some(cache), http, day("2026-11-02"), now, || now.into()).unwrap()
}

fn source<'a>(rep: &'a RefreshReport, id: &str) -> &'a SourceReport {
    rep.sources.iter().find(|s| s.id == id).unwrap()
}

const NOW: &str = "2026-11-02T10:00:00Z";

fn eurostat_fake() -> Fake {
    Fake::default().on(eurostat::BASE, 200, &fixture("eurostat/hicp_ea_tr.json"))
}

#[test]
fn try_base_fetches_only_the_turkiye_index_and_caches_it() {
    let mut s = pension_store();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = eurostat_fake();
    let rep = run(&mut s, &mut cache, &http, NOW);

    let euro: Vec<String> = http
        .calls()
        .into_iter()
        .map(|c| c.0)
        .filter(|u| u.starts_with(eurostat::BASE))
        .collect();
    assert_eq!(
        euro,
        [eurostat::url("TR:I15", ym("2025-02")).unwrap()],
        "one request from the month before the first activity"
    );
    assert_eq!(http.calls_to(bls::URL), 0);
    assert_eq!(http.calls_to(ons::BASE), 0);

    let e = source(&rep, "eurostat");
    assert_eq!(e.status, SourceStatus::Ok);
    assert_eq!(e.updated, 5);
    assert_eq!(e.messages, ["HICP Türkiye through Sep 2026"]);
    for id in ["bls", "ons"] {
        assert_eq!(source(&rep, id).status, SourceStatus::Idle);
    }
    let cached = cache.inflation_index("eurostat", "TR:I15").unwrap();
    assert_eq!(cached.len(), 5);
    assert_eq!(cached[4].month, "2026-09");
    assert_eq!(cached[4].value, "1675.96");
    assert!(
        cache
            .inflation_check("eurostat", "TR:I15")
            .unwrap()
            .unwrap()
            .ok
    );
}

#[test]
fn second_refresh_within_a_day_makes_no_request() {
    let mut s = pension_store();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    run(&mut s, &mut cache, &eurostat_fake(), NOW);
    let http = eurostat_fake();
    let rep = run(&mut s, &mut cache, &http, "2026-11-02T13:00:00Z");
    assert_eq!(http.calls_to(eurostat::BASE), 0);
    assert_eq!(source(&rep, "eurostat").status, SourceStatus::Idle);

    // A day later the incomplete series is asked for again.
    let http = eurostat_fake();
    run(&mut s, &mut cache, &http, "2026-11-03T11:00:00Z");
    assert_eq!(http.calls_to(eurostat::BASE), 1);
}

#[test]
fn up_to_date_series_makes_no_request() {
    let mut s = pension_store();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let mut rows = vec![];
    let mut m = ym("2025-01");
    while m <= ym("2026-10") {
        rows.push(CachedIndexValue {
            month: m.to_string(),
            value: "100".into(),
        });
        m = m.shift(1);
    }
    cache
        .commit_inflation("eurostat", "TR:I15", &rows, true, "2026-10-01T00:00:00Z")
        .unwrap();
    let http = eurostat_fake();
    let rep = run(&mut s, &mut cache, &http, NOW);
    assert_eq!(http.calls_to(eurostat::BASE), 0);
    assert_eq!(
        source(&rep, "eurostat").messages,
        ["HICP Türkiye: up to date through Oct 2026"]
    );
}

#[test]
fn disabled_provider_and_empty_database_make_no_request() {
    let mut s = pension_store();
    set_enabled(&mut s, "eurostat", false).unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = eurostat_fake();
    let rep = run(&mut s, &mut cache, &http, NOW);
    assert_eq!(http.calls_to(eurostat::BASE), 0);
    assert_eq!(source(&rep, "eurostat").status, SourceStatus::Disabled);

    let mut empty = Store::open_in_memory().unwrap();
    let rep = run(&mut empty, &mut cache, &http, NOW);
    assert_eq!(http.calls_to(eurostat::BASE), 0);
    assert_eq!(source(&rep, "eurostat").status, SourceStatus::Idle);
}

#[test]
fn usd_base_asks_bls_only_and_unsupported_base_asks_nobody() {
    let mut s = pension_store();
    s.set_setting("base_currency", "USD").unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = Fake::default().on(bls::URL, 200, &fixture("bls/cpi_2017_2026.json"));
    let rep = run(&mut s, &mut cache, &http, NOW);
    assert_eq!(http.calls_to(bls::URL), 1);
    assert_eq!(http.calls_to(eurostat::BASE), 0);
    assert_eq!(source(&rep, "bls").status, SourceStatus::Ok);
    let cached = cache.inflation_index("bls", "CUUR0000SA0").unwrap();
    assert_eq!(cached.first().unwrap().month, "2025-02");
    assert_eq!(cached.last().unwrap().value, "336.94");

    s.set_setting("base_currency", "CHF").unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = Fake::default();
    let rep = run(&mut s, &mut cache, &http, NOW);
    for id in inflation::PROVIDERS {
        let r = source(&rep, id);
        assert_eq!(r.status, SourceStatus::Idle);
        assert_eq!(r.messages, ["no inflation index for CHF"]);
    }
    assert_eq!(http.calls_to(bls::URL) + http.calls_to(eurostat::BASE), 0);
}

#[test]
fn failure_keeps_cached_values_and_other_sources() {
    let mut s = pension_store();
    s.set_setting("base_currency", "GBP").unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    cache
        .commit_inflation(
            "ons",
            "D7BT",
            &[CachedIndexValue {
                month: "2026-08".into(),
                value: "143.5".into(),
            }],
            true,
            "2026-10-01T00:00:00Z",
        )
        .unwrap();
    let http = Fake::default().on(ons::BASE, 503, "busy").on(
        tcmb::TODAY_URL,
        200,
        &fixture("tcmb/today.xml"),
    );
    let rep = run(&mut s, &mut cache, &http, NOW);
    let o = source(&rep, "ons");
    assert_eq!(o.status, SourceStatus::Failed);
    assert_eq!(o.messages, ["UK CPI: HTTP 503"]);
    assert_eq!(source(&rep, "tcmb").status, SourceStatus::Ok);
    assert_eq!(
        cache.inflation_index("ons", "D7BT").unwrap()[0].value,
        "143.5"
    );
    assert!(!cache.inflation_check("ons", "D7BT").unwrap().unwrap().ok);
}
