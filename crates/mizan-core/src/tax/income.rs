//! Dividends, distributions and interest received in taxable accounts, with
//! tax withheld, converted into the report currency on the payment date.

use std::collections::HashMap;

use mizan_import::{Code, Date, Money};
use mizan_storage::{AccountId, StoredActivity};
use rust_decimal::Decimal;

use super::rates::Rates;
use super::rows::{Missing, Treatments};
use super::{Country, TaxYear};
use crate::income::payments;
pub use crate::income::{gross, IncomeKind};
use crate::replay::KeyMap;

#[derive(Clone, Debug, PartialEq)]
pub struct IncomeRow {
    /// Canonical instrument key; `None` for interest on cash.
    pub key: Option<String>,
    pub account: AccountId,
    pub date: Date,
    pub kind: IncomeKind,
    pub gross: Option<Decimal>,
    pub withheld: Option<Decimal>,
    pub missing: Option<Missing>,
}

/// Income rows dated in `year` from taxable accounts.
pub fn income_rows(
    accounts: &[(AccountId, Vec<StoredActivity>)],
    keys: &KeyMap,
    t: &Treatments,
    rates: &Rates,
    country: Country,
    year: TaxYear,
) -> Vec<IncomeRow> {
    let to: Code = country.currency();
    payments(accounts, keys)
        .into_iter()
        .filter(|p| t.taxable(p.account) && year.contains(p.date))
        .map(|p| {
            // Turkish reports use TCMB rates only; others prefer the statement's.
            let fx = if country == Country::Tr {
                None
            } else {
                p.fx.as_ref()
            };
            let conv = |m: &Money| {
                rates
                    .convert(m.amount.value(), &m.currency, &to, p.date, fx)
                    .ok_or(Missing::NoRate)
            };
            let g = p.gross.as_ref().ok_or(Missing::CostUnknown).and_then(conv);
            let w = p
                .withheld
                .iter()
                .map(conv)
                .try_fold(Decimal::ZERO, |acc, x| x.map(|v| acc + v));
            let missing = match (&g, &w) {
                (Err(e), _) | (_, Err(e)) => Some(e.clone()),
                _ => None,
            };
            IncomeRow {
                key: p.key,
                account: p.account,
                date: p.date,
                kind: p.kind,
                gross: g.ok(),
                withheld: w.ok(),
                missing,
            }
        })
        .collect()
}

/// Total gross and withheld per income kind, over rows that could be computed.
pub fn totals(rows: &[&IncomeRow]) -> HashMap<IncomeKind, (Decimal, Decimal)> {
    let mut t: HashMap<IncomeKind, (Decimal, Decimal)> = HashMap::new();
    for r in rows.iter().filter(|r| r.missing.is_none()) {
        let e = t.entry(r.kind).or_default();
        e.0 += r.gross.unwrap_or_default();
        e.1 += r.withheld.unwrap_or_default();
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::returns::PriceBook;
    use crate::tax::testing::*;
    use mizan_import::{Charge, ChargeType, ImportRecord, Kind, Link, LinkRole};

    fn book() -> PriceBook {
        let mut rates = HashMap::new();
        rates.insert(code("USD"), vec![(day("2025-03-01"), d("34.00"))]);
        rates.insert(code("GBP"), vec![(day("2025-03-01"), d("42.50"))]);
        PriceBook::from_parts(HashMap::new(), rates)
    }

    fn dividend() -> ImportRecord {
        let mut r = record(Kind::Dividend, "2025-03-03", "AAPL");
        r.net_cash = Some(m("85.00", "USD"));
        r.charges = vec![Charge {
            charge_type: ChargeType::WithholdingTax,
            amount: m("15.00", "USD"),
        }];
        r
    }

    fn rows(records: Vec<ImportRecord>, country: Country, year: &str) -> Vec<IncomeRow> {
        let b = book();
        income_rows(
            &[(1, records.into_iter().map(|r| stored(1, r)).collect())],
            &KeyMap::new(),
            &Treatments::default(),
            &Rates::new(&b),
            country,
            TaxYear::parse(country, year).unwrap(),
        )
    }

    #[test]
    fn dividend_with_withholding() {
        let us = rows(vec![dividend()], Country::Us, "2025");
        assert_eq!(
            (us[0].gross, us[0].withheld),
            (Some(d("100.00")), Some(d("15.00")))
        );
        let uk = rows(vec![dividend()], Country::Uk, "2024-25");
        assert_eq!(
            (uk[0].gross, uk[0].withheld),
            (Some(d("80")), Some(d("12")))
        );
        let tr = rows(vec![dividend()], Country::Tr, "2025");
        assert_eq!(
            (tr[0].gross, tr[0].withheld),
            (Some(d("3400.00")), Some(d("510.00")))
        );
    }

    #[test]
    fn linked_tax_activity_counts_as_withheld() {
        let mut div = record(Kind::Dividend, "2025-03-03", "AAPL");
        div.gross = Some(m("100.00", "USD"));
        div.net_cash = Some(m("100.00", "USD"));
        div.external_id = Some("D1".into());
        let mut tax = record(Kind::Tax, "2025-03-03", "AAPL");
        tax.net_cash = Some(m("15.00", "USD"));
        tax.links = vec![Link {
            role: LinkRole::Related,
            target: "D1".into(),
        }];
        let r = rows(vec![div, tax], Country::Us, "2025");
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].gross, r[0].withheld),
            (Some(d("100.00")), Some(d("15.00")))
        );
    }

    #[test]
    fn interest_without_instrument_and_missing_rate() {
        let mut i = record(Kind::Interest, "2025-07-01", "");
        i.net_cash = Some(m("10.00", "EUR"));
        let r = rows(vec![i], Country::Us, "2025");
        assert_eq!(r[0].key, None);
        assert_eq!(r[0].kind, IncomeKind::Interest);
        assert_eq!(r[0].missing, Some(Missing::NoRate));
    }
}
