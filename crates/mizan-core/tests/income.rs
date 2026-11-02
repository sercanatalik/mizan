use std::str::FromStr;

use mizan_core::income::income_view;
use mizan_core::portfolio::{AccountScope, Period};
use mizan_core::returns::PriceBook;
use mizan_core::tax::report::{report, TaxInputs};
use mizan_core::tax::{Country, TaxYear};
use mizan_core::*;
use mizan_import::{parse_batch_value, Date};
use mizan_storage::{FxRow, Store};
use rust_decimal::Decimal;
use serde_json::{json, Value};

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

fn import(store: &mut Store, file: char, hint: &str, records: Vec<Value>) {
    let records: Vec<Value> = records
        .into_iter()
        .enumerate()
        .map(|(i, mut r)| {
            r["source"] = json!({"adapter": "t", "adapter_version": "1", "file_sha256": file.to_string().repeat(64), "locator": format!("row {}", i + 1)});
            r["account"] = json!({"hint": hint, "name_hint": hint});
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

fn rate(s: &mut Store, ccy: &str, date: &str, r: &str) {
    s.upsert_fx_rate(
        &FxRow {
            currency: ccy.into(),
            date: date.into(),
            source: "tcmb".into(),
            try_per_unit: r.into(),
        },
        "t",
    )
    .unwrap();
}

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

const AAPL: &str = r#"{"isin": "US0378331005", "symbol": "AAPL", "currency": "USD"}"#;

/// A USD broker with a dividend (withholding) in 2025 and 2026, and interest
/// without a rate; base currency TRY.
fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    let aapl: Value = serde_json::from_str(AAPL).unwrap();
    import(
        &mut s,
        'a',
        "IBKR",
        vec![
            json!({"kind": "dividend", "date": "2025-03-03", "instrument": aapl, "net_cash": money("85.00", "USD"), "charges": [{"type": "withholding_tax", "amount": money("15.00", "USD")}]}),
            json!({"kind": "dividend", "date": "2026-03-03", "instrument": aapl, "net_cash": money("42.50", "USD"), "charges": [{"type": "withholding_tax", "amount": money("7.50", "USD")}]}),
            json!({"kind": "interest", "date": "2026-05-04", "net_cash": money("3.00", "EUR")}),
        ],
    );
    rate(&mut s, "USD", "2025-03-03", "34.00");
    rate(&mut s, "USD", "2026-03-03", "40.00");
    s
}

fn view(s: &Store, period: Period, today: &str) -> mizan_core::income::IncomeView {
    let book = PriceBook::load(s, None).unwrap();
    income_view(s, &book, AccountScope::All, period, day(today)).unwrap()
}

#[test]
fn dividend_with_withholding_converted_at_payment_date() {
    let s = store();
    let v = view(&s, Period::Ytd, "2026-11-02");
    let p = v.payments.iter().find(|p| p.kind == "dividend").unwrap();
    assert_eq!(p.label, "AAPL");
    assert_eq!(
        (p.gross.as_deref(), p.withheld.as_str(), p.net.as_deref()),
        (Some("50.00"), "7.50", Some("42.50"))
    );
    assert_eq!(
        (
            p.base_gross.as_deref(),
            p.base_withheld.as_deref(),
            p.base_net.as_deref()
        ),
        (Some("2000.00"), Some("300.00"), Some("1700.00"))
    );
}

#[test]
fn missing_rate_is_listed_and_left_out_of_totals() {
    let s = store();
    let v = view(&s, Period::Ytd, "2026-11-02");
    let i = v.payments.iter().find(|p| p.kind == "interest").unwrap();
    assert_eq!(i.label, "Interest · EUR");
    assert_eq!(i.note.as_deref(), Some("not converted: no rate"));
    assert_eq!(i.base_net, None);
    assert_eq!(v.not_converted, 1);
    assert_eq!(
        (
            v.totals.gross.as_str(),
            v.totals.withheld.as_str(),
            v.totals.net.as_str()
        ),
        ("2000.00", "300.00", "1700.00")
    );
}

#[test]
fn ytd_compares_with_the_same_dates_last_year() {
    let s = store();
    let v = view(&s, Period::Ytd, "2026-11-02");
    assert_eq!(
        (v.previous_from.as_deref(), v.previous_to.as_deref()),
        (Some("2025-01-01"), Some("2025-11-02"))
    );
    let prev = v.previous.unwrap();
    assert_eq!(
        (
            prev.gross.as_str(),
            prev.withheld.as_str(),
            prev.net.as_str()
        ),
        ("3400.00", "510.00", "2890.00")
    );
    let since = view(&s, Period::SinceStart, "2026-11-02");
    assert!(since.previous.is_none());
    assert_eq!(since.from.as_deref(), Some("2025-03-03"));
    let m3 = view(&s, Period::ThreeMonths, "2026-11-02");
    assert_eq!(
        (m3.previous_from.as_deref(), m3.previous_to.as_deref()),
        (Some("2026-05-03"), Some("2026-08-02"))
    );
}

#[test]
fn every_month_is_listed_and_instruments_sorted_by_net() {
    let s = store();
    let v = view(&s, Period::Ytd, "2026-11-02");
    let months: Vec<&str> = v.months.iter().map(|m| m.month.as_str()).collect();
    assert_eq!(months.len(), 11);
    assert_eq!(months[0], "2026-01");
    let march = v.months.iter().find(|m| m.month == "2026-03").unwrap();
    assert_eq!(
        (march.dividends.as_str(), march.interest.as_str()),
        ("1700.00", "0.00")
    );
    let jan = &v.months[0];
    assert_eq!(
        (jan.dividends.as_str(), jan.interest.as_str()),
        ("0.00", "0.00"),
        "a quiet month is listed empty"
    );
    assert_eq!(
        v.instruments.len(),
        1,
        "the unconverted interest is not an instrument row"
    );
    assert_eq!(
        (
            v.instruments[0].label.as_str(),
            v.instruments[0].payments,
            v.instruments[0].net.as_str()
        ),
        ("AAPL", 1, "1700.00")
    );
}

#[test]
fn scope_limits_income_to_one_account() {
    let mut s = store();
    import(
        &mut s,
        'b',
        "BANK",
        vec![json!({"kind": "interest", "date": "2026-06-01", "net_cash": money("12.00", "TRY")})],
    );
    let bank = s
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.name == "BANK")
        .unwrap()
        .id;
    let book = PriceBook::load(&s, None).unwrap();
    let v = income_view(
        &s,
        &book,
        AccountScope::One(bank),
        Period::Ytd,
        day("2026-11-02"),
    )
    .unwrap();
    assert_eq!(v.payments.len(), 1);
    assert_eq!(v.totals.net, "12.00");
    assert_eq!(v.instruments[0].label, "Interest · TRY");
}

#[test]
fn income_agrees_with_the_turkish_tax_report() {
    let s = store();
    let since = view(&s, Period::SinceStart, "2026-11-02");
    let inputs = TaxInputs::load(&s, None).unwrap();
    let mut compared = 0;
    for year in ["2025", "2026"] {
        let r = report(
            &inputs,
            Country::Tr,
            TaxYear::parse(Country::Tr, year).unwrap(),
            "t",
        );
        for section in &r.income {
            for row in section.rows.iter().filter(|r| r.gross.is_some()) {
                let p = since
                    .payments
                    .iter()
                    .find(|p| p.date == row.date && p.base_gross.is_some())
                    .unwrap_or_else(|| panic!("payment on {}", row.date));
                assert_eq!(
                    d(p.base_gross.as_deref().unwrap()),
                    d(row.gross.as_deref().unwrap()),
                    "gross on {}",
                    row.date
                );
                assert_eq!(
                    d(p.base_withheld.as_deref().unwrap()),
                    d(row.withheld.as_deref().unwrap_or("0")),
                    "withheld on {}",
                    row.date
                );
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 2, "both dividends were compared");
}

#[test]
fn csv_has_one_row_per_payment() {
    use mizan_core::income::{income_csv, income_file_name};
    let s = store();
    let v = view(&s, Period::Ytd, "2026-11-02");
    let csv = income_csv(&v);
    let lines: Vec<&str> = csv.split("\r\n").filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "date,account,instrument,type,currency,gross,withheld,net,gross_TRY,withheld_TRY,net_TRY,note");
    assert_eq!(
        lines[1],
        "2026-03-03,IBKR,AAPL,dividend,USD,50.00,7.50,42.50,2000.00,300.00,1700.00,"
    );
    assert!(lines[2].ends_with(",not converted: no rate"));
    assert_eq!(
        income_file_name(&v, "All accounts"),
        "mizan-income-all-accounts-2026-01-01-2026-11-02.csv"
    );
}
