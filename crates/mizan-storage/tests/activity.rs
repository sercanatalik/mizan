//! Activity queries, totals and manual-entry batches (activity-view and
//! manual-activities specs).

use mizan_import::{ImportRecord, InstrumentHint};
use mizan_storage::*;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::str::FromStr;

fn sha(c: char) -> String {
    c.to_string().repeat(64)
}

fn rec(adapter: &str, file: char, v: Value) -> ImportRecord {
    let mut v = v;
    v["source"] = json!({"adapter": adapter, "adapter_version": "1", "file_sha256": sha(file), "locator": "row 1"});
    v["account"] = json!({"hint": "U1"});
    serde_json::from_value(v).unwrap()
}

fn batch(
    adapter: &str,
    file: char,
    account: AccountId,
    items: Vec<(Option<AssetId>, ImportRecord)>,
) -> NewBatch {
    NewBatch {
        adapter: adapter.into(),
        adapter_version: "1".into(),
        file_sha256: sha(file),
        activities: items
            .into_iter()
            .enumerate()
            .map(|(i, (asset_id, record))| NewActivity {
                account_id: account,
                asset_id,
                fingerprint: format!("fp-{file}-{i}"),
                source_order: i as i64,
                basis_unknown: false,
                record,
            })
            .collect(),
        assertions: vec![],
    }
}

fn hint(symbol: &str, name: &str, isin: &str) -> InstrumentHint {
    InstrumentHint {
        symbol: Some(symbol.into()),
        name: Some(name.into()),
        isin: Some(isin.into()),
        ..Default::default()
    }
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// Two accounts; an imported batch with a deposit, a buy of AAPL and a dividend;
/// a manual batch with a buy of ASML.
fn seeded() -> (
    Store,
    AccountId,
    AccountId,
    AssetId,
    AssetId,
    BatchId,
    BatchId,
) {
    let mut s = Store::open_in_memory().unwrap();
    let ibkr = s.create_account("IBKR", "U1").unwrap();
    let degiro = s.create_account("Degiro", "D1").unwrap();
    let aapl = s
        .create_asset(
            "isin:US0378331005",
            &hint("AAPL", "Apple Inc.", "US0378331005"),
        )
        .unwrap();
    let asml = s
        .create_asset(
            "isin:NL0010273215",
            &hint("ASML", "ASML Holding", "NL0010273215"),
        )
        .unwrap();
    let imported = s
        .commit_batch(&batch(
            "ibkr-flex",
            'a',
            ibkr,
            vec![
                (None, rec("ibkr-flex", 'a', json!({"kind": "deposit", "date": "2025-01-02", "net_cash": {"amount": "1000.00", "currency": "USD"}}))),
                (Some(aapl), rec("ibkr-flex", 'a', json!({"kind": "buy", "date": "2025-01-03", "instrument": {"isin": "US0378331005"}, "quantity": "4", "net_cash": {"amount": "600.00", "currency": "USD"}}))),
                (Some(aapl), rec("ibkr-flex", 'a', json!({"kind": "dividend", "date": "2025-05-15", "instrument": {"isin": "US0378331005"}, "net_cash": {"amount": "8.50", "currency": "USD"}}))),
            ],
        ))
        .unwrap();
    let manual = s
        .commit_batch(&batch(
            MANUAL_ADAPTER,
            'c',
            degiro,
            vec![(Some(asml), rec(MANUAL_ADAPTER, 'c', json!({"kind": "buy", "date": "2024-03-04", "instrument": {"isin": "NL0010273215"}, "quantity": "2", "net_cash": {"amount": "1300.00", "currency": "EUR"}})))],
        ))
        .unwrap();
    (s, ibkr, degiro, aapl, asml, imported, manual)
}

fn kinds(p: &StoredPage) -> Vec<String> {
    p.rows
        .iter()
        .map(|r| r.record.kind.as_str().to_string())
        .collect()
}

#[test]
fn newest_first_with_total() {
    let (s, ..) = seeded();
    let p = s
        .query_activities(&ActivityFilter::default(), 0, 200)
        .unwrap();
    assert_eq!(p.total, 4);
    let dates: Vec<String> = p.rows.iter().map(|r| r.record.date.to_string()).collect();
    assert_eq!(
        dates,
        ["2025-05-15", "2025-01-03", "2025-01-02", "2024-03-04"]
    );
}

#[test]
fn each_filter_and_combinations() {
    let (s, ibkr, degiro, ..) = seeded();
    let f = |f: ActivityFilter| s.query_activities(&f, 0, 200).unwrap();
    assert_eq!(
        f(ActivityFilter {
            accounts: vec![degiro],
            ..Default::default()
        })
        .total,
        1
    );
    assert_eq!(
        kinds(&f(ActivityFilter {
            kinds: vec!["dividend".into()],
            ..Default::default()
        })),
        ["dividend"]
    );
    assert_eq!(
        f(ActivityFilter {
            accounts: vec![ibkr],
            kinds: vec!["dividend".into(), "buy".into()],
            ..Default::default()
        })
        .total,
        2
    );
    assert_eq!(
        f(ActivityFilter {
            text: Some("us0378".into()),
            ..Default::default()
        })
        .total,
        2
    );
    assert_eq!(
        f(ActivityFilter {
            text: Some("asml hold".into()),
            ..Default::default()
        })
        .total,
        1
    );
    assert_eq!(
        f(ActivityFilter {
            text: Some("nothing".into()),
            ..Default::default()
        })
        .total,
        0
    );
    assert_eq!(
        f(ActivityFilter {
            from: Some("2025-01-03".into()),
            to: Some("2025-05-14".into()),
            ..Default::default()
        })
        .total,
        1
    );
    assert_eq!(
        f(ActivityFilter {
            source: Some(SourceFilter::Manual),
            ..Default::default()
        })
        .total,
        1
    );
    assert_eq!(
        f(ActivityFilter {
            source: Some(SourceFilter::Imported),
            ..Default::default()
        })
        .total,
        3
    );
}

#[test]
fn text_matches_through_merges() {
    let (mut s, _, _, aapl, asml, ..) = seeded();
    // ASML merged into AAPL: searching AAPL's ISIN also finds the ASML buy.
    s.merge_assets(asml, aapl).unwrap();
    let p = s
        .query_activities(
            &ActivityFilter {
                text: Some("US0378".into()),
                ..Default::default()
            },
            0,
            200,
        )
        .unwrap();
    assert_eq!(p.total, 3);
}

#[test]
fn paging_over_450_rows() {
    let mut s = Store::open_in_memory().unwrap();
    let a = s.create_account("A", "A1").unwrap();
    let items: Vec<(Option<AssetId>, ImportRecord)> = (0..450)
        .map(|i| {
            let date = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap() + chrono::Duration::days(i);
            (None, rec("t", 'e', json!({"kind": "deposit", "date": date.to_string(), "net_cash": {"amount": "1.00", "currency": "TRY"}})))
        })
        .collect();
    s.commit_batch(&batch("t", 'e', a, items)).unwrap();
    let all = ActivityFilter::default();
    let p1 = s.query_activities(&all, 0, 200).unwrap();
    let p3 = s.query_activities(&all, 400, 200).unwrap();
    assert_eq!((p1.total, p1.rows.len(), p3.rows.len()), (450, 200, 50));
    assert!(p1.rows[0].record.date > p1.rows[199].record.date);
    assert_eq!(
        p3.rows.last().unwrap().record.date.to_string(),
        "2020-01-01"
    );
}

#[test]
fn totals_per_currency() {
    let (s, ibkr, ..) = seeded();
    let (count, t) = s
        .activity_totals(&ActivityFilter {
            accounts: vec![ibkr],
            kinds: vec!["deposit".into(), "buy".into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(count, 2);
    let usd = &t["USD"];
    assert_eq!(
        (usd.cash_in, usd.cash_out, usd.net()),
        (d("1000.00"), d("600.00"), d("400.00"))
    );
    let (count, t) = s.activity_totals(&ActivityFilter::default()).unwrap();
    assert_eq!(count, 4);
    assert_eq!(t["EUR"].cash_out, d("1300.00"));
    assert_eq!(t["USD"].cash_in, d("1008.50"));
}

#[test]
fn replace_and_delete_only_manual_batches() {
    let (mut s, _, degiro, _, asml, imported, manual) = seeded();
    let replacement = batch(
        MANUAL_ADAPTER,
        'd',
        degiro,
        vec![(
            Some(asml),
            rec(
                MANUAL_ADAPTER,
                'd',
                json!({"kind": "buy", "date": "2024-03-04", "instrument": {"isin": "NL0010273215"}, "quantity": "3", "net_cash": {"amount": "1950.00", "currency": "EUR"}}),
            ),
        )],
    );
    // An imported batch is refused and stays.
    assert!(matches!(
        s.replace_manual_batch(imported, &replacement),
        Err(StorageError::Invalid(_))
    ));
    assert!(matches!(
        s.delete_manual_batch(imported),
        Err(StorageError::Invalid(_))
    ));
    assert_eq!(
        s.query_activities(&ActivityFilter::default(), 0, 10)
            .unwrap()
            .total,
        4
    );

    let new_id = s.replace_manual_batch(manual, &replacement).unwrap();
    assert_eq!(s.list_batches().unwrap().len(), 1, "still one import");
    let m = s
        .query_activities(
            &ActivityFilter {
                source: Some(SourceFilter::Manual),
                ..Default::default()
            },
            0,
            10,
        )
        .unwrap();
    assert_eq!(m.total, 1);
    assert_eq!(m.rows[0].batch_id, new_id);
    assert_eq!(m.rows[0].record.quantity.unwrap().value(), d("3"));

    s.delete_manual_batch(new_id).unwrap();
    assert_eq!(
        s.query_activities(&ActivityFilter::default(), 0, 10)
            .unwrap()
            .total,
        3
    );
    assert_eq!(s.list_assets().unwrap().len(), 2, "assets are kept");
    assert_eq!(s.list_accounts().unwrap().len(), 2, "accounts are kept");
}

#[test]
fn failed_replacement_keeps_the_original() {
    let (mut s, _, degiro, _, asml, _, manual) = seeded();
    let mut bad = batch(
        MANUAL_ADAPTER,
        'd',
        degiro,
        vec![(
            Some(asml),
            rec(
                MANUAL_ADAPTER,
                'd',
                json!({"kind": "buy", "date": "2024-03-04", "quantity": "3", "net_cash": {"amount": "1.00", "currency": "EUR"}}),
            ),
        )],
    );
    bad.activities[0].account_id = 999; // violates the account foreign key
    let before = s
        .query_activities(&ActivityFilter::default(), 0, 10)
        .unwrap();
    assert!(s.replace_manual_batch(manual, &bad).is_err());
    let after = s
        .query_activities(&ActivityFilter::default(), 0, 10)
        .unwrap();
    assert_eq!(before, after);
    assert!(s.batch(manual).unwrap().is_some());
}

#[test]
fn import_history_leaves_out_manual_entries() {
    let (s, _, _, _, _, imported, manual) = seeded();
    let ids: Vec<BatchId> = s
        .list_batches()
        .unwrap()
        .into_iter()
        .map(|b| b.id)
        .collect();
    assert_eq!(ids, [imported]);
    assert_eq!(s.batch(manual).unwrap().unwrap().adapter, MANUAL_ADAPTER);
}

/// 50,000 activities: a filtered page and the totals each take under 200 ms.
/// Timing is only meaningful in a release build:
/// `cargo test -p mizan-storage --release --test activity large`
#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs a release build")]
fn large_history_is_fast() {
    let mut s = Store::open_in_memory().unwrap();
    let a = s.create_account("A", "A1").unwrap();
    let aapl = s
        .create_asset(
            "isin:US0378331005",
            &hint("AAPL", "Apple Inc.", "US0378331005"),
        )
        .unwrap();
    let start = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
    let items: Vec<(Option<AssetId>, ImportRecord)> = (0..50_000)
        .map(|i| {
            let date = start + chrono::Duration::days(i / 5);
            let kind = ["buy", "sell", "dividend", "deposit", "fee"][(i % 5) as usize];
            let asset = (kind != "deposit" && kind != "fee").then_some(aapl);
            let mut v = json!({"kind": kind, "date": date.to_string(), "net_cash": {"amount": "10.00", "currency": "USD"}});
            if asset.is_some() {
                v["instrument"] = json!({"isin": "US0378331005"});
                if kind != "dividend" {
                    v["quantity"] = json!("1");
                }
            }
            (asset, rec("t", 'f', v))
        })
        .collect();
    s.commit_batch(&batch("t", 'f', a, items)).unwrap();
    let f = ActivityFilter {
        kinds: vec!["dividend".into()],
        text: Some("aapl".into()),
        ..Default::default()
    };
    let t = std::time::Instant::now();
    let page = s.query_activities(&f, 0, 200).unwrap();
    let q = t.elapsed();
    let t = std::time::Instant::now();
    let (count, _) = s.activity_totals(&ActivityFilter::default()).unwrap();
    let tot = t.elapsed();
    assert_eq!((page.total, page.rows.len(), count), (10_000, 200, 50_000));
    assert!(q.as_millis() < 200, "query took {q:?}");
    assert!(tot.as_millis() < 200, "totals took {tot:?}");
}

fn imported_buy(s: &Store) -> StoredActivity {
    s.query_activities(
        &ActivityFilter {
            kinds: vec!["buy".into()],
            source: Some(SourceFilter::Imported),
            ..Default::default()
        },
        0,
        1,
    )
    .unwrap()
    .rows
    .remove(0)
}

fn with_quantity(r: &ImportRecord, q: &str) -> ImportRecord {
    let mut r = r.clone();
    r.quantity = Some(mizan_import::Dec(d(q)));
    r
}

fn amend(s: &mut Store, a: &StoredActivity, q: &str, at: &str) {
    s.amend_activity(
        &AmendedActivity {
            id: a.id,
            account_id: a.account_id,
            asset_id: a.asset_id,
            basis_unknown: false,
            record: with_quantity(&a.record, q),
        },
        at,
    )
    .unwrap();
}

#[test]
fn amending_keeps_the_imported_record_and_fingerprint() {
    let (mut s, ..) = seeded();
    let buy = imported_buy(&s);
    amend(&mut s, &buy, "5", "2026-11-02T10:00:00Z");
    let amended = imported_buy(&s);
    amend(&mut s, &amended, "6", "2026-11-03T10:00:00Z");
    let now = imported_buy(&s);
    assert_eq!(now.record.quantity.unwrap().value(), d("6"));
    assert_eq!(now.fingerprint, buy.fingerprint, "fingerprint unchanged");
    assert_eq!(now.record.source, buy.record.source, "provenance unchanged");
    let (original, at) = s.amendment(buy.id).unwrap().unwrap();
    assert_eq!(
        original.quantity.unwrap().value(),
        d("4"),
        "the first imported values are kept"
    );
    assert_eq!(at, "2026-11-03T10:00:00Z");
    assert!(s.amended_ids().unwrap().contains(&buy.id));

    s.revert_activity(buy.id, buy.account_id, buy.asset_id)
        .unwrap();
    assert_eq!(imported_buy(&s).record, buy.record);
    assert!(s.amendment(buy.id).unwrap().is_none());
    assert!(s.amended_ids().unwrap().is_empty());
}

#[test]
fn undo_removes_amended_activities_and_their_imported_values() {
    let (mut s, _, _, _, _, imported, _) = seeded();
    let buy = imported_buy(&s);
    amend(&mut s, &buy, "5", "t");
    s.undo_batch(imported).unwrap();
    assert!(s.amended_ids().unwrap().is_empty());
    assert_eq!(
        s.query_activities(&ActivityFilter::default(), 0, 10)
            .unwrap()
            .total,
        1
    );
}

#[test]
fn manual_entries_are_not_amended_in_place() {
    let (mut s, ..) = seeded();
    let manual = s
        .query_activities(
            &ActivityFilter {
                source: Some(SourceFilter::Manual),
                ..Default::default()
            },
            0,
            1,
        )
        .unwrap()
        .rows
        .remove(0);
    let r = s.amend_activity(
        &AmendedActivity {
            id: manual.id,
            account_id: manual.account_id,
            asset_id: manual.asset_id,
            basis_unknown: false,
            record: manual.record.clone(),
        },
        "t",
    );
    assert!(matches!(r, Err(StorageError::Invalid(_))));
}
