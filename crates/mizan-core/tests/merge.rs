use mizan_core::portfolio::holdings;
use mizan_core::*;
use mizan_import::parse_batch_value;
use mizan_storage::Store;
use serde_json::{json, Value};

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

fn src(file: char, row: usize) -> Value {
    json!({"adapter": "t", "adapter_version": "1", "file_sha256": file.to_string().repeat(64), "locator": format!("row {row}")})
}

fn buy(file: char, row: usize, ext: &str, instrument: Value, qty: &str) -> Value {
    json!({"source": src(file, row), "external_id": ext, "account": {"hint": "DEG"}, "kind": "buy", "date": "2026-05-0".to_string() + &row.to_string(),
           "instrument": instrument, "quantity": qty, "net_cash": {"amount": "100.00", "currency": "EUR"}})
}

fn import(store: &mut Store, records: Vec<Value>, assertions: Vec<Value>) -> Prepared {
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
    p
}

fn positions(store: &Store) -> Vec<(String, String)> {
    holdings(store, today()).unwrap().accounts[0]
        .positions
        .iter()
        .map(|p| (p.instrument_key.clone(), p.quantity.clone()))
        .collect()
}

#[test]
fn merged_assets_combine_everywhere() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        vec![
            buy('a', 1, "B1", json!({"isin": "IE00BK5BQT80"}), "10"),
            buy('a', 2, "B2", json!({"symbol": "VWCE", "mic": "XETR"}), "5"),
        ],
        vec![],
    );
    assert_eq!(positions(&s).len(), 2, "two assets before the merge");

    let isin = s.asset_for_key("isin:IE00BK5BQT80").unwrap().unwrap();
    let sym = s.asset_for_key("sym:VWCE@XETR").unwrap().unwrap();
    let m = s.merge_assets(sym, isin).unwrap();

    // Holdings: one combined position, cost combined too.
    let h = holdings(&s, today()).unwrap();
    let ps = &h.accounts[0].positions;
    assert_eq!(ps.len(), 1);
    assert_eq!(
        (ps[0].instrument_key.as_str(), ps[0].quantity.as_str()),
        ("isin:IE00BK5BQT80", "15")
    );
    assert_eq!(ps[0].cost.total.as_deref(), Some("200.00"));

    // Balance checks: a statement asserting 17 under the symbol key counts the combined position.
    let p = import(
        &mut s,
        vec![buy(
            'b',
            3,
            "B3",
            json!({"symbol": "VWCE", "mic": "XETR"}),
            "2",
        )],
        vec![
            json!({"source": src('b', 9), "account": {"hint": "DEG"}, "as_of": "2026-05-31",
                    "positions": [{"instrument": {"symbol": "VWCE", "mic": "XETR"}, "quantity": "17"}]}),
        ],
    );
    // Import resolution: the symbol key resolves to the ISIN asset, no new asset is created.
    assert_eq!(p.records[0].resolved.asset_id, Some(isin));
    assert_eq!(s.count("assets").unwrap(), 2);
    let checks = report(&s, &p, &Decisions::new());
    assert!(checks.is_ok());
    assert_eq!(
        positions(&s),
        vec![("isin:IE00BK5BQT80".to_string(), "17".to_string())]
    );
    let latest = s
        .latest_assertion_results(s.account_for_hint("DEG").unwrap().unwrap())
        .unwrap();
    assert!(latest[0].passed, "{:?}", latest[0].detail);

    // Undo: both positions come back.
    s.undo_merge(m).unwrap();
    let mut back = positions(&s);
    back.sort();
    assert_eq!(
        back,
        vec![
            ("isin:IE00BK5BQT80".to_string(), "10".to_string()),
            ("sym:VWCE@XETR".to_string(), "7".to_string())
        ]
    );
}

#[test]
fn fingerprints_stay_raw_after_merge() {
    let mut s = Store::open_in_memory().unwrap();
    let records = vec![buy(
        'a',
        1,
        "",
        json!({"symbol": "VWCE", "mic": "XETR"}),
        "5",
    )];
    let mut records_no_ext = records.clone();
    records_no_ext[0]
        .as_object_mut()
        .unwrap()
        .remove("external_id");
    import(&mut s, records_no_ext.clone(), vec![]);
    let isin = s
        .create_asset("isin:IE00BK5BQT80", &Default::default())
        .unwrap();
    let sym = s.asset_for_key("sym:VWCE@XETR").unwrap().unwrap();
    s.merge_assets(sym, isin).unwrap();
    // Re-importing the same file is still recognized as a duplicate.
    let parsed =
        parse_batch_value(json!({"schema": "mizan.import/1", "records": records_no_ext})).unwrap();
    let rb = {
        let mut r = MappingResolver {
            store: &mut s,
            policy: CREATE,
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    let p = prepare(&s, rb).unwrap();
    assert!(matches!(p.records[0].outcome, Outcome::Duplicate { .. }));
}

fn today() -> mizan_import::Date {
    mizan_import::Date::parse("2026-11-01").unwrap()
}
