//! The daily inflation index of the base currency, from monthly values in the
//! history cache. Each monthly value applies to the 15th of its month; days in
//! between are interpolated geometrically, days after the last published month
//! keep its value and are provisional, and days before the first have none.

use std::str::FromStr;

use chrono::NaiveDate;
use mizan_import::{Code, Date};
use mizan_storage::inflation::{series_for, InflationSeries, YearMonth};
use std::collections::BTreeMap;

use mizan_storage::{HistoryCache, SeriesKind, StorageError, Store};
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::Decimal;

/// A monthly index series from the history cache with this database's edits
/// applied: edited and added months replace or extend the cache, and ignored
/// months are left out.
pub fn monthly_values(
    store: &Store,
    cache: Option<&HistoryCache>,
    source: &str,
    id: &str,
) -> Result<BTreeMap<YearMonth, Decimal>, StorageError> {
    let mut out = BTreeMap::new();
    if let Some(c) = cache {
        for v in c.inflation_index(source, id)? {
            if let (Some(m), Ok(x)) = (YearMonth::parse(&v.month), Decimal::from_str(&v.value)) {
                out.insert(m, x);
            }
        }
    }
    for e in store.series_edits(SeriesKind::Index, &format!("{source}:{id}"))? {
        let Some(m) = YearMonth::parse(&e.period) else {
            continue;
        };
        match e.value.as_deref().map(Decimal::from_str) {
            Some(Ok(x)) if !e.ignored => {
                out.insert(m, x);
            }
            _ => {
                out.remove(&m);
            }
        }
    }
    Ok(out)
}

/// The index on one day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexDay {
    pub value: Decimal,
    /// After the 15th of the last published month: carried forward, not measured.
    pub provisional: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DailyIndex {
    pub series: InflationSeries,
    /// (15th of the month, month, value), oldest first.
    anchors: Vec<(NaiveDate, YearMonth, Decimal)>,
}

impl DailyIndex {
    /// Build from monthly values in any order; non-positive values are ignored.
    pub fn new(
        series: InflationSeries,
        values: impl IntoIterator<Item = (YearMonth, Decimal)>,
    ) -> Option<Self> {
        let mut anchors: Vec<_> = values
            .into_iter()
            .filter(|(_, v)| *v > Decimal::ZERO)
            .map(|(m, v)| (m.anchor(), m, v))
            .collect();
        anchors.sort_by_key(|a| a.0);
        anchors.dedup_by_key(|a| a.0);
        (!anchors.is_empty()).then_some(DailyIndex { series, anchors })
    }

    /// First month with a value.
    pub fn first(&self) -> YearMonth {
        self.anchors[0].1
    }

    /// Latest published month.
    pub fn through(&self) -> YearMonth {
        self.anchors[self.anchors.len() - 1].1
    }

    pub fn on(&self, day: Date) -> Option<IndexDay> {
        let d = day.0;
        let n = self.anchors.partition_point(|a| a.0 <= d);
        let (a_day, _, a) = *self.anchors.get(n.checked_sub(1)?)?;
        let Some(&(b_day, _, b)) = self.anchors.get(n) else {
            return Some(IndexDay {
                value: a,
                provisional: d > a_day,
            });
        };
        if d == a_day {
            return Some(IndexDay {
                value: a,
                provisional: false,
            });
        }
        // a · (b / a)^t in f64: a ratio of index levels, never money.
        let t = (d - a_day).num_days() as f64 / (b_day - a_day).num_days() as f64;
        let ratio = (b / a).to_f64()?;
        let value = a * Decimal::from_f64(ratio.powf(t))?;
        Some(IndexDay {
            value,
            provisional: false,
        })
    }
}

/// The inflation index for real returns, or why there is none.
#[derive(Clone, Debug, PartialEq)]
pub enum Inflation {
    Ready(DailyIndex),
    Unavailable(String),
}

impl Inflation {
    /// Load the base currency's index from the cache. Never touches the network.
    pub fn load(
        store: &Store,
        cache: Option<&HistoryCache>,
        base: &Code,
    ) -> Result<Self, StorageError> {
        let Some(series) = series_for(base.as_str()) else {
            return Ok(Inflation::Unavailable(format!(
                "no inflation index for {base}"
            )));
        };
        let values = monthly_values(store, cache, series.source, series.id)?;
        Ok(match DailyIndex::new(*series, values) {
            Some(idx) => Inflation::Ready(idx),
            None => Inflation::Unavailable(format!(
                "the {base} inflation index has not been downloaded yet"
            )),
        })
    }

    pub fn index(&self) -> Option<&DailyIndex> {
        match self {
            Inflation::Ready(i) => Some(i),
            Inflation::Unavailable(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ym(s: &str) -> YearMonth {
        YearMonth::parse(s).unwrap()
    }
    fn day(s: &str) -> Date {
        Date::parse(s).unwrap()
    }
    fn index() -> DailyIndex {
        DailyIndex::new(
            *series_for("TRY").unwrap(),
            [
                (ym("2026-10"), Decimal::new(10200, 2)),
                (ym("2026-09"), Decimal::new(10000, 2)),
            ],
        )
        .unwrap()
    }

    #[test]
    fn anchors_interpolation_and_provisional_days() {
        let i = index();
        assert_eq!(i.first(), ym("2026-09"));
        assert_eq!(i.through(), ym("2026-10"));
        let on = |d| i.on(day(d)).unwrap();
        assert_eq!(on("2026-10-15").value, Decimal::new(10200, 2));
        assert!(!on("2026-10-15").provisional);
        assert_eq!(on("2026-09-15").value, Decimal::new(10000, 2));
        let mid = on("2026-09-30");
        assert_eq!(mid.value.round_dp(3), Decimal::new(100995, 3));
        assert!(!mid.provisional);
        let late = on("2026-11-02");
        assert_eq!(late.value, Decimal::new(10200, 2));
        assert!(late.provisional);
        assert_eq!(i.on(day("2026-09-14")), None);
    }

    #[test]
    fn unavailable_reasons() {
        let chf = Code::parse("CHF").unwrap();
        assert_eq!(
            Inflation::load(&Store::open_in_memory().unwrap(), None, &chf).unwrap(),
            Inflation::Unavailable("no inflation index for CHF".into())
        );
        let gbp = Code::parse("GBP").unwrap();
        let cache = HistoryCache::open_in_memory().unwrap();
        assert_eq!(
            Inflation::load(&Store::open_in_memory().unwrap(), Some(&cache), &gbp).unwrap(),
            Inflation::Unavailable("the GBP inflation index has not been downloaded yet".into())
        );
    }
}
