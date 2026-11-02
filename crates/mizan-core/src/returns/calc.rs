//! Time- and money-weighted returns over a daily value series.
//!
//! Conventions (see `docs/returns.md`):
//! - Flows arrive at the start of their day.
//! - TWR links daily returns `V_d / (V_{d-1} + F_d)`; days with nothing invested are skipped.
//! - MWR is the internal rate of return over actual days / 365. The solver
//!   works in `f64`: it finds a rate, never a money amount, and the result is
//!   rounded to 2 decimal places for display.

use mizan_import::Date;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::Decimal;

/// One day of a series as the return functions see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub date: Date,
    /// Value at the end of the day.
    pub value: Decimal,
    /// External flows that day (positive = money in).
    pub flow: Decimal,
}

/// A return, or why there is none.
#[derive(Clone, Debug, PartialEq)]
pub enum Rate {
    /// Cumulative return over the period (0.0201 = +2.01%), and the annualized
    /// rate when the period is longer than a year.
    Value {
        cumulative: Decimal,
        annualized: Option<Decimal>,
    },
    NotAvailable(String),
}

pub const NOTHING_INVESTED: &str = "nothing invested in this period";
pub const NO_UNIQUE_RATE: &str = "no unique rate for these cash flows";

fn to_f64(d: Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
}

fn from_f64(x: f64) -> Decimal {
    Decimal::from_f64(x).unwrap_or_default()
}

/// `(1 + r)^(365 / days) - 1`
pub fn annualize(cumulative: Decimal, days: i64) -> Decimal {
    from_f64((1.0 + to_f64(cumulative)).powf(365.0 / days as f64) - 1.0)
}

/// Time-weighted return. `start_value` is the value at the end of the day
/// before the first point (zero when the period starts at the first activity).
pub fn twr(start_value: Decimal, points: &[Point]) -> Rate {
    let mut growth = Decimal::ONE;
    let mut invested = false;
    let mut prev = start_value;
    for p in points {
        let base = prev + p.flow;
        if base > Decimal::ZERO {
            growth *= p.value / base;
            invested = true;
        }
        prev = p.value;
    }
    if !invested {
        return Rate::NotAvailable(NOTHING_INVESTED.into());
    }
    let cumulative = growth - Decimal::ONE;
    let days = points.len() as i64;
    Rate::Value {
        cumulative,
        annualized: (days > 365).then(|| annualize(cumulative, days)),
    }
}

/// Daily growth index (1.0 at the start) for the growth chart, one per point.
pub fn twr_index(start_value: Decimal, points: &[Point]) -> Vec<Decimal> {
    let mut growth = Decimal::ONE;
    let mut prev = start_value;
    points
        .iter()
        .map(|p| {
            let base = prev + p.flow;
            if base > Decimal::ZERO {
                growth *= p.value / base;
            }
            prev = p.value;
            growth
        })
        .collect()
}

fn npv(rate: f64, flows: &[(f64, f64)]) -> f64 {
    flows
        .iter()
        .map(|(t, cf)| cf / (1.0 + rate).powf(t / 365.0))
        .sum()
}

fn d_npv(rate: f64, flows: &[(f64, f64)]) -> f64 {
    flows
        .iter()
        .map(|(t, cf)| -(t / 365.0) * cf / (1.0 + rate).powf(t / 365.0 + 1.0))
        .sum()
}

/// Annual internal rate of return of dated cash flows `(days from start, amount)`.
pub fn xirr(flows: &[(f64, f64)]) -> Option<f64> {
    let pos = flows.iter().any(|(_, c)| *c > 0.0);
    let neg = flows.iter().any(|(_, c)| *c < 0.0);
    if !pos || !neg {
        return None;
    }
    // Newton from 10%.
    let mut r = 0.1;
    for _ in 0..100 {
        let f = npv(r, flows);
        let df = d_npv(r, flows);
        if df == 0.0 || !f.is_finite() {
            break;
        }
        let next = r - f / df;
        if !next.is_finite() || next <= -0.9999 {
            break;
        }
        if (next - r).abs() < 1e-12 {
            return Some(next);
        }
        r = next;
    }
    // Bisection fallback over -99.99% .. +10000%.
    let (mut lo, mut hi) = (-0.9999_f64, 100.0_f64);
    let (mut flo, fhi) = (npv(lo, flows), npv(hi, flows));
    if !(flo.is_finite() && fhi.is_finite()) || flo.signum() == fhi.signum() {
        return None;
    }
    for _ in 0..300 {
        let mid = (lo + hi) / 2.0;
        let fm = npv(mid, flows);
        if fm.abs() < 1e-9 || (hi - lo) < 1e-12 {
            return Some(mid);
        }
        if fm.signum() == flo.signum() {
            lo = mid;
            flo = fm;
        } else {
            hi = mid;
        }
    }
    Some((lo + hi) / 2.0)
}

/// Money-weighted return. The start value counts as invested at the end of the
/// day before the first point; each day's flows at the start of that day; the
/// final value is taken out at the end of the last day.
pub fn mwr(start_value: Decimal, points: &[Point]) -> Rate {
    let Some(last) = points.last() else {
        return Rate::NotAvailable(NOTHING_INVESTED.into());
    };
    let days = points.len() as i64;
    let mut flows: Vec<(f64, f64)> = Vec::new();
    if !start_value.is_zero() {
        flows.push((0.0, -to_f64(start_value)));
    }
    for (i, p) in points.iter().enumerate() {
        if !p.flow.is_zero() {
            flows.push((i as f64, -to_f64(p.flow)));
        }
    }
    if !last.value.is_zero() {
        flows.push((days as f64, to_f64(last.value)));
    }
    if flows.iter().all(|(_, c)| *c == 0.0) || flows.is_empty() {
        return Rate::NotAvailable(NOTHING_INVESTED.into());
    }
    if !flows.iter().any(|(_, c)| *c < 0.0) {
        return Rate::NotAvailable(NOTHING_INVESTED.into());
    }
    let Some(r) = xirr(&flows) else {
        return Rate::NotAvailable(NO_UNIQUE_RATE.into());
    };
    if days <= 365 {
        let period = (1.0 + r).powf(days as f64 / 365.0) - 1.0;
        Rate::Value {
            cumulative: from_f64(period),
            annualized: None,
        }
    } else {
        let cumulative = (1.0 + r).powf(days as f64 / 365.0) - 1.0;
        Rate::Value {
            cumulative: from_f64(cumulative),
            annualized: Some(from_f64(r)),
        }
    }
}

/// `+2.01%` / `−3.50%` (2 decimal places, typographic minus).
pub fn pct(r: Decimal) -> String {
    let p = (r * Decimal::ONE_HUNDRED).round_dp(2);
    let s = format!("{:.2}", p.abs());
    if p.is_sign_negative() && !p.is_zero() {
        format!("−{s}%")
    } else {
        format!("+{s}%")
    }
}
