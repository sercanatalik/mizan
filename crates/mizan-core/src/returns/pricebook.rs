//! Every stored price and rate, loaded once, with "newest on or before day D" lookups.
//!
//! Sources, in order of precedence on the same date: manual prices, provider
//! prices in the user database, then the shared market-history cache (TEFAS
//! fund prices through each asset's fund code, and exchange rates).

use std::collections::HashMap;
use std::str::FromStr;

use mizan_import::{Code, Date};
use mizan_storage::{HistoryCache, PriceSourceKind, StorageError, Store, MANUAL};
use rust_decimal::Decimal;

/// A dated price in a currency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatedPrice {
    pub date: Date,
    pub value: Decimal,
    pub currency: Code,
}

/// Newest entry dated on or before `day` in a series sorted by date.
pub fn on_or_before<T>(series: &[T], day: Date, date_of: impl Fn(&T) -> Date) -> Option<&T> {
    let n = series.partition_point(|e| date_of(e) <= day);
    n.checked_sub(1).map(|i| &series[i])
}

#[derive(Clone, Debug, Default)]
pub struct PriceBook {
    /// Canonical instrument key → prices sorted by date (one per date).
    prices: HashMap<String, Vec<DatedPrice>>,
    /// Currency → (date, TRY per unit) sorted by date.
    rates: HashMap<Code, Vec<(Date, Decimal)>>,
}

fn dec(s: &str) -> Option<Decimal> {
    Decimal::from_str(s).ok()
}

/// Values by date, each with the rank of its source (lower wins).
type Ranked<V> = HashMap<Date, (u8, V)>;

/// Insert `(date, value)` keeping one entry per date; `rank` lower wins.
fn put<V>(map: &mut HashMap<Date, (u8, V)>, date: Date, rank: u8, v: V) {
    match map.get(&date) {
        Some((r, _)) if *r <= rank => {}
        _ => {
            map.insert(date, (rank, v));
        }
    }
}

impl PriceBook {
    /// Load every price of every asset and every rate.
    pub fn load(store: &Store, cache: Option<&HistoryCache>) -> Result<Self, StorageError> {
        let keys = store.key_map()?;
        let sources = store.price_sources()?;
        let mut by_key: HashMap<String, Ranked<(Decimal, Code)>> = HashMap::new();
        for a in store.list_assets()? {
            let key = keys
                .get(&a.instrument_key)
                .cloned()
                .unwrap_or_else(|| a.instrument_key.clone());
            let slot = by_key.entry(key).or_default();
            for p in store.prices_for(a.id)? {
                let (Ok(date), Some(v), Ok(c)) = (
                    Date::parse(&p.date),
                    dec(&p.value),
                    Code::parse(&p.currency),
                ) else {
                    continue;
                };
                let rank = if p.source == MANUAL { 0 } else { 1 };
                put(slot, date, rank, (v, c));
            }
            match (cache, sources.get(&a.id)) {
                (Some(cache), Some(PriceSourceKind::Tefas { fund_code })) => {
                    let code = fund_code.trim().to_uppercase();
                    for p in cache.fund_prices(&code)? {
                        if let (Ok(date), Some(v)) = (Date::parse(&p.date), dec(&p.value)) {
                            put(slot, date, 2, (v, Code::parse("TRY").expect("valid")));
                        }
                    }
                }
                (Some(cache), Some(src)) => {
                    if let Some((provider, symbol)) = src.listed() {
                        let symbol = symbol.trim().to_uppercase();
                        for p in cache.listed_prices(provider, &symbol)? {
                            if let (Ok(date), Some(v), Ok(c)) = (
                                Date::parse(&p.date),
                                dec(&p.close),
                                Code::parse(&p.currency),
                            ) {
                                put(slot, date, 2, (v, c));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let prices = by_key
            .into_iter()
            .map(|(k, m)| {
                let mut v: Vec<DatedPrice> = m
                    .into_iter()
                    .map(|(date, (_, (value, currency)))| DatedPrice {
                        date,
                        value,
                        currency,
                    })
                    .collect();
                v.sort_by_key(|p| p.date);
                (k, v)
            })
            .collect();

        let mut currencies: Vec<String> = store.fx_currencies()?;
        let mut rates_by: HashMap<Code, HashMap<Date, (u8, Decimal)>> = HashMap::new();
        if let Some(cache) = cache {
            currencies.extend(cache.rate_currencies()?);
        }
        currencies.sort();
        currencies.dedup();
        for c in currencies {
            let Ok(code) = Code::parse(&c) else { continue };
            let slot = rates_by.entry(code).or_default();
            for r in store.fx_rates_for(&c)? {
                if let (Ok(d), Some(v)) = (Date::parse(&r.date), dec(&r.try_per_unit)) {
                    put(slot, d, 0, v);
                }
            }
            if let Some(cache) = cache {
                for r in cache.rates(&c)? {
                    if let (Ok(d), Some(v)) = (Date::parse(&r.date), dec(&r.value)) {
                        put(slot, d, 1, v);
                    }
                }
            }
        }
        let rates = rates_by
            .into_iter()
            .map(|(c, m)| {
                let mut v: Vec<(Date, Decimal)> = m.into_iter().map(|(d, (_, r))| (d, r)).collect();
                v.sort_by_key(|(d, _)| *d);
                (c, v)
            })
            .collect();
        Ok(PriceBook { prices, rates })
    }

    /// An empty book with the given prices and rates (tests).
    pub fn from_parts(
        prices: HashMap<String, Vec<DatedPrice>>,
        rates: HashMap<Code, Vec<(Date, Decimal)>>,
    ) -> Self {
        let mut b = PriceBook { prices, rates };
        for v in b.prices.values_mut() {
            v.sort_by_key(|p| p.date);
        }
        for v in b.rates.values_mut() {
            v.sort_by_key(|(d, _)| *d);
        }
        b
    }

    /// Newest price of an instrument dated on or before `day`.
    pub fn price_on(&self, key: &str, day: Date) -> Option<&DatedPrice> {
        on_or_before(self.prices.get(key)?, day, |p| p.date)
    }

    /// Prices of an instrument, oldest first.
    pub fn prices(&self, key: &str) -> &[DatedPrice] {
        self.prices.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// TRY per one unit of `currency`, newest dated on or before `day`. TRY is 1.
    pub fn try_per_unit(&self, currency: &Code, day: Date) -> Option<Decimal> {
        if currency.as_str() == "TRY" {
            return Some(Decimal::ONE);
        }
        on_or_before(self.rates.get(currency)?, day, |(d, _)| *d).map(|(_, r)| *r)
    }

    /// TRY per one unit of `currency` with the date it was published, newest
    /// on or before `day`. TRY is 1, dated `day`.
    pub fn dated_rate(&self, currency: &Code, day: Date) -> Option<(Date, Decimal)> {
        if currency.as_str() == "TRY" {
            return Some((day, Decimal::ONE));
        }
        on_or_before(self.rates.get(currency)?, day, |(d, _)| *d).copied()
    }

    /// Convert `amount` between currencies through TRY with rates for `day`.
    pub fn convert(&self, amount: Decimal, from: &Code, to: &Code, day: Date) -> Option<Decimal> {
        if from == to {
            return Some(amount);
        }
        let f = self.try_per_unit(from, day)?;
        let t = self.try_per_unit(to, day)?;
        (!t.is_zero()).then(|| amount * f / t)
    }

    /// Instrument keys with at least one price.
    pub fn priced_keys(&self) -> impl Iterator<Item = &str> {
        self.prices
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, _)| k.as_str())
    }
}
