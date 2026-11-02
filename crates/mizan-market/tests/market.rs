use std::collections::{BTreeSet, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use mizan_import::{Date, InstrumentHint};
use mizan_market::*;
use mizan_storage::{PriceRow, PriceSourceKind, Store};

fn fixture(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

type Queue = Vec<(String, VecDeque<Result<Response, HttpError>>)>;

/// Serves queued responses per URL, records calls with a simulated clock.
#[derive(Default)]
struct FakeHttp {
    queue: Mutex<Queue>,
    calls: Mutex<Vec<(u64, String, Option<String>)>>,
    clock: Mutex<u64>,
}

impl FakeHttp {
    fn on(self, url: &str, r: Result<Response, HttpError>) -> Self {
        {
            let mut q = self.queue.lock().unwrap();
            match q.iter_mut().find(|(u, _)| u == url) {
                Some((_, v)) => v.push_back(r),
                None => q.push((url.into(), VecDeque::from([r]))),
            }
        }
        self
    }
    fn ok(self, url: &str, body: &str) -> Self {
        self.on(
            url,
            Ok(Response {
                status: 200,
                body: body.into(),
            }),
        )
    }
    fn next(&self, url: &str, body: Option<&str>) -> Result<Response, HttpError> {
        self.calls.lock().unwrap().push((
            *self.clock.lock().unwrap(),
            url.into(),
            body.map(str::to_owned),
        ));
        let mut q = self.queue.lock().unwrap();
        q.iter_mut()
            .find(|(u, _)| u == url)
            .and_then(|(_, v)| v.pop_front())
            .unwrap_or_else(|| Err(HttpError(format!("no response queued for {url}"))))
    }
    fn calls_to(&self, prefix: &str) -> Vec<(u64, Option<String>)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, u, _)| u.starts_with(prefix))
            .map(|(t, _, b)| (*t, b.clone()))
            .collect()
    }
}

impl Http for FakeHttp {
    fn get(&self, url: &str) -> Result<Response, HttpError> {
        self.next(url, None)
    }
    fn post_json(&self, url: &str, body: &str) -> Result<Response, HttpError> {
        self.next(url, Some(body))
    }
    fn pause(&self, d: Duration) {
        *self.clock.lock().unwrap() += d.as_secs();
    }
}

const EMPTY: &str =
    r#"{"errorCode":null,"errorMessage":"Index 0 out of bounds for length 0","resultList":null}"#;

// ---- 4.1 registry ----

#[test]
fn registry_lists_built_in_providers_enabled_by_default() {
    let ids: Vec<&str> = PROVIDERS.iter().map(|p| p.id).collect();
    assert_eq!(ids, vec!["tefas", "tcmb", "yahoo", "twelvedata"]);
    let mut s = Store::open_in_memory().unwrap();
    assert!(ids.iter().all(|id| is_enabled(&s, id)));
    set_enabled(&mut s, "tefas", false).unwrap();
    assert!(!is_enabled(&s, "tefas"));
    assert!(set_enabled(&mut s, "nope", true).is_err());
}

// ---- 4.2 TCMB ----

#[test]
fn tcmb_parses_recorded_file_with_its_own_date() {
    let rates = tcmb::parse(&fixture("tcmb/today.xml")).unwrap();
    assert!(rates.len() >= 20, "{}", rates.len());
    let get = |c: &str| rates.iter().find(|r| r.currency.as_str() == c).unwrap();
    assert_eq!(get("USD").try_per_unit.to_string(), "49.7747");
    assert_eq!(get("JPY").try_per_unit.to_string(), "0.315605", "unit 100");
    // The file's date, not the day it was fetched (a Sunday).
    assert!(rates.iter().all(|r| r.date == day("2026-10-30")));
}

#[test]
fn tcmb_unit_scaling_and_tolerance() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Tarih_Date Tarih="31.10.2026" Date="10/31/2026">
 <Currency Kod="JPY" CurrencyCode="JPY"><Unit>100</Unit><ForexBuying>21.50</ForexBuying><ForexSelling>21.60</ForexSelling></Currency>
 <Currency Kod="XDR" CurrencyCode="XDR"><Unit>1</Unit><ForexBuying/><ForexSelling/></Currency>
</Tarih_Date>"#;
    let rates = tcmb::parse(xml).unwrap();
    assert_eq!(
        rates.len(),
        1,
        "a currency without a forex buying rate is skipped"
    );
    assert_eq!(rates[0].try_per_unit.to_string(), "0.215");
    assert_eq!(rates[0].date, day("2026-10-31"));
    assert!(tcmb::parse("<html>not rates</html>").is_err());
}

#[test]
fn tcmb_missing_file_keeps_stored_rates() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_fx_rate(
        &mizan_storage::FxRow {
            currency: "USD".into(),
            date: "2026-10-30".into(),
            source: "tcmb".into(),
            try_per_unit: "49.7747".into(),
        },
        "t",
    )
    .unwrap();
    set_enabled(&mut s, "tefas", false).unwrap();
    let http = FakeHttp::default().on(
        tcmb::TODAY_URL,
        Ok(Response {
            status: 404,
            body: String::new(),
        }),
    );
    let rep = refresh(
        &mut s,
        None,
        &http,
        day("2026-11-01"),
        "2026-11-01T10:00:00Z",
        || "2026-11-01T10:00:01Z".into(),
    )
    .unwrap();
    let tc = rep.sources.iter().find(|r| r.id == "tcmb").unwrap();
    assert_eq!((tc.status, tc.updated), (SourceStatus::Ok, 0));
    assert_eq!(
        s.latest_fx_rate("USD", "2026-11-01")
            .unwrap()
            .unwrap()
            .try_per_unit,
        "49.7747"
    );
}

// ---- 4.3 TEFAS ----

#[test]
fn tefas_parses_recorded_responses_exactly() {
    let rows = tefas::parse(&fixture("tefas/fund_aft_week.json")).unwrap();
    assert_eq!(rows[0].code, "AFT");
    assert_eq!(
        (rows[0].date, rows[0].price.to_string()),
        (day("2026-10-30"), "0.959756".into())
    );
    let bulk = tefas::parse(&fixture("tefas/bulk_yat_two_days.json")).unwrap();
    let codes: BTreeSet<&str> = bulk.iter().map(|r| r.code.as_str()).collect();
    assert_eq!(codes, BTreeSet::from(["AAL", "AFT", "TCD"]));
    assert!(tefas::parse(&fixture("tefas/empty_range.json"))
        .unwrap()
        .is_empty());
    assert!(matches!(
        tefas::parse(&fixture("tefas/error_bad_date.json")),
        Err(tefas::TefasError::Api(_))
    ));
}

#[test]
fn tefas_request_body_uses_verified_format() {
    let b: serde_json::Value = serde_json::from_str(&tefas::request_body(
        "YAT",
        day("2026-10-26"),
        day("2026-11-01"),
    ))
    .unwrap();
    assert_eq!(b["fonTipi"], "YAT");
    assert_eq!(b["fonKodu"], serde_json::Value::Null, "bulk: all funds");
    assert_eq!(
        (b["basTarih"].as_str(), b["bitTarih"].as_str()),
        (Some("20261026"), Some("20261101"))
    );
}

#[test]
fn tefas_latest_prices_unknown_codes_and_throttle() {
    let http = FakeHttp::default()
        .ok(tefas::URL, &fixture("tefas/bulk_yat_two_days.json"))
        .ok(tefas::URL, EMPTY);
    let codes = BTreeSet::from(["AFT".to_string(), "tcd".to_string(), "ZZZ".to_string()]);
    let f = tefas::fetch_latest(&http, &codes, day("2026-04-12"));
    assert_eq!(f.error, None);
    assert_eq!(
        f.latest["AFT"].date,
        day("2026-04-10"),
        "latest of two days"
    );
    assert!(
        f.latest.contains_key("TCD"),
        "codes are matched case-insensitively"
    );
    assert_eq!(f.not_found, vec!["ZZZ".to_string()]);

    // Two requests (YAT, then EMK for the missing code), at least 10 s apart.
    let calls = http.calls_to(tefas::URL);
    assert_eq!(calls.len(), 2);
    assert!(calls[1].0 - calls[0].0 >= 10);
    assert!(calls[1].1.as_deref().unwrap().contains("\"EMK\""));
    // No 60-second window holds more than 6 calls.
    for (t, _) in &calls {
        assert!(
            calls
                .iter()
                .filter(|(u, _)| *u >= *t && *u < t + 60)
                .count()
                <= 6
        );
    }
}

#[test]
fn tefas_stops_early_when_all_codes_are_found() {
    let http = FakeHttp::default().ok(tefas::URL, &fixture("tefas/bulk_yat_two_days.json"));
    let f = tefas::fetch_latest(
        &http,
        &BTreeSet::from(["AFT".to_string()]),
        day("2026-04-12"),
    );
    assert!(f.not_found.is_empty());
    assert_eq!(http.calls_to(tefas::URL).len(), 1, "no EMK request needed");
}

// ---- 4.4 refresh ----

fn store_with_assets() -> (Store, i64, i64) {
    let mut s = Store::open_in_memory().unwrap();
    let aft = s
        .create_asset(
            "sym:AFT@",
            &InstrumentHint {
                symbol: Some("AFT".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let man = s
        .create_asset("isin:MANUAL", &InstrumentHint::default())
        .unwrap();
    s.set_price_source(
        aft,
        Some(&PriceSourceKind::Tefas {
            fund_code: "AFT".into(),
        }),
    )
    .unwrap();
    s.set_price_source(man, Some(&PriceSourceKind::Manual))
        .unwrap();
    (s, aft, man)
}

fn run(s: &mut Store, http: &FakeHttp) -> RefreshReport {
    refresh(
        s,
        None,
        http,
        day("2026-11-01"),
        "2026-11-01T10:00:00Z",
        || "2026-11-01T10:00:05Z".into(),
    )
    .unwrap()
}

#[test]
fn refresh_stores_prices_and_rates() {
    let (mut s, aft, man) = store_with_assets();
    let http = FakeHttp::default()
        .ok(tefas::URL, &fixture("tefas/fund_aft_week.json"))
        .ok(tcmb::TODAY_URL, &fixture("tcmb/today.xml"));
    let rep = run(&mut s, &http);
    assert!(!rep.any_failed(), "{rep:?}");
    let p = s.latest_price(aft, "2026-11-01").unwrap().unwrap();
    assert_eq!(
        (
            p.value.as_str(),
            p.date.as_str(),
            p.source.as_str(),
            p.currency.as_str()
        ),
        ("0.959756", "2026-10-30", "tefas", "TRY")
    );
    assert_eq!(p.fetched_at, "2026-11-01T10:00:00Z");
    assert!(
        s.latest_price(man, "2026-11-01").unwrap().is_none(),
        "manual asset never fetched"
    );
    assert_eq!(
        s.latest_fx_rate("JPY", "2026-11-01")
            .unwrap()
            .unwrap()
            .try_per_unit,
        "0.315605"
    );
    assert_eq!(
        s.setting("last_refresh_at").unwrap().as_deref(),
        Some("2026-11-01T10:00:05Z")
    );
    assert_eq!(
        s.last_refresh_report().unwrap().unwrap().0,
        "2026-11-01T10:00:05Z"
    );
}

#[test]
fn tefas_failure_is_isolated_and_keeps_stored_prices() {
    let (mut s, aft, _) = store_with_assets();
    s.upsert_provider_price(&PriceRow {
        asset_id: aft,
        date: "2026-10-29".into(),
        source: "tefas".into(),
        value: "0.95".into(),
        currency: "TRY".into(),
        fetched_at: "old".into(),
    })
    .unwrap();
    let http = FakeHttp::default()
        .on(tefas::URL, Err(HttpError("connection reset".into())))
        .ok(tcmb::TODAY_URL, &fixture("tcmb/today.xml"));
    let rep = run(&mut s, &http);
    let tf = rep.sources.iter().find(|r| r.id == "tefas").unwrap();
    assert_eq!(tf.status, SourceStatus::Failed);
    assert!(tf.messages[0].contains("connection reset"));
    assert_eq!(
        rep.sources.iter().find(|r| r.id == "tcmb").unwrap().status,
        SourceStatus::Ok
    );
    assert_eq!(
        s.latest_price(aft, "2026-11-01").unwrap().unwrap().value,
        "0.95",
        "unchanged"
    );
    assert!(
        s.latest_fx_rate("USD", "2026-11-01").unwrap().is_some(),
        "rates still refreshed"
    );
    assert_eq!(
        s.setting("last_refresh_at").unwrap(),
        None,
        "a failed refresh is not a successful one"
    );
}

#[test]
fn unknown_fund_code_is_reported_per_asset() {
    let (mut s, aft, _) = store_with_assets();
    s.set_price_source(
        aft,
        Some(&PriceSourceKind::Tefas {
            fund_code: "ZZZ".into(),
        }),
    )
    .unwrap();
    let http = FakeHttp::default()
        .ok(tefas::URL, &fixture("tefas/fund_aft_week.json"))
        .ok(tefas::URL, EMPTY)
        .ok(tcmb::TODAY_URL, &fixture("tcmb/today.xml"));
    let rep = run(&mut s, &http);
    let tf = rep.sources.iter().find(|r| r.id == "tefas").unwrap();
    assert_eq!(tf.status, SourceStatus::Ok);
    assert_eq!(tf.messages, vec!["ZZZ: fund not found".to_string()]);
}

#[test]
fn disabled_providers_and_manual_assets_make_no_requests() {
    let (mut s, aft, _) = store_with_assets();
    for p in PROVIDERS {
        set_enabled(&mut s, p.id, false).unwrap();
    }
    let http = FakeHttp::default();
    let rep = run(&mut s, &http);
    assert!(http.calls.lock().unwrap().is_empty());
    assert!(rep
        .sources
        .iter()
        .all(|r| r.status == SourceStatus::Disabled));

    // Only manual sources: TEFAS is idle and sends nothing.
    set_enabled(&mut s, "tefas", true).unwrap();
    s.set_price_source(aft, Some(&PriceSourceKind::Manual))
        .unwrap();
    let rep = run(&mut s, &http);
    assert!(http.calls_to(tefas::URL).is_empty());
    assert_eq!(rep.sources[0].status, SourceStatus::Idle);
}

// ---- history cache (returns engine) ----

#[test]
fn refresh_also_writes_the_history_cache() {
    let (mut s, _aft, man) = store_with_assets();
    s.add_manual_price(man, "2026-10-30", "12.50", "TRY", "2026-11-01T09:00:00Z")
        .unwrap();
    let mut cache = mizan_storage::HistoryCache::open_in_memory().unwrap();
    let http = FakeHttp::default()
        .ok(tefas::URL, &fixture("tefas/fund_aft_week.json"))
        .ok(tcmb::TODAY_URL, &fixture("tcmb/today.xml"));
    let rep = refresh(
        &mut s,
        Some(&mut cache),
        &http,
        day("2026-11-01"),
        "2026-11-01T10:00:00Z",
        || "2026-11-01T10:00:05Z".into(),
    )
    .unwrap();
    assert!(!rep.any_failed(), "{rep:?}");
    let p = cache.fund_price_on("AFT", "2026-11-01").unwrap().unwrap();
    assert_eq!(
        (p.date.as_str(), p.value.as_str()),
        ("2026-10-30", "0.959756")
    );
    assert_eq!(cache.fund_type("AFT").unwrap().as_deref(), Some("YAT"));
    let jpy = cache.rates("JPY").unwrap();
    assert_eq!(
        (jpy[0].date.as_str(), jpy[0].value.as_str()),
        ("2026-10-30", "0.315605")
    );
    assert_eq!(
        cache.fetched("tcmb", "*").unwrap().get("2026-10-30"),
        Some(&mizan_storage::FetchStatus::Ok)
    );
    // Manual prices stay private to the user database.
    assert!(cache
        .all_text_values()
        .unwrap()
        .iter()
        .all(|v| v != "12.50"));
}

#[test]
fn tefas_month_of_history_for_one_fund() {
    let http = FakeHttp::default().ok(tefas::URL, &fixture("tefas/fund_aft_2022_01.json"));
    let got =
        tefas::fetch_range(&http, "YAT", "AFT", day("2022-01-01"), day("2022-01-31")).unwrap();
    let tefas::MonthFetch::Prices(prices) = got else {
        panic!("expected prices")
    };
    assert_eq!(prices.len(), 21);
    assert_eq!(prices.last().unwrap().date, day("2022-01-03"));
    assert_eq!(prices.last().unwrap().price.to_string(), "0.16621");
    let body = http.calls_to(tefas::URL)[0].1.clone().unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["fonKodu"], "AFT");
    assert_eq!(v["fonTipi"], "YAT");
    assert_eq!(
        (v["basTarih"].as_str(), v["bitTarih"].as_str()),
        (Some("20220101"), Some("20220131"))
    );
}

#[test]
fn tefas_too_old_and_range_refusals() {
    let http = FakeHttp::default()
        .ok(tefas::URL, &fixture("tefas/error_too_old.json"))
        .ok(tefas::URL, &fixture("tefas/error_range.json"));
    assert_eq!(
        tefas::fetch_range(&http, "YAT", "AFT", day("2020-01-01"), day("2020-01-31")).unwrap(),
        tefas::MonthFetch::TooOld
    );
    let e =
        tefas::fetch_range(&http, "YAT", "AFT", day("2022-01-01"), day("2022-06-30")).unwrap_err();
    assert!(e.to_string().contains("1 ayı aşamaz"), "{e}");
}

#[test]
fn tefas_earliest_history_date_is_five_years_back_plus_a_day() {
    assert_eq!(
        tefas::earliest_history_date(day("2026-11-02")),
        day("2021-11-03")
    );
    assert_eq!(
        tefas::earliest_history_date(day("2028-02-29")),
        day("2023-03-01")
    );
}

#[test]
fn tcmb_archive_day_and_holiday() {
    let d = day("2025-10-30");
    assert_eq!(
        tcmb::archive_url(d),
        "https://www.tcmb.gov.tr/kurlar/202510/30102025.xml"
    );
    let holiday = day("2025-10-29");
    let http = FakeHttp::default()
        .ok(
            &tcmb::archive_url(d),
            &fixture("tcmb/archive_2025-10-30.xml"),
        )
        .on(
            &tcmb::archive_url(holiday),
            Ok(Response {
                status: 404,
                body: String::new(),
            }),
        );
    let rates = tcmb::fetch_day(&http, d).unwrap().unwrap();
    let jpy = rates.iter().find(|r| r.currency.as_str() == "JPY").unwrap();
    // Published per 100 JPY: 27.1907 / 100.
    assert_eq!(jpy.try_per_unit.to_string(), "0.271907");
    assert_eq!(jpy.date, d);
    assert_eq!(tcmb::fetch_day(&http, holiday).unwrap(), None);
}
