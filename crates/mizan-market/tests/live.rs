//! Live checks against the built-in providers. Ignored by default; CI never runs them.
//! Run manually: `cargo test -p mizan-market --test live -- --ignored`

use std::collections::BTreeSet;

use mizan_market::{tcmb, tefas, yahoo, ReqwestHttp};

fn today() -> mizan_import::Date {
    mizan_import::Date(chrono::Local::now().date_naive())
}

#[test]
#[ignore = "network"]
fn tcmb_today_has_rates() {
    let http = ReqwestHttp::new().unwrap();
    match tcmb::fetch(&http).unwrap() {
        Some(rates) => assert!(
            rates.iter().any(|r| r.currency.as_str() == "USD"),
            "{rates:?}"
        ),
        None => eprintln!("TCMB published no file today (weekend or holiday)"),
    }
}

#[test]
#[ignore = "network"]
fn tefas_returns_a_recent_price_for_a_large_fund() {
    let http = ReqwestHttp::new().unwrap();
    let f = tefas::fetch_latest(&http, &BTreeSet::from(["AFT".to_string()]), today());
    assert_eq!(f.error, None);
    let p = f.latest.get("AFT").expect("AFT listed");
    assert!(p.price > rust_decimal::Decimal::ZERO);
    eprintln!("AFT {} on {}", p.price, p.date);
}

#[test]
#[ignore = "network"]
fn yahoo_returns_recent_closes_and_history_across_a_split() {
    let http = ReqwestHttp::new().unwrap();
    for symbol in ["AAPL", "VWCE.DE"] {
        let closes = yahoo::fetch_latest(&http, symbol).unwrap();
        let last = closes.last().expect("recent closes");
        assert!(last.value > rust_decimal::Decimal::ZERO);
        eprintln!("{symbol} {} {} on {}", last.value, last.currency, last.date);
    }
    // The span runs through today, so the 2020 split is undone (raw close ≈ 499).
    let d = mizan_import::Date::parse("2020-08-28").unwrap();
    let span = yahoo::fetch_span(&http, "AAPL", d, today()).unwrap();
    let c = span.iter().find(|c| c.date == d).expect("2020-08-28 close");
    assert!(c.value > rust_decimal::Decimal::from(450), "{}", c.value);
}

#[test]
#[ignore = "network"]
fn inflation_indices_are_published_for_every_supported_currency() {
    use mizan_market::inflation::{self, YearMonth, SERIES};
    let http = ReqwestHttp::new().unwrap();
    let from = YearMonth::of(today().0).shift(-14);
    for s in SERIES {
        let v = inflation::fetch(&s, &http, from, today())
            .unwrap_or_else(|e| panic!("{}: {e}", s.full_label()));
        let last = v
            .last()
            .unwrap_or_else(|| panic!("{}: no values", s.full_label()));
        assert!(v.len() >= 10, "{}: only {} months", s.full_label(), v.len());
        assert!(
            last.month >= YearMonth::of(today().0).shift(-3),
            "{}: latest is {}",
            s.full_label(),
            last.month
        );
        eprintln!("{} {} = {}", s.full_label(), last.month, last.value);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
