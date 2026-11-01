use mizan_core::*;
use mizan_import::{parse_batch_value, Date, ImportRecord, Kind};
use mizan_storage::{NewActivity, NewBatch, Store};
use serde_json::{json, Value};

fn src(l: &str) -> Value {
    json!({"adapter": "t", "adapter_version": "1", "file_sha256": "0".repeat(64), "locator": l})
}

fn rec(v: Value) -> ImportRecord {
    let p = parse_batch_value(json!({"schema": "mizan.import/1", "records": [v]})).unwrap();
    assert!(p.is_clean(), "{:?}", p.errors);
    p.records.into_iter().next().unwrap().item
}

fn buy(net: &str) -> ImportRecord {
    rec(json!({
        "source": src("row 10"), "account": {"hint": "U1"}, "kind": "buy", "date": "2026-03-02",
        "instrument": {"symbol": "XYZ"}, "quantity": "10",
        "price": {"amount": "150.00", "currency": "EUR"},
        "charges": [{"type": "commission", "amount": {"amount": "1.00", "currency": "EUR"}}],
        "net_cash": {"amount": net, "currency": "EUR"}
    }))
}

// ---- 6.1 ----

#[test]
fn consistent_buy_has_no_warning() {
    assert!(check_record(0, &buy("1501.00")).is_empty());
    // Within half a minor unit.
    assert!(check_record(0, &buy("1501.004")).is_empty());
}

#[test]
fn inconsistent_buy_warns_and_keeps_figures() {
    let r = buy("1600.00");
    let before = r.clone();
    let w = check_record(3, &r);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].index, 3);
    assert_eq!(w[0].locator, "row 10");
    assert!(
        w[0].message.contains("1501 EUR") || w[0].message.contains("1501.00 EUR"),
        "{}",
        w[0].message
    );
    assert!(w[0].message.contains("99"), "{}", w[0].message);
    assert_eq!(r, before, "figures unchanged");
}

#[test]
fn sell_subtracts_charges_and_uses_multiplier_and_fx() {
    let sell = rec(json!({
        "source": src("row 131"), "account": {"hint": "D1"}, "kind": "sell", "date": "2026-09-12",
        "instrument": {"symbol": "SAP"}, "quantity": "8",
        "price": {"amount": "214.50", "currency": "EUR"},
        "charges": [{"type": "commission", "amount": {"amount": "3.90", "currency": "EUR"}}],
        "net_cash": {"amount": "1702.10", "currency": "EUR"}
    }));
    let w = check_record(0, &sell);
    assert_eq!(
        w.len(),
        1,
        "8 × 214.50 − 3.90 = 1712.10, statement shows 1702.10"
    );
    assert!(w[0].message.contains("10"), "{}", w[0].message);

    let option = rec(json!({
        "source": src("row 2"), "account": {"hint": "U1"}, "kind": "buy", "date": "2026-03-02",
        "instrument": {"symbol": "AAPL 261218C200", "multiplier": "100"}, "quantity": "2",
        "price": {"amount": "3.10", "currency": "USD"},
        "net_cash": {"amount": "571.20", "currency": "EUR"},
        "fx": {"rate": "0.92", "from": "USD", "to": "EUR"}
    }));
    assert_eq!(
        check_record(0, &option).len(),
        1,
        "2 × 3.10 × 100 × 0.92 = 570.40, statement shows 571.20"
    );
}

#[test]
fn fx_conversion_that_matches_has_no_warning() {
    let option = rec(json!({
        "source": src("row 2"), "account": {"hint": "U1"}, "kind": "buy", "date": "2026-03-02",
        "instrument": {"symbol": "OPT", "multiplier": "100"}, "quantity": "2",
        "price": {"amount": "3.10", "currency": "USD"},
        "net_cash": {"amount": "570.40", "currency": "EUR"},
        "fx": {"rate": "0.92", "from": "USD", "to": "EUR"}
    }));
    assert!(check_record(0, &option).is_empty());
}

#[test]
fn income_gross_minus_charges() {
    let ok = rec(json!({
        "source": src("row 184"), "account": {"hint": "U1"}, "kind": "dividend", "date": "2026-03-14",
        "instrument": {"isin": "US0378331005"},
        "gross": {"amount": "24.00", "currency": "USD"},
        "charges": [{"type": "withholding_tax", "amount": {"amount": "3.60", "currency": "USD"}}],
        "net_cash": {"amount": "20.40", "currency": "USD"}
    }));
    assert!(check_record(0, &ok).is_empty());
    let mut bad = ok.clone();
    bad.net_cash.as_mut().unwrap().amount = "24.00".parse().unwrap();
    assert_eq!(check_record(0, &bad).len(), 1);
}

#[test]
fn unconvertible_currencies_skip_the_check() {
    let r = rec(json!({
        "source": src("row 1"), "account": {"hint": "U1"}, "kind": "buy", "date": "2026-03-02",
        "instrument": {"symbol": "X"}, "quantity": "1",
        "price": {"amount": "100", "currency": "USD"},
        "net_cash": {"amount": "1", "currency": "EUR"}
    }));
    assert!(
        check_record(0, &r).is_empty(),
        "no FX on the statement: nothing to compare"
    );
}

// ---- 6.2 ----

#[test]
fn missing_figures_are_never_derived() {
    let r = rec(json!({
        "source": src("row 7"), "account": {"hint": "U1"}, "kind": "buy", "date": "2026-03-02",
        "instrument": {"symbol": "XYZ"}, "quantity": "10",
        "net_cash": {"amount": "1501.00", "currency": "EUR"}
    }));
    assert!(check_record(0, &r).is_empty());

    let mut store = Store::open_in_memory().unwrap();
    let acct = store.create_account("A", "U1").unwrap();
    store
        .commit_batch(&NewBatch {
            adapter: "t".into(),
            adapter_version: "1".into(),
            file_sha256: "0".repeat(64),
            activities: vec![NewActivity {
                account_id: acct,
                asset_id: None,
                fingerprint: "fp".into(),
                source_order: 0,
                basis_unknown: false,
                record: r,
            }],
            assertions: vec![],
        })
        .unwrap();
    let stored = &store.activities_until(acct, "2026-12-31").unwrap()[0].record;
    assert!(
        stored.price.is_none() && stored.gross.is_none(),
        "price and gross stay absent"
    );
    assert_eq!(
        stored.net_cash.as_ref().unwrap().amount.to_string(),
        "1501.00"
    );
}

// ---- 6.3 ----

fn input(file: &str, order: i64, kind: Kind, key: Option<&str>) -> OrderInput {
    OrderInput {
        account: "U1".into(),
        date: Date::parse("2026-03-02").unwrap(),
        file: file.into(),
        source_order: order,
        kind,
        instrument_key: key.map(str::to_owned),
        locator: format!("row {order}"),
    }
}

#[test]
fn source_order_preserved_within_a_file() {
    // Sell listed before buy in the source: source order wins over kind priority within one file.
    let items = [
        input("f1", 11, Kind::Buy, Some("isin:X")),
        input("f1", 10, Kind::Sell, Some("isin:X")),
    ];
    let (ord, diags) = order(&items);
    assert_eq!(ord, vec![1, 0]);
    assert!(diags.is_empty());

    let items = [
        input("f1", 10, Kind::Buy, Some("isin:X")),
        input("f1", 11, Kind::Sell, Some("isin:X")),
    ];
    assert_eq!(
        order(&items).0,
        vec![0, 1],
        "buy at row 10 before sell at row 11"
    );
}

#[test]
fn files_interleave_by_kind_priority() {
    let items = [
        input("f2", 1, Kind::Fee, None),
        input("f1", 5, Kind::Dividend, Some("isin:Y")),
        input("f2", 0, Kind::Deposit, None),
        input("f1", 4, Kind::Buy, Some("isin:Z")),
    ];
    let (ord, diags) = order(&items);
    let kinds: Vec<Kind> = ord.iter().map(|&i| items[i].kind).collect();
    assert_eq!(
        kinds,
        vec![Kind::Deposit, Kind::Buy, Kind::Dividend, Kind::Fee]
    );
    assert!(diags.is_empty());
}

#[test]
fn ambiguous_cross_file_order_is_flagged() {
    let items = [
        input("f1", 3, Kind::Buy, Some("isin:X")),
        input("f2", 8, Kind::Sell, Some("isin:X")),
        input("f2", 9, Kind::Sell, Some("isin:OTHER")),
    ];
    let (ord, diags) = order(&items);
    assert_eq!(ord.len(), 3);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].instrument_key, "isin:X");
    assert_eq!(
        diags[0].records,
        vec!["f1:row 3".to_string(), "f2:row 8".to_string()]
    );

    // Same direction across files: order does not change the result, no diagnostic.
    let items = [
        input("f1", 3, Kind::Buy, Some("isin:X")),
        input("f2", 8, Kind::Buy, Some("isin:X")),
    ];
    assert!(order(&items).1.is_empty());
}

#[test]
fn ordering_is_deterministic_across_input_permutations() {
    let a = [
        input("f2", 1, Kind::Fee, None),
        input("f1", 5, Kind::Dividend, Some("isin:Y")),
        input("f2", 0, Kind::Deposit, None),
    ];
    let b = [a[2].clone(), a[0].clone(), a[1].clone()];
    let ka: Vec<_> = order(&a).0.iter().map(|&i| a[i].clone()).collect();
    let kb: Vec<_> = order(&b).0.iter().map(|&i| b[i].clone()).collect();
    assert_eq!(ka, kb);
}
