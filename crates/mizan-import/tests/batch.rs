use mizan_import::*;
use serde_json::{json, Value};

const SHA: &str = "9f2c00000000000000000000000000000000000000000000000000000000a71e";

fn source(locator: &str) -> Value {
    json!({"adapter": "test-csv", "adapter_version": "1.0.0", "file_sha256": SHA, "locator": locator})
}

fn dividend() -> Value {
    json!({
        "source": source("row 184"),
        "external_id": "1234567890",
        "account": {"hint": "U1234567", "name_hint": "IBKR Margin"},
        "kind": "dividend",
        "date": "2026-03-14",
        "instrument": {"isin": "US0378331005", "symbol": "AAPL", "mic": "XNAS", "name": "APPLE INC", "class_hint": "equity", "currency": "USD"},
        "quantity": "100",
        "gross": {"amount": "24.00", "currency": "USD"},
        "charges": [{"type": "withholding_tax", "amount": {"amount": "3.60", "currency": "USD"}}],
        "net_cash": {"amount": "20.40", "currency": "USD"},
        "memo": "AAPL Cash Dividend USD 0.24 per Share"
    })
}

fn batch(records: Vec<Value>) -> Value {
    json!({"schema": "mizan.import/1", "records": records})
}

fn parse(v: Value) -> ParsedBatch {
    parse_batch_value(v).expect("batch accepted")
}

fn single_error(v: Value) -> ItemError {
    let p = parse(batch(vec![v]));
    assert!(p.records.is_empty(), "record should be rejected");
    assert_eq!(p.errors.len(), 1, "{:?}", p.errors);
    p.errors.into_iter().next().unwrap()
}

fn with(mut base: Value, key: &str, value: Value) -> Value {
    base[key] = value;
    base
}

// ---- 2.1 versioned batch ----

#[test]
fn supported_version_accepted() {
    let p = parse(batch(vec![dividend()]));
    assert!(p.is_clean(), "{:?}", p.errors);
    assert_eq!(p.records.len(), 1);
}

#[test]
fn unsupported_version_rejected_whole() {
    let mut b = batch(vec![dividend()]);
    b["schema"] = json!("mizan.import/9");
    let err = parse_batch_value(b).unwrap_err();
    assert_eq!(err, BatchError::UnsupportedVersion("mizan.import/9".into()));
    assert!(err.to_string().contains("mizan.import/9"));
}

#[test]
fn missing_version_rejected() {
    let err = parse_batch_value(json!({"records": []})).unwrap_err();
    assert_eq!(err, BatchError::MissingSchema);
}

// ---- 2.3 provenance ----

#[test]
fn provenance_present() {
    let p = parse(batch(vec![dividend()]));
    let src = &p.records[0].item.source;
    assert_eq!(src.adapter, "test-csv");
    assert_eq!(src.adapter_version, "1.0.0");
    assert_eq!(src.file_sha256.as_str(), SHA);
    assert_eq!(src.locator, "row 184");
}

#[test]
fn missing_hash_rejected_naming_record() {
    let mut r = dividend();
    r["source"].as_object_mut().unwrap().remove("file_sha256");
    let e = single_error(r);
    assert_eq!(e.item, ItemKind::Record);
    assert_eq!(e.index, 0);
    assert_eq!(e.locator.as_deref(), Some("row 184"));
    assert!(e.message.contains("file_sha256"), "{}", e.message);
    assert!(e.to_string().starts_with("record #0 (row 184)"), "{e}");
}

#[test]
fn missing_locator_rejected() {
    let mut r = dividend();
    r["source"].as_object_mut().unwrap().remove("locator");
    let e = single_error(r);
    assert!(e.message.contains("locator"), "{}", e.message);
}

#[test]
fn one_bad_record_does_not_reject_the_others() {
    let mut bad = dividend();
    bad["kind"] = json!("unknown");
    let p = parse(batch(vec![dividend(), bad, dividend()]));
    assert_eq!(p.records.len(), 2);
    assert_eq!(p.records[1].index, 2);
    assert_eq!(p.errors.len(), 1);
    assert_eq!(p.errors[0].index, 1);
}

// ---- 2.4 kinds ----

#[test]
fn closed_kind_list_has_24_kinds_that_round_trip() {
    assert_eq!(Kind::ALL.len(), 24);
    for k in Kind::ALL {
        let v = serde_json::to_value(k).unwrap();
        assert_eq!(v, json!(k.as_str()));
        assert_eq!(serde_json::from_value::<Kind>(v).unwrap(), k);
    }
}

#[test]
fn known_kind_accepted_unknown_rejected() {
    assert!(parse(batch(vec![dividend()])).is_clean());
    let e = single_error(with(dividend(), "kind", json!("unknown")));
    assert!(e.message.contains("unknown variant"), "{}", e.message);
}

// ---- 2.5 hints ----

#[test]
fn hints_only_accepted() {
    let p = parse(batch(vec![dividend()]));
    let i = p.records[0].item.instrument.as_ref().unwrap();
    assert_eq!(i.isin.as_deref(), Some("US0378331005"));
    assert_eq!(i.symbol.as_deref(), Some("AAPL"));
}

#[test]
fn internal_identifiers_rejected() {
    let e = single_error(with(dividend(), "asset_id", json!(42)));
    assert!(
        e.message.contains("internal Mizan identifier"),
        "{}",
        e.message
    );

    let mut r = dividend();
    r["account"]["account_id"] = json!(7);
    let e = single_error(r);
    assert!(
        e.message.contains("\"account_id\" in \"account\""),
        "{}",
        e.message
    );
}

#[test]
fn instrument_hint_needs_an_identifier() {
    let e = single_error(with(dividend(), "instrument", json!({"currency": "USD"})));
    assert!(e.message.contains("at least one of"), "{}", e.message);
}

// ---- 2.2 / 2.6 decimals and dates inside records ----

#[test]
fn float_amount_rejects_record() {
    let e = single_error(with(
        dividend(),
        "net_cash",
        json!({"amount": 20.4, "currency": "USD"}),
    ));
    assert!(e.message.contains("strings"), "{}", e.message);
}

#[test]
fn record_dates() {
    let p = parse(batch(vec![with(
        dividend(),
        "settle_date",
        json!("2026-03-18"),
    )]));
    let r = &p.records[0].item;
    assert_eq!(r.date.to_string(), "2026-03-14");
    assert_eq!(r.settle_date.unwrap().to_string(), "2026-03-18");

    let e = single_error(with(dividend(), "date", json!("2026-03-14T09:30:00Z")));
    assert!(e.message.contains("no time component"), "{}", e.message);
}

// ---- 2.7 diagnostics ----

#[test]
fn diagnostics_round_trip_and_are_not_records() {
    let diag =
        json!({"severity": "warning", "message": "Unrecognized description", "locator": "row 87"});
    let p = parse(json!({"schema": "mizan.import/1", "diagnostics": [diag.clone()]}));
    assert_eq!(p.diagnostics.len(), 1);
    assert!(p.records.is_empty());
    assert_eq!(serde_json::to_value(&p.diagnostics[0]).unwrap(), diag);

    // A diagnostic placed in the records array is rejected, never accepted as a record.
    let p = parse(batch(vec![diag]));
    assert!(p.records.is_empty());
    assert_eq!(p.errors.len(), 1);
}

// ---- 3.1 charges ----

#[test]
fn dividend_with_withholding_validates() {
    let p = parse(batch(vec![dividend()]));
    let r = &p.records[0].item;
    assert_eq!(r.gross.as_ref().unwrap().amount.to_string(), "24.00");
    assert_eq!(r.charges.len(), 1);
    assert_eq!(r.charges[0].charge_type, ChargeType::WithholdingTax);
    assert_eq!(r.charges[0].amount.amount.to_string(), "3.60");
    assert_eq!(r.net_cash.as_ref().unwrap().amount.to_string(), "20.40");
}

#[test]
fn standalone_fee_and_negative_charge() {
    let fee = json!({
        "source": source("row 3"), "account": {"hint": "U1"}, "kind": "fee", "date": "2026-03-01",
        "net_cash": {"amount": "5.00", "currency": "EUR"}, "memo": "Monthly account fee"
    });
    assert!(parse(batch(vec![fee])).is_clean());

    let mut r = dividend();
    r["charges"][0]["amount"]["amount"] = json!("-3.60");
    let e = single_error(r);
    assert!(e.message.contains("must not be negative"), "{}", e.message);

    let mut r = dividend();
    r["charges"][0]["type"] = json!("gst");
    assert!(single_error(r).message.contains("unknown variant"));
}

// ---- 3.2 legs ----

fn spinoff() -> Value {
    json!({
        "source": source("row 40"), "account": {"hint": "U1"}, "kind": "spinoff", "date": "2026-04-01",
        "instrument": {"symbol": "A"},
        "legs": [{"instrument": {"symbol": "C"}, "quantity_delta": "50", "cost_basis_share": "0.10"}]
    })
}

#[test]
fn spinoff_with_legs_validates() {
    let p = parse(batch(vec![spinoff()]));
    assert!(p.is_clean(), "{:?}", p.errors);
    let leg = &p.records[0].item.legs[0];
    assert_eq!(leg.quantity_delta.to_string(), "50");
    assert_eq!(leg.cost_basis_share.unwrap().to_string(), "0.10");
}

#[test]
fn legs_on_buy_rejected_and_spinoff_needs_legs() {
    let mut buy = spinoff();
    buy["kind"] = json!("buy");
    assert!(single_error(buy)
        .message
        .contains("only exchange and spinoff"));

    let mut s = spinoff();
    s.as_object_mut().unwrap().remove("legs");
    assert!(single_error(s).message.contains("at least one leg"));
}

// ---- 3.3 split ----

fn split(new: &str, old: &str) -> Value {
    json!({
        "source": source("row 5"), "account": {"hint": "U1"}, "kind": "split", "date": "2026-06-10",
        "instrument": {"symbol": "NVDA"}, "split": {"new": new, "old": old}
    })
}

#[test]
fn split_ratios() {
    let p = parse(batch(vec![split("4", "1")]));
    let s = p.records[0].item.split.as_ref().unwrap();
    assert_eq!(
        (s.new.to_string(), s.old.to_string()),
        ("4".into(), "1".into())
    );
    assert!(single_error(split("0", "1"))
        .message
        .contains("must be positive"));
    assert!(single_error(split("4", "-1"))
        .message
        .contains("must be positive"));
}

// ---- 3.4 position transfers ----

fn position_in(with_basis: bool) -> Value {
    let mut r = json!({
        "source": source("row 9"), "account": {"hint": "DEG-4417"}, "kind": "position_in", "date": "2026-05-02",
        "instrument": {"isin": "IE00BK5BQT80"}, "quantity": "20"
    });
    if with_basis {
        r["cost_basis"] = json!({"amount": "2000.00", "currency": "USD"});
        r["acquisition_date"] = json!("2024-11-15");
    }
    r
}

#[test]
fn position_in_with_and_without_basis() {
    let p = parse(batch(vec![position_in(true), position_in(false)]));
    assert!(p.is_clean(), "{:?}", p.errors);
    assert_eq!(
        p.records[0]
            .item
            .cost_basis
            .as_ref()
            .unwrap()
            .amount
            .to_string(),
        "2000.00"
    );
    assert!(p.records[1].item.cost_basis.is_none());
}

#[test]
fn position_rules() {
    let mut r = position_in(false);
    r.as_object_mut().unwrap().remove("quantity");
    assert!(single_error(r).message.contains("must carry a quantity"));

    let mut out = position_in(true);
    out["kind"] = json!("position_out");
    assert!(single_error(out)
        .message
        .contains("only allowed on position_in"));
}

// ---- 3.5 adjustments ----

#[test]
fn adjustment_needs_memo() {
    let adj = json!({
        "source": source("row 1"), "account": {"hint": "U1"}, "kind": "adjustment", "date": "2026-01-01",
        "net_cash": {"amount": "0.01", "currency": "EUR"}
    });
    assert!(single_error(adj.clone()).message.contains("memo"));
    assert!(parse(batch(vec![with(
        adj,
        "memo",
        json!("Rounding difference on opening balance")
    )]))
    .is_clean());
}

// ---- 3.6 links ----

#[test]
fn broker_cancellation_link_validates() {
    let mut cancel = dividend();
    cancel["external_id"] = json!("1234567999");
    cancel["links"] = json!([{"role": "reverses", "target": "1234567890"}]);
    let p = parse(batch(vec![dividend(), cancel]));
    assert!(p.is_clean(), "{:?}", p.errors);
    assert_eq!(p.records[1].item.links[0].role, LinkRole::Reverses);

    let mut bad = dividend();
    bad["links"] = json!([{"role": "replaces", "target": "x"}]);
    assert!(single_error(bad).message.contains("unknown variant"));
}

// ---- 3.7 assertions ----

#[test]
fn closing_balance_assertion_validates_and_empty_rejected() {
    let a = json!({
        "source": source("page 4"), "account": {"hint": "U1234567"}, "as_of": "2026-03-31",
        "cash": [{"amount": "1520.33", "currency": "USD"}],
        "positions": [{"instrument": {"isin": "US0378331005"}, "quantity": "100"}]
    });
    let p = parse(json!({"schema": "mizan.import/1", "assertions": [a.clone()]}));
    assert!(p.is_clean(), "{:?}", p.errors);
    assert_eq!(p.assertions[0].item.cash[0].amount.to_string(), "1520.33");

    let mut empty = a;
    empty.as_object_mut().unwrap().remove("cash");
    empty.as_object_mut().unwrap().remove("positions");
    let p = parse(json!({"schema": "mizan.import/1", "assertions": [empty]}));
    assert!(p.assertions.is_empty());
    assert!(p.errors[0]
        .message
        .contains("at least one cash balance or position"));
    assert_eq!(p.errors[0].item, ItemKind::Assertion);
}

// ---- docs ----

#[test]
fn documented_example_is_a_clean_batch() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/import-schema.md");
    let doc = std::fs::read_to_string(path).unwrap();
    let start = doc.find("```json\n").expect("example block") + "```json\n".len();
    let end = start + doc[start..].find("```").unwrap();
    let p = parse_batch(&doc[start..end]).expect("example accepted");
    assert!(p.is_clean(), "{:?}", p.errors);
    assert_eq!(
        (p.records.len(), p.assertions.len(), p.diagnostics.len()),
        (1, 1, 1)
    );
}
