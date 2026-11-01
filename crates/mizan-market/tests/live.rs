//! Live checks against TEFAS and TCMB. Ignored by default; CI never runs them.
//! Run manually: `cargo test -p mizan-market --test live -- --ignored`

use std::collections::BTreeSet;

use mizan_market::{tcmb, tefas, ReqwestHttp};

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
