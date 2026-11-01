//! Deterministic ordering of records that share an account and a date.
//!
//! Within one source file, records keep their source order. Records from
//! different files are interleaved by kind priority (corporate actions, cash
//! in, acquisitions, disposals, income, cash out, adjustments). When records
//! for the same instrument from different files move its quantity in opposite
//! directions on the same day, their relative order changes the intraday
//! position and lots, so an [`OrderingDiagnostic`] asks the user to confirm.

use std::collections::BTreeMap;

use mizan_import::{Date, Kind};
use serde::Serialize;

/// Lower runs earlier in a day.
pub fn kind_priority(kind: Kind) -> u8 {
    use Kind::*;
    match kind {
        Split | Exchange | Spinoff => 0,
        Deposit | InternalTransfer | FxConversion => 1,
        Buy | BuyToCover | PositionIn | OptionExercise | OptionAssignment => 2,
        Sell | ShortSell | PositionOut | OptionExpiry => 3,
        Dividend | Interest | Distribution | Reward | ReturnOfCapital => 4,
        Withdrawal | Fee | Tax => 5,
        Adjustment => 6,
    }
}

/// +1 if the kind adds units of its instrument, -1 if it removes them, 0 otherwise.
fn direction(kind: Kind) -> i8 {
    use Kind::*;
    match kind {
        Buy | BuyToCover | PositionIn => 1,
        Sell | ShortSell | PositionOut => -1,
        _ => 0,
    }
}

/// What the ordering needs to know about one record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderInput {
    /// Account identity (an internal id or a hint, as long as it is consistent).
    pub account: String,
    pub date: Date,
    /// Source file identity, e.g. its SHA-256.
    pub file: String,
    /// Position within its file.
    pub source_order: i64,
    pub kind: Kind,
    pub instrument_key: Option<String>,
    pub locator: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OrderingDiagnostic {
    pub account: String,
    pub date: String,
    pub instrument_key: String,
    /// Locators of the records involved, as `file:locator`.
    pub records: Vec<String>,
    pub message: String,
}

/// Returns the indices of `items` in processing order, plus diagnostics for
/// same-day orders that cannot be decided safely.
pub fn order(items: &[OrderInput]) -> (Vec<usize>, Vec<OrderingDiagnostic>) {
    let mut groups: BTreeMap<(&str, Date), Vec<usize>> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        groups
            .entry((it.account.as_str(), it.date))
            .or_default()
            .push(i);
    }

    let mut out = Vec::with_capacity(items.len());
    let mut diags = Vec::new();

    for ((account, date), idxs) in groups {
        // One queue per file, in source order. Files are ranked by identity for determinism.
        let mut per_file: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &i in &idxs {
            per_file.entry(items[i].file.as_str()).or_default().push(i);
        }
        let mut queues: Vec<Vec<usize>> = per_file
            .into_values()
            .map(|mut q| {
                q.sort_by_key(|&i| (items[i].source_order, i));
                q.reverse(); // pop from the end = next in source order
                q
            })
            .collect();

        // k-way merge: take the queue head with the lowest kind priority; ties go to the lower-ranked file.
        while let Some((qi, _)) = queues
            .iter()
            .enumerate()
            .filter_map(|(qi, q)| q.last().map(|&i| (qi, kind_priority(items[i].kind))))
            .min_by_key(|&(qi, p)| (p, qi))
        {
            out.push(queues[qi].pop().expect("non-empty"));
        }

        // Opposite-direction moves of one instrument from different files.
        let mut by_instrument: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &i in &idxs {
            if let (Some(k), d) = (items[i].instrument_key.as_deref(), direction(items[i].kind)) {
                if d != 0 {
                    by_instrument.entry(k).or_default().push(i);
                }
            }
        }
        for (key, rs) in by_instrument {
            let conflicting = rs.iter().any(|&a| {
                rs.iter().any(|&b| {
                    items[a].file != items[b].file
                        && direction(items[a].kind) != direction(items[b].kind)
                })
            });
            if conflicting {
                diags.push(OrderingDiagnostic {
                    account: account.to_owned(),
                    date: date.to_string(),
                    instrument_key: key.to_owned(),
                    records: rs.iter().map(|&i| format!("{}:{}", items[i].file, items[i].locator)).collect(),
                    message: format!(
                        "Records for {key} on {date} come from different files and move the position in opposite directions; their order changes the intraday position and lots. Confirm the order."
                    ),
                });
            }
        }
    }
    (out, diags)
}
