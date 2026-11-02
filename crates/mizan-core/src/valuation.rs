//! Turning positions into values with stored prices and exchange rates.
//!
//! Pure functions over a [`Market`] (latest quote per asset, TRY rate per
//! currency) and an explicit `today`. No network, no clock.

use std::str::FromStr;

use mizan_import::{Code, Date};
use mizan_storage::{AssetId, StorageError, Store};
use rust_decimal::Decimal;

use crate::consistency::minor_digits;

/// A price older than this many days is flagged as stale.
pub const STALE_DAYS: i64 = 7;
pub const DEFAULT_BASE: &str = "TRY";

/// The latest usable price of an asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quote {
    pub date: Date,
    pub value: Decimal,
    pub currency: Code,
    pub source: String,
}

/// Read access to stored prices and rates.
pub trait Market {
    /// Newest price dated on or before `today` (manual wins a date tie).
    fn quote(&self, asset: AssetId, today: Date) -> Result<Option<Quote>, StorageError>;
    /// TRY per one unit of `currency`, newest dated on or before `today`. TRY itself is 1.
    fn try_per_unit(&self, currency: &Code, today: Date) -> Result<Option<Decimal>, StorageError>;
}

fn parse_dec(s: &str) -> Option<Decimal> {
    Decimal::from_str(s).ok()
}

impl Market for Store {
    fn quote(&self, asset: AssetId, today: Date) -> Result<Option<Quote>, StorageError> {
        let Some(p) = self.latest_price(asset, &today.to_string())? else {
            return Ok(None);
        };
        Ok(
            match (
                Date::parse(&p.date),
                parse_dec(&p.value),
                Code::parse(&p.currency),
            ) {
                (Ok(date), Some(value), Ok(currency)) => Some(Quote {
                    date,
                    value,
                    currency,
                    source: p.source,
                }),
                _ => None,
            },
        )
    }

    fn try_per_unit(&self, currency: &Code, today: Date) -> Result<Option<Decimal>, StorageError> {
        if currency.as_str() == "TRY" {
            return Ok(Some(Decimal::ONE));
        }
        Ok(self
            .latest_fx_rate(currency.as_str(), &today.to_string())?
            .and_then(|r| parse_dec(&r.try_per_unit)))
    }
}

/// Prices and rates through a [`crate::returns::PriceBook`]: the user
/// database, series edits and the history cache in one order of sources.
pub struct BookMarket<'a> {
    pub store: &'a Store,
    pub book: &'a crate::returns::PriceBook,
}

impl Market for BookMarket<'_> {
    fn quote(&self, asset: AssetId, today: Date) -> Result<Option<Quote>, StorageError> {
        let keys = self.store.key_map()?;
        let canonical = self.store.canonical_asset(asset)?;
        let Some(row) = self
            .store
            .list_assets()?
            .into_iter()
            .find(|a| a.id == canonical)
        else {
            return Ok(None);
        };
        let key = keys
            .get(&row.instrument_key)
            .cloned()
            .unwrap_or(row.instrument_key);
        let Some(p) = self.book.price_on(&key, today) else {
            return Ok(None);
        };
        // Where the value came from: the user database's own row for that
        // date, else an edit, else the history cache of the price source.
        let stored = self
            .store
            .latest_price(canonical, &p.date.to_string())?
            .filter(|r| r.date == p.date.to_string() && parse_dec(&r.value) == Some(p.value));
        let source = match stored {
            Some(r) => r.source,
            None => {
                let series = self
                    .store
                    .price_sources()?
                    .get(&canonical)
                    .and_then(crate::returns::pricebook::series_key);
                let edited = match &series {
                    Some(sk) => self
                        .store
                        .series_edits(mizan_storage::SeriesKind::Price, sk)?
                        .iter()
                        .any(|e| e.period == p.date.to_string() && !e.ignored),
                    None => false,
                };
                if edited {
                    "edited".into()
                } else {
                    series
                        .and_then(|s| s.split_once(':').map(|(p, _)| p.to_string()))
                        .unwrap_or_else(|| "history".into())
                }
            }
        };
        Ok(Some(Quote {
            date: p.date,
            value: p.value,
            currency: p.currency.clone(),
            source,
        }))
    }

    fn try_per_unit(&self, currency: &Code, today: Date) -> Result<Option<Decimal>, StorageError> {
        Ok(self.book.try_per_unit(currency, today))
    }
}

/// The base currency setting, `TRY` when unset or invalid.
pub fn base_currency(store: &Store) -> Result<Code, StorageError> {
    let s = store
        .setting("base_currency")?
        .unwrap_or_else(|| DEFAULT_BASE.into());
    Ok(Code::parse(&s).unwrap_or_else(|_| Code::parse(DEFAULT_BASE).expect("valid")))
}

/// Convert `amount` between currencies through TRY. `None` when a rate is missing.
pub fn convert(
    m: &dyn Market,
    amount: Decimal,
    from: &Code,
    to: &Code,
    today: Date,
) -> Result<Option<Decimal>, StorageError> {
    if from == to {
        return Ok(Some(amount));
    }
    let (Some(f), Some(t)) = (m.try_per_unit(from, today)?, m.try_per_unit(to, today)?) else {
        return Ok(None);
    };
    if t.is_zero() {
        return Ok(None);
    }
    Ok(Some(amount * f / t))
}

pub fn is_stale(price_date: Date, today: Date) -> bool {
    (today.0 - price_date.0).num_days() > STALE_DAYS
}

/// Round to the currency's minor unit for display.
pub fn money(amount: Decimal, ccy: &Code) -> String {
    let digits = minor_digits(ccy);
    let mut r = amount.round_dp(digits);
    r.rescale(digits);
    r.to_string()
}

/// Unrealized gain, or the reason it is not available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gain {
    Amount { amount: Decimal, pct: Decimal },
    NotAvailable(&'static str),
}

/// Gain is value minus cost, only when cost is known and in the price currency.
pub fn unrealized(
    cost: Option<(Decimal, &Code)>,
    unknown_basis: bool,
    value: Decimal,
    value_ccy: &Code,
) -> Gain {
    if unknown_basis {
        return Gain::NotAvailable("unknown cost basis");
    }
    match cost {
        None => Gain::NotAvailable("cost not available"),
        Some((_, c)) if c != value_ccy => {
            Gain::NotAvailable("cost and price in different currencies")
        }
        Some((c, _)) if c.is_zero() => Gain::NotAvailable("zero cost"),
        Some((c, _)) => {
            let amount = value - c;
            Gain::Amount {
                amount,
                pct: amount / c * Decimal::ONE_HUNDRED,
            }
        }
    }
}

pub fn signed_money(amount: Decimal, ccy: &Code) -> String {
    let s = money(amount.abs(), ccy);
    if amount.is_sign_negative() && !amount.is_zero() {
        format!("−{s}")
    } else {
        format!("+{s}")
    }
}

pub fn signed_pct(pct: Decimal) -> String {
    let s = pct.abs().round_dp(1).to_string();
    if pct.is_sign_negative() && !pct.is_zero() {
        format!("−{s}%")
    } else {
        format!("+{s}%")
    }
}

/// Sums values in the base currency and counts what could not be included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Total {
    pub base: Code,
    pub sum: Decimal,
    pub excluded_no_price: usize,
    pub excluded_no_rate: usize,
    /// Oldest price date that went into the sum.
    pub oldest_price: Option<Date>,
}

impl Total {
    pub fn new(base: Code) -> Self {
        Total {
            base,
            sum: Decimal::ZERO,
            excluded_no_price: 0,
            excluded_no_rate: 0,
            oldest_price: None,
        }
    }

    /// Add an amount in `ccy`; returns its base value when it could be converted.
    pub fn add(
        &mut self,
        m: &dyn Market,
        amount: Decimal,
        ccy: &Code,
        today: Date,
    ) -> Result<Option<Decimal>, StorageError> {
        match convert(m, amount, ccy, &self.base, today)? {
            Some(v) => {
                self.sum += v;
                Ok(Some(v))
            }
            None => {
                self.excluded_no_rate += 1;
                Ok(None)
            }
        }
    }

    pub fn add_priced(
        &mut self,
        m: &dyn Market,
        amount: Decimal,
        ccy: &Code,
        price_date: Date,
        today: Date,
    ) -> Result<Option<Decimal>, StorageError> {
        let v = self.add(m, amount, ccy, today)?;
        if v.is_some() {
            self.oldest_price = Some(self.oldest_price.map_or(price_date, |d| d.min(price_date)));
        }
        Ok(v)
    }

    pub fn no_price(&mut self) {
        self.excluded_no_price += 1;
    }

    pub fn merge(&mut self, other: &Total) {
        self.sum += other.sum;
        self.excluded_no_price += other.excluded_no_price;
        self.excluded_no_rate += other.excluded_no_rate;
        self.oldest_price = match (self.oldest_price, other.oldest_price) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake {
        quotes: HashMap<AssetId, Quote>,
        rates: HashMap<String, Decimal>,
    }

    impl Market for Fake {
        fn quote(&self, asset: AssetId, today: Date) -> Result<Option<Quote>, StorageError> {
            Ok(self.quotes.get(&asset).filter(|q| q.date <= today).cloned())
        }
        fn try_per_unit(&self, c: &Code, _: Date) -> Result<Option<Decimal>, StorageError> {
            if c.as_str() == "TRY" {
                return Ok(Some(Decimal::ONE));
            }
            Ok(self.rates.get(c.as_str()).copied())
        }
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }
    fn c(s: &str) -> Code {
        Code::parse(s).unwrap()
    }
    fn day(s: &str) -> Date {
        Date::parse(s).unwrap()
    }
    fn fake() -> Fake {
        Fake {
            quotes: HashMap::new(),
            rates: HashMap::from([("USD".into(), d("34.00")), ("EUR".into(), d("37.00"))]),
        }
    }

    #[test]
    fn position_value_with_minor_unit_rounding() {
        assert_eq!(money(d("1000") * d("1.234567"), &c("TRY")), "1234.57");
    }

    #[test]
    fn cross_rate_through_try() {
        let v = convert(
            &fake(),
            d("100.00"),
            &c("USD"),
            &c("EUR"),
            day("2026-11-01"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(money(v, &c("EUR")), "91.89");
        assert_eq!(
            convert(&fake(), d("5"), &c("TRY"), &c("TRY"), day("2026-11-01")).unwrap(),
            Some(d("5"))
        );
        assert_eq!(
            money(
                convert(&fake(), d("34"), &c("TRY"), &c("USD"), day("2026-11-01"))
                    .unwrap()
                    .unwrap(),
                &c("USD")
            ),
            "1.00"
        );
        assert_eq!(
            convert(&fake(), d("1"), &c("JPY"), &c("TRY"), day("2026-11-01")).unwrap(),
            None
        );
    }

    #[test]
    fn staleness() {
        assert!(
            !is_stale(day("2026-10-25"), day("2026-11-01")),
            "7 days is not stale"
        );
        assert!(
            is_stale(day("2026-10-20"), day("2026-11-01")),
            "12 days is stale"
        );
    }

    #[test]
    fn gains() {
        let try_ = c("TRY");
        assert_eq!(
            unrealized(Some((d("1000.00"), &try_)), false, d("1234.567"), &try_),
            Gain::Amount {
                amount: d("234.567"),
                pct: d("23.4567")
            }
        );
        if let Gain::Amount { amount, pct } =
            unrealized(Some((d("1000.00"), &try_)), false, d("1234.567"), &try_)
        {
            assert_eq!(
                (signed_money(amount, &try_), signed_pct(pct)),
                ("+234.57".into(), "+23.5%".into())
            );
        }
        assert_eq!(
            unrealized(None, true, d("1"), &try_),
            Gain::NotAvailable("unknown cost basis")
        );
        assert_eq!(
            unrealized(Some((d("1"), &c("USD"))), false, d("2"), &try_),
            Gain::NotAvailable("cost and price in different currencies")
        );
        assert_eq!(signed_money(d("-3.456"), &try_), "−3.46");
    }

    #[test]
    fn totals_count_exclusions() {
        let m = fake();
        let today = day("2026-11-01");
        let mut t = Total::new(c("TRY"));
        t.add_priced(&m, d("100"), &c("USD"), day("2026-10-30"), today)
            .unwrap();
        t.add_priced(&m, d("50"), &c("TRY"), day("2026-10-28"), today)
            .unwrap();
        t.add(&m, d("10"), &c("TRY"), today).unwrap(); // cash
        t.no_price();
        assert!(t.add(&m, d("1"), &c("JPY"), today).unwrap().is_none());
        assert_eq!(t.sum, d("3460.00"));
        assert_eq!((t.excluded_no_price, t.excluded_no_rate), (1, 1));
        assert_eq!(t.oldest_price, Some(day("2026-10-28")));
    }
}
