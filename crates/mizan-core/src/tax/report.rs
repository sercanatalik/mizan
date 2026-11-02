//! The Tax screen's read model: a country's report for one tax year, built
//! from the user database and the history cache only.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use mizan_import::Date;
use mizan_storage::inflation::PPI_TR;
use mizan_storage::{
    AccountId, HistoryCache, PriceSourceKind, StorageError, Store, StoredActivity,
};
use rust_decimal::Decimal;
use serde::Serialize;

use super::events::{events, Event, EventKind, TaxAccount};
use super::income::{income_rows, totals as income_totals, IncomeKind, IncomeRow};
use super::rates::Rates;
use super::rows::{round2, GainRow, Rule, Term, Treatments};
use super::tr::MonthlyIndex;
use super::{tr, uk, us, Country, TaxTreatment, TaxYear};
use crate::replay::KeyMap;
use crate::returns::PriceBook;

pub const NOTICE: &str = "This is a working paper to help you prepare your tax return, not tax advice. Mizan does not work out tax owed, allowances, exemptions or whether you must file. Check every figure before you use it.";

/// Where a security is from, for Turkish reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// TR-prefixed ISIN or a TEFAS price source: taxed at source.
    Domestic,
    /// Another ISIN: declared.
    Foreign,
    /// No ISIN and no TEFAS price source.
    Unknown,
}

/// What the report needs to know about an instrument.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstrumentInfo {
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub isin: Option<String>,
    pub tefas: bool,
}

impl InstrumentInfo {
    pub fn origin(&self) -> Origin {
        match &self.isin {
            _ if self.tefas => Origin::Domestic,
            Some(i) if i.to_uppercase().starts_with("TR") => Origin::Domestic,
            Some(_) => Origin::Foreign,
            None => Origin::Unknown,
        }
    }

    fn description(&self, key: &str) -> String {
        match (&self.name, &self.symbol) {
            (Some(n), Some(s)) if n != s => format!("{n} ({s})"),
            (Some(n), _) => n.clone(),
            (None, Some(s)) => s.clone(),
            (None, None) => key
                .trim_start_matches(|c| c != ':')
                .trim_start_matches(':')
                .to_owned(),
        }
    }
}

/// Everything a report reads, loaded once.
pub struct TaxInputs {
    pub accounts: Vec<TaxAccount>,
    pub activities: Vec<(AccountId, Vec<StoredActivity>)>,
    pub keys: KeyMap,
    pub book: PriceBook,
    pub index: MonthlyIndex,
    pub instruments: HashMap<String, InstrumentInfo>,
}

impl TaxInputs {
    pub fn load(store: &Store, cache: Option<&HistoryCache>) -> Result<Self, StorageError> {
        let keys = store.key_map()?;
        let canon = |k: &str| keys.get(k).cloned().unwrap_or_else(|| k.to_owned());
        let mut accounts = Vec::new();
        let mut activities = Vec::new();
        for a in store.list_accounts()? {
            activities.push((a.id, store.all_activities(a.id)?));
            accounts.push(TaxAccount {
                id: a.id,
                name: a.name,
                treatment: a.tax_treatment,
            });
        }
        let tefas: BTreeSet<i64> = store
            .price_sources()?
            .into_iter()
            .filter(|(_, s)| matches!(s, PriceSourceKind::Tefas { .. }))
            .map(|(id, _)| id)
            .collect();
        let mut instruments: HashMap<String, InstrumentInfo> = HashMap::new();
        for a in store.list_assets()? {
            let info = instruments.entry(canon(&a.instrument_key)).or_default();
            let clean = |s: &Option<String>| {
                s.as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            info.symbol = info.symbol.take().or_else(|| clean(&a.hint.symbol));
            info.name = info
                .name
                .take()
                .or_else(|| clean(&a.name))
                .or_else(|| clean(&a.hint.name));
            // A TR ISIN on any merged asset makes the instrument domestic.
            match (clean(&info.isin), clean(&a.hint.isin)) {
                (None, i) => info.isin = i,
                (Some(old), Some(new))
                    if new.to_uppercase().starts_with("TR")
                        && !old.to_uppercase().starts_with("TR") =>
                {
                    info.isin = Some(new)
                }
                _ => {}
            }
            info.tefas |= tefas.contains(&a.id);
        }
        let index: MonthlyIndex =
            crate::returns::inflation::monthly_values(store, cache, PPI_TR.source, PPI_TR.id)?;
        Ok(TaxInputs {
            accounts,
            activities,
            keys,
            book: PriceBook::load(store, cache)?,
            index,
            instruments,
        })
    }

    fn treatments(&self) -> Treatments {
        Treatments(self.accounts.iter().map(|a| (a.id, a.treatment)).collect())
    }

    fn info(&self, key: &str) -> InstrumentInfo {
        self.instruments.get(key).cloned().unwrap_or_else(|| {
            // An instrument key carries its ISIN when it has one.
            InstrumentInfo {
                isin: key.strip_prefix("isin:").map(str::to_owned),
                ..Default::default()
            }
        })
    }

    fn account_name(&self, id: Option<AccountId>) -> Option<String> {
        match id {
            Some(id) => self
                .accounts
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.name.clone()),
            None => Some("several accounts".into()),
        }
    }
}

// ---------------------------------------------------------------- views

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TaxYearOption {
    pub label: String,
    pub first_day: String,
    pub last_day: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TaxYears {
    pub country: Country,
    /// Newest first.
    pub years: Vec<TaxYearOption>,
    /// The most recent year that has ended, else the newest.
    pub default: Option<String>,
    /// The tax year containing today.
    pub current: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TaxAccountView {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: i64,
    pub name: String,
    /// `taxable`, `isa` or `pension`.
    pub treatment: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GainRowView {
    pub description: String,
    pub symbol: Option<String>,
    pub isin: Option<String>,
    pub account: Option<String>,
    pub date_acquired: Option<String>,
    pub date_sold: String,
    pub quantity: String,
    pub proceeds: Option<String>,
    pub cost: Option<String>,
    pub indexed_cost: Option<String>,
    pub adjustment_code: Option<String>,
    pub adjustment: Option<String>,
    pub gain: Option<String>,
    /// UK: "same day", "30 days" or "pool".
    pub rule: Option<String>,
    /// US: "short" or "long".
    pub term: Option<String>,
    /// Why the row has no gain.
    pub reason: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GainTotals {
    /// Disposals with a computed gain.
    pub disposals: u32,
    pub proceeds: String,
    pub cost: String,
    pub adjustments: String,
    pub gains: String,
    pub losses: String,
    pub net: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GainSection {
    /// `disposals`, `short`, `long`, `declared`, `taxed_at_source`, `origin_unknown`.
    pub id: String,
    pub title: String,
    pub note: Option<String>,
    /// Whether this section counts towards the report's totals.
    pub counted: bool,
    pub rows: Vec<GainRowView>,
    pub totals: GainTotals,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IncomeRowView {
    pub date: String,
    /// `dividends` or `interest`.
    pub kind: String,
    pub description: String,
    pub symbol: Option<String>,
    pub isin: Option<String>,
    pub account: Option<String>,
    pub gross: Option<String>,
    pub withheld: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IncomeTotal {
    pub kind: String,
    pub gross: String,
    pub withheld: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IncomeSection {
    /// `income`, `declared`, `taxed_at_source`, `origin_unknown`.
    pub id: String,
    pub title: String,
    pub note: Option<String>,
    pub counted: bool,
    pub rows: Vec<IncomeRowView>,
    pub totals: Vec<IncomeTotal>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SummaryFigure {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReasonCount {
    pub reason: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TaxReport {
    pub country: Country,
    pub year: String,
    pub first_day: String,
    pub last_day: String,
    pub currency: String,
    pub notice: String,
    pub computed_at: String,
    pub rate_source: String,
    /// Rows (gains and income) that could not be computed, by reason.
    pub not_computed: Vec<ReasonCount>,
    /// Totals leave out rows that could not be computed.
    pub incomplete: bool,
    pub summary: Vec<SummaryFigure>,
    pub gains: Vec<GainSection>,
    pub income: Vec<IncomeSection>,
    pub notes: Vec<String>,
    /// Every account with its tax treatment.
    pub accounts: Vec<TaxAccountView>,
    /// Accounts left out of the report (ISA and pension).
    pub excluded: Vec<TaxAccountView>,
    /// Whether any taxable activity exists at all.
    pub has_activity: bool,
}

/// Plain decimal with two places, for example `-1234.50`.
fn s(d: Decimal) -> String {
    let mut r = round2(d);
    if r.is_zero() {
        r = Decimal::ZERO;
    }
    r.rescale(2);
    r.to_string()
}

fn opt(d: Option<Decimal>) -> Option<String> {
    d.map(s)
}

fn account_view(a: &TaxAccount) -> TaxAccountView {
    TaxAccountView {
        id: a.id,
        name: a.name.clone(),
        treatment: a.treatment.as_str().into(),
    }
}

fn row_view(inputs: &TaxInputs, r: &GainRow) -> GainRowView {
    let info = inputs.info(&r.key);
    GainRowView {
        description: info.description(&r.key),
        symbol: info.symbol.clone(),
        isin: info.isin.clone(),
        account: inputs.account_name(r.account),
        date_acquired: r.acquired.map(|d| d.to_string()),
        date_sold: r.sold.to_string(),
        quantity: r.qty.normalize().to_string(),
        proceeds: opt(r.proceeds),
        cost: opt(r.cost),
        indexed_cost: opt(r.indexed_cost),
        adjustment_code: r.code.map(str::to_owned),
        adjustment: opt(r.adjustment),
        gain: opt(r.gain),
        rule: (r.rule != Rule::Fifo).then(|| r.rule.label().to_owned()),
        term: r.term.map(|t| match t {
            Term::Short => "short".into(),
            Term::Long => "long".into(),
        }),
        reason: r.missing.as_ref().map(|m| m.reason()),
        notes: r.notes.clone(),
    }
}

/// Totals over computed rows. Gains and losses are counted per disposal
/// (instrument and date), so a disposal split across matching rules or lots
/// counts once.
fn gain_totals(rows: &[&GainRow]) -> GainTotals {
    let ok: Vec<&&GainRow> = rows.iter().filter(|r| r.gain.is_some()).collect();
    let mut per: BTreeMap<(Date, &str), Decimal> = BTreeMap::new();
    for r in &ok {
        *per.entry((r.sold, r.key.as_str())).or_default() += r.gain.unwrap();
    }
    let sum =
        |f: &dyn Fn(&GainRow) -> Option<Decimal>| ok.iter().filter_map(|r| f(r)).sum::<Decimal>();
    let gains: Decimal = per.values().filter(|g| **g > Decimal::ZERO).sum();
    let losses: Decimal = -per
        .values()
        .filter(|g| **g < Decimal::ZERO)
        .sum::<Decimal>();
    GainTotals {
        disposals: per.len() as u32,
        proceeds: s(sum(&|r| r.proceeds)),
        cost: s(sum(&|r| r.indexed_cost.or(r.cost))),
        adjustments: s(sum(&|r| r.adjustment)),
        gains: s(gains),
        losses: s(losses),
        net: s(gains - losses),
    }
}

fn section(
    inputs: &TaxInputs,
    id: &str,
    title: &str,
    note: Option<&str>,
    counted: bool,
    rows: &[&GainRow],
) -> GainSection {
    GainSection {
        id: id.into(),
        title: title.into(),
        note: note.map(str::to_owned),
        counted,
        rows: rows.iter().map(|r| row_view(inputs, r)).collect(),
        totals: gain_totals(rows),
    }
}

fn income_section(
    inputs: &TaxInputs,
    id: &str,
    title: &str,
    note: Option<&str>,
    counted: bool,
    rows: &[&IncomeRow],
) -> IncomeSection {
    let mut totals: Vec<(IncomeKind, (Decimal, Decimal))> =
        income_totals(rows).into_iter().collect();
    totals.sort_by_key(|(k, _)| *k);
    IncomeSection {
        id: id.into(),
        title: title.into(),
        note: note.map(str::to_owned),
        counted,
        rows: rows
            .iter()
            .map(|r| {
                let info = r.key.as_deref().map(|k| (inputs.info(k), k));
                IncomeRowView {
                    date: r.date.to_string(),
                    kind: r.kind.label().into(),
                    description: match &info {
                        Some((i, k)) => i.description(k),
                        None => "Cash".into(),
                    },
                    symbol: info.as_ref().and_then(|(i, _)| i.symbol.clone()),
                    isin: info.as_ref().and_then(|(i, _)| i.isin.clone()),
                    account: inputs.account_name(Some(r.account)),
                    gross: opt(r.gross),
                    withheld: opt(r.withheld),
                    reason: r.missing.as_ref().map(|m| m.reason()),
                }
            })
            .collect(),
        totals: totals
            .into_iter()
            .map(|(k, (g, w))| IncomeTotal {
                kind: k.label().into(),
                gross: s(g),
                withheld: s(w),
            })
            .collect(),
    }
}

fn is_taxable_activity(e: &Event) -> bool {
    matches!(
        e.kind,
        EventKind::Dispose { .. } | EventKind::NotCovered { disposal: true, .. }
    )
}

fn income_kind(r: &mizan_import::ImportRecord) -> bool {
    matches!(
        r.kind,
        mizan_import::Kind::Dividend
            | mizan_import::Kind::Distribution
            | mizan_import::Kind::Interest
    )
}

/// Tax years with a disposal or income in a taxable account, newest first.
pub fn years(inputs: &TaxInputs, country: Country, today: Date) -> TaxYears {
    let t = inputs.treatments();
    let mut set: BTreeSet<TaxYear> = BTreeSet::new();
    for e in events(&inputs.activities, &inputs.keys) {
        if t.taxable(e.account) && is_taxable_activity(&e) {
            set.insert(country.year_of(e.date));
        }
    }
    for (account, stored) in &inputs.activities {
        if t.taxable(*account) {
            for s in stored.iter().filter(|s| income_kind(&s.record)) {
                set.insert(country.year_of(s.record.date));
            }
        }
    }
    let years: Vec<TaxYear> = set.into_iter().rev().collect();
    let default = years
        .iter()
        .find(|y| y.last_day() < today)
        .or(years.first())
        .map(|y| y.label());
    TaxYears {
        country,
        years: years
            .iter()
            .map(|y| TaxYearOption {
                label: y.label(),
                first_day: y.first_day().to_string(),
                last_day: y.last_day().to_string(),
            })
            .collect(),
        default,
        current: country.year_of(today).label(),
    }
}

/// Build one country's report for one tax year.
pub fn report(inputs: &TaxInputs, country: Country, year: TaxYear, computed_at: &str) -> TaxReport {
    let t = inputs.treatments();
    let rates = Rates::new(&inputs.book);
    let ev = events(&inputs.activities, &inputs.keys);
    let gains = match country {
        Country::Uk => uk::match_disposals(&ev, &t, &rates, year),
        Country::Us => us::match_disposals(&ev, &t, &rates, year),
        Country::Tr => tr::match_disposals(&ev, &t, &rates, &inputs.index, year),
    };
    let income = income_rows(&inputs.activities, &inputs.keys, &t, &rates, country, year);
    let all_gains: Vec<&GainRow> = gains.iter().collect();
    let all_income: Vec<&IncomeRow> = income.iter().collect();
    let origin = |key: &str| inputs.info(key).origin();
    let income_origin = |r: &IncomeRow| r.key.as_deref().map(origin).unwrap_or(Origin::Unknown);

    let mut notes = Vec::new();
    let (gain_sections, income_sections, rate_source) = match country {
        Country::Uk => (
            vec![section(inputs, "disposals", "Disposals", None, true, &all_gains)],
            vec![income_section(inputs, "income", "Dividends and interest", None, true, &all_income)],
            "TCMB forex buying rates crossed through TRY on each transaction date; a statement's own rate where it converts directly into GBP",
        ),
        Country::Us => {
            notes.push("The Form 8949 box (A to F) depends on what your broker reported to the IRS; Mizan does not choose it.".into());
            notes.push("Whether dividends are qualified is not determined.".into());
            let short: Vec<&GainRow> = gains.iter().filter(|r| r.term != Some(Term::Long)).collect();
            let long: Vec<&GainRow> = gains.iter().filter(|r| r.term == Some(Term::Long)).collect();
            (
                vec![
                    section(inputs, "short", "Short-term (held one year or less)", None, true, &short),
                    section(inputs, "long", "Long-term (held more than one year)", None, true, &long),
                ],
                vec![income_section(inputs, "income", "Dividends and interest", None, true, &all_income)],
                "TCMB forex buying rates crossed through TRY on each transaction date; a statement's own rate where it converts directly into USD",
            )
        }
        Country::Tr => {
            let by = |o: Origin| gains.iter().filter(|r| origin(&r.key) == o).collect::<Vec<_>>();
            let inc = |o: Origin| income.iter().filter(|r| income_origin(r) == o).collect::<Vec<_>>();
            let source_note = Some("Turkish funds and Borsa Istanbul securities are taxed at source and are not added to the totals.");
            let unknown_note = Some("These have no ISIN and no TEFAS price source, so Mizan cannot tell whether they are Turkish or foreign. They are not added to the totals.");
            (
                vec![
                    section(inputs, "declared", "Foreign securities", None, true, &by(Origin::Foreign)),
                    section(inputs, "taxed_at_source", "Taxed at source", source_note, false, &by(Origin::Domestic)),
                    section(inputs, "origin_unknown", "Origin unknown", unknown_note, false, &by(Origin::Unknown)),
                ],
                vec![
                    income_section(inputs, "declared", "Foreign dividends and interest", None, true, &inc(Origin::Foreign)),
                    income_section(inputs, "taxed_at_source", "Taxed at source", source_note, false, &inc(Origin::Domestic)),
                    income_section(inputs, "origin_unknown", "Origin unknown", unknown_note, false, &inc(Origin::Unknown)),
                ],
                "TCMB forex buying rates on each transaction date",
            )
        }
    };

    // Rows that could not be computed, in every section.
    let mut reasons: BTreeMap<String, u32> = BTreeMap::new();
    for r in gains.iter().filter_map(|r| r.missing.as_ref()) {
        *reasons.entry(r.reason()).or_default() += 1;
    }
    for r in income.iter().filter_map(|r| r.missing.as_ref()) {
        *reasons.entry(r.reason()).or_default() += 1;
    }
    let incomplete = !reasons.is_empty();

    let counted: Vec<&GainRow> = match country {
        Country::Tr => gains
            .iter()
            .filter(|r| origin(&r.key) == Origin::Foreign)
            .collect(),
        _ => all_gains.clone(),
    };
    let tot = gain_totals(&counted);
    let ccy = country.currency().to_string();
    let fig = |label: &str, value: String| SummaryFigure {
        label: label.into(),
        value,
    };
    let mut summary = vec![
        fig("Disposals", tot.disposals.to_string()),
        fig("Proceeds", tot.proceeds.clone()),
    ];
    match country {
        Country::Uk => summary.push(fig("Allowable costs", tot.cost.clone())),
        Country::Us => {
            summary.push(fig("Cost basis", tot.cost.clone()));
            summary.push(fig("Wash-sale adjustments", tot.adjustments.clone()));
        }
        Country::Tr => summary.push(fig("Cost (indexed where applied)", tot.cost.clone())),
    }
    summary.push(fig("Gains", tot.gains.clone()));
    summary.push(fig("Losses", tot.losses.clone()));
    summary.push(fig("Net gain", tot.net.clone()));
    let counted_income: Vec<&IncomeRow> = match country {
        Country::Tr => income
            .iter()
            .filter(|r| income_origin(r) == Origin::Foreign)
            .collect(),
        _ => all_income.clone(),
    };
    let it = income_totals(&counted_income);
    for k in [IncomeKind::Dividend, IncomeKind::Interest] {
        let (g, w) = it.get(&k).copied().unwrap_or_default();
        let name = match k {
            IncomeKind::Dividend => "Dividends",
            IncomeKind::Interest => "Interest",
        };
        summary.push(fig(name, s(g)));
        if !w.is_zero() {
            summary.push(fig(&format!("{name}: tax withheld"), s(w)));
        }
    }

    let has_activity = !years(inputs, country, year.last_day()).years.is_empty();
    TaxReport {
        country,
        year: year.label(),
        first_day: year.first_day().to_string(),
        last_day: year.last_day().to_string(),
        currency: ccy,
        notice: NOTICE.into(),
        computed_at: computed_at.into(),
        rate_source: rate_source.into(),
        not_computed: reasons
            .into_iter()
            .map(|(reason, count)| ReasonCount { reason, count })
            .collect(),
        incomplete,
        summary,
        gains: gain_sections,
        income: income_sections,
        notes,
        accounts: inputs.accounts.iter().map(account_view).collect(),
        excluded: inputs
            .accounts
            .iter()
            .filter(|a| a.treatment != TaxTreatment::Taxable)
            .map(account_view)
            .collect(),
        has_activity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tax::testing::*;
    use mizan_import::{Charge, ChargeType, ImportRecord, Kind};

    fn inputs(records: Vec<ImportRecord>, rates: &[(&str, &str, &str)]) -> TaxInputs {
        let mut fx: HashMap<mizan_import::Code, Vec<(Date, Decimal)>> = HashMap::new();
        for (c, dt, r) in rates {
            fx.entry(code(c)).or_default().push((day(dt), d(r)));
        }
        TaxInputs {
            accounts: vec![TaxAccount {
                id: 1,
                name: "Broker".into(),
                treatment: TaxTreatment::Taxable,
            }],
            activities: vec![(1, records.into_iter().map(|r| stored(1, r)).collect())],
            keys: KeyMap::new(),
            book: PriceBook::from_parts(HashMap::new(), fx),
            index: MonthlyIndex::new(),
            instruments: HashMap::new(),
        }
    }

    fn isin(kind: Kind, date: &str, isin: &str, qty: &str, total: &str, ccy: &str) -> ImportRecord {
        let mut r = deal(kind, date, "", qty, total, ccy);
        r.instrument = Some(mizan_import::InstrumentHint {
            isin: Some(isin.into()),
            ..Default::default()
        });
        r
    }

    fn fig<'a>(r: &'a TaxReport, label: &str) -> &'a str {
        &r.summary.iter().find(|f| f.label == label).unwrap().value
    }

    fn year(c: Country, y: &str) -> TaxYear {
        TaxYear::parse(c, y).unwrap()
    }

    #[test]
    fn uk_summary_figures() {
        let i = inputs(
            vec![
                deal(Kind::Buy, "2024-05-01", "A", "10", "1000", "GBP"),
                deal(Kind::Sell, "2025-06-01", "A", "10", "1600", "GBP"),
                deal(Kind::Buy, "2024-05-01", "B", "10", "1000", "GBP"),
                deal(Kind::Sell, "2025-07-01", "B", "10", "750", "GBP"),
            ],
            &[],
        );
        let r = report(
            &i,
            Country::Uk,
            year(Country::Uk, "2025-26"),
            "2026-11-02T10:00:00Z",
        );
        assert_eq!(r.currency, "GBP");
        assert_eq!(fig(&r, "Disposals"), "2");
        assert_eq!(fig(&r, "Gains"), "600.00");
        assert_eq!(fig(&r, "Losses"), "250.00");
        assert_eq!(fig(&r, "Net gain"), "350.00");
        assert_eq!(r.gains[0].rows[0].rule.as_deref(), Some("pool"));
        assert!(!r.incomplete);
        assert_eq!(r.notice, NOTICE);
        assert!(r
            .summary
            .iter()
            .all(|f| !f.label.to_lowercase().contains("tax owed")));
    }

    #[test]
    fn us_short_and_long_groups_and_wash_row() {
        let i = inputs(
            vec![
                deal(Kind::Buy, "2023-01-10", "X", "10", "1000", "USD"),
                deal(Kind::Buy, "2024-06-01", "X", "10", "1500", "USD"),
                deal(Kind::Sell, "2025-02-01", "X", "15", "3000", "USD"),
                deal(Kind::Buy, "2025-01-02", "W", "10", "1000", "USD"),
                deal(Kind::Sell, "2025-06-02", "W", "10", "800", "USD"),
                deal(Kind::Buy, "2025-06-20", "W", "10", "850", "USD"),
            ],
            &[],
        );
        let r = report(&i, Country::Us, year(Country::Us, "2025"), "t");
        let short = &r.gains[0];
        let long = &r.gains[1];
        assert_eq!((short.id.as_str(), long.id.as_str()), ("short", "long"));
        assert_eq!(long.rows.len(), 1);
        assert_eq!(long.totals.net, "1000.00");
        let w = short
            .rows
            .iter()
            .find(|x| x.adjustment_code.is_some())
            .unwrap();
        assert_eq!(
            (w.adjustment_code.as_deref(), w.adjustment.as_deref()),
            (Some("W"), Some("200.00"))
        );
        assert!(r.notes.iter().any(|n| n.contains("qualified")));
        assert!(r.notes.iter().any(|n| n.contains("box")));
    }

    #[test]
    fn turkish_sections_by_origin() {
        let i = inputs(
            vec![
                isin(Kind::Buy, "2024-01-02", "TRATHYO91Q19", "10", "1000", "TRY"),
                isin(
                    Kind::Sell,
                    "2025-03-03",
                    "TRATHYO91Q19",
                    "10",
                    "1500",
                    "TRY",
                ),
                isin(Kind::Buy, "2024-01-02", "US0378331005", "10", "1000", "TRY"),
                isin(
                    Kind::Sell,
                    "2025-03-03",
                    "US0378331005",
                    "10",
                    "1200",
                    "TRY",
                ),
                deal(Kind::Buy, "2024-01-02", "NOISIN", "10", "100", "TRY"),
                deal(Kind::Sell, "2025-03-03", "NOISIN", "10", "900", "TRY"),
            ],
            &[],
        );
        let r = report(&i, Country::Tr, year(Country::Tr, "2025"), "t");
        let ids: Vec<(&str, usize, bool)> = r
            .gains
            .iter()
            .map(|g| (g.id.as_str(), g.rows.len(), g.counted))
            .collect();
        assert_eq!(
            ids,
            [
                ("declared", 1, true),
                ("taxed_at_source", 1, false),
                ("origin_unknown", 1, false)
            ]
        );
        assert_eq!(r.gains[0].rows[0].isin.as_deref(), Some("US0378331005"));
        assert_eq!(
            fig(&r, "Net gain"),
            "200.00",
            "only the foreign share counts"
        );
        assert_eq!(fig(&r, "Disposals"), "1");
    }

    #[test]
    fn turkish_domestic_income_is_taxed_at_source() {
        let mut div = isin(
            Kind::Dividend,
            "2025-05-02",
            "TRATHYO91Q19",
            "1",
            "0",
            "TRY",
        );
        div.quantity = None;
        div.net_cash = Some(m("90", "TRY"));
        let mut foreign = isin(
            Kind::Dividend,
            "2025-05-02",
            "US0378331005",
            "1",
            "0",
            "TRY",
        );
        foreign.quantity = None;
        foreign.net_cash = Some(m("85", "TRY"));
        foreign.charges = vec![Charge {
            charge_type: ChargeType::WithholdingTax,
            amount: m("15", "TRY"),
        }];
        let i = inputs(vec![div, foreign], &[]);
        let r = report(&i, Country::Tr, year(Country::Tr, "2025"), "t");
        assert_eq!(r.income[0].rows.len(), 1);
        assert_eq!(r.income[1].id, "taxed_at_source");
        assert_eq!(r.income[1].rows.len(), 1);
        assert_eq!(fig(&r, "Dividends"), "100.00");
        assert_eq!(fig(&r, "Dividends: tax withheld"), "15.00");
    }

    #[test]
    fn incomplete_totals() {
        let inn = position(Kind::PositionIn, "2024-01-02", "A", "10");
        let i = inputs(
            vec![
                inn,
                deal(Kind::Sell, "2025-06-02", "A", "10", "500", "GBP"),
                deal(Kind::Buy, "2025-01-02", "B", "1", "100", "EUR"),
                deal(Kind::Sell, "2025-06-02", "B", "1", "200", "EUR"),
            ],
            &[],
        );
        let r = report(&i, Country::Us, year(Country::Us, "2025"), "t");
        assert!(r.incomplete);
        let reasons: Vec<(&str, u32)> = r
            .not_computed
            .iter()
            .map(|x| (x.reason.as_str(), x.count))
            .collect();
        assert_eq!(reasons, [("cost unknown", 1), ("no exchange rate", 1)]);
        let rows: usize = r.gains.iter().map(|g| g.rows.len()).sum();
        assert_eq!(rows, 2, "both rows stay visible");
        assert_eq!(fig(&r, "Disposals"), "0");
    }

    #[test]
    fn uk_tax_year_boundaries() {
        let i = inputs(
            vec![
                deal(Kind::Buy, "2024-01-02", "A", "30", "300", "GBP"),
                deal(Kind::Sell, "2025-04-05", "A", "10", "200", "GBP"),
                deal(Kind::Sell, "2025-04-06", "A", "10", "200", "GBP"),
                deal(Kind::Sell, "2026-04-05", "A", "5", "100", "GBP"),
                deal(Kind::Sell, "2026-04-06", "A", "5", "100", "GBP"),
            ],
            &[],
        );
        let r = report(&i, Country::Uk, year(Country::Uk, "2025-26"), "t");
        let dates: Vec<&str> = r.gains[0]
            .rows
            .iter()
            .map(|x| x.date_sold.as_str())
            .collect();
        assert_eq!(dates, ["2025-04-06", "2026-04-05"]);
        assert_eq!(
            (r.first_day.as_str(), r.last_day.as_str()),
            ("2025-04-06", "2026-04-05")
        );
    }

    #[test]
    fn default_year_is_the_last_ended() {
        let mut recs = vec![deal(Kind::Buy, "2023-01-02", "A", "40", "400", "USD")];
        for y in ["2023", "2024", "2025", "2026"] {
            recs.push(deal(
                Kind::Sell,
                &format!("{y}-06-01"),
                "A",
                "1",
                "20",
                "USD",
            ));
        }
        let i = inputs(recs, &[]);
        let y = years(&i, Country::Us, day("2026-11-02"));
        let labels: Vec<&str> = y.years.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, ["2026", "2025", "2024", "2023"]);
        assert_eq!(y.default.as_deref(), Some("2025"));
    }

    #[test]
    fn no_taxable_activity() {
        let i = inputs(
            vec![deal(Kind::Buy, "2023-01-02", "A", "40", "400", "USD")],
            &[],
        );
        let y = years(&i, Country::Us, day("2026-11-02"));
        assert!(y.years.is_empty() && y.default.is_none());
        assert_eq!(y.current, "2026");
        assert_eq!(years(&i, Country::Uk, day("2026-11-02")).current, "2026-27");
        let r = report(&i, Country::Us, year(Country::Us, "2025"), "t");
        assert!(!r.has_activity);
    }

    #[test]
    fn pension_account_is_excluded() {
        let mut i = inputs(
            vec![
                deal(Kind::Buy, "2024-01-02", "A", "10", "100", "TRY"),
                deal(Kind::Sell, "2025-06-02", "A", "10", "200", "TRY"),
            ],
            &[],
        );
        i.accounts[0].treatment = TaxTreatment::Pension;
        i.accounts[0].name = "Anadolu Hayat".into();
        let r = report(&i, Country::Tr, year(Country::Tr, "2025"), "t");
        assert!(r.gains.iter().all(|g| g.rows.is_empty()));
        assert_eq!(r.excluded.len(), 1);
        assert_eq!(
            (
                r.excluded[0].name.as_str(),
                r.excluded[0].treatment.as_str()
            ),
            ("Anadolu Hayat", "pension")
        );
    }
}
