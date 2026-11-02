//! Small builders for tax rule tests.

use std::str::FromStr;

use mizan_import::{
    AccountHint, Charge, ChargeType, Code, Date, Dec, ImportRecord, InstrumentHint, Kind, Money,
    Provenance, Sha256Hex,
};
use mizan_storage::{AccountId, StoredActivity};
use rust_decimal::Decimal;

use crate::resolve::instrument_key;

pub fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

pub fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

pub fn code(s: &str) -> Code {
    Code::parse(s).unwrap()
}

pub fn m(amount: &str, ccy: &str) -> Money {
    Money {
        amount: Dec(d(amount)),
        currency: code(ccy),
    }
}

pub fn hint(symbol: &str) -> InstrumentHint {
    InstrumentHint {
        symbol: Some(symbol.into()),
        ..Default::default()
    }
}

pub fn key_of(symbol: &str) -> String {
    instrument_key(&hint(symbol)).unwrap()
}

pub fn record(kind: Kind, date: &str, symbol: &str) -> ImportRecord {
    ImportRecord {
        source: Provenance {
            adapter: "test".into(),
            adapter_version: "1".into(),
            file_sha256: Sha256Hex::parse(&"a".repeat(64)).unwrap(),
            locator: format!("{date} {symbol}"),
        },
        external_id: None,
        account: AccountHint {
            hint: "A".into(),
            name_hint: None,
        },
        kind,
        date: day(date),
        settle_date: None,
        instrument: (!symbol.is_empty()).then(|| hint(symbol)),
        quantity: None,
        price: None,
        gross: None,
        charges: vec![],
        net_cash: None,
        fx: None,
        legs: vec![],
        split: None,
        cost_basis: None,
        acquisition_date: None,
        links: vec![],
        memo: None,
        raw: None,
    }
}

/// A buy or sell of `qty` at `price`, with an optional commission.
pub fn trade(
    kind: Kind,
    date: &str,
    symbol: &str,
    qty: &str,
    price: &str,
    ccy: &str,
    commission: Option<&str>,
) -> ImportRecord {
    let mut r = record(kind, date, symbol);
    r.quantity = Some(Dec(d(qty)));
    r.price = Some(m(price, ccy));
    if let Some(c) = commission {
        r.charges = vec![Charge {
            charge_type: ChargeType::Commission,
            amount: m(c, ccy),
        }];
    }
    r
}

/// A buy or sell with a total amount (net of charges) instead of a price.
pub fn deal(
    kind: Kind,
    date: &str,
    symbol: &str,
    qty: &str,
    total: &str,
    ccy: &str,
) -> ImportRecord {
    let mut r = record(kind, date, symbol);
    r.quantity = Some(Dec(d(qty)));
    r.net_cash = Some(m(total, ccy));
    r
}

pub fn position(kind: Kind, date: &str, symbol: &str, qty: &str) -> ImportRecord {
    let mut r = record(kind, date, symbol);
    r.quantity = Some(Dec(d(qty)));
    r
}

pub fn stored(account: AccountId, record: ImportRecord) -> StoredActivity {
    StoredActivity {
        id: 0,
        batch_id: 1,
        account_id: account,
        asset_id: None,
        fingerprint: String::new(),
        source_order: 0,
        basis_unknown: false,
        record,
    }
}
