//! Cross-checks between the redundant figures a statement shows.
//!
//! Checks only read the record. A mismatch produces a warning for review; no
//! figure is ever changed or derived.

use mizan_import::{Code, ImportRecord, Kind};
use rust_decimal::Decimal;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConsistencyWarning {
    /// Position in the batch's `records` array.
    pub index: usize,
    pub locator: String,
    pub message: String,
}

/// Digits after the decimal point for a currency's minor unit.
pub fn minor_digits(code: &Code) -> u32 {
    match code.as_str() {
        "JPY" | "KRW" | "CLP" | "ISK" | "VND" | "HUF" | "XOF" | "XAF" | "UGX" | "PYG" => 0,
        "BHD" | "KWD" | "OMR" | "JOD" | "TND" | "LYD" | "IQD" => 3,
        _ => 2,
    }
}

/// Half of one minor unit: the largest difference still treated as consistent.
pub fn tolerance(code: &Code) -> Decimal {
    Decimal::new(5, minor_digits(code) + 1)
}

/// Convert `amount` from `from` into `to` using the record's FX, if possible.
fn convert(r: &ImportRecord, amount: Decimal, from: &Code, to: &Code) -> Option<Decimal> {
    if from == to {
        return Some(amount);
    }
    let fx = r.fx.as_ref()?;
    let rate = fx.rate.value();
    if &fx.from == from && &fx.to == to {
        Some(amount * rate)
    } else if &fx.from == to && &fx.to == from && !rate.is_zero() {
        Some(amount / rate)
    } else {
        None
    }
}

/// Sum of all charges converted into `ccy`, or `None` if one cannot be converted.
fn charges_in(r: &ImportRecord, ccy: &Code) -> Option<Decimal> {
    r.charges.iter().try_fold(Decimal::ZERO, |acc, c| {
        Some(acc + convert(r, c.amount.amount.value(), &c.amount.currency, ccy)?)
    })
}

fn fmt(d: Decimal, ccy: &Code) -> String {
    format!(
        "{} {ccy}",
        d.round_dp(minor_digits(ccy).max(d.scale().min(8)))
            .normalize()
    )
}

/// Check one record's figures. Returns warnings; the record is untouched.
pub fn check_record(index: usize, r: &ImportRecord) -> Vec<ConsistencyWarning> {
    let mut out = Vec::new();
    let mut warn = |message: String| {
        out.push(ConsistencyWarning {
            index,
            locator: r.source.locator.clone(),
            message,
        });
    };

    match r.kind {
        Kind::Buy | Kind::Sell | Kind::ShortSell | Kind::BuyToCover => {
            let (Some(q), Some(price)) = (r.quantity, r.price.as_ref()) else {
                return out;
            };
            let mult = r
                .instrument
                .as_ref()
                .and_then(|i| i.multiplier)
                .map(|m| m.value())
                .unwrap_or(Decimal::ONE);
            let value = q.value() * price.amount.value() * mult;
            let pccy = &price.currency;

            if let Some(gross) = &r.gross {
                if let Some(v) = convert(r, value, pccy, &gross.currency) {
                    let diff = (gross.amount.value() - v).abs();
                    if diff > tolerance(&gross.currency) {
                        warn(format!(
                            "quantity × price × multiplier = {}, but the statement's gross is {} ({} apart)",
                            fmt(v, &gross.currency),
                            fmt(gross.amount.value(), &gross.currency),
                            fmt(diff, &gross.currency)
                        ));
                    }
                }
            }

            if let Some(net) = &r.net_cash {
                let nccy = &net.currency;
                let (Some(v), Some(ch)) = (convert(r, value, pccy, nccy), charges_in(r, nccy))
                else {
                    return out;
                };
                let outflow = matches!(r.kind, Kind::Buy | Kind::BuyToCover);
                let expected = if outflow { v + ch } else { v - ch };
                let diff = (net.amount.value() - expected).abs();
                if diff > tolerance(nccy) {
                    warn(format!(
                        "quantity × price × multiplier {} charges = {}, but the statement's net cash is {} ({} apart)",
                        if outflow { "+" } else { "−" },
                        fmt(expected, nccy),
                        fmt(net.amount.value(), nccy),
                        fmt(diff, nccy)
                    ));
                }
            }
        }
        Kind::Dividend
        | Kind::Interest
        | Kind::Distribution
        | Kind::Reward
        | Kind::ReturnOfCapital => {
            let (Some(gross), Some(net)) = (r.gross.as_ref(), r.net_cash.as_ref()) else {
                return out;
            };
            let nccy = &net.currency;
            let (Some(g), Some(ch)) = (
                convert(r, gross.amount.value(), &gross.currency, nccy),
                charges_in(r, nccy),
            ) else {
                return out;
            };
            let expected = g - ch;
            let diff = (net.amount.value() - expected).abs();
            if diff > tolerance(nccy) {
                warn(format!(
                    "gross − charges = {}, but the statement's net cash is {} ({} apart)",
                    fmt(expected, nccy),
                    fmt(net.amount.value(), nccy),
                    fmt(diff, nccy)
                ));
            }
        }
        _ => {}
    }
    out
}
