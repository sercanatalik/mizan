use mizan_core::*;
use mizan_import::{parse_batch_value, Code, ImportRecord, ParsedBatch};
use mizan_storage::Store;
use rust_decimal::Decimal;
use serde_json::{json, Value};

fn sha(c: char) -> String {
    c.to_string().repeat(64)
}

fn src(file: char, locator: &str) -> Value {
    json!({"adapter": "test-csv", "adapter_version": "1.0.0", "file_sha256": sha(file), "locator": locator})
}

fn parsed(records: Vec<Value>, assertions: Vec<Value>) -> ParsedBatch {
    let p = parse_batch_value(
        json!({"schema": "mizan.import/1", "records": records, "assertions": assertions}),
    )
    .unwrap();
    assert!(p.is_clean(), "{:?}", p.errors);
    p
}

fn rec(v: Value) -> ImportRecord {
    parsed(vec![v], vec![]).records.remove(0).item
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

fn run(store: &mut Store, p: ParsedBatch) -> Prepared {
    let rb = {
        let mut r = MappingResolver {
            store,
            policy: CREATE,
        };
        resolve_batch(p, &mut r).unwrap()
    };
    prepare(store, rb).unwrap()
}

fn import_all(store: &mut Store, p: ParsedBatch) -> Prepared {
    let prepared = run(store, p);
    commit(store, &prepared, &Decisions::new()).unwrap();
    prepared
}

// ---- 7.1 fingerprints ----

fn trade(file: char, row: &str, ext: Option<&str>, date: &str) -> Value {
    let mut v = json!({
        "source": src(file, row), "account": {"hint": "U1"}, "kind": "buy", "date": date,
        "instrument": {"isin": "NL0010273215"}, "quantity": "2",
        "net_cash": {"amount": "1284.60", "currency": "EUR"}
    });
    if let Some(e) = ext {
        v["external_id"] = json!(e);
    }
    v
}

#[test]
fn same_external_id_same_fingerprint_across_batches() {
    let a = rec(trade('a', "row 1", Some("T-77"), "2026-07-01"));
    let mut b = rec(trade('b', "row 9", Some("T-77"), "2026-07-01"));
    b.quantity = Some("3".parse().unwrap()); // figures differ; the external id decides
    assert_eq!(fingerprints([&a]), fingerprints([&b]));
    assert!(fingerprints([&a])[0].starts_with("x:"));
}

#[test]
fn identical_same_day_trades_get_occurrence_indexes() {
    let a = rec(trade('a', "row 1", None, "2026-07-01"));
    let b = rec(trade('a', "row 2", None, "2026-07-01"));
    let fps = fingerprints([&a, &b]);
    assert_ne!(fps[0], fps[1]);
    assert!(fps[0].ends_with("#0") && fps[1].ends_with("#1"), "{fps:?}");
    assert_eq!(fps[0].trim_end_matches("#0"), fps[1].trim_end_matches("#1"));

    // Locator does not matter; normalized quantity does not matter.
    let mut c = rec(trade('f', "row 99", None, "2026-07-01"));
    c.quantity = Some("2.00".parse().unwrap());
    assert_eq!(fingerprints([&c])[0], fps[0]);
}

// ---- 7.2 overlapping re-import ----

fn monthly(file: char, months: &[u32]) -> ParsedBatch {
    let recs = months
        .iter()
        .map(|m| {
            json!({
                "source": src(file, &format!("row {m}")), "external_id": format!("DIV-2026-{m:02}"),
                "account": {"hint": "U1"}, "kind": "interest", "date": format!("2026-{m:02}-28"),
                "net_cash": {"amount": "3.18", "currency": "EUR"}
            })
        })
        .collect();
    parsed(recs, vec![])
}

#[test]
fn overlapping_statements_propose_only_the_surplus() {
    let mut store = Store::open_in_memory().unwrap();
    import_all(&mut store, monthly('a', &[3, 4, 5, 6]));

    let p = run(&mut store, monthly('b', &[5, 6, 7, 8]));
    let r = report(&store, &p, &Decisions::new()).unwrap();
    let new_dates: Vec<&str> = r.new_records.iter().map(|s| s.date.as_str()).collect();
    assert_eq!(new_dates, vec!["2026-07-28", "2026-08-28"]);
    assert_eq!(r.summary.duplicates, 2);
    assert_eq!(r.summary.pending_confirmations, 0);
}

#[test]
fn count_based_surplus_for_identical_records() {
    let mut store = Store::open_in_memory().unwrap();
    // File A has two identical buys; re-importing the same file with a third one adds only the third.
    import_all(
        &mut store,
        parsed(
            vec![
                trade('a', "row 1", None, "2026-07-01"),
                trade('a', "row 2", None, "2026-07-01"),
            ],
            vec![],
        ),
    );
    let p = run(
        &mut store,
        parsed(
            vec![
                trade('a', "row 1", None, "2026-07-01"),
                trade('a', "row 2", None, "2026-07-01"),
                trade('a', "row 3", None, "2026-07-01"),
            ],
            vec![],
        ),
    );
    let outcomes: Vec<&Outcome> = p.records.iter().map(|r| &r.outcome).collect();
    assert!(matches!(outcomes[0], Outcome::Duplicate { .. }));
    assert!(matches!(outcomes[1], Outcome::Duplicate { .. }));
    assert_eq!(outcomes[2], &Outcome::New);
}

// ---- 7.3 cross-file ambiguity ----

#[test]
fn cross_file_match_without_external_id_needs_confirmation() {
    let mut store = Store::open_in_memory().unwrap();
    import_all(
        &mut store,
        parsed(vec![trade('a', "row 188", None, "2026-07-01")], vec![]),
    );

    let p = run(
        &mut store,
        parsed(vec![trade('b', "row 12", None, "2026-07-01")], vec![]),
    );
    assert!(
        matches!(&p.records[0].outcome, Outcome::NeedsConfirmation { existing_locator, .. } if existing_locator == "row 188")
    );

    let r = report(&store, &p, &Decisions::new()).unwrap();
    assert_eq!(r.summary.pending_confirmations, 1);
    assert_eq!(r.confirmations[0].record.locator, "row 12");
    assert_eq!(
        r.summary.new, 0,
        "undecided records are not proposed as new"
    );

    // The user says it was a second trade.
    let decisions = Decisions::from([(0, Decision::New)]);
    let r = report(&store, &p, &decisions).unwrap();
    assert_eq!(r.summary.new, 1);
    assert_eq!(r.summary.pending_confirmations, 0);
    let res = commit(&mut store, &p, &decisions).unwrap();
    assert_eq!(res.committed, 1);
    assert_eq!(store.count("activities").unwrap(), 2);
}

// ---- 8.1 replay ----

fn bal(records: &[Value]) -> Balances {
    let rs: Vec<ImportRecord> = records.iter().map(|v| rec(v.clone())).collect();
    replay(rs.iter())
}

fn base(kind: &str) -> Value {
    json!({"source": src('a', "row 1"), "account": {"hint": "U1"}, "kind": kind, "date": "2026-03-01"})
}

fn with(mut v: Value, k: &str, x: Value) -> Value {
    v[k] = x;
    v
}

fn eur(a: &str) -> Value {
    json!({"amount": a, "currency": "EUR"})
}

#[test]
fn replay_cash_kinds() {
    let e = Code::parse("EUR").unwrap();
    let cases: Vec<(Value, &str)> = vec![
        (with(base("deposit"), "net_cash", eur("100")), "100"),
        (with(base("withdrawal"), "net_cash", eur("40")), "-40"),
        (with(base("interest"), "net_cash", eur("3.18")), "3.18"),
        (
            with(
                with(base("fee"), "net_cash", eur("2.50")),
                "memo",
                json!("fee"),
            ),
            "-2.50",
        ),
        (with(base("tax"), "net_cash", eur("7")), "-7"),
        (
            with(base("internal_transfer"), "net_cash", eur("-25")),
            "-25",
        ),
        (
            with(
                with(base("adjustment"), "net_cash", eur("0.01")),
                "memo",
                json!("rounding"),
            ),
            "0.01",
        ),
    ];
    for (v, expected) in cases {
        assert_eq!(
            bal(std::slice::from_ref(&v)).cash_in(&e),
            d(expected),
            "{v}"
        );
    }
}

#[test]
fn replay_trades_income_and_charges() {
    let e = Code::parse("EUR").unwrap();
    let inst = json!({"isin": "X1"});
    let buy = json!({"source": src('a', "row 1"), "account": {"hint": "U1"}, "kind": "buy", "date": "2026-03-01",
        "instrument": inst, "quantity": "10", "net_cash": eur("1501.00")});
    let sell = with(
        with(
            with(buy.clone(), "kind", json!("sell")),
            "quantity",
            json!("4"),
        ),
        "net_cash",
        eur("700"),
    );
    let short = with(
        with(buy.clone(), "kind", json!("short_sell")),
        "quantity",
        json!("8"),
    );
    let cover = with(
        with(buy.clone(), "kind", json!("buy_to_cover")),
        "quantity",
        json!("8"),
    );
    let b = bal(&[buy.clone(), sell]);
    assert_eq!(b.position("isin:X1"), d("6"));
    assert_eq!(b.cash_in(&e), d("-801.00"));
    let b = bal(&[short, cover]);
    assert_eq!(b.position("isin:X1"), d("0"));

    // No net cash: gross plus charges for a buy, quantity × price when there is no gross.
    let derived = json!({"source": src('a', "row 1"), "account": {"hint": "U1"}, "kind": "buy", "date": "2026-03-01",
        "instrument": inst, "quantity": "10", "price": eur("150.00"),
        "charges": [{"type": "commission", "amount": eur("1.00")}]});
    assert_eq!(bal(&[derived]).cash_in(&e), d("-1501.00"));

    let div = json!({"source": src('a', "row 2"), "account": {"hint": "U1"}, "kind": "dividend", "date": "2026-03-01",
        "instrument": inst, "gross": eur("24.00"), "charges": [{"type": "withholding_tax", "amount": eur("3.60")}]});
    assert_eq!(bal(&[div]).cash_in(&e), d("20.40"));
    for k in ["distribution", "reward", "return_of_capital"] {
        assert_eq!(
            bal(&[with(
                with(base(k), "net_cash", eur("5")),
                "instrument",
                inst.clone()
            )])
            .cash_in(&e),
            d("5"),
            "{k}"
        );
    }
}

#[test]
fn replay_position_moves_corporate_actions_and_options() {
    let e = Code::parse("EUR").unwrap();
    let inst = json!({"isin": "X1"});
    let pin = json!({"source": src('a', "row 1"), "account": {"hint": "U1"}, "kind": "position_in", "date": "2026-03-01",
        "instrument": inst, "quantity": "20", "charges": [{"type": "other", "amount": eur("15")}]});
    let pout = with(
        with(pin.clone(), "kind", json!("position_out")),
        "quantity",
        json!("5"),
    );
    let split = json!({"source": src('a', "row 3"), "account": {"hint": "U1"}, "kind": "split", "date": "2026-03-02",
        "instrument": inst, "split": {"new": "4", "old": "1"}});
    let spin = json!({"source": src('a', "row 4"), "account": {"hint": "U1"}, "kind": "spinoff", "date": "2026-03-03",
        "instrument": inst, "legs": [{"instrument": {"isin": "C1"}, "quantity_delta": "30"}], "net_cash": eur("1.25")});
    let exch = json!({"source": src('a', "row 5"), "account": {"hint": "U1"}, "kind": "exchange", "date": "2026-03-04",
        "instrument": {"isin": "C1"}, "legs": [
            {"instrument": {"isin": "C1"}, "quantity_delta": "-30"},
            {"instrument": {"isin": "D1"}, "quantity_delta": "15"}]});
    let b = bal(&[pin, pout, split, spin, exch]);
    assert_eq!(b.position("isin:X1"), d("60"), "(20 - 5) × 4");
    assert_eq!(b.position("isin:C1"), d("0"));
    assert_eq!(b.position("isin:D1"), d("15"));
    assert_eq!(
        b.cash_in(&e),
        d("-28.75"),
        "two transfer charges, cash in lieu"
    );

    let opt = json!({"symbol": "OPT"});
    let buy = json!({"source": src('a', "row 1"), "account": {"hint": "U1"}, "kind": "buy", "date": "2026-03-01",
        "instrument": opt, "quantity": "3"});
    let ex = with(
        with(buy.clone(), "kind", json!("option_exercise")),
        "quantity",
        json!("1"),
    );
    let asg = with(
        with(buy.clone(), "kind", json!("option_assignment")),
        "quantity",
        json!("1"),
    );
    assert_eq!(bal(&[buy.clone(), ex, asg]).position("sym:OPT@"), d("3"));
    let exp = with(base("option_expiry"), "instrument", opt);
    assert_eq!(bal(&[buy, exp]).position("sym:OPT@"), d("0"));

    let fx = json!({"source": src('a', "row 9"), "account": {"hint": "U1"}, "kind": "fx_conversion", "date": "2026-03-01",
        "gross": {"amount": "100", "currency": "USD"}, "net_cash": eur("92")});
    let b = bal(&[fx]);
    assert_eq!(b.cash_in(&Code::parse("USD").unwrap()), d("-100"));
    assert_eq!(b.cash_in(&e), d("92"));
}

// ---- 8.2 assertions ----

fn statement(file: char, with_second_buy: bool, aapl: &str) -> ParsedBatch {
    let usd = |a: &str| json!({"amount": a, "currency": "USD"});
    let mut recs = vec![
        json!({"source": src(file, "row 1"), "external_id": "D1", "account": {"hint": "U1234567"}, "kind": "deposit",
               "date": "2026-03-01", "net_cash": usd("16520.33")}),
        json!({"source": src(file, "row 2"), "external_id": "B1", "account": {"hint": "U1234567"}, "kind": "buy",
               "date": "2026-03-02", "instrument": {"isin": "US0378331005"}, "quantity": "90", "net_cash": usd("13500.00")}),
    ];
    if with_second_buy {
        recs.push(json!({"source": src(file, "row 3"), "external_id": "B2", "account": {"hint": "U1234567"}, "kind": "buy",
               "date": "2026-03-20", "instrument": {"isin": "US0378331005"}, "quantity": "10", "net_cash": usd("1500.00")}));
    }
    let a = json!({"source": src(file, "page 4"), "account": {"hint": "U1234567"}, "as_of": "2026-03-31",
        "cash": [usd(if with_second_buy { "1520.33" } else { "3020.33" })],
        "positions": [{"instrument": {"isin": "US0378331005"}, "quantity": aapl}]});
    parsed(recs, vec![a])
}

#[test]
fn assertion_passes() {
    let mut store = Store::open_in_memory().unwrap();
    let p = run(&mut store, statement('a', true, "100"));
    let r = report(&store, &p, &Decisions::new()).unwrap();
    let a = &r.assertion_results[0];
    assert!(a.passed, "{a:?}");
    assert_eq!(a.lines[0].label, "USD");
    assert_eq!(a.lines[0].expected, "1520.33");
    assert_eq!(a.lines[1].label, "isin:US0378331005");
    assert_eq!(a.lines[1].computed, "100");
}

#[test]
fn assertion_fails_with_expected_and_computed() {
    let mut store = Store::open_in_memory().unwrap();
    let p = run(&mut store, statement('a', false, "100"));
    let r = report(&store, &p, &Decisions::new()).unwrap();
    let a = &r.assertion_results[0];
    assert!(!a.passed);
    let line = a
        .lines
        .iter()
        .find(|l| l.label == "isin:US0378331005")
        .unwrap();
    assert_eq!(
        (line.expected.as_str(), line.computed.as_str(), line.passed),
        ("100", "90", false)
    );
    assert_eq!(r.summary.assertions_failed, 1);

    // Results are stored with the batch.
    commit(&mut store, &p, &Decisions::new()).unwrap();
    assert_eq!(store.count("assertion_results").unwrap(), 1);
}

#[test]
fn assertion_includes_previously_stored_activities() {
    let mut store = Store::open_in_memory().unwrap();
    import_all(&mut store, statement('a', false, "90"));
    // A later statement for the same account only has the second buy, but asserts the full balance.
    let usd = |a: &str| json!({"amount": a, "currency": "USD"});
    let later = parsed(
        vec![
            json!({"source": src('b', "row 1"), "external_id": "B2", "account": {"hint": "U1234567"}, "kind": "buy",
            "date": "2026-04-02", "instrument": {"isin": "US0378331005"}, "quantity": "10", "net_cash": usd("1500.00")}),
        ],
        vec![
            json!({"source": src('b', "page 2"), "account": {"hint": "U1234567"}, "as_of": "2026-04-30",
            "cash": [usd("1520.33")], "positions": [{"instrument": {"isin": "US0378331005"}, "quantity": "100"}]}),
        ],
    );
    let p = run(&mut store, later);
    let r = report(&store, &p, &Decisions::new()).unwrap();
    assert!(
        r.assertion_results[0].passed,
        "{:?}",
        r.assertion_results[0]
    );
}

// ---- 9.1 unknown basis ----

#[test]
fn position_in_without_basis_is_flagged_never_zero() {
    let mut store = Store::open_in_memory().unwrap();
    let pin = |row: &str, basis: bool| {
        let mut v = json!({"source": src('a', row), "external_id": row, "account": {"hint": "DEG"}, "kind": "position_in",
            "date": "2026-05-02", "instrument": {"isin": "IE00BK5BQT80"}, "quantity": "20"});
        if basis {
            v["cost_basis"] = eur("2000.00");
        }
        v
    };
    let p = import_all(
        &mut store,
        parsed(vec![pin("row 1", false), pin("row 2", true)], vec![]),
    );
    assert!(p.records[0].basis_unknown);
    assert!(!p.records[1].basis_unknown);

    let acct = store.account_for_hint("DEG").unwrap().unwrap();
    let stored = store.activities_until(acct, "2026-12-31").unwrap();
    let no_basis = stored
        .iter()
        .find(|s| s.record.source.locator == "row 1")
        .unwrap();
    assert!(no_basis.basis_unknown);
    assert!(
        no_basis.record.cost_basis.is_none(),
        "never stored as a zero basis"
    );
    let with_basis = stored
        .iter()
        .find(|s| s.record.source.locator == "row 2")
        .unwrap();
    assert!(!with_basis.basis_unknown);
}

// ---- unresolved hints block commit ----

#[test]
fn commit_refuses_unresolved_hints() {
    let mut store = Store::open_in_memory().unwrap();
    let rb = {
        let mut r = MappingResolver {
            store: &mut store,
            policy: CreatePolicy::default(),
        };
        resolve_batch(statement('a', true, "100"), &mut r).unwrap()
    };
    let p = prepare(&store, rb).unwrap();
    let rep = report(&store, &p, &Decisions::new()).unwrap();
    assert_eq!(rep.summary.unresolved, 2, "account and instrument");
    assert!(rep.assertion_results[0].note.is_some());
    assert!(matches!(
        commit(&mut store, &p, &Decisions::new()),
        Err(CommitError::Unresolved(_))
    ));
    assert_eq!(store.count("activities").unwrap(), 0);
}

// ---- undo ----

#[test]
fn reimport_after_undo_is_new_again() {
    let mut store = Store::open_in_memory().unwrap();
    let p = run(&mut store, monthly('a', &[3, 4]));
    let res = commit(&mut store, &p, &Decisions::new()).unwrap();
    let again = run(&mut store, monthly('a', &[3, 4]));
    assert!(again
        .records
        .iter()
        .all(|r| matches!(r.outcome, Outcome::Duplicate { .. })));

    store.undo_batch(res.batch_id).unwrap();
    let after = run(&mut store, monthly('a', &[3, 4]));
    assert!(after.records.iter().all(|r| r.outcome == Outcome::New));
}
