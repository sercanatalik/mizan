//! The Accounts screen and account page (account-detail spec), the second
//! currency, and merges with the import pipeline.

use mizan_core::accounts::*;
use mizan_core::returns::{returns_view, Inputs};
use mizan_core::valuation::BookMarket;
use mizan_core::*;
use mizan_import::{parse_batch_value, Date};
use mizan_storage::{CachedFundPrice, CachedRate, HistoryCache, PriceSourceKind, Store};
use serde_json::{json, Value};

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

fn money(a: &str, c: &str) -> Value {
    json!({"amount": a, "currency": c})
}

fn fund(sym: &str) -> Value {
    json!({"symbol": sym, "class_hint": "fund", "currency": "TRY"})
}

/// Commit records and assertions from file `file` for account hint `hint`.
fn import(
    store: &mut Store,
    file: char,
    hint: &str,
    records: Vec<Value>,
    assertions: Vec<Value>,
) -> Prepared {
    let stamp = |mut r: Value, i: usize| {
        r["source"] = json!({"adapter": "t", "adapter_version": "1", "file_sha256": file.to_string().repeat(64), "locator": format!("row {}", i + 1)});
        r["account"] = json!({"hint": hint, "name_hint": hint});
        r
    };
    let records: Vec<Value> = records
        .into_iter()
        .enumerate()
        .map(|(i, r)| stamp(r, i))
        .collect();
    let assertions: Vec<Value> = assertions
        .into_iter()
        .enumerate()
        .map(|(i, r)| stamp(r, 100 + i))
        .collect();
    let parsed = parse_batch_value(
        json!({"schema": "mizan.import/1", "records": records, "assertions": assertions}),
    )
    .unwrap();
    let rb = {
        let mut r = MappingResolver {
            store,
            policy: CREATE,
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    let p = prepare(store, rb).unwrap();
    commit(store, &p, &Decisions::new()).unwrap();
    p
}

fn price(c: &mut HistoryCache, code: &str, date: &str, p: &str) {
    c.upsert_fund_price(
        &CachedFundPrice {
            code: code.into(),
            date: date.into(),
            price: p.into(),
        },
        "t",
    )
    .unwrap();
}

fn rate(c: &mut HistoryCache, date: &str, r: &str) {
    c.upsert_rate(
        &CachedRate {
            currency: "USD".into(),
            date: date.into(),
            try_per_unit: r.into(),
            source: "tcmb".into(),
        },
        "t",
    )
    .unwrap();
}

const TODAY: &str = "2026-10-28";

/// A pension (P1) holding AH1 and BGL, bought 2025-10-01 with contributions,
/// a dividend each in 2024 and 2025, two balance checks; a second account (B1)
/// with a buy and no deposit (negative cash) and an unpriced position.
fn setup() -> (Store, HistoryCache) {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "deposit", "date": "2024-01-02", "net_cash": money("2000.00", "TRY")}),
            json!({"kind": "dividend", "date": "2024-06-03", "instrument": fund("AH1"), "net_cash": money("8.50", "TRY"), "charges": [{"type": "withholding_tax", "amount": money("1.50", "TRY")}]}),
            json!({"kind": "dividend", "date": "2025-06-02", "instrument": fund("AH1"), "net_cash": money("17.00", "TRY"), "charges": [{"type": "withholding_tax", "amount": money("3.00", "TRY")}]}),
            json!({"kind": "buy", "date": "2025-10-01", "instrument": fund("AH1"), "quantity": "1000", "net_cash": money("293.90", "TRY")}),
            json!({"kind": "buy", "date": "2025-10-01", "instrument": fund("BGL"), "quantity": "10000", "net_cash": money("600.00", "TRY")}),
        ],
        vec![
            json!({"as_of": "2026-09-30", "positions": [{"instrument": fund("AH1"), "quantity": "1000"}]}),
            json!({"as_of": "2026-10-27", "positions": [{"instrument": fund("BGL"), "quantity": "9999"}]}),
        ],
    );
    import(
        &mut s,
        'b',
        "B1",
        vec![
            json!({"kind": "buy", "date": "2026-10-01", "instrument": fund("AH1"), "quantity": "100", "net_cash": money("38.00", "TRY")}),
            json!({"kind": "buy", "date": "2026-10-01", "instrument": fund("ZZZ"), "quantity": "5", "net_cash": money("5.00", "TRY")}),
        ],
        vec![],
    );
    for a in s.list_assets().unwrap() {
        let code = a.hint.symbol.clone().unwrap();
        if code != "ZZZ" {
            s.set_price_source(a.id, Some(&PriceSourceKind::Tefas { fund_code: code }))
                .unwrap();
        }
    }
    let mut c = HistoryCache::open_in_memory().unwrap();
    for (d, ah1, bgl) in [
        ("2025-10-01", "0.2939", "0.0600"),
        ("2025-10-27", "0.2939", "0.0600"),
        ("2026-09-26", "0.3800", "0.0620"),
        ("2026-10-26", "0.3849", "0.0602"),
        ("2026-10-27", "0.3850", "0.0603"),
    ] {
        price(&mut c, "AH1", d, ah1);
        price(&mut c, "BGL", d, bgl);
    }
    for d in [
        "2024-01-02",
        "2024-06-03",
        "2025-06-02",
        "2025-10-01",
        "2025-10-27",
        "2026-09-26",
        "2026-10-01",
        "2026-10-26",
        "2026-10-27",
        "2026-10-28",
    ] {
        rate(&mut c, d, "49.7872");
    }
    (s, c)
}

struct Loaded {
    book: mizan_core::returns::PriceBook,
    series: Vec<mizan_core::returns::AccountSeries>,
    view: mizan_core::returns::ReturnsView,
}

fn load(s: &Store, c: &HistoryCache) -> Loaded {
    let inputs = Inputs::load(s, Some(c)).unwrap();
    let series = inputs.series(day(TODAY));
    let view = returns_view(
        &series,
        &inputs.book,
        &inputs.base,
        None,
        &inputs.inflation,
        day(TODAY),
    );
    Loaded {
        book: inputs.book,
        series,
        view,
    }
}

fn detail_of(s: &Store, l: &Loaded, hint: &str) -> AccountDetail {
    let id = s.account_for_hint(hint).unwrap().unwrap();
    let market = BookMarket {
        store: s,
        book: &l.book,
    };
    let i = DetailInputs {
        store: s,
        market: &market,
        book: &l.book,
        series: &l.series,
        returns: &l.view,
        today: day(TODAY),
    };
    account_detail(&i, id).unwrap()
}

#[test]
fn list_accounts() {
    let (s, c) = setup();
    let l = load(&s, &c);
    let market = BookMarket {
        store: &s,
        book: &l.book,
    };
    let list = accounts_list(&s, &market, day(TODAY)).unwrap();
    assert_eq!(list.accounts.len(), 2);
    let p1 = &list.accounts[0];
    assert_eq!(
        (
            p1.name.as_str(),
            p1.activities,
            p1.identifiers.clone(),
            p1.tax_treatment.as_str()
        ),
        ("P1", 5, vec!["P1".to_string()], "taxable")
    );
    let check = p1.latest_check.as_ref().unwrap();
    assert_eq!(
        (check.as_of.as_str(), check.passed, check.failed.clone()),
        ("2026-10-27", false, vec!["BGL".to_string()])
    );
    assert!(p1.value.is_some() && p1.merge.is_none());
    assert!(list.merged.is_empty());
}

#[test]
fn summary_tiles_second_currency_and_last_day() {
    let (s, c) = setup();
    let l = load(&s, &c);
    let d = detail_of(&s, &l, "P1");
    let sm = &d.summary;
    // AH1 1000 × 0.3850 + BGL 10000 × 0.0603 = 988.00 TRY; cash 2000 + 8.50 + 17 − 893.90 = 1131.60.
    assert_eq!(sm.positions_value, "988.00");
    assert_eq!(sm.cash_value, "1131.60");
    assert_eq!(sm.value, "2119.60");
    assert_eq!(sm.second_currency, "USD");
    assert_eq!(sm.value_second.as_deref(), Some("42.57"));
    assert!(sm
        .rate_note
        .as_deref()
        .unwrap()
        .starts_with("USD/TRY 49.7872"));
    // Last day: AH1 +0.0001 × 1000 = +0.10, BGL +0.0001 × 10000 = +1.00; previous 384.90 + 602.00.
    let ld = sm.last_day.as_ref().unwrap();
    assert_eq!(
        (ld.amount.as_str(), ld.pct.as_str(), ld.date.as_str()),
        ("+1.10", "+0.1%", "2026-10-27")
    );
    assert!(sm.one_year.value.is_some(), "{:?}", sm.one_year);
    assert!(sm.one_year_second.is_some());
    assert_eq!(d.income.len(), 2);
    assert_eq!(
        (
            d.income[0].year,
            d.income[0].gross.as_str(),
            d.income[0].withheld.as_str()
        ),
        (2025, "20.00", "3.00")
    );
    assert_eq!(
        (d.income[1].year, d.income[1].gross.as_str()),
        (2024, "10.00")
    );
    assert_eq!(d.checks.len(), 2);
    assert_eq!(d.checks[0].as_of, "2026-10-27", "newest first");
    assert_eq!(d.checks[0].failed, ["BGL"]);
    assert!(d.checks[1].passed);
}

#[test]
fn negative_cash_and_missing_price() {
    let (s, c) = setup();
    let l = load(&s, &c);
    let d = detail_of(&s, &l, "B1");
    let sm = &d.summary;
    assert_eq!(sm.positions_value, "38.50");
    assert_eq!(sm.cash_value, "-43.00");
    assert_eq!(sm.value, "-4.50");
    assert_eq!(sm.notes, ["1 position left out: no price"]);
    let zzz = d
        .holdings
        .iter()
        .find(|h| h.symbol.as_deref() == Some("ZZZ"))
        .unwrap();
    assert!(zzz.value.is_none() && zzz.note.is_some());
}

#[test]
fn positions_of_one_account_with_period_returns() {
    let (s, c) = setup();
    let l = load(&s, &c);
    let d = detail_of(&s, &l, "P1");
    let syms: Vec<&str> = d
        .holdings
        .iter()
        .filter_map(|h| h.symbol.as_deref())
        .collect();
    assert_eq!(syms, ["AH1", "BGL"]);
    let w: f64 = d
        .holdings
        .iter()
        .filter_map(|h| h.weight.as_deref())
        .map(|w| w.trim_end_matches('%').parse::<f64>().unwrap())
        .sum();
    assert!((w - 100.0).abs() < 0.2, "{w}");
    let ah1 = &d.holdings[0];
    assert_eq!(ah1.year.as_deref(), Some("+31.0%"), "0.2939 → 0.3850");
    assert_eq!(ah1.month.as_deref(), Some("+1.3%"), "0.3800 → 0.3850");
    assert_eq!(ah1.day.as_deref(), Some("+0.0%"));
    assert_eq!(d.total_value, "988.00");
    assert_eq!(d.total_value_second.as_deref(), Some("19.84"));
    // Same figures as Performance.
    let perf = l
        .view
        .periods
        .iter()
        .find(|p| p.period == mizan_core::portfolio::Period::SinceStart)
        .unwrap();
    let row = perf.accounts.iter().find(|a| a.name == "P1").unwrap();
    let mine = d
        .returns
        .iter()
        .find(|r| r.period == mizan_core::portfolio::Period::SinceStart)
        .unwrap();
    assert_eq!(mine.twr, row.twr);
    assert!(!d.history.is_empty());
    assert_eq!(d.recent.len(), 5);
    assert!(d.delete_blocker.is_some());
    assert_eq!(d.merge_targets.len(), 1);
}

#[test]
fn current_units_chart_in_usd_with_a_short_history() {
    let (s, mut c) = setup();
    // A fund priced only for the last month.
    let mut s = s;
    import(
        &mut s,
        'c',
        "P1",
        vec![
            json!({"kind": "buy", "date": "2026-10-20", "instrument": fund("AH2"), "quantity": "100", "net_cash": money("40.00", "TRY")}),
        ],
        vec![],
    );
    let ah2 = s
        .list_assets()
        .unwrap()
        .into_iter()
        .find(|a| a.hint.symbol.as_deref() == Some("AH2"))
        .unwrap();
    s.set_price_source(
        ah2.id,
        Some(&PriceSourceKind::Tefas {
            fund_code: "AH2".into(),
        }),
    )
    .unwrap();
    price(&mut c, "AH2", "2026-10-20", "0.4391");
    let l = load(&s, &c);
    let id = s.account_for_hint("P1").unwrap().unwrap();
    let market = BookMarket {
        store: &s,
        book: &l.book,
    };
    let chart = current_units_chart(
        &s,
        &market,
        &l.book,
        id,
        ChartPeriod::OneYear,
        "USD",
        day(TODAY),
    )
    .unwrap();
    assert_eq!(chart.currency, "USD");
    assert_eq!(chart.bands.len(), 3);
    assert!(chart.dates.len() <= CHART_POINTS + 1);
    assert_eq!(chart.dates.first().map(String::as_str), Some("2025-10-28"));
    let ah1 = chart.bands.iter().find(|b| b.name == "AH1").unwrap();
    assert!(!ah1.short_history);
    assert_eq!(
        ah1.values.last().unwrap().as_deref(),
        Some("7.73"),
        "1000 × 0.3850 / 49.7872"
    );
    let ah2b = chart.bands.iter().find(|b| b.name == "AH2").unwrap();
    assert!(ah2b.short_history);
    assert!(ah2b.values[0].is_none(), "no band before its first price");
    assert!(current_units_chart(
        &s,
        &market,
        &l.book,
        id,
        ChartPeriod::OneMonth,
        "1X",
        day(TODAY)
    )
    .is_err());
}

#[test]
fn second_currency_setting() {
    let mut s = Store::open_in_memory().unwrap();
    assert_eq!(second_currency(&s).unwrap().as_str(), "USD");
    s.set_setting("base_currency", "USD").unwrap();
    assert_eq!(second_currency(&s).unwrap().as_str(), "EUR");
    set_second_currency(&mut s, "gbp").unwrap();
    assert_eq!(second_currency(&s).unwrap().as_str(), "GBP");
    assert!(set_second_currency(&mut s, "JPY").is_err());
}

#[test]
fn merge_preview_and_re_import_after_a_merge_still_deduplicates() {
    let (mut s, _) = setup();
    let p1 = s.account_for_hint("P1").unwrap().unwrap();
    let b1 = s.account_for_hint("B1").unwrap().unwrap();
    let pv = s.merge_preview(b1).unwrap();
    assert_eq!((pv.activities, pv.assertions, pv.hints), (2, 0, 1));
    s.merge_accounts(b1, p1, "t").unwrap();
    // The same B1 statement again: its records are already stored (now in P1).
    let stamp = |mut r: Value, i: usize| {
        r["source"] = json!({"adapter": "t", "adapter_version": "1", "file_sha256": "b".repeat(64), "locator": format!("row {}", i + 1)});
        r["account"] = json!({"hint": "B1", "name_hint": "B1"});
        r
    };
    let records: Vec<Value> = vec![
        json!({"kind": "buy", "date": "2026-10-01", "instrument": fund("AH1"), "quantity": "100", "net_cash": money("38.00", "TRY")}),
        json!({"kind": "buy", "date": "2026-10-01", "instrument": fund("ZZZ"), "quantity": "5", "net_cash": money("5.00", "TRY")}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, r)| stamp(r, i))
    .collect();
    let parsed =
        parse_batch_value(json!({"schema": "mizan.import/1", "records": records})).unwrap();
    let rb = {
        let mut r = MappingResolver {
            store: &mut s,
            policy: CREATE,
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    assert_eq!(
        rb.records[0].account_id,
        Some(p1),
        "the identifier now resolves to the target"
    );
    let p = prepare(&s, rb).unwrap();
    assert!(p
        .records
        .iter()
        .all(|r| matches!(r.outcome, Outcome::Duplicate { .. })));
}
