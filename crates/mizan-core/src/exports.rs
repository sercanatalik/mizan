//! CSV exports of stored data: every activity, and holdings as of a day.
//! RFC 4180 like the tax files: comma-separated, CRLF line ends.

use mizan_storage::{StorageError, Store};

use crate::activity::{page, ActivityQuery, ActivityRow, PAGE_SIZE};
use crate::portfolio::Holdings;
use crate::tax::csv::line;

/// Every stored activity, oldest first.
pub fn all_activities(store: &Store) -> Result<Vec<ActivityRow>, StorageError> {
    let q = ActivityQuery::default();
    let mut rows = Vec::new();
    let mut offset = 0;
    loop {
        let p = page(store, &q, offset)?;
        let n = p.rows.len() as i64;
        rows.extend(p.rows);
        offset += n;
        if n < PAGE_SIZE || offset >= p.total {
            break;
        }
    }
    rows.reverse();
    Ok(rows)
}

fn o(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("")
}

pub fn activities_csv(rows: &[ActivityRow]) -> String {
    let mut out = line(&[
        "date",
        "account",
        "kind",
        "instrument",
        "symbol",
        "isin",
        "quantity",
        "price",
        "price_currency",
        "net_cash",
        "net_cash_currency",
        "charges",
        "source",
        "import_file",
        "row",
    ]);
    for r in rows {
        let charges = r
            .charges
            .iter()
            .map(|c| format!("{} {}", c.amount, c.currency))
            .collect::<Vec<_>>()
            .join("; ");
        let price = r.price.as_ref();
        let net = r.net_cash.as_ref();
        out.push_str(&line(&[
            &r.date,
            &r.account,
            &r.kind,
            o(&r.instrument),
            o(&r.symbol),
            o(&r.isin),
            o(&r.quantity),
            price.map_or("", |p| p.amount.as_str()),
            price.map_or("", |p| p.currency.as_str()),
            net.map_or("", |n| n.amount.as_str()),
            net.map_or("", |n| n.currency.as_str()),
            &charges,
            if r.source.manual {
                "manual"
            } else {
                r.source.adapter.as_str()
            },
            if r.source.manual {
                ""
            } else {
                r.source.file.as_str()
            },
            if r.source.manual {
                ""
            } else {
                r.source.locator.as_str()
            },
        ]));
    }
    out
}

/// One row per position and cash line, with values in the base currency.
pub fn holdings_csv(h: &Holdings) -> String {
    let base = &h.base_currency;
    let mut out = line(&[
        "account",
        "instrument",
        "symbol",
        "isin",
        "class",
        "quantity",
        "price",
        "price_currency",
        "price_date",
        "value",
        &format!("value_{base}"),
        "weight_pct",
        "average_cost",
        "total_cost",
        "cost_currency",
        "note",
    ]);
    for a in &h.accounts {
        for p in &a.positions {
            let pr = p.price.as_ref();
            let note = [p.value_note.as_deref(), p.gain_note.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; ");
            out.push_str(&line(&[
                &a.name,
                o(&p.name),
                o(&p.symbol),
                o(&p.isin),
                &p.class,
                &p.quantity,
                pr.map_or("", |x| x.value.as_str()),
                pr.map_or("", |x| x.currency.as_str()),
                pr.map_or("", |x| x.date.as_str()),
                o(&p.value),
                o(&p.base_value),
                o(&p.weight),
                o(&p.cost.average),
                o(&p.cost.total),
                o(&p.cost.currency),
                &note,
            ]));
        }
        for c in &a.cash {
            out.push_str(&line(&[
                &a.name,
                "cash",
                "",
                "",
                "cash",
                &c.amount,
                "",
                "",
                "",
                &c.amount,
                o(&c.base_value),
                "",
                "",
                "",
                &c.currency,
                "",
            ]));
        }
    }
    out
}

/// A file name part from free text: lower case, other characters as `-`.
pub fn slug(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "account".into()
    } else {
        s
    }
}
