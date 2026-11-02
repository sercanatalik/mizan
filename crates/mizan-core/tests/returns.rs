//! Returns engine: daily value series, returns and benchmark (returns spec).

use mizan_core::returns::series::{FlowAmount, PriceKind};
use mizan_core::returns::*;
use mizan_core::*;
use mizan_import::{parse_batch_value, Code, Date};
use mizan_storage::{
    CachedFundPrice, CachedIndexValue, CachedListedPrice, CachedRate, HistoryCache, PriceRow,
    PriceSourceKind, Store,
};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::str::FromStr;

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn code(s: &str) -> Code {
    Code::parse(s).unwrap()
}

/// Commit records; each gets account `hint` unless it has one.
fn import(store: &mut Store, file: char, hint: &str, records: Vec<Value>) {
    let records: Vec<Value> = records
        .into_iter()
        .enumerate()
        .map(|(i, mut r)| {
            r["source"] = json!({"adapter": "t", "adapter_version": "1", "file_sha256": file.to_string().repeat(64), "locator": format!("row {}", i + 1)});
            if r.get("account").is_none() {
                r["account"] = json!({"hint": hint, "name_hint": hint});
            }
            r
        })
        .collect();
    let parsed =
        parse_batch_value(json!({"schema": "mizan.import/1", "records": records})).unwrap();
    assert!(parsed.is_clean(), "{:?}", parsed.errors);
    let rb = {
        let mut r = MappingResolver {
            store,
            policy: CREATE,
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    let p = prepare(store, rb).unwrap();
    commit(store, &p, &Decisions::new()).unwrap();
}

fn money(a: &str, c: &str) -> Value {
    json!({"amount": a, "currency": c})
}

fn fund(sym: &str) -> Value {
    json!({"symbol": sym, "class_hint": "fund", "currency": "TRY"})
}

fn asset_id(store: &Store, symbol: &str) -> i64 {
    store
        .list_assets()
        .unwrap()
        .into_iter()
        .find(|a| a.hint.symbol.as_deref() == Some(symbol))
        .unwrap()
        .id
}

fn key(store: &Store, symbol: &str) -> String {
    store
        .list_assets()
        .unwrap()
        .into_iter()
        .find(|a| a.hint.symbol.as_deref() == Some(symbol))
        .unwrap()
        .instrument_key
}

fn provider_price(store: &mut Store, asset: i64, date: &str, value: &str) {
    store
        .upsert_provider_price(&PriceRow {
            asset_id: asset,
            date: date.into(),
            source: "tefas".into(),
            value: value.into(),
            currency: "TRY".into(),
            fetched_at: "t".into(),
        })
        .unwrap();
}

fn series_of(store: &Store, cache: Option<&HistoryCache>, today: &str) -> Vec<AccountSeries> {
    Inputs::load(store, cache).unwrap().series(day(today))
}

// ---- 5.1 price book ----

#[test]
fn price_book_carries_friday_into_the_weekend() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "buy", "date": "2026-10-20", "instrument": fund("AFT"), "quantity": "1000", "net_cash": money("1100.00", "TRY")}),
        ],
    );
    let a = asset_id(&s, "AFT");
    provider_price(&mut s, a, "2026-10-23", "1.20");
    let book = PriceBook::load(&s, None).unwrap();
    let p = book.price_on(&key(&s, "AFT"), day("2026-10-25")).unwrap();
    assert_eq!((p.date, p.value), (day("2026-10-23"), d("1.20")));
    assert!(book.price_on(&key(&s, "AFT"), day("2026-10-22")).is_none());
}

#[test]
fn manual_price_wins_on_the_same_date() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "buy", "date": "2026-10-20", "instrument": fund("AFT"), "quantity": "1000", "net_cash": money("1100.00", "TRY")}),
        ],
    );
    let a = asset_id(&s, "AFT");
    provider_price(&mut s, a, "2026-10-30", "1.20");
    s.add_manual_price(a, "2026-10-30", "1.25", "TRY", "t")
        .unwrap();
    let book = PriceBook::load(&s, None).unwrap();
    assert_eq!(
        book.price_on(&key(&s, "AFT"), day("2026-10-30"))
            .unwrap()
            .value,
        d("1.25")
    );
}

#[test]
fn cache_only_prices_and_rates_are_used() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "buy", "date": "2022-01-03", "instrument": fund("AFT"), "quantity": "1000", "net_cash": money("166.21", "TRY")}),
        ],
    );
    let a = asset_id(&s, "AFT");
    s.set_price_source(
        a,
        Some(&PriceSourceKind::Tefas {
            fund_code: "aft".into(),
        }),
    )
    .unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    cache
        .upsert_fund_price(
            &CachedFundPrice {
                code: "AFT".into(),
                date: "2022-01-31".into(),
                price: "0.152312".into(),
            },
            "t",
        )
        .unwrap();
    cache
        .upsert_rate(
            &CachedRate {
                currency: "USD".into(),
                date: "2022-01-31".into(),
                try_per_unit: "13.3".into(),
                source: "tcmb".into(),
            },
            "t",
        )
        .unwrap();
    let book = PriceBook::load(&s, Some(&cache)).unwrap();
    assert_eq!(
        book.price_on(&key(&s, "AFT"), day("2022-02-01"))
            .unwrap()
            .value,
        d("0.152312")
    );
    assert_eq!(
        book.try_per_unit(&code("USD"), day("2022-02-02")),
        Some(d("13.3"))
    );
    assert_eq!(
        book.try_per_unit(&code("TRY"), day("2000-01-01")),
        Some(Decimal::ONE)
    );
}

// ---- 5.2 replay ----

#[test]
fn replay_records_balances_costs_trade_prices_and_flows() {
    let mut s = Store::open_in_memory().unwrap();
    let stock = json!({"isin": "US0000000001", "currency": "USD"});
    import(
        &mut s,
        'a',
        "U1",
        vec![
            json!({"kind": "deposit", "date": "2026-01-02", "net_cash": money("10000.00", "USD")}),
            json!({"kind": "buy", "date": "2026-01-05", "instrument": stock, "quantity": "10", "price": money("182.40", "USD"), "net_cash": money("1825.00", "USD")}),
            json!({"kind": "split", "date": "2026-03-02", "instrument": stock, "split": {"new": "2", "old": "1"}}),
            json!({"kind": "position_in", "date": "2026-04-01", "instrument": stock, "quantity": "5"}),
            json!({"kind": "internal_transfer", "date": "2026-05-01", "net_cash": money("-500.00", "USD")}),
        ],
    );
    let inputs = Inputs::load(&s, None).unwrap();
    let r = &inputs.accounts[0];
    let k = "isin:US0000000001";
    assert_eq!(r.first, Some(day("2026-01-02")));
    let qty: Vec<(Date, Decimal)> = r.qty[k].clone();
    assert_eq!(
        qty,
        vec![
            (day("2026-01-05"), d("10")),
            (day("2026-03-02"), d("20")),
            (day("2026-04-01"), d("25"))
        ]
    );
    assert_eq!(
        r.trade_prices[k],
        vec![(day("2026-01-05"), d("182.40"), code("USD"))]
    );
    assert_eq!(
        r.avg_cost[k][0],
        (day("2026-01-05"), Some((d("182.5"), code("USD"))))
    );
    let flows: Vec<(Date, bool, FlowAmount)> = r
        .flows
        .iter()
        .map(|f| (f.date, f.internal, f.amount.clone()))
        .collect();
    assert_eq!(
        flows,
        vec![
            (
                day("2026-01-02"),
                false,
                FlowAmount::Cash {
                    currency: code("USD"),
                    amount: d("10000.00")
                }
            ),
            (
                day("2026-04-01"),
                false,
                FlowAmount::Units {
                    key: k.into(),
                    quantity: d("5")
                }
            ),
            (
                day("2026-05-01"),
                true,
                FlowAmount::Cash {
                    currency: code("USD"),
                    amount: d("-500.00")
                }
            ),
        ]
    );
}

// ---- 5.3 daily grid (spec scenarios) ----

#[test]
fn weekend_uses_fridays_price() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "position_in", "date": "2026-10-19", "instrument": fund("AFT"), "quantity": "1000"}),
        ],
    );
    let a = asset_id(&s, "AFT");
    provider_price(&mut s, a, "2026-10-19", "1.10");
    provider_price(&mut s, a, "2026-10-23", "1.20");
    let series = series_of(&s, None, "2026-10-25");
    let sunday = series[0].day(day("2026-10-25")).unwrap();
    assert_eq!(sunday.value, Some(d("1200.00")));
    assert_eq!(sunday.estimated, 0);
    assert_eq!(series[0].days.len(), 7);
}

#[test]
fn activity_applies_from_its_date() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "deposit", "date": "2026-03-01", "net_cash": money("1000.00", "TRY")}),
            json!({"kind": "buy", "date": "2026-03-05", "instrument": fund("AFT"), "quantity": "100", "net_cash": money("100.00", "TRY")}),
        ],
    );
    let a = asset_id(&s, "AFT");
    provider_price(&mut s, a, "2026-03-01", "1.50");
    let series = series_of(&s, None, "2026-03-06");
    let v = |x: &str| series[0].day(day(x)).unwrap().value.unwrap();
    assert_eq!(v("2026-03-04"), d("1000.00"));
    // 900 cash + 100 units at 1.50
    assert_eq!(v("2026-03-05"), d("1050.00"));
    assert_eq!(
        series[0].day(day("2026-03-01")).unwrap().external_flow,
        d("1000.00")
    );
}

#[test]
fn pension_history_older_than_five_years_uses_trade_prices() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "deposit", "date": "2019-04-02", "net_cash": money("600.00", "TRY")}),
            json!({"kind": "buy", "date": "2019-04-02", "instrument": fund("AH2"), "quantity": "1500", "net_cash": money("600.00", "TRY")}),
        ],
    );
    let a = asset_id(&s, "AH2");
    s.set_price_source(
        a,
        Some(&PriceSourceKind::Tefas {
            fund_code: "AH2".into(),
        }),
    )
    .unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    cache
        .upsert_fund_price(
            &CachedFundPrice {
                code: "AH2".into(),
                date: "2021-11-22".into(),
                price: "0.90".into(),
            },
            "t",
        )
        .unwrap();
    let series = series_of(&s, Some(&cache), "2021-11-23");
    let early = series[0].day(day("2020-06-01")).unwrap();
    assert_eq!(early.value, Some(d("600.00")));
    assert_eq!(early.estimated, 1);
    let late = series[0].day(day("2021-11-22")).unwrap();
    assert_eq!(late.value, Some(d("1350.00")));
    assert_eq!(late.estimated, 0);
}

#[test]
fn stock_without_a_provider_is_valued_at_its_trade_price() {
    let mut s = Store::open_in_memory().unwrap();
    s.set_setting("base_currency", "USD").unwrap();
    import(
        &mut s,
        'a',
        "U1",
        vec![
            json!({"kind": "position_in", "date": "2026-01-05", "instrument": {"isin": "US0000000001", "currency": "USD"}, "quantity": "10", "price": money("182.40", "USD")}),
            json!({"kind": "buy", "date": "2026-01-06", "instrument": {"isin": "US0000000001", "currency": "USD"}, "quantity": "1", "price": money("182.40", "USD"), "net_cash": money("182.40", "USD")}),
        ],
    );
    let series = series_of(&s, None, "2026-01-08");
    let v = series[0].day(day("2026-01-07")).unwrap();
    // 11 shares at 182.40, minus the cash paid for the buy.
    assert_eq!(v.value, Some(d("1824.00")));
    assert_eq!(v.estimated, 1);
    // The position_in on day 1 has no trade or cost yet: excluded and counted.
    assert_eq!(series[0].days[0].excluded, 1);
}

#[test]
fn listed_stock_uses_cached_closes_instead_of_cost() {
    let mut s = Store::open_in_memory().unwrap();
    s.set_setting("base_currency", "USD").unwrap();
    import(
        &mut s,
        'a',
        "U1",
        vec![
            json!({"kind": "buy", "date": "2026-01-05", "instrument": {"symbol": "AAPL", "isin": "US0000000001", "currency": "USD"}, "quantity": "10", "price": money("182.40", "USD"), "net_cash": money("1824.00", "USD")}),
        ],
    );
    let a = asset_id(&s, "AAPL");
    s.set_price_source(
        a,
        Some(&PriceSourceKind::Yahoo {
            symbol: "aapl".into(),
        }),
    )
    .unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let close = |date: &str, close: &str| CachedListedPrice {
        source: "yahoo".into(),
        symbol: "AAPL".into(),
        date: date.into(),
        close: close.into(),
        currency: "USD".into(),
    };
    cache
        .commit_listed(
            "yahoo",
            "AAPL",
            &[("2026".into(), mizan_storage::FetchStatus::Ok)],
            &[close("2026-01-05", "185.00"), close("2026-01-06", "190.50")],
            "t",
        )
        .unwrap();
    // Same symbol from another provider is not used.
    cache
        .upsert_listed_price(
            &CachedListedPrice {
                source: "twelvedata".into(),
                ..close("2026-01-07", "999.00")
            },
            "t",
        )
        .unwrap();
    let series = series_of(&s, Some(&cache), "2026-01-07");
    let acc = &series[0];
    assert_eq!(acc.day(day("2026-01-05")).unwrap().value, Some(d("26.00")));
    assert_eq!(acc.day(day("2026-01-06")).unwrap().value, Some(d("81.00")));
    let last = acc.day(day("2026-01-07")).unwrap();
    // 10 × 190.50 carried into 2026-01-07, minus the 1824.00 paid.
    assert_eq!(last.value, Some(d("81.00")));
    assert!(
        acc.days.iter().all(|v| v.estimated == 0),
        "no cost estimates"
    );
}

#[test]
fn days_without_rates_have_no_value() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "U1",
        vec![json!({"kind": "deposit", "date": "2023-03-01", "net_cash": money("1000.00", "USD")})],
    );
    let mut cache = HistoryCache::open_in_memory().unwrap();
    cache
        .upsert_rate(
            &CachedRate {
                currency: "USD".into(),
                date: "2025-01-02".into(),
                try_per_unit: "35.2".into(),
                source: "tcmb".into(),
            },
            "t",
        )
        .unwrap();
    let series = series_of(&s, Some(&cache), "2025-01-03");
    let before = series[0].day(day("2024-12-31")).unwrap();
    assert_eq!(before.value, None);
    assert_eq!(before.missing_rate, Some(code("USD")));
    assert_eq!(
        series[0].day(day("2025-01-02")).unwrap().value,
        Some(d("35200.0"))
    );
}

#[test]
fn unit_value_prefers_market_then_trade_then_cost() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "buy", "date": "2026-01-05", "instrument": fund("AFT"), "quantity": "100", "net_cash": money("120.00", "TRY")}),
        ],
    );
    let inputs = Inputs::load(&s, None).unwrap();
    let k = key(&s, "AFT");
    let (p, _, kind) =
        series::unit_value(&inputs.accounts[0], &inputs.book, &k, day("2026-01-06")).unwrap();
    assert_eq!((p, kind), (d("1.2"), PriceKind::TradePrice));
}

// ---- 6.1 / 6.2 return formulas ----

use mizan_core::portfolio::Period;
use mizan_core::returns::calc::{self, Point, Rate};

fn pts(rows: &[(&str, &str, &str)]) -> Vec<Point> {
    rows.iter()
        .map(|(dt, v, f)| Point {
            date: day(dt),
            value: d(v),
            flow: d(f),
        })
        .collect()
}

fn shown(r: Rate) -> String {
    match r {
        Rate::Value { cumulative, .. } => calc::pct(cumulative),
        Rate::NotAvailable(why) => format!("n/a: {why}"),
    }
}

#[test]
fn deposit_does_not_count_as_gain() {
    // Worth 10000; +1% on day 1; 10000 deposited at the start of day 2, then +1%.
    let points = pts(&[
        ("2026-03-02", "10100", "0"),
        ("2026-03-03", "20301", "10000"),
    ]);
    assert_eq!(shown(calc::twr(d("10000"), &points)), "+2.01%");
}

#[test]
fn annualized_over_three_years() {
    let mut points = Vec::new();
    let mut dt = day("2023-01-01");
    for i in 0..1096 {
        let v = if i == 1095 { d("20000") } else { d("10000") };
        points.push(Point {
            date: dt,
            value: v,
            flow: if i == 0 { d("10000") } else { Decimal::ZERO },
        });
        dt = Date(dt.0.succ_opt().unwrap());
    }
    let Rate::Value {
        cumulative,
        annualized,
    } = calc::twr(Decimal::ZERO, &points)
    else {
        panic!()
    };
    assert_eq!(calc::pct(cumulative), "+100.00%");
    let expected = 2f64.powf(365.0 / 1096.0) - 1.0;
    assert_eq!(
        calc::pct(annualized.unwrap()),
        calc::pct(Decimal::try_from(expected).unwrap())
    );
}

#[test]
fn money_weighted_single_deposit() {
    let mut points = Vec::new();
    let mut dt = day("2025-11-03");
    for i in 0..365 {
        points.push(Point {
            date: dt,
            value: if i == 364 { d("11000") } else { d("10000") },
            flow: if i == 0 { d("10000") } else { Decimal::ZERO },
        });
        dt = Date(dt.0.succ_opt().unwrap());
    }
    assert_eq!(shown(calc::mwr(Decimal::ZERO, &points)), "+10.00%");
}

#[test]
fn xirr_matches_a_known_value() {
    // A textbook example: -10000, 2750, 4250, 3250, 2750 over 2008-2009 -> 37.34%.
    let flows = [
        (0.0, -10000.0),
        (60.0, 2750.0),
        (303.0, 4250.0),
        (411.0, 3250.0),
        (456.0, 2750.0),
    ];
    let r = calc::xirr(&flows).unwrap();
    assert!((r - 0.373362535).abs() < 1e-6, "{r}");
}

#[test]
fn not_available_reasons() {
    let flat = pts(&[("2026-03-02", "0", "0"), ("2026-03-03", "0", "0")]);
    assert_eq!(
        shown(calc::twr(Decimal::ZERO, &flat)),
        "n/a: nothing invested in this period"
    );
    assert_eq!(
        shown(calc::mwr(Decimal::ZERO, &flat)),
        "n/a: nothing invested in this period"
    );
    assert_eq!(calc::xirr(&[(0.0, 100.0), (10.0, 50.0)]), None);
}

// ---- 6.1 periods / 6.3 aggregation through the engine ----

fn view(
    store: &Store,
    cache: Option<&HistoryCache>,
    bench: Option<&Benchmark>,
    today: &str,
) -> ReturnsView {
    let inputs = Inputs::load(store, cache).unwrap();
    let series = inputs.series(day(today));
    returns_view(
        &series,
        &inputs.book,
        &inputs.base,
        bench,
        &inputs.inflation,
        day(today),
    )
}

fn period(v: &ReturnsView, p: Period) -> &PeriodReturns {
    v.periods.iter().find(|x| x.period == p).unwrap()
}

#[test]
fn young_account_period_is_clamped_to_its_first_activity() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![json!({"kind": "deposit", "date": "2026-08-14", "net_cash": money("1000.00", "TRY")})],
    );
    let v = view(&s, None, None, "2026-11-02");
    let y = period(&v, Period::Last12Months);
    assert_eq!(y.accounts[0].from, "2026-08-14");
    assert!(y.accounts[0].clamped);
    assert_eq!(y.from.as_deref(), Some("2026-08-14"));
    let m = period(&v, Period::OneMonth);
    assert_eq!(m.accounts[0].from, "2026-10-03");
    assert!(!m.accounts[0].clamped);
    assert_eq!(m.twr.value.as_deref(), Some("+0.00%"));
}

#[test]
fn transfer_between_own_accounts_is_no_portfolio_flow() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "A",
        vec![
            json!({"kind": "deposit", "date": "2026-01-02", "net_cash": money("10000.00", "TRY")}),
        ],
    );
    import(
        &mut s,
        'b',
        "A",
        vec![
            json!({"kind": "internal_transfer", "date": "2026-02-02", "net_cash": money("-5000.00", "TRY")}),
        ],
    );
    import(
        &mut s,
        'c',
        "B",
        vec![
            json!({"kind": "internal_transfer", "date": "2026-02-02", "net_cash": money("5000.00", "TRY")}),
        ],
    );
    let inputs = Inputs::load(&s, None).unwrap();
    let series = inputs.series(day("2026-03-01"));
    let a = series.iter().find(|x| x.name == "A").unwrap();
    let b = series.iter().find(|x| x.name == "B").unwrap();
    assert_eq!(
        a.day(day("2026-02-02")).unwrap().internal_flow,
        d("-5000.00")
    );
    assert_eq!(
        b.day(day("2026-02-02")).unwrap().internal_flow,
        d("5000.00")
    );
    let v = returns_view(
        &series,
        &inputs.book,
        &inputs.base,
        None,
        &inputs.inflation,
        day("2026-03-01"),
    );
    let ytd = period(&v, Period::Ytd);
    assert_eq!(ytd.twr.value.as_deref(), Some("+0.00%"));
    assert_eq!(ytd.mwr.value.as_deref(), Some("+0.00%"));
    for acc in &ytd.accounts {
        assert_eq!(acc.twr.value.as_deref(), Some("+0.00%"), "{}", acc.name);
    }
    let h = value_history(&series, &inputs.base, day("2026-03-01"));
    let last = h.points.last().unwrap();
    assert_eq!(
        (last.value.as_deref(), last.contributions.as_str()),
        (Some("10000.00"), "10000.00")
    );
}

#[test]
fn estimated_portfolio_return_is_marked() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "E",
        vec![
            json!({"kind": "deposit", "date": "2025-12-01", "net_cash": money("1000.00", "TRY")}),
            json!({"kind": "buy", "date": "2026-01-01", "instrument": fund("XYZ"), "quantity": "100", "net_cash": money("100.00", "TRY")}),
        ],
    );
    import(
        &mut s,
        'b',
        "C1",
        vec![json!({"kind": "deposit", "date": "2025-12-01", "net_cash": money("500.00", "TRY")})],
    );
    import(
        &mut s,
        'c',
        "C2",
        vec![json!({"kind": "deposit", "date": "2025-12-01", "net_cash": money("500.00", "TRY")})],
    );
    let v = view(&s, None, None, "2026-02-09");
    let ytd = period(&v, Period::Ytd);
    let e = ytd.accounts.iter().find(|a| a.name == "E").unwrap();
    assert_eq!(e.twr.estimated_days, 40);
    assert_eq!(ytd.twr.estimated_days, 40);
    assert_eq!(ytd.mwr.estimated_days, 40);
    let c1 = ytd.accounts.iter().find(|a| a.name == "C1").unwrap();
    assert_eq!(c1.twr.estimated_days, 0);
}

fn fund_benchmark(s: &mut Store, prices: &[(&str, &str)]) -> Benchmark {
    import(
        s,
        'f',
        "P1",
        vec![json!({"kind": "deposit", "date": "2025-06-02", "net_cash": money("1000.00", "TRY")})],
    );
    let a = s
        .create_asset(
            "sym:AFT@",
            &mizan_import::InstrumentHint {
                symbol: Some("AFT".into()),
                ..Default::default()
            },
        )
        .unwrap();
    for (dt, p) in prices {
        provider_price(s, a, dt, p);
    }
    Benchmark {
        key: "sym:AFT@".into(),
        asset_id: a,
        name: "AFT".into(),
    }
}

#[test]
fn fund_as_benchmark() {
    let mut s = Store::open_in_memory().unwrap();
    let b = fund_benchmark(&mut s, &[("2025-12-31", "1.00"), ("2026-11-02", "1.18")]);
    let v = view(&s, None, Some(&b), "2026-11-02");
    let ytd = period(&v, Period::Ytd);
    assert_eq!(
        ytd.benchmark.as_ref().unwrap().figure.value.as_deref(),
        Some("+18.00%")
    );
    let g = &ytd.growth;
    assert_eq!(g.first().unwrap().benchmark.as_deref(), Some("100.00"));
    assert_eq!(g.last().unwrap().benchmark.as_deref(), Some("118.00"));
    assert_eq!(g.last().unwrap().portfolio.as_deref(), Some("100.00"));
}

#[test]
fn benchmark_too_young() {
    let mut s = Store::open_in_memory().unwrap();
    let b = fund_benchmark(&mut s, &[("2026-03-02", "1.00")]);
    let v = view(&s, None, Some(&b), "2026-11-02");
    let ytd = period(&v, Period::Ytd);
    assert_eq!(
        ytd.benchmark.as_ref().unwrap().figure.reason.as_deref(),
        Some("no price before 2026-01-01")
    );
}

#[test]
fn missing_rates_exclude_an_account_from_older_periods_only() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "U1",
        vec![json!({"kind": "deposit", "date": "2023-03-01", "net_cash": money("1000.00", "USD")})],
    );
    import(
        &mut s,
        'b',
        "P1",
        vec![json!({"kind": "deposit", "date": "2023-03-01", "net_cash": money("1000.00", "TRY")})],
    );
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let mut dt = day("2025-01-02");
    while dt <= day("2025-03-01") {
        cache
            .upsert_rate(
                &CachedRate {
                    currency: "USD".into(),
                    date: dt.to_string(),
                    try_per_unit: "35".into(),
                    source: "tcmb".into(),
                },
                "t",
            )
            .unwrap();
        dt = Date(dt.0.succ_opt().unwrap());
    }
    let v = view(&s, Some(&cache), None, "2025-03-01");
    let all = period(&v, Period::SinceStart);
    let u1 = all.accounts.iter().find(|a| a.name == "U1").unwrap();
    assert_eq!(
        u1.twr.reason.as_deref(),
        Some("exchange rates before 2025-01-02 are missing")
    );
    assert_eq!(all.excluded.len(), 1);
    assert!(
        all.twr.value.is_some(),
        "portfolio still shown for the other account"
    );
    let m = period(&v, Period::OneMonth);
    let u1 = m.accounts.iter().find(|a| a.name == "U1").unwrap();
    assert_eq!(u1.twr.value.as_deref(), Some("+0.00%"));
    assert!(m.excluded.is_empty());
}

// ---- 6.5 the worked example in docs/returns.md ----

#[test]
fn docs_worked_example_gives_the_documented_figures() {
    let doc = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/returns.md"
    ))
    .unwrap();
    let example = &doc[doc.find("<!-- returns-example -->").unwrap()..];
    assert!(example.contains("**+2.01%**") && example.contains("**+2.26%**"));

    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'd',
        "P1",
        vec![
            json!({"kind": "deposit", "date": "2026-03-02", "net_cash": money("10000.00", "TRY")}),
            json!({"kind": "buy", "date": "2026-03-02", "instrument": fund("AFT"), "quantity": "1000", "net_cash": money("10000.00", "TRY")}),
            json!({"kind": "deposit", "date": "2026-03-04", "net_cash": money("10100.00", "TRY")}),
            json!({"kind": "buy", "date": "2026-03-04", "instrument": fund("AFT"), "quantity": "1000", "net_cash": money("10100.00", "TRY")}),
        ],
    );
    let a = asset_id(&s, "AFT");
    provider_price(&mut s, a, "2026-03-02", "10.00");
    provider_price(&mut s, a, "2026-03-03", "10.10");
    provider_price(&mut s, a, "2026-03-04", "10.201");
    let v = view(&s, None, None, "2026-03-04");
    let all = period(&v, Period::SinceStart);
    assert_eq!(all.twr.value.as_deref(), Some("+2.01%"));
    assert_eq!(all.mwr.value.as_deref(), Some("+2.26%"));
    assert_eq!(all.twr.estimated_days, 0);
}

// ---- real (inflation-adjusted) returns ----

/// A TRY index cache with the given monthly values.
fn index_cache(values: &[(&str, &str)]) -> HistoryCache {
    let mut c = HistoryCache::open_in_memory().unwrap();
    let rows: Vec<CachedIndexValue> = values
        .iter()
        .map(|(m, v)| CachedIndexValue {
            month: m.to_string(),
            value: v.to_string(),
        })
        .collect();
    c.commit_inflation("eurostat", "TR:I15", &rows, true, "t")
        .unwrap();
    c
}

/// 10000.00 TRY deposited and invested in a fund at 1.00 on 2025-01-15, worth
/// `end_price` on 2026-10-15. The index is flat into the first day (100), then
/// reaches `end_index` on 2026-10-15.
fn real_case(end_price: &str, end_index: &str) -> ReturnsView {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "R",
        vec![
            json!({"kind": "deposit", "date": "2025-01-15", "net_cash": money("10000.00", "TRY")}),
            json!({"kind": "buy", "date": "2025-01-15", "instrument": fund("AFT"), "quantity": "10000", "net_cash": money("10000.00", "TRY")}),
        ],
    );
    let a = asset_id(&s, "AFT");
    provider_price(&mut s, a, "2025-01-15", "1.00");
    provider_price(&mut s, a, "2026-10-15", end_price);
    let cache = index_cache(&[
        ("2024-12", "100"),
        ("2025-01", "100"),
        ("2026-10", end_index),
    ]);
    view(&s, Some(&cache), None, "2026-10-15")
}

#[test]
fn real_return_after_inflation_gain() {
    let v = real_case("1.40", "130");
    let all = period(&v, Period::SinceStart);
    assert_eq!(all.twr.value.as_deref(), Some("+40.00%"));
    assert_eq!(all.real_twr.value.as_deref(), Some("+7.69%"));
    assert!(all.real_twr.annualized.is_some(), "longer than a year");
    assert_eq!(all.accounts[0].real_twr.value.as_deref(), Some("+7.69%"));
    assert!(!all.real_provisional);
    assert_eq!(
        v.inflation.label.as_deref(),
        Some("HICP Türkiye (Eurostat)")
    );
    assert_eq!(v.inflation.through.as_deref(), Some("Oct 2026"));
    assert_eq!(v.inflation.unavailable, None);
    let last = all.growth.last().unwrap();
    assert_eq!(last.portfolio.as_deref(), Some("140.00"));
    assert_eq!(last.portfolio_real.as_deref(), Some("107.69"));
}

#[test]
fn real_loss_despite_nominal_gain() {
    let v = real_case("1.40", "150");
    let all = period(&v, Period::SinceStart);
    assert_eq!(all.real_twr.value.as_deref(), Some("−6.67%"));
}

#[test]
fn deposit_that_kept_pace_with_inflation_has_no_real_return() {
    let v = real_case("1.30", "130");
    let all = period(&v, Period::SinceStart);
    assert_eq!(all.real_mwr.value.as_deref(), Some("+0.00%"));
    assert_eq!(all.real_twr.value.as_deref(), Some("+0.00%"));
    assert_eq!(all.accounts[0].real_mwr.value.as_deref(), Some("+0.00%"));
}

#[test]
fn index_starting_after_the_period_start_reads_not_available() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "R",
        vec![json!({"kind": "deposit", "date": "2019-04-02", "net_cash": money("1000.00", "TRY")})],
    );
    let cache = index_cache(&[("2021-05", "100"), ("2026-10", "300")]);
    let v = view(&s, Some(&cache), None, "2026-11-02");
    let all = period(&v, Period::SinceStart);
    assert_eq!(
        all.real_twr.reason.as_deref(),
        Some("no inflation index before May 2021")
    );
    assert!(all.twr.value.is_some(), "nominal stays shown");
    let y = period(&v, Period::Last12Months);
    assert!(y.real_twr.value.is_some());
    assert!(
        y.real_provisional,
        "after the 15th of the last published month"
    );
}

#[test]
fn real_figures_keep_the_estimate_mark() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "E",
        vec![
            json!({"kind": "deposit", "date": "2025-12-01", "net_cash": money("1000.00", "TRY")}),
            json!({"kind": "buy", "date": "2026-01-01", "instrument": fund("XYZ"), "quantity": "100", "net_cash": money("100.00", "TRY")}),
        ],
    );
    let cache = index_cache(&[("2025-10", "100"), ("2026-01", "110")]);
    let v = view(&s, Some(&cache), None, "2026-02-09");
    let ytd = period(&v, Period::Ytd);
    assert_eq!(ytd.twr.estimated_days, 40);
    assert_eq!(ytd.real_twr.estimated_days, 40);
    assert_eq!(ytd.real_mwr.estimated_days, 40);
}

#[test]
fn unsupported_or_missing_index_makes_real_unavailable() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "R",
        vec![json!({"kind": "deposit", "date": "2026-01-02", "net_cash": money("1000.00", "TRY")})],
    );
    s.set_setting("base_currency", "CHF").unwrap();
    let cache = index_cache(&[("2025-10", "100")]);
    let v = view(&s, Some(&cache), None, "2026-02-09");
    assert_eq!(
        v.inflation.unavailable.as_deref(),
        Some("no inflation index for CHF")
    );

    // Switching to a base without a cached index needs no network: the engine
    // only reads the cache it is given.
    s.set_setting("base_currency", "GBP").unwrap();
    let v = view(&s, Some(&cache), None, "2026-02-09");
    assert_eq!(
        v.inflation.unavailable.as_deref(),
        Some("the GBP inflation index has not been downloaded yet")
    );
}

#[test]
fn real_benchmark_matching_inflation_is_zero() {
    let mut s = Store::open_in_memory().unwrap();
    let b = fund_benchmark(&mut s, &[("2025-12-31", "1.00"), ("2026-10-15", "1.18")]);
    // Index 100 through the year start, 118 on 2026-10-15.
    let cache = index_cache(&[
        ("2025-11", "100"),
        ("2025-12", "100"),
        ("2026-01", "100"),
        ("2026-10", "118"),
    ]);
    let v = view(&s, Some(&cache), Some(&b), "2026-10-15");
    let ytd = period(&v, Period::Ytd);
    let bench = ytd.benchmark.as_ref().unwrap();
    assert_eq!(bench.figure.value.as_deref(), Some("+18.00%"));
    assert_eq!(bench.real.value.as_deref(), Some("+0.00%"));
    assert_eq!(
        ytd.growth.last().unwrap().benchmark_real.as_deref(),
        Some("100.00")
    );
}
