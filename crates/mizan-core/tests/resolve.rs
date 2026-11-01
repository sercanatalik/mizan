use mizan_core::*;
use mizan_import::{parse_batch_value, InstrumentHint};
use mizan_storage::Store;
use serde_json::{json, Value};

fn hint(v: Value) -> InstrumentHint {
    serde_json::from_value(v).unwrap()
}

#[test]
fn instrument_key_priority_table() {
    let cases = [
        (
            json!({"isin": " us0378331005 ", "figi": "BBG000B9XRY4", "symbol": "AAPL"}),
            Some("isin:US0378331005"),
        ),
        (
            json!({"figi": "bbg000b9xry4", "cusip": "037833100", "symbol": "AAPL"}),
            Some("figi:BBG000B9XRY4"),
        ),
        (
            json!({"cusip": "037833100", "symbol": "AAPL", "mic": "XNAS"}),
            Some("cusip:037833100"),
        ),
        (
            json!({"symbol": "aapl", "mic": "xnas", "name": "Apple"}),
            Some("sym:AAPL@XNAS"),
        ),
        (json!({"symbol": "AAPL"}), Some("sym:AAPL@")),
        (json!({"name": "  APPLE   INC. "}), Some("name:apple inc")),
        (
            json!({"name": "Xtr. MSCI-World Swap"}),
            Some("name:xtr msci world swap"),
        ),
        (json!({"currency": "USD"}), None),
        (
            json!({"isin": "US0378331005", "option": {"underlying": "aapl", "expiry": "2026-12-18", "strike": "200.00", "right": "call"}}),
            Some("opt:AAPL:2026-12-18:C:200"),
        ),
    ];
    for (h, expected) in cases {
        assert_eq!(instrument_key(&hint(h.clone())).as_deref(), expected, "{h}");
    }
}

fn records() -> mizan_import::ParsedBatch {
    let src = |l: &str| json!({"adapter": "t", "adapter_version": "1", "file_sha256": "0".repeat(64), "locator": l});
    parse_batch_value(json!({"schema": "mizan.import/1", "records": [
        {"source": src("row 1"), "account": {"hint": "U1", "name_hint": "IBKR"}, "kind": "buy", "date": "2026-03-02",
         "instrument": {"isin": "US0378331005"}, "quantity": "10"},
        {"source": src("row 2"), "account": {"hint": "U1"}, "kind": "dividend", "date": "2026-03-14",
         "instrument": {"isin": "US0378331005"}, "net_cash": {"amount": "20.40", "currency": "USD"}},
        {"source": src("row 3"), "account": {"hint": "U1"}, "kind": "deposit", "date": "2026-03-01",
         "net_cash": {"amount": "2000.00", "currency": "USD"}}
    ]}))
    .unwrap()
}

#[test]
fn hints_resolve_to_internal_ids() {
    let mut store = Store::open_in_memory().unwrap();
    let acct = store.create_account("Interactive Brokers", "U1").unwrap();
    let asset = store
        .create_asset("isin:US0378331005", &hint(json!({"isin": "US0378331005"})))
        .unwrap();

    let batch = records();
    let mut r = MappingResolver {
        store: &mut store,
        policy: CreatePolicy::default(),
    };
    let (resolved, items) = resolve_records(&batch.records, &mut r).unwrap();
    assert!(items.is_empty(), "{items:?}");
    assert_eq!(resolved.len(), 3);
    assert!(resolved
        .iter()
        .all(|x| x.account_id == Some(acct) && x.is_resolved()));
    assert_eq!(resolved[0].asset_id, Some(asset));
    assert_eq!(resolved[2].asset_id, None, "a deposit names no instrument");
}

#[test]
fn unresolved_hints_become_review_items_not_errors() {
    let mut store = Store::open_in_memory().unwrap();
    let batch = records();
    let mut r = MappingResolver {
        store: &mut store,
        policy: CreatePolicy::default(),
    };
    let (resolved, items) = resolve_records(&batch.records, &mut r).unwrap();
    assert_eq!(resolved.len(), 3, "records are kept");
    assert!(resolved.iter().all(|x| !x.is_resolved()));
    // One item per distinct hint, not per record.
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(matches!(&items[0], ReviewItem::UnresolvedAccount { hint } if hint.hint == "U1"));
    assert!(
        matches!(&items[1], ReviewItem::UnresolvedInstrument { key: Some(k), .. } if k == "isin:US0378331005")
    );
}

#[test]
fn create_policy_creates_missing_accounts_and_assets() {
    let mut store = Store::open_in_memory().unwrap();
    let batch = records();
    {
        let mut r = MappingResolver {
            store: &mut store,
            policy: CreatePolicy {
                create_accounts: true,
                create_assets: true,
            },
        };
        let (resolved, items) = resolve_records(&batch.records, &mut r).unwrap();
        assert!(items.is_empty());
        assert!(resolved.iter().all(|x| x.is_resolved()));
        assert_eq!(
            resolved[0].asset_id, resolved[1].asset_id,
            "same ISIN, same asset"
        );
    }
    assert_eq!(store.count("accounts").unwrap(), 1);
    assert_eq!(store.count("assets").unwrap(), 1);
}
