//! CSV export of a report: one file for gains rows and one for income rows.
//!
//! RFC 4180: comma-separated, CRLF line ends, fields quoted when they hold a
//! comma, a quote or a line break. Dates are ISO and amounts plain decimals.
//! Rows that could not be computed are included with their reason.

use super::report::{GainRowView, IncomeRowView, TaxReport};
use super::Country;

fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn line(cells: &[&str]) -> String {
    let mut l = cells.iter().map(|c| field(c)).collect::<Vec<_>>().join(",");
    l.push_str("\r\n");
    l
}

fn o(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("")
}

/// Column headers of the gains file for a country.
pub fn gains_header(country: Country) -> &'static [&'static str] {
    match country {
        Country::Uk => &[
            "section",
            "date_sold",
            "description",
            "symbol",
            "isin",
            "account",
            "quantity",
            "rule",
            "date_acquired",
            "proceeds",
            "allowable_cost",
            "gain",
            "reason",
            "notes",
        ],
        // Form 8949 columns (a) to (h), then supporting detail.
        Country::Us => &[
            "section",
            "description",
            "date_acquired",
            "date_sold",
            "proceeds",
            "cost_basis",
            "adjustment_code",
            "adjustment",
            "gain",
            "quantity",
            "account",
            "reason",
            "notes",
        ],
        Country::Tr => &[
            "section",
            "description",
            "isin",
            "account",
            "quantity",
            "date_acquired",
            "date_sold",
            "cost",
            "indexed_cost",
            "proceeds",
            "gain",
            "reason",
            "notes",
        ],
    }
}

pub const INCOME_HEADER: [&str; 10] = [
    "section",
    "date",
    "type",
    "description",
    "symbol",
    "isin",
    "account",
    "gross",
    "tax_withheld",
    "reason",
];

fn gains_cells<'a>(
    country: Country,
    section: &'a str,
    r: &'a GainRowView,
    notes: &'a str,
) -> Vec<&'a str> {
    match country {
        Country::Uk => vec![
            section,
            &r.date_sold,
            &r.description,
            o(&r.symbol),
            o(&r.isin),
            o(&r.account),
            &r.quantity,
            o(&r.rule),
            o(&r.date_acquired),
            o(&r.proceeds),
            o(&r.cost),
            o(&r.gain),
            o(&r.reason),
            notes,
        ],
        Country::Us => vec![
            section,
            &r.description,
            o(&r.date_acquired),
            &r.date_sold,
            o(&r.proceeds),
            o(&r.cost),
            o(&r.adjustment_code),
            o(&r.adjustment),
            o(&r.gain),
            &r.quantity,
            o(&r.account),
            o(&r.reason),
            notes,
        ],
        Country::Tr => vec![
            section,
            &r.description,
            o(&r.isin),
            o(&r.account),
            &r.quantity,
            o(&r.date_acquired),
            &r.date_sold,
            o(&r.cost),
            o(&r.indexed_cost),
            o(&r.proceeds),
            o(&r.gain),
            o(&r.reason),
            notes,
        ],
    }
}

pub fn gains_csv(r: &TaxReport) -> String {
    let mut out = line(gains_header(r.country));
    for s in &r.gains {
        for row in &s.rows {
            let notes = row.notes.join("; ");
            out.push_str(&line(&gains_cells(r.country, &s.id, row, &notes)));
        }
    }
    out
}

fn income_cells<'a>(section: &'a str, r: &'a IncomeRowView) -> Vec<&'a str> {
    vec![
        section,
        &r.date,
        &r.kind,
        &r.description,
        o(&r.symbol),
        o(&r.isin),
        o(&r.account),
        o(&r.gross),
        o(&r.withheld),
        o(&r.reason),
    ]
}

pub fn income_csv(r: &TaxReport) -> String {
    let mut out = line(&INCOME_HEADER);
    for s in &r.income {
        for row in &s.rows {
            out.push_str(&line(&income_cells(&s.id, row)));
        }
    }
    out
}

/// File names for a report, for example `mizan-uk-2025-26-gains.csv`.
pub fn file_names(r: &TaxReport) -> (String, String) {
    let base = format!("mizan-{}-{}", r.country.as_str(), r.year);
    (format!("{base}-gains.csv"), format!("{base}-income.csv"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("Apple, Inc."), "\"Apple, Inc.\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(line(&["a", "b,c"]), "a,\"b,c\"\r\n");
    }
}
