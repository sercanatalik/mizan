use mizan_core::portfolio::{
    contributions, holdings, overview, AttentionItem, CostState, Period, Position,
};
use mizan_core::*;
use mizan_import::{parse_batch, parse_batch_value, Date};
use mizan_storage::Store;
use serde_json::{json, Value};

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

fn src(file: char, row: usize) -> Value {
    json!({"adapter": "t", "adapter_version": "1", "file_sha256": file.to_string().repeat(64), "locator": format!("row {row}")})
}

/// Import records (and assertions) for account U1 and commit them.
fn import(store: &mut Store, file: char, records: Vec<Value>, assertions: Vec<Value>) {
    let records: Vec<Value> = records
        .into_iter()
        .enumerate()
        .map(|(i, mut r)| {
            r["source"] = src(file, i + 1);
            if r.get("account").is_none() {
                r["account"] = json!({"hint": "U1", "name_hint": "Broker"});
            }
            r
        })
        .collect();
    let assertions: Vec<Value> = assertions
        .into_iter()
        .map(|mut a| {
            a["source"] = src(file, 999);
            a
        })
        .collect();
    let parsed = parse_batch_value(
        json!({"schema": "mizan.import/1", "records": records, "assertions": assertions}),
    )
    .unwrap();
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

fn usd(a: &str) -> Value {
    json!({"amount": a, "currency": "USD"})
}

fn eur(a: &str) -> Value {
    json!({"amount": a, "currency": "EUR"})
}

fn rec(kind: &str, date: &str, isin: Option<&str>, qty: Option<&str>, net: Option<Value>) -> Value {
    let mut r = json!({"kind": kind, "date": date});
    if let Some(i) = isin {
        r["instrument"] = json!({"isin": i});
    }
    if let Some(q) = qty {
        r["quantity"] = json!(q);
    }
    if let Some(n) = net {
        r["net_cash"] = n;
    }
    r
}

fn position<'a>(h: &'a mizan_core::portfolio::Holdings, key: &str) -> &'a Position {
    h.accounts
        .iter()
        .flat_map(|a| &a.positions)
        .find(|p| p.instrument_key == key)
        .unwrap_or_else(|| panic!("{key}"))
}

// ---- 2.1 provisional average cost ----

#[test]
fn average_cost_after_partial_sale() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        vec![
            rec(
                "buy",
                "2026-01-05",
                Some("A"),
                Some("40"),
                Some(usd("6001.00")),
            ),
            rec(
                "buy",
                "2026-02-05",
                Some("A"),
                Some("10"),
                Some(usd("1601.00")),
            ),
            rec(
                "sell",
                "2026-03-05",
                Some("A"),
                Some("10"),
                Some(usd("1700.00")),
            ),
        ],
        vec![],
    );
    let h = holdings(&s).unwrap();
    let p = position(&h, "isin:A");
    assert_eq!(p.quantity, "40");
    assert_eq!(p.cost.state, CostState::Known);
    assert_eq!(p.cost.total.as_deref(), Some("6081.60"));
    assert_eq!(p.cost.average.as_deref(), Some("152.04"));
    assert_eq!(p.cost.currency.as_deref(), Some("USD"));
}

#[test]
fn split_keeps_cost() {
    let mut s = Store::open_in_memory().unwrap();
    let mut split = rec("split", "2026-02-01", Some("S"), None, None);
    split["split"] = json!({"new": "4", "old": "1"});
    import(
        &mut s,
        'a',
        vec![
            rec(
                "buy",
                "2026-01-05",
                Some("S"),
                Some("10"),
                Some(eur("1000.00")),
            ),
            split,
        ],
        vec![],
    );
    let h = holdings(&s).unwrap();
    let p = position(&h, "isin:S");
    assert_eq!(
        (
            p.quantity.as_str(),
            p.cost.total.as_deref(),
            p.cost.average.as_deref()
        ),
        ("40", Some("1000.00"), Some("25.00"))
    );
}

#[test]
fn spinoff_moves_cost_share() {
    let mut s = Store::open_in_memory().unwrap();
    let mut spin = rec("spinoff", "2026-04-10", Some("A"), None, None);
    spin["legs"] = json!([{"instrument": {"isin": "NEW"}, "quantity_delta": "10", "cost_basis_share": "0.05"}]);
    import(
        &mut s,
        'a',
        vec![
            rec(
                "buy",
                "2026-01-05",
                Some("A"),
                Some("40"),
                Some(usd("6001.00")),
            ),
            rec(
                "buy",
                "2026-02-05",
                Some("A"),
                Some("10"),
                Some(usd("1601.00")),
            ),
            rec(
                "sell",
                "2026-03-05",
                Some("A"),
                Some("10"),
                Some(usd("1700.00")),
            ),
            spin,
        ],
        vec![],
    );
    let h = holdings(&s).unwrap();
    assert_eq!(
        position(&h, "isin:NEW").cost.total.as_deref(),
        Some("304.08")
    );
    assert_eq!(position(&h, "isin:NEW").quantity, "10");
    assert_eq!(
        position(&h, "isin:A").cost.total.as_deref(),
        Some("5777.52")
    );
}

#[test]
fn transfer_without_basis_is_unknown_not_zero() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        vec![rec(
            "position_in",
            "2026-03-03",
            Some("V"),
            Some("20"),
            None,
        )],
        vec![],
    );
    let h = holdings(&s).unwrap();
    let p = position(&h, "isin:V");
    assert_eq!(p.cost.state, CostState::UnknownBasis);
    assert_eq!(p.cost.unknown_units.as_deref(), Some("20"));
    assert_eq!(
        (p.cost.total.as_ref(), p.cost.average.as_ref()),
        (None, None),
        "never shown as zero"
    );
}

#[test]
fn transfer_with_basis_and_return_of_capital() {
    let mut s = Store::open_in_memory().unwrap();
    let mut pin = rec("position_in", "2026-03-03", Some("V"), Some("20"), None);
    pin["cost_basis"] = eur("2000.00");
    let roc = rec(
        "return_of_capital",
        "2026-04-01",
        Some("V"),
        None,
        Some(eur("100.00")),
    );
    import(&mut s, 'a', vec![pin, roc], vec![]);
    let h = holdings(&s).unwrap();
    assert_eq!(
        position(&h, "isin:V").cost.total.as_deref(),
        Some("1900.00")
    );
    assert_eq!(
        position(&h, "isin:V").cost.average.as_deref(),
        Some("95.00")
    );
}

#[test]
fn mixed_currency_and_short_are_not_available() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        vec![
            rec(
                "buy",
                "2026-01-05",
                Some("M"),
                Some("1"),
                Some(usd("100.00")),
            ),
            rec(
                "buy",
                "2026-01-06",
                Some("M"),
                Some("1"),
                Some(eur("90.00")),
            ),
            rec(
                "short_sell",
                "2026-01-07",
                Some("SH"),
                Some("8"),
                Some(usd("800.00")),
            ),
        ],
        vec![],
    );
    let h = holdings(&s).unwrap();
    let m = position(&h, "isin:M");
    assert_eq!(m.cost.state, CostState::MixedCurrency);
    assert!(m.cost.total.is_none());
    let sh = position(&h, "isin:SH");
    assert_eq!(sh.quantity, "-8");
    assert_eq!(sh.cost.state, CostState::Short);
    assert!(sh.cost.total.is_none());
}

#[test]
fn exchange_moves_cost_to_received_instrument() {
    let mut s = Store::open_in_memory().unwrap();
    let mut ex = rec("exchange", "2026-05-01", Some("OLD"), None, None);
    ex["legs"] = json!([
        {"instrument": {"isin": "OLD"}, "quantity_delta": "-10"},
        {"instrument": {"isin": "NEWX"}, "quantity_delta": "5"}
    ]);
    import(
        &mut s,
        'a',
        vec![
            rec(
                "buy",
                "2026-01-05",
                Some("OLD"),
                Some("10"),
                Some(usd("500.00")),
            ),
            ex,
        ],
        vec![],
    );
    let h = holdings(&s).unwrap();
    assert!(h.accounts[0]
        .positions
        .iter()
        .all(|p| p.instrument_key != "isin:OLD"));
    assert_eq!(
        position(&h, "isin:NEWX").cost.total.as_deref(),
        Some("500.00")
    );
}

// ---- 2.2 holdings ----

#[test]
fn quantities_match_replay_closed_hidden_and_cash_listed() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        vec![
            rec("deposit", "2026-01-02", None, None, Some(usd("10000.00"))),
            rec(
                "buy",
                "2026-01-05",
                Some("A"),
                Some("40"),
                Some(usd("6001.00")),
            ),
            rec(
                "buy",
                "2026-02-05",
                Some("A"),
                Some("10"),
                Some(usd("1601.00")),
            ),
            rec(
                "sell",
                "2026-03-05",
                Some("A"),
                Some("5"),
                Some(usd("800.00")),
            ),
            rec(
                "buy",
                "2026-01-05",
                Some("GONE"),
                Some("3"),
                Some(usd("30.00")),
            ),
            rec(
                "sell",
                "2026-01-06",
                Some("GONE"),
                Some("3"),
                Some(usd("33.00")),
            ),
        ],
        vec![],
    );
    let h = holdings(&s).unwrap();
    let acct = &h.accounts[0];
    assert_eq!(acct.name, "Broker");
    assert_eq!(position(&h, "isin:A").quantity, "45");
    assert!(
        acct.positions
            .iter()
            .all(|p| p.instrument_key != "isin:GONE"),
        "closed position hidden"
    );
    let stored = s.all_activities(acct.account_id).unwrap();
    let replayed = replay(mizan_core::portfolio::ordered(&stored));
    assert_eq!(replayed.position("isin:A").normalize().to_string(), "45");
    assert_eq!(acct.cash.len(), 1);
    assert_eq!(
        (acct.cash[0].currency.as_str(), acct.cash[0].amount.as_str()),
        ("USD", "3201.00")
    );
}

fn e2e(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/data/e2e/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn import_file(store: &mut Store, json: &str) {
    let parsed = parse_batch(json).unwrap();
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

#[test]
fn failed_balance_check_flags_position_and_overview() {
    let mut s = Store::open_in_memory().unwrap();
    import_file(&mut s, &e2e("seed.json"));
    // Commit without confirming the ASML buy: the statement says 3, Mizan computes 1.
    import_file(&mut s, &e2e("batch.json"));

    let h = holdings(&s).unwrap();
    let asml = position(&h, "isin:NL0010273215");
    let flag = asml.check_flag.as_ref().expect("flagged");
    assert_eq!(
        (
            flag.statement_quantity.as_str(),
            flag.computed_quantity.as_str()
        ),
        ("3", "1")
    );
    assert_eq!(flag.as_of, "2026-09-30");
    assert_eq!(asml.name.as_deref(), Some("ASML Holding"));

    let o = overview(&s).unwrap();
    let acct = &o.accounts[0];
    let check = acct.latest_check.as_ref().unwrap();
    assert!(!check.passed);
    let displays: Vec<&str> = check.failed.iter().map(|f| f.display.as_str()).collect();
    assert_eq!(displays, vec!["cash", "ASML Holding"]);
    assert!(o
        .attention
        .iter()
        .any(|a| matches!(a, AttentionItem::FailedChecks { failed, .. } if failed.len() == 2)));
    assert!(o.attention.iter().any(|a| matches!(a,
        AttentionItem::UnknownBasis { units, display, account_name, .. }
            if units == "20" && display == "Vanguard FTSE All-World" && account_name == "Degiro")));
}

// ---- 2.3 overview ----

#[test]
fn overview_passing_no_check_and_in_order() {
    let mut s = Store::open_in_memory().unwrap();
    assert!(overview(&s).unwrap().accounts.is_empty());

    import(
        &mut s,
        'a',
        vec![rec(
            "deposit",
            "2026-10-01",
            None,
            None,
            Some(usd("100.00")),
        )],
        vec![json!({"account": {"hint": "U1"}, "as_of": "2026-10-31", "cash": [usd("100.00")]})],
    );
    import(
        &mut s,
        'b',
        vec![{
            let mut r = rec("deposit", "2026-10-01", None, None, Some(eur("5.00")));
            r["account"] = json!({"hint": "SAV", "name_hint": "Savings"});
            r
        }],
        vec![],
    );
    let o = overview(&s).unwrap();
    let broker = o.accounts.iter().find(|a| a.name == "Broker").unwrap();
    let check = broker.latest_check.as_ref().unwrap();
    assert!(check.passed);
    assert_eq!(check.as_of, "2026-10-31");
    assert!(check.failed.is_empty());
    let savings = o.accounts.iter().find(|a| a.name == "Savings").unwrap();
    assert!(savings.latest_check.is_none(), "no balance check yet");
    assert_eq!(savings.activity_count, 1);
    assert!(o.attention.is_empty(), "everything is in order");
}

// ---- 2.4 contributions ----

#[test]
fn net_contributions_by_period() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        vec![
            rec("deposit", "2025-03-01", None, None, Some(usd("2000.00"))),
            rec("deposit", "2026-01-02", None, None, Some(usd("10000.00"))),
            rec("withdrawal", "2026-06-01", None, None, Some(usd("1500.00"))),
            {
                let mut t = rec(
                    "internal_transfer",
                    "2026-06-02",
                    None,
                    None,
                    Some(usd("-300.00")),
                );
                t["memo"] = json!("to savings");
                t
            },
            rec("deposit", "2026-12-01", None, None, Some(usd("999.00"))),
        ],
        vec![],
    );
    let c = contributions(&s, Date::parse("2026-11-01").unwrap()).unwrap();
    let get = |p: Period| c.periods.iter().find(|x| x.period == p).unwrap();

    let ytd = get(Period::Ytd);
    assert_eq!(ytd.accounts[0].by_currency[0].amount, "+8500.00");
    assert_eq!(ytd.accounts[0].from.as_deref(), Some("2026-01-01"));
    assert_eq!(ytd.total[0].amount, "+8500.00");
    assert_eq!(ytd.to, "2026-11-01");

    let all = get(Period::SinceStart);
    assert_eq!(all.accounts[0].by_currency[0].amount, "+10500.00");
    assert_eq!(all.accounts[0].from.as_deref(), Some("2025-03-01"));

    let m12 = get(Period::Last12Months);
    assert_eq!(m12.accounts[0].from.as_deref(), Some("2025-11-02"));
    assert_eq!(m12.accounts[0].by_currency[0].amount, "+8500.00");
}
