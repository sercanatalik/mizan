//! Read models for the Performance and Overview screens.

use mizan_import::{Code, Date};
use rust_decimal::Decimal;
use serde::Serialize;

use super::calc::{self, pct, Point, Rate};
use super::inflation::{DailyIndex, Inflation};
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
    pub(crate) fn of(rate: Rate, estimated_days: u32) -> Self {
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

    /// Same as [`Figure::of`], keeping the estimate mark of a nominal figure.
    fn real(rate: Result<Rate, String>, estimated_days: u32) -> Self {
        match rate {
            Ok(r) => Self::of(r, estimated_days),
            Err(why) => Self::missing(why),
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
    /// Inflation-adjusted time-weighted return.
    pub real_twr: Figure,
    /// Inflation-adjusted money-weighted return.
    pub real_mwr: Figure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BenchmarkReturn {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub asset_id: i64,
    pub name: String,
    pub figure: Figure,
    /// Inflation-adjusted benchmark return.
    pub real: Figure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GrowthPoint {
    pub date: String,
    /// Portfolio time-weighted growth, 100 at the period start.
    pub portfolio: Option<String>,
    /// Benchmark growth, 100 at the period start.
    pub benchmark: Option<String>,
    /// Portfolio growth after inflation, 100 at the period start.
    pub portfolio_real: Option<String>,
    /// Benchmark growth after inflation, 100 at the period start.
    pub benchmark_real: Option<String>,
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
    pub real_twr: Figure,
    pub real_mwr: Figure,
    /// The period ends after the last published inflation month.
    pub real_provisional: bool,
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
    pub inflation: InflationView,
    pub periods: Vec<PeriodReturns>,
}

/// The inflation index behind the real figures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InflationView {
    /// For example "HICP Türkiye (Eurostat)".
    pub label: Option<String>,
    /// Latest published month, for example "Sep 2026".
    pub through: Option<String>,
    /// Why real figures cannot be shown at all.
    pub unavailable: Option<String>,
}

impl InflationView {
    fn of(inflation: &Inflation) -> Self {
        match inflation {
            Inflation::Ready(i) => InflationView {
                label: Some(i.series.full_label()),
                through: Some(i.through().label()),
                unavailable: None,
            },
            Inflation::Unavailable(why) => InflationView {
                label: None,
                through: None,
                unavailable: Some(why.clone()),
            },
        }
    }
}

/// Index levels at the end of the day before `from` and on `to`, and whether
/// `to` is provisional.
struct Deflator<'a> {
    index: &'a DailyIndex,
    start: Decimal,
    end: Decimal,
    provisional: bool,
}

impl<'a> Deflator<'a> {
    fn new(inflation: &'a Inflation, from: Date, to: Date) -> Result<Self, String> {
        let index = match inflation {
            Inflation::Ready(i) => i,
            Inflation::Unavailable(why) => return Err(why.clone()),
        };
        let missing = || format!("no inflation index before {}", index.first().label());
        let start = index.on(prev(from)).ok_or_else(missing)?;
        let end = index.on(to).ok_or_else(missing)?;
        Ok(Deflator {
            index,
            start: start.value,
            end: end.value,
            provisional: end.provisional,
        })
    }

    /// `I(day) / I(start)`; days on or after the period start always have an index.
    fn growth(&self, day: Date) -> Decimal {
        self.index
            .on(day)
            .map(|d| d.value / self.start)
            .unwrap_or(Decimal::ONE)
    }

    /// `(1 + nominal) / (1 + inflation) − 1`, annualized like the nominal rate.
    fn real(&self, rate: &Rate, days: i64) -> Rate {
        match rate {
            Rate::Value { cumulative, .. } => {
                let real = (Decimal::ONE + cumulative) * self.start / self.end - Decimal::ONE;
                Rate::Value {
                    cumulative: real,
                    annualized: (days > 365).then(|| calc::annualize(real, days)),
                }
            }
            Rate::NotAvailable(why) => Rate::NotAvailable(why.clone()),
        }
    }

    /// Money-weighted return on values and flows in start-of-period money.
    fn real_mwr(&self, start_value: Decimal, points: &[Point]) -> Rate {
        let deflated: Vec<Point> = points
            .iter()
            .map(|p| {
                let g = self.growth(p.date);
                Point {
                    date: p.date,
                    value: p.value / g,
                    flow: p.flow / g,
                }
            })
            .collect();
        calc::mwr(start_value, &deflated)
    }
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

pub(crate) fn account_points(
    s: &AccountSeries,
    from: Date,
    today: Date,
) -> (Decimal, Vec<Point>, u32) {
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
) -> (Result<Rate, String>, Option<(Decimal, Date)>) {
    let no_price = || Err(format!("no price before {from}"));
    let Some(start) = book.price_on(&b.key, from) else {
        return (no_price(), None);
    };
    let Some(end) = book.price_on(&b.key, today) else {
        return (no_price(), None);
    };
    let (Some(s), Some(e)) = (
        book.convert(start.value, &start.currency, base, from),
        book.convert(end.value, &end.currency, base, today),
    ) else {
        return (Err("no exchange rate for the benchmark".into()), None);
    };
    if s.is_zero() {
        return (no_price(), None);
    }
    let cumulative = e / s - Decimal::ONE;
    let days = (today.0 - from.0).num_days() + 1;
    (
        Ok(Rate::Value {
            cumulative,
            annualized: (days > 365).then(|| calc::annualize(cumulative, days)),
        }),
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
    inflation: &Inflation,
    today: Date,
) -> ReturnsView {
    let active: Vec<&AccountSeries> = series
        .iter()
        .filter(|s| s.first().is_some_and(|f| f <= today))
        .collect();
    let periods = Period::ALL
        .iter()
        .map(|&period| period_returns(period, &active, book, base, benchmark, inflation, today))
        .collect();
    ReturnsView {
        base_currency: base.to_string(),
        inflation: InflationView::of(inflation),
        periods,
    }
}

fn period_returns(
    period: Period,
    active: &[&AccountSeries],
    book: &PriceBook,
    base: &Code,
    benchmark: Option<&Benchmark>,
    inflation: &Inflation,
    today: Date,
) -> PeriodReturns {
    let mut accounts = Vec::new();
    let mut included: Vec<&AccountSeries> = Vec::new();
    let mut excluded = Vec::new();
    for s in active {
        let first = s.first().expect("active");
        let w = window(period, first, today);
        let (twr, mwr, real_twr, real_mwr) = match missing_reason(s, w.from, today) {
            Some(why) => {
                excluded.push(format!("{}: {why}", s.name));
                let m = Figure::missing(why);
                (m.clone(), m.clone(), m.clone(), m)
            }
            None => {
                included.push(s);
                let (start, points, est) = account_points(s, w.from, today);
                let twr = calc::twr(start, &points);
                let deflator = Deflator::new(inflation, w.from, today);
                let days = points.len() as i64;
                (
                    Figure::of(twr.clone(), est),
                    Figure::of(calc::mwr(start, &points), est),
                    Figure::real(
                        deflator
                            .as_ref()
                            .map(|d| d.real(&twr, days))
                            .map_err(Clone::clone),
                        est,
                    ),
                    Figure::real(
                        deflator
                            .as_ref()
                            .map(|d| d.real_mwr(start, &points))
                            .map_err(Clone::clone),
                        est,
                    ),
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
            real_twr,
            real_mwr,
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
            mwr: Figure::missing(why.clone()),
            real_twr: Figure::missing(why.clone()),
            real_mwr: Figure::missing(why),
            real_provisional: false,
            benchmark: benchmark.map(|b| BenchmarkReturn {
                asset_id: b.asset_id,
                name: b.name.clone(),
                figure: Figure::missing(calc::NOTHING_INVESTED),
                real: Figure::missing(calc::NOTHING_INVESTED),
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
    let twr_rate = calc::twr(start_value, &points);
    let deflator = Deflator::new(inflation, w.from, today);
    let days = points.len() as i64;
    let real_twr = Figure::real(
        deflator
            .as_ref()
            .map(|d| d.real(&twr_rate, days))
            .map_err(Clone::clone),
        estimated,
    );
    let real_mwr = Figure::real(
        deflator
            .as_ref()
            .map(|d| d.real_mwr(start_value, &points))
            .map_err(Clone::clone),
        estimated,
    );
    let real_provisional = deflator.as_ref().is_ok_and(|d| d.provisional);
    let twr = Figure::of(twr_rate, estimated);
    let mwr = Figure::of(calc::mwr(start_value, &points), estimated);

    let (bench, bench_base) = match benchmark {
        Some(b) => {
            let (rate, base_price) = benchmark_return(b, book, base, w.from, today);
            let bench_days = (today.0 - w.from.0).num_days() + 1;
            let figure = Figure::real(rate.clone(), 0);
            let real = Figure::real(
                rate.and_then(|r| {
                    deflator
                        .as_ref()
                        .map(|d| d.real(&r, bench_days))
                        .map_err(Clone::clone)
                }),
                0,
            );
            (
                Some(BenchmarkReturn {
                    asset_id: b.asset_id,
                    name: b.name.clone(),
                    figure,
                    real,
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
            let bench_growth = match (benchmark, bench_base) {
                (Some(b), Some((s, _))) => book
                    .price_on(&b.key, p.date)
                    .and_then(|q| book.convert(q.value, &q.currency, base, p.date))
                    .map(|v| v / s),
                _ => None,
            };
            let infl = deflator.as_ref().ok().map(|d| d.growth(p.date));
            GrowthPoint {
                date: p.date.to_string(),
                portfolio: Some(index_str(g)),
                benchmark: bench_growth.map(index_str),
                portfolio_real: infl.map(|i| index_str(g / i)),
                benchmark_real: bench_growth.zip(infl).map(|(b, i)| index_str(b / i)),
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
        real_twr,
        real_mwr,
        real_provisional,
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
