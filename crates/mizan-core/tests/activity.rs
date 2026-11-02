//! Activity screen and manual entries (activity-view and manual-activities specs).

use mizan_core::activity::*;
use mizan_core::portfolio::holdings;
use mizan_core::*;
use mizan_import::{parse_batch_value, Date};
use mizan_storage::Store;
use serde_json::{json, Value};

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

/// Commit records; each gets account `hint` unless it has one.
fn import(store: &mut Store, file: char, hint: &str, records: Vec<Value>) {
    let records: Vec<Value> = records
        .into_iter()
        .enumerate()
        .map(|(i, mut r)| {
            r["source"] = json!({"adapter": "t", "adapter_version": "1", "file_sha256": file.to_string().repeat(64), "locator": format!("row {}", i + 1)});
            if r.get("account").is_none() {
                r["account"] = json!({"hint": hint, "name_hint": hint});
            }
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

fn ctx() -> ManualContext {
    ManualContext {
        app_version: "0.1.0".into(),
        now: "2026-11-02T10:00:00Z".into(),
    }
}

/// An imported account "IBKR" with a buy of 10 AAPL (row 12 of its file) and a dividend.
fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    let aapl =
        json!({"isin": "US0378331005", "symbol": "AAPL", "name": "Apple Inc.", "currency": "USD"});
    import(
        &mut s,
        'a',
        "U1",
        vec![
            json!({"kind": "deposit", "date": "2025-01-02", "net_cash": money("2000.00", "USD")}),
            json!({"kind": "buy", "date": "2025-01-03", "instrument": aapl, "quantity": "10", "price": money("150.00", "USD"), "charges": [{"type": "commission", "amount": money("1.00", "USD")}], "net_cash": money("1501.00", "USD")}),
            json!({"kind": "dividend", "date": "2025-05-15", "instrument": aapl, "gross": money("10.00", "USD"), "charges": [{"type": "withholding_tax", "amount": money("1.50", "USD")}], "net_cash": money("8.50", "USD")}),
        ],
    );
    s
}

fn first(s: &Store, kind: &str) -> ActivityRow {
    page(
        s,
        &ActivityQuery {
            kinds: vec![kind.into()],
            ..Default::default()
        },
        0,
    )
    .unwrap()
    .rows
    .remove(0)
}

fn ibkr(s: &Store) -> i64 {
    s.list_accounts().unwrap()[0].id
}

fn aapl(s: &Store) -> i64 {
    s.list_assets().unwrap()[0].id
}

fn form(kind: &str, s: &Store) -> ActivityForm {
    ActivityForm {
        kind: kind.into(),
        date: "2025-06-02".into(),
        account_id: Some(ibkr(s)),
        currency: "USD".into(),
        ..Default::default()
    }
}

fn some(v: &str) -> Option<String> {
    Some(v.into())
}

#[test]
fn imported_buy_row() {
    let s = store();
    let r = first(&s, "buy");
    assert_eq!(r.date, "2025-01-03");
    assert_eq!(r.account, "U1");
    assert_eq!(r.instrument.as_deref(), Some("AAPL · Apple Inc."));
    assert_eq!(r.quantity.as_deref(), Some("10"));
    assert_eq!(
        r.price
            .as_ref()
            .map(|p| (p.amount.as_str(), p.currency.as_str())),
        Some(("150", "USD"))
    );
    assert_eq!(r.charges[0].amount, "1");
    assert_eq!(
        (
            r.source.adapter.as_str(),
            r.source.locator.as_str(),
            r.source.manual
        ),
        ("t", "row 2", false)
    );
    assert!(r.editable && !r.deletable);
}

#[test]
fn page_is_newest_first_with_totals() {
    let s = store();
    let p = page(&s, &ActivityQuery::default(), 0).unwrap();
    assert_eq!(p.total, 3);
    let kinds: Vec<&str> = p.rows.iter().map(|r| r.kind.as_str()).collect();
    assert_eq!(kinds, ["dividend", "buy", "deposit"]);
    let usd = &p.totals[0];
    assert_eq!(
        (
            usd.cash_in.as_str(),
            usd.cash_out.as_str(),
            usd.net.as_str()
        ),
        ("2008.50", "1501.00", "507.50")
    );
    assert!(p.has_any);
    let none = page(
        &s,
        &ActivityQuery {
            text: some("zzz"),
            ..Default::default()
        },
        0,
    )
    .unwrap();
    assert_eq!((none.total, none.has_any), (0, true));
}

#[test]
fn imported_dividend_detail() {
    let s = store();
    let r = first(&s, "dividend");
    let dt = detail(&s, r.id).unwrap();
    assert_eq!(dt.gross.as_ref().unwrap().amount, "10");
    assert_eq!(dt.charges[0].kind, "withholding_tax");
    assert_eq!(dt.charges[0].amount, "1.5");
    assert_eq!(dt.file_sha256, "a".repeat(64));
    let link = dt.import.unwrap();
    assert_eq!(link.batch_id, r.source.batch_id);
    assert_eq!(dt.form.kind, "dividend", "imports are editable too");
    assert!(dt.row.editable && !dt.row.deletable);
    assert!(!dt.row.source.amended && dt.amended_at.is_none());
}

#[test]
fn manual_buy_appears_and_counts_in_holdings() {
    let mut s = store();
    let mut f = form("buy", &s);
    f.new_asset = Some(NewAssetForm {
        symbol: some("VWCE"),
        name: some("Vanguard FTSE All-World"),
        isin: some("IE00BK5BQT80"),
        currency: some("EUR"),
        class: some("etf"),
    });
    f.currency = "EUR".into();
    f.date = "2024-03-04".into();
    f.quantity = some("5");
    f.price = some("110.20");
    f.commission = some("2.00");
    let id = save_form(&mut s, &f, &ctx(), false, None).unwrap();
    let dt = detail(&s, id).unwrap();
    assert_eq!(dt.row.source.locator, "manual entry");
    assert_eq!(dt.row.source.adapter, "manual");
    assert_eq!(
        dt.row.source.added_at.as_deref(),
        Some("2026-11-02T10:00:00Z")
    );
    assert_eq!(dt.row.net_cash.as_ref().unwrap().amount, "553");
    assert!(dt.row.editable && dt.row.deletable);
    let h = holdings(&s, Date::parse("2026-11-02").unwrap()).unwrap();
    let vwce = h.accounts[0]
        .positions
        .iter()
        .find(|p| p.symbol.as_deref() == Some("VWCE"))
        .unwrap();
    assert_eq!(vwce.quantity, "5");
    // Manual entries are not imports.
    assert_eq!(s.list_batches().unwrap().len(), 1);
}

#[test]
fn every_manual_kind_builds_a_record() {
    let mut s = store();
    let a = aapl(&s);
    let mut cases: Vec<(&str, ActivityForm)> = Vec::new();
    for kind in ["deposit", "withdrawal", "interest", "fee", "tax"] {
        let mut f = form(kind, &s);
        f.net_cash = some("12.34");
        cases.push((kind, f));
    }
    for kind in ["buy", "sell"] {
        let mut f = form(kind, &s);
        f.asset_id = Some(a);
        f.quantity = some("1");
        f.price = some("200");
        cases.push((kind, f));
    }
    let mut div = form("dividend", &s);
    div.asset_id = Some(a);
    div.gross = some("10");
    div.withholding = some("1.5");
    cases.push(("dividend", div));
    let mut tin = form("position_in", &s);
    tin.asset_id = Some(a);
    tin.quantity = some("20");
    tin.cost_basis = some("1500");
    tin.acquisition_date = some("2019-07-01");
    cases.push(("position_in", tin));
    let mut tout = form("position_out", &s);
    tout.asset_id = Some(a);
    tout.quantity = some("2");
    cases.push(("position_out", tout));
    let mut split = form("split", &s);
    split.asset_id = Some(a);
    split.split_new = some("4");
    split.split_old = some("1");
    cases.push(("split", split));
    assert_eq!(cases.len(), MANUAL_KINDS.len());
    for (kind, f) in cases {
        let id =
            save_form(&mut s, &f, &ctx(), true, None).unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        let dt = detail(&s, id).unwrap();
        assert_eq!(dt.row.kind, kind);
        match kind {
            "dividend" => assert_eq!(dt.row.net_cash.as_ref().unwrap().amount, "8.5"),
            "position_in" => {
                assert_eq!(dt.cost_basis.as_ref().unwrap().amount, "1500");
                assert_eq!(dt.acquisition_date.as_deref(), Some("2019-07-01"));
            }
            "split" => assert_eq!(dt.split.as_deref(), Some("4 for 1")),
            "sell" => assert_eq!(dt.row.net_cash.as_ref().unwrap().amount, "200"),
            _ => {}
        }
        // The form round-trips for editing.
        assert_eq!(dt.form.kind, kind);
    }
}

#[test]
fn new_account_is_created_on_save() {
    let mut s = store();
    let mut f = form("deposit", &s);
    f.account_id = None;
    f.new_account = some("Old broker");
    f.net_cash = some("100");
    let id = save_form(&mut s, &f, &ctx(), false, None).unwrap();
    let accounts = s.list_accounts().unwrap();
    assert_eq!(accounts.len(), 2);
    assert_eq!(detail(&s, id).unwrap().row.account, "Old broker");
    // Checking alone never creates anything.
    let mut g = f.clone();
    g.new_account = some("Another");
    check_form(&s, &g, &ctx(), None).unwrap();
    assert_eq!(s.list_accounts().unwrap().len(), 2);
}

#[test]
fn missing_quantity_blocks_saving() {
    let mut s = store();
    let mut f = form("buy", &s);
    f.asset_id = Some(aapl(&s));
    f.price = some("150");
    let c = check_form(&s, &f, &ctx(), None).unwrap();
    assert_eq!(c.errors[0].field, "quantity");
    assert_eq!(c.errors[0].message, "a quantity is required");
    assert!(matches!(
        save_form(&mut s, &f, &ctx(), false, None),
        Err(SaveError::Invalid(_))
    ));
    assert_eq!(page(&s, &ActivityQuery::default(), 0).unwrap().total, 3);
    f.quantity = some("ten");
    assert_eq!(
        check_form(&s, &f, &ctx(), None).unwrap().errors[0].message,
        "\"ten\" is not a number"
    );
}

#[test]
fn figures_that_disagree_warn_but_save() {
    let mut s = store();
    let mut f = form("buy", &s);
    f.asset_id = Some(aapl(&s));
    f.quantity = some("10");
    f.price = some("150");
    f.net_cash = some("1600");
    let c = check_form(&s, &f, &ctx(), None).unwrap();
    assert!(c.errors.is_empty());
    assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
    save_form(&mut s, &f, &ctx(), false, None).unwrap();
}

#[test]
fn entering_an_imported_trade_again_needs_confirmation() {
    let mut s = store();
    let mut f = form("buy", &s);
    f.date = "2025-01-03".into();
    f.asset_id = Some(aapl(&s));
    f.quantity = some("10");
    f.price = some("150.00");
    f.commission = some("1.00");
    let c = check_form(&s, &f, &ctx(), None).unwrap();
    let dup = c.duplicate.unwrap();
    assert_eq!(
        (dup.date.as_str(), dup.source.as_str()),
        ("2025-01-03", "t, row 2")
    );
    assert!(matches!(
        save_form(&mut s, &f, &ctx(), false, None),
        Err(SaveError::NeedsConfirmation(_))
    ));
    save_form(&mut s, &f, &ctx(), true, None).unwrap();
    assert_eq!(page(&s, &ActivityQuery::default(), 0).unwrap().total, 4);
}

#[test]
fn correcting_a_price_replaces_the_entry_and_recalculates_cost() {
    let mut s = store();
    let mut f = form("buy", &s);
    f.asset_id = Some(aapl(&s));
    f.quantity = some("5");
    f.price = some("110.20");
    let id = save_form(&mut s, &f, &ctx(), false, None).unwrap();
    let mut edit = detail(&s, id).unwrap().form;
    // Editing an unchanged entry is not a duplicate of itself.
    assert!(check_form(&s, &edit, &ctx(), Some(id))
        .unwrap()
        .duplicate
        .is_none());
    edit.price = some("110.02");
    let new_id = save_form(&mut s, &edit, &ctx(), false, Some(id)).unwrap();
    let dt = detail(&s, new_id).unwrap();
    assert_eq!(dt.row.price.as_ref().unwrap().amount, "110.02");
    assert_eq!(
        dt.row.source.added_at.as_deref(),
        Some("2026-11-02T10:00:00Z")
    );
    let manual = page(
        &s,
        &ActivityQuery {
            source: some("manual"),
            ..Default::default()
        },
        0,
    )
    .unwrap();
    assert_eq!(manual.total, 1);
    let h = holdings(&s, Date::parse("2026-11-02").unwrap()).unwrap();
    let pos = h.accounts[0]
        .positions
        .iter()
        .find(|p| p.symbol.as_deref() == Some("AAPL"))
        .unwrap();
    assert_eq!(pos.quantity, "15");
    assert_eq!(pos.cost.total.as_deref(), Some("2051.10"));
}

#[test]
fn imported_activities_cannot_be_deleted_one_by_one() {
    let mut s = store();
    let r = first(&s, "buy");
    assert!(delete_entry(&mut s, r.id).is_err());
    assert_eq!(page(&s, &ActivityQuery::default(), 0).unwrap().total, 3);
}

fn amended_ctx() -> ManualContext {
    ManualContext {
        app_version: "0.1.0".into(),
        now: "2026-11-03T09:00:00Z".into(),
    }
}

#[test]
fn correcting_an_imported_price_amends_in_place() {
    let mut s = store();
    let buy = first(&s, "buy");
    let mut f = detail(&s, buy.id).unwrap().form;
    assert_eq!(
        (f.price.as_deref(), f.net_cash.as_deref()),
        (Some("150"), None),
        "the total was worked out"
    );
    f.price = some("151.00");
    let id = save_form(&mut s, &f, &amended_ctx(), false, Some(buy.id)).unwrap();
    assert_eq!(id, buy.id, "same activity");
    let dt = detail(&s, id).unwrap();
    assert_eq!(dt.row.price.as_ref().unwrap().amount, "151");
    assert_eq!(dt.row.net_cash.as_ref().unwrap().amount, "1511");
    assert!(dt.row.source.amended);
    assert_eq!(dt.amended_at.as_deref(), Some("2026-11-03T09:00:00Z"));
    assert_eq!(
        (
            dt.row.source.adapter.as_str(),
            dt.row.source.locator.as_str()
        ),
        ("t", "row 2")
    );
    let h = holdings(&s, Date::parse("2026-11-02").unwrap()).unwrap();
    let pos = h.accounts[0]
        .positions
        .iter()
        .find(|p| p.symbol.as_deref() == Some("AAPL"))
        .unwrap();
    assert_eq!(pos.cost.total.as_deref(), Some("1511"));
}

#[test]
fn re_import_after_amending_keeps_the_amendment() {
    let mut s = store();
    let buy = first(&s, "buy");
    let mut f = detail(&s, buy.id).unwrap().form;
    f.price = some("151.00");
    save_form(&mut s, &f, &amended_ctx(), false, Some(buy.id)).unwrap();
    // The same statement again: every record is recognised as already stored.
    let aapl =
        json!({"isin": "US0378331005", "symbol": "AAPL", "name": "Apple Inc.", "currency": "USD"});
    let records: Vec<Value> = vec![
        json!({"kind": "deposit", "date": "2025-01-02", "net_cash": money("2000.00", "USD")}),
        json!({"kind": "buy", "date": "2025-01-03", "instrument": aapl, "quantity": "10", "price": money("150.00", "USD"), "charges": [{"type": "commission", "amount": money("1.00", "USD")}], "net_cash": money("1501.00", "USD")}),
        json!({"kind": "dividend", "date": "2025-05-15", "instrument": aapl, "gross": money("10.00", "USD"), "charges": [{"type": "withholding_tax", "amount": money("1.50", "USD")}], "net_cash": money("8.50", "USD")}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, mut r)| {
        r["source"] = json!({"adapter": "t", "adapter_version": "1", "file_sha256": "a".repeat(64), "locator": format!("row {}", i + 1)});
        r["account"] = json!({"hint": "U1", "name_hint": "U1"});
        r
    })
    .collect();
    let parsed =
        parse_batch_value(json!({"schema": "mizan.import/1", "records": records})).unwrap();
    let rb = {
        let mut r = MappingResolver {
            store: &mut s,
            policy: CREATE,
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    let prepared = prepare(&s, rb).unwrap();
    assert!(prepared
        .records
        .iter()
        .all(|r| matches!(r.outcome, Outcome::Duplicate { .. })));
    let p = page(&s, &ActivityQuery::default(), 0).unwrap();
    assert_eq!(p.total, 3, "no duplicates");
    assert_eq!(first(&s, "buy").price.unwrap().amount, "151");
}

#[test]
fn undoing_the_import_removes_the_amended_activity() {
    let mut s = store();
    let buy = first(&s, "buy");
    let mut f = detail(&s, buy.id).unwrap().form;
    f.quantity = some("9");
    save_form(&mut s, &f, &amended_ctx(), false, Some(buy.id)).unwrap();
    s.undo_batch(buy.source.batch_id).unwrap();
    assert_eq!(page(&s, &ActivityQuery::default(), 0).unwrap().total, 0);
    assert!(s.amended_ids().unwrap().is_empty());
}

#[test]
fn revert_restores_the_imported_values() {
    let mut s = store();
    let buy = first(&s, "buy");
    let mut f = detail(&s, buy.id).unwrap().form;
    f.price = some("151.00");
    save_form(&mut s, &f, &amended_ctx(), false, Some(buy.id)).unwrap();
    revert_entry(&mut s, buy.id).unwrap();
    let dt = detail(&s, buy.id).unwrap();
    assert_eq!(dt.row.price.as_ref().unwrap().amount, "150");
    assert!(!dt.row.source.amended && dt.amended_at.is_none());
    assert!(
        revert_entry(&mut s, buy.id).is_err(),
        "nothing left to revert"
    );
}

#[test]
fn an_imported_dividend_can_become_interest() {
    let mut s = store();
    let div = first(&s, "dividend");
    let mut f = detail(&s, div.id).unwrap().form;
    f.kind = "interest".into();
    f.net_cash = some("8.50");
    save_form(&mut s, &f, &amended_ctx(), false, Some(div.id)).unwrap();
    assert_eq!(detail(&s, div.id).unwrap().row.kind, "interest");
}

#[test]
fn an_exchange_keeps_its_legs_when_its_date_changes() {
    let mut s = store();
    import(
        &mut s,
        'b',
        "U1",
        vec![
            json!({"kind": "exchange", "date": "2025-07-01", "instrument": {"isin": "US0378331005"}, "legs": [
                {"instrument": {"isin": "US0378331005"}, "quantity_delta": "-10"},
                {"instrument": {"isin": "US5949181045", "symbol": "MSFT"}, "quantity_delta": "4", "cost_basis_share": "1"}
            ]}),
        ],
    );
    let x = first(&s, "exchange");
    let mut f = detail(&s, x.id).unwrap().form;
    assert_eq!(f.kind, "exchange");
    let c = check_form(&s, &f, &amended_ctx(), Some(x.id)).unwrap();
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    f.date = "2025-07-02".into();
    save_form(&mut s, &f, &amended_ctx(), false, Some(x.id)).unwrap();
    let dt = detail(&s, x.id).unwrap();
    assert_eq!(dt.row.date, "2025-07-02");
    assert_eq!(dt.legs.len(), 2);
    assert!(dt.row.source.amended);
}

#[test]
fn deleting_a_manual_dividend_keeps_account_and_asset() {
    let mut s = store();
    let mut f = form("dividend", &s);
    f.asset_id = Some(aapl(&s));
    f.net_cash = some("5");
    let id = save_form(&mut s, &f, &ctx(), false, None).unwrap();
    delete_entry(&mut s, id).unwrap();
    assert_eq!(page(&s, &ActivityQuery::default(), 0).unwrap().total, 3);
    assert_eq!(
        (
            s.list_accounts().unwrap().len(),
            s.list_assets().unwrap().len()
        ),
        (1, 1)
    );
}

#[test]
fn edited_tag_on_amended_imports_and_edited_manual_entries() {
    let mut s = store();
    // Untouched rows carry no tag.
    assert!(page(&s, &ActivityQuery::default(), 0)
        .unwrap()
        .rows
        .iter()
        .all(|r| !r.source.edited && r.source.edited_at.is_none()));

    // An amended import.
    let buy = first(&s, "buy");
    let mut f = detail(&s, buy.id).unwrap().form;
    f.price = some("151.00");
    save_form(&mut s, &f, &amended_ctx(), false, Some(buy.id)).unwrap();
    let r = first(&s, "buy");
    assert!(r.source.edited);
    assert_eq!(r.source.edited_at.as_deref(), Some("2026-11-03T09:00:00Z"));

    // A manual entry: new, then edited.
    let mut m = form("dividend", &s);
    m.asset_id = Some(aapl(&s));
    m.net_cash = some("5");
    let id = save_form(&mut s, &m, &ctx(), false, None).unwrap();
    assert!(
        !detail(&s, id).unwrap().row.source.edited,
        "a new entry is not edited"
    );
    let mut e = detail(&s, id).unwrap().form;
    e.net_cash = some("6");
    let id = save_form(&mut s, &e, &amended_ctx(), false, Some(id)).unwrap();
    let dt = detail(&s, id).unwrap();
    assert!(dt.row.source.edited);
    assert_eq!(
        dt.row.source.edited_at.as_deref(),
        Some("2026-11-03T09:00:00Z")
    );
    assert_eq!(
        dt.row.source.added_at.as_deref(),
        Some("2026-11-02T10:00:00Z"),
        "added date kept"
    );

    // Reverting the import clears its tag.
    revert_entry(&mut s, buy.id).unwrap();
    assert!(!first(&s, "buy").source.edited);
}
