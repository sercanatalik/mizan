//! Read models for the Performance and Overview screens.

use mizan_import::{Code, Date};
use rust_decimal::Decimal;
use serde::Serialize;

use super::calc::{self, pct, Point, Rate};
use super::pricebook::PriceBook;
use super::series::AccountSeries;
use crate::portfolio::{period_start, Period};
use crate::valuation::money;

/// A return as shown: the figure, or the reason it is missing, plus the estimate mark.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Figure {
    /// Cumulative over the period, for example `+2.01%`.
    pub value: Option<String>,
    /// Annualized rate, for periods longer than a year.
    pub annualized: Option<String>,
    /// Why there is no value.
    pub reason: Option<String>,
    /// Days in the period that used trade prices or costs; 0 when exact.
    pub estimated_days: u32,
}

impl Figure {
    fn of(rate: Rate, estimated_days: u32) -> Self {
        match rate {
            Rate::Value {
                cumulative,
                annualized,
            } => Figure {
                value: Some(pct(cumulative)),
                annualized: annualized.map(pct),
                reason: None,
                estimated_days,
            },
            Rate::NotAvailable(why) => Self::missing(why),
        }
    }

    fn missing(why: impl Into<String>) -> Self {
        Figure {
            value: None,
            annualized: None,
            reason: Some(why.into()),
            estimated_days: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountReturns {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub account_id: i64,
    pub name: String,
    /// First day counted.
    pub from: String,
    /// The period started before the account's first activity.
    pub clamped: bool,
    pub twr: Figure,
    pub mwr: Figure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BenchmarkReturn {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub asset_id: i64,
    pub name: String,
    pub figure: Figure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GrowthPoint {
    pub date: String,
    /// Portfolio time-weighted growth, 100 at the period start.
    pub portfolio: Option<String>,
    /// Benchmark growth, 100 at the period start.
    pub benchmark: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PeriodReturns {
    pub period: Period,
    /// First day counted for the portfolio; `None` without activities.
    pub from: Option<String>,
    pub clamped: bool,
    pub to: String,
    pub twr: Figure,
    pub mwr: Figure,
    pub benchmark: Option<BenchmarkReturn>,
    pub accounts: Vec<AccountReturns>,
    /// Accounts left out of the portfolio figures, with the reason.
    pub excluded: Vec<String>,
    pub growth: Vec<GrowthPoint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReturnsView {
    pub base_currency: String,
    pub periods: Vec<PeriodReturns>,
}

/// The benchmark asset: its canonical instrument key, id and display name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Benchmark {
    pub key: String,
    pub asset_id: i64,
    pub name: String,
}

fn prev(d: Date) -> Date {
    Date(d.0.pred_opt().expect("valid day"))
}

fn next(d: Date) -> Date {
    Date(d.0.succ_opt().expect("valid day"))
}

/// Why an account's figures are missing in `[from - 1, today]`, if they are.
fn missing_reason(s: &AccountSeries, from: Date, today: Date) -> Option<String> {
    let first = s.first()?;
    let check_from = if from > first { prev(from) } else { from };
    let mut last_missing = None;
    let mut d = check_from;
    while d <= today {
        if s.day(d).is_some_and(|v| v.value.is_none()) {
            last_missing = Some(d);
        }
        d = next(d);
    }
    last_missing.map(|m| format!("exchange rates before {} are missing", next(m)))
}

struct Window {
    from: Date,
    clamped: bool,
}

fn window(period: Period, first: Date, today: Date) -> Window {
    match period_start(period, today) {
        Some(s) if s >= first => Window {
            from: s,
            clamped: false,
        },
        _ => Window {
            from: first,
            clamped: true,
        },
    }
}

fn value_at(s: &AccountSeries, d: Date) -> Decimal {
    s.day(d).and_then(|v| v.value).unwrap_or_default()
}

fn account_points(s: &AccountSeries, from: Date, today: Date) -> (Decimal, Vec<Point>, u32) {
    let start_value = if s.first().is_some_and(|f| from > f) {
        value_at(s, prev(from))
    } else {
        Decimal::ZERO
    };
    let mut points = Vec::new();
    let mut estimated = 0;
    let mut d = from;
    while d <= today {
        if let Some(v) = s.day(d) {
            points.push(Point {
                date: d,
                value: v.value.unwrap_or_default(),
                flow: v.external_flow + v.internal_flow,
            });
            if v.estimated > 0 {
                estimated += 1;
            }
        }
        d = next(d);
    }
    (start_value, points, estimated)
}

fn benchmark_return(
    b: &Benchmark,
    book: &PriceBook,
    base: &Code,
    from: Date,
    today: Date,
) -> (Figure, Option<(Decimal, Date)>) {
    let Some(start) = book.price_on(&b.key, from) else {
        return (Figure::missing(format!("no price before {from}")), None);
    };
    let Some(end) = book.price_on(&b.key, today) else {
        return (Figure::missing(format!("no price before {from}")), None);
    };
    let (Some(s), Some(e)) = (
        book.convert(start.value, &start.currency, base, from),
        book.convert(end.value, &end.currency, base, today),
    ) else {
        return (Figure::missing("no exchange rate for the benchmark"), None);
    };
    if s.is_zero() {
        return (Figure::missing(format!("no price before {from}")), None);
    }
    let cumulative = e / s - Decimal::ONE;
    let days = (today.0 - from.0).num_days() + 1;
    (
        Figure {
            value: Some(pct(cumulative)),
            annualized: (days > 365).then(|| pct(calc::annualize(cumulative, days))),
            reason: None,
            estimated_days: 0,
        },
        Some((s, from)),
    )
}

fn index_str(x: Decimal) -> String {
    format!("{:.2}", (x * Decimal::ONE_HUNDRED).round_dp(2))
}

/// Returns for every period.
pub fn returns_view(
    series: &[AccountSeries],
    book: &PriceBook,
    base: &Code,
    benchmark: Option<&Benchmark>,
    today: Date,
) -> ReturnsView {
    let active: Vec<&AccountSeries> = series
        .iter()
        .filter(|s| s.first().is_some_and(|f| f <= today))
        .collect();
    let periods = Period::ALL
        .iter()
        .map(|&period| period_returns(period, &active, book, base, benchmark, today))
        .collect();
    ReturnsView {
        base_currency: base.to_string(),
        periods,
    }
}

fn period_returns(
    period: Period,
    active: &[&AccountSeries],
    book: &PriceBook,
    base: &Code,
    benchmark: Option<&Benchmark>,
    today: Date,
) -> PeriodReturns {
    let mut accounts = Vec::new();
    let mut included: Vec<&AccountSeries> = Vec::new();
    let mut excluded = Vec::new();
    for s in active {
        let first = s.first().expect("active");
        let w = window(period, first, today);
        let (twr, mwr) = match missing_reason(s, w.from, today) {
            Some(why) => {
                excluded.push(format!("{}: {why}", s.name));
                (Figure::missing(why.clone()), Figure::missing(why))
            }
            None => {
                included.push(s);
                let (start, points, est) = account_points(s, w.from, today);
                (
                    Figure::of(calc::twr(start, &points), est),
                    Figure::of(calc::mwr(start, &points), est),
                )
            }
        };
        accounts.push(AccountReturns {
            account_id: s.account_id,
            name: s.name.clone(),
            from: w.from.to_string(),
            clamped: w.clamped,
            twr,
            mwr,
        });
    }

    let first = included.iter().filter_map(|s| s.first()).min();
    let Some(first) = first else {
        let why = if active.is_empty() {
            calc::NOTHING_INVESTED.to_string()
        } else {
            "no account can be valued for this period".to_string()
        };
        return PeriodReturns {
            period,
            from: None,
            clamped: false,
            to: today.to_string(),
            twr: Figure::missing(why.clone()),
            mwr: Figure::missing(why),
            benchmark: benchmark.map(|b| BenchmarkReturn {
                asset_id: b.asset_id,
                name: b.name.clone(),
                figure: Figure::missing(calc::NOTHING_INVESTED),
            }),
            accounts,
            excluded,
            growth: vec![],
        };
    };
    let w = window(period, first, today);
    let start_value: Decimal = if w.from > first {
        included.iter().map(|s| value_at(s, prev(w.from))).sum()
    } else {
        Decimal::ZERO
    };
    let mut points = Vec::new();
    let mut estimated = 0;
    let mut d = w.from;
    while d <= today {
        let mut value = Decimal::ZERO;
        let mut flow = Decimal::ZERO;
        let mut est = false;
        for s in &included {
            if let Some(v) = s.day(d) {
                value += v.value.unwrap_or_default();
                flow += v.external_flow;
                est |= v.estimated > 0;
            }
        }
        if est {
            estimated += 1;
        }
        points.push(Point {
            date: d,
            value,
            flow,
        });
        d = next(d);
    }
    let twr = Figure::of(calc::twr(start_value, &points), estimated);
    let mwr = Figure::of(calc::mwr(start_value, &points), estimated);

    let (bench, bench_base) = match benchmark {
        Some(b) => {
            let (figure, base_price) = benchmark_return(b, book, base, w.from, today);
            (
                Some(BenchmarkReturn {
                    asset_id: b.asset_id,
                    name: b.name.clone(),
                    figure,
                }),
                base_price,
            )
        }
        None => (None, None),
    };
    let index = calc::twr_index(start_value, &points);
    let growth = points
        .iter()
        .zip(index)
        .map(|(p, g)| {
            let benchmark = match (benchmark, bench_base) {
                (Some(b), Some((s, _))) => book
                    .price_on(&b.key, p.date)
                    .and_then(|q| book.convert(q.value, &q.currency, base, p.date))
                    .map(|v| index_str(v / s)),
                _ => None,
            };
            GrowthPoint {
                date: p.date.to_string(),
                portfolio: Some(index_str(g)),
                benchmark,
            }
        })
        .collect();

    PeriodReturns {
        period,
        from: Some(w.from.to_string()),
        clamped: w.clamped,
        to: today.to_string(),
        twr,
        mwr,
        benchmark: bench,
        accounts,
        excluded,
        growth,
    }
}

// ---------------------------------------------------------------- value history

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ValuePoint {
    pub date: String,
    /// Portfolio value in the base currency; `None` when some account can't be valued that day.
    pub value: Option<String>,
    /// Cumulative net contributions (external flows) in the base currency.
    pub contributions: String,
    /// Some position was valued at a trade price or cost.
    pub estimated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ValueHistory {
    pub base_currency: String,
    pub points: Vec<ValuePoint>,
    /// Days before today that could be valued.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub valued_days: usize,
}

/// Daily portfolio value and cumulative net contributions, from the first activity to today.
pub fn value_history(series: &[AccountSeries], base: &Code, today: Date) -> ValueHistory {
    let first = series.iter().filter_map(|s| s.first()).min();
    let mut points = Vec::new();
    let mut valued_days = 0;
    if let Some(first) = first {
        let mut contributions = Decimal::ZERO;
        let mut d = first;
        while d <= today {
            let mut value = Some(Decimal::ZERO);
            let mut estimated = false;
            for s in series {
                if let Some(v) = s.day(d) {
                    value = value.zip(v.value).map(|(a, b)| a + b);
                    contributions += v.external_flow;
                    estimated |= v.estimated > 0;
                }
            }
            if value.is_some() && d < today {
                valued_days += 1;
            }
            points.push(ValuePoint {
                date: d.to_string(),
                value: value.map(|v| money(v, base)),
                contributions: money(contributions, base),
                estimated,
            });
            d = next(d);
        }
    }
    ValueHistory {
        base_currency: base.to_string(),
        points,
        valued_days,
    }
}
