//! Declarative mapping adapters for CSV and XLSX files.
//!
//! A mapping is one TOML document: `[adapter]` (the manifest), `[detect]`,
//! `[input]`, `[account]`, ordered `[[rules]]` choosing each row's kind, and
//! `[fields]` describing how columns become record fields. See
//! `docs/adapters.md` for the reference.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use mizan_import::Dec;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml::Table as TomlTable;

use crate::input::{Table, TableOptions};
use crate::manifest::Manifest;
use crate::run::{AdapterError, RunOutput};
use crate::{Adapter, FileInput};

// ---------------------------------------------------------------- document

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Detect {
    /// Every one of these header columns must be present.
    pub required_columns: Vec<String>,
    /// Optional regular expression on the file name.
    pub file_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputOptions {
    #[serde(flatten)]
    pub table: TableOptions,
    #[serde(default = "dot")]
    pub decimal: String,
    #[serde(default)]
    pub thousands: String,
    #[serde(default = "iso")]
    pub date_format: String,
}

fn dot() -> String {
    ".".into()
}
fn iso() -> String {
    "%Y-%m-%d".into()
}

impl Default for InputOptions {
    fn default() -> Self {
        InputOptions {
            table: TableOptions::default(),
            decimal: dot(),
            thousands: String::new(),
            date_format: iso(),
        }
    }
}

/// One step applied to a cell value, in order.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Transform {
    /// `"trim"`, `"abs"`, `"negate"`, `"upper"`.
    Named(String),
    /// `{ regex = "…" }`: the first capture group (or the whole match).
    Regex { regex: String },
    /// `{ lookup = { "A" = "x" }, default = "y" }`.
    Lookup {
        lookup: BTreeMap<String, String>,
        #[serde(default)]
        default: Option<String>,
    },
}

/// Where a field's value comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Source {
    pub column: Option<String>,
    #[serde(rename = "const")]
    pub constant: Option<String>,
    pub concat: Option<Vec<String>>,
    pub separator: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transform: Vec<Transform>,
    /// A missing or empty value omits the field instead of rejecting the row.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub column: String,
    #[serde(default)]
    pub equals: Option<String>,
    #[serde(default)]
    pub matches: Option<String>,
    /// A record kind, or `"skip"` for rows that carry no transaction.
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccountSpec {
    pub hint: Source,
    #[serde(default)]
    pub name_hint: Option<Source>,
}

/// A whole mapping document (everything in the TOML file).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MappingDoc {
    pub adapter: toml::Value,
    #[serde(default)]
    pub detect: Detect,
    #[serde(default)]
    pub input: InputOptions,
    pub account: AccountSpec,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Nested tables of [`Source`]s, shaped like an import record.
    pub fields: TomlTable,
}

impl MappingDoc {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("invalid mapping: {}", e.message()))
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------- compiled mapping

enum Step {
    Trim,
    Upper,
    Abs,
    Negate,
    Regex(Regex),
    Lookup(BTreeMap<String, String>, Option<String>),
}

struct Leaf {
    path: Vec<String>,
    source: Source,
    steps: Vec<Step>,
    kind: LeafKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeafKind {
    Text,
    Decimal,
    Date,
}

fn leaf_kind(path: &[String]) -> LeafKind {
    match path.last().map(String::as_str) {
        Some(
            "amount" | "quantity" | "rate" | "multiplier" | "new" | "old" | "strike"
            | "quantity_delta" | "cost_basis_share",
        ) => LeafKind::Decimal,
        Some("date" | "settle_date" | "acquisition_date" | "expiry" | "as_of") => LeafKind::Date,
        _ => LeafKind::Text,
    }
}

fn compile_steps(ts: &[Transform]) -> Result<Vec<Step>, String> {
    ts.iter()
        .map(|t| match t {
            Transform::Named(n) => match n.as_str() {
                "trim" => Ok(Step::Trim),
                "upper" => Ok(Step::Upper),
                "abs" => Ok(Step::Abs),
                "negate" => Ok(Step::Negate),
                other => Err(format!("unknown transform {other:?}")),
            },
            Transform::Regex { regex } => Regex::new(regex)
                .map(Step::Regex)
                .map_err(|e| format!("invalid regex {regex:?}: {e}")),
            Transform::Lookup { lookup, default } => {
                Ok(Step::Lookup(lookup.clone(), default.clone()))
            }
        })
        .collect()
}

fn is_source(t: &TomlTable) -> bool {
    t.contains_key("column") || t.contains_key("const") || t.contains_key("concat")
}

fn collect_leaves(prefix: &[String], table: &TomlTable, out: &mut Vec<Leaf>) -> Result<(), String> {
    for (k, v) in table {
        let mut path = prefix.to_vec();
        path.push(k.clone());
        let toml::Value::Table(t) = v else {
            return Err(format!(
                "field {} must be a table like {{ column = \"…\" }}",
                path.join(".")
            ));
        };
        if is_source(t) {
            let source: Source =
                toml::Value::Table(t.clone())
                    .try_into()
                    .map_err(|e: toml::de::Error| {
                        format!("field {}: {}", path.join("."), e.message())
                    })?;
            let steps = compile_steps(&source.transform)
                .map_err(|e| format!("field {}: {e}", path.join(".")))?;
            out.push(Leaf {
                kind: leaf_kind(&path),
                path,
                source,
                steps,
            });
        } else {
            collect_leaves(&path, t, out)?;
        }
    }
    Ok(())
}

struct CompiledRule {
    column: String,
    test: RuleTest,
    kind: String,
}

enum RuleTest {
    Equals(String),
    Matches(Regex),
}

pub struct MappingAdapter {
    manifest: Manifest,
    pub doc: MappingDoc,
    leaves: Vec<Leaf>,
    account_hint: Leaf,
    account_name: Option<Leaf>,
    rules: Vec<CompiledRule>,
    file_name: Option<Regex>,
}

impl MappingAdapter {
    pub fn from_toml(manifest: Manifest, text: &str) -> Result<Self, String> {
        Self::from_doc(manifest, MappingDoc::from_toml(text)?)
    }

    pub fn from_doc(manifest: Manifest, doc: MappingDoc) -> Result<Self, String> {
        let mut leaves = Vec::new();
        collect_leaves(&[], &doc.fields, &mut leaves)?;
        if !leaves.iter().any(|l| l.path == ["date"]) {
            return Err("fields must map `date`".into());
        }
        let leaf = |path: &str, s: &Source| -> Result<Leaf, String> {
            Ok(Leaf {
                path: vec![path.into()],
                source: s.clone(),
                steps: compile_steps(&s.transform)?,
                kind: LeafKind::Text,
            })
        };
        let rules = doc
            .rules
            .iter()
            .map(|r| {
                let test = match (&r.equals, &r.matches) {
                    (Some(e), None) => RuleTest::Equals(e.clone()),
                    (None, Some(m)) => RuleTest::Matches(
                        Regex::new(m).map_err(|e| format!("rule regex {m:?}: {e}"))?,
                    ),
                    _ => {
                        return Err(format!(
                            "rule on {:?} needs exactly one of `equals` or `matches`",
                            r.column
                        ))
                    }
                };
                let valid = r.kind == "skip"
                    || serde_json::from_value::<mizan_import::Kind>(json!(r.kind)).is_ok();
                if !valid {
                    return Err(format!("rule kind {:?} is not a record kind", r.kind));
                }
                Ok(CompiledRule {
                    column: r.column.clone(),
                    test,
                    kind: r.kind.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if rules.is_empty() {
            return Err("a mapping needs at least one [[rules]] entry".into());
        }
        Ok(MappingAdapter {
            account_hint: leaf("hint", &doc.account.hint)?,
            account_name: doc
                .account
                .name_hint
                .as_ref()
                .map(|s| leaf("name_hint", s))
                .transpose()?,
            file_name: doc
                .detect
                .file_name
                .as_deref()
                .map(Regex::new)
                .transpose()
                .map_err(|e| e.to_string())?,
            manifest,
            leaves,
            rules,
            doc,
        })
    }

    fn read(&self, file: &FileInput) -> Result<Table, String> {
        file.table(&self.doc.input.table)
    }
}

// ---------------------------------------------------------------- values

/// Parse a locale-formatted number into a canonical decimal string.
pub fn parse_number(text: &str, decimal: &str, thousands: &str) -> Option<String> {
    let mut t = text.trim().replace(['\u{a0}', ' '], "");
    let mut negative = false;
    if t.starts_with('(') && t.ends_with(')') {
        negative = true;
        t = t[1..t.len() - 1].to_string();
    }
    if let Some(rest) = t.strip_suffix('-') {
        negative = true;
        t = rest.to_string();
    }
    if !thousands.is_empty() {
        t = t.replace(thousands, "");
    }
    if decimal != "." {
        if t.contains('.') {
            return None;
        }
        t = t.replace(decimal, ".");
    }
    let t = t.strip_prefix('+').unwrap_or(&t);
    let d = Dec::parse(t).ok()?;
    let v = if negative { -d.value() } else { d.value() };
    Some(v.to_string())
}

pub fn parse_date(text: &str, format: &str) -> Option<String> {
    NaiveDate::parse_from_str(text.trim(), format)
        .ok()
        .map(|d| d.format("%Y-%m-%d").to_string())
}

impl MappingAdapter {
    fn raw(&self, leaf: &Leaf, table: &Table, row: &[String]) -> Result<Option<String>, String> {
        let cell = |name: &str| -> Result<String, String> {
            let i = table
                .column(name)
                .ok_or_else(|| format!("column {name:?} not found"))?;
            Ok(row.get(i).cloned().unwrap_or_default())
        };
        let s = &leaf.source;
        let v = if let Some(c) = &s.constant {
            c.clone()
        } else if let Some(col) = &s.column {
            cell(col)?
        } else if let Some(cols) = &s.concat {
            cols.iter()
                .map(|c| cell(c))
                .collect::<Result<Vec<_>, _>>()?
                .join(s.separator.as_deref().unwrap_or(" "))
        } else {
            String::new()
        };
        Ok(Some(v))
    }

    /// Value of one field for a row: `Ok(None)` when an optional field is empty.
    fn value(&self, leaf: &Leaf, table: &Table, row: &[String]) -> Result<Option<String>, String> {
        let name = leaf.path.join(".");
        let mut v = self.raw(leaf, table, row)?.unwrap_or_default();
        let input = &self.doc.input;
        let mut numeric = false;
        let to_number = |v: &str| {
            parse_number(v, &input.decimal, &input.thousands)
                .ok_or_else(|| format!("{name}: {v:?} is not a number"))
        };
        for step in &leaf.steps {
            match step {
                Step::Trim => v = v.trim().to_string(),
                Step::Upper => v = v.to_uppercase(),
                Step::Regex(re) => {
                    v = re
                        .captures(&v)
                        .map(|c| {
                            c.get(1)
                                .or_else(|| c.get(0))
                                .map(|m| m.as_str().to_string())
                                .unwrap_or_default()
                        })
                        .unwrap_or_default()
                }
                Step::Lookup(map, default) => {
                    v = match map.get(v.trim()) {
                        Some(x) => x.clone(),
                        None => default.clone().unwrap_or(v),
                    }
                }
                Step::Abs | Step::Negate => {
                    if v.trim().is_empty() {
                        continue;
                    }
                    let n: rust_decimal::Decimal =
                        (if numeric { v.clone() } else { to_number(&v)? })
                            .parse()
                            .expect("canonical");
                    v = (if matches!(step, Step::Abs) {
                        n.abs()
                    } else {
                        -n
                    })
                    .to_string();
                    numeric = true;
                }
            }
        }
        if v.trim().is_empty() || v.trim() == "-" {
            return if leaf.source.optional {
                Ok(None)
            } else {
                Err(format!("{name} is empty"))
            };
        }
        match leaf.kind {
            LeafKind::Text => Ok(Some(v.trim().to_string())),
            LeafKind::Decimal if numeric => Ok(Some(v)),
            LeafKind::Decimal => to_number(&v).map(Some),
            LeafKind::Date => parse_date(&v, &input.date_format).map(Some).ok_or_else(|| {
                format!(
                    "{name}: {v:?} does not match date format {}",
                    input.date_format
                )
            }),
        }
    }

    fn kind_for(&self, table: &Table, row: &[String]) -> Result<Option<String>, String> {
        for r in &self.rules {
            let i = table
                .column(&r.column)
                .ok_or_else(|| format!("rule column {:?} not found", r.column))?;
            let cell = row.get(i).map(|s| s.trim()).unwrap_or("");
            let hit = match &r.test {
                RuleTest::Equals(e) => cell == e.trim(),
                RuleTest::Matches(re) => re.is_match(cell),
            };
            if hit {
                return Ok(Some(r.kind.clone()));
            }
        }
        Ok(None)
    }

    fn describe(&self, table: &Table, row: &[String]) -> String {
        self.rules
            .first()
            .and_then(|r| table.column(&r.column))
            .and_then(|i| row.get(i))
            .cloned()
            .unwrap_or_default()
    }

    fn record(&self, table: &Table, line: usize, row: &[String]) -> Result<Option<Value>, String> {
        let Some(kind) = self.kind_for(table, row)? else {
            return Err(format!("no rule matches {:?}", self.describe(table, row)));
        };
        if kind == "skip" {
            return Ok(None);
        }
        let mut rec = Map::new();
        rec.insert("locator".into(), json!(format!("row {line}")));
        rec.insert("kind".into(), json!(kind));
        let mut account = Map::new();
        account.insert(
            "hint".into(),
            json!(self.value(&self.account_hint, table, row)?),
        );
        if let Some(n) = &self.account_name {
            if let Some(v) = self.value(n, table, row)? {
                account.insert("name_hint".into(), json!(v));
            }
        }
        rec.insert("account".into(), Value::Object(account));
        for leaf in &self.leaves {
            if let Some(v) = self.value(leaf, table, row)? {
                insert_path(&mut rec, &leaf.path, Value::String(v));
            }
        }
        Ok(Some(Value::Object(rec)))
    }
}

fn insert_path(obj: &mut Map<String, Value>, path: &[String], v: Value) {
    match path {
        [last] => {
            obj.insert(last.clone(), v);
        }
        [first, rest @ ..] => {
            let child = obj
                .entry(first.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(m) = child {
                insert_path(m, rest, v);
            }
        }
        [] => {}
    }
}

impl Adapter for MappingAdapter {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn detect(&self, file: &FileInput) -> u8 {
        if !self.manifest.files.contains(&file.kind) || self.doc.detect.required_columns.is_empty()
        {
            return 0;
        }
        let Ok(table) = self.read(file) else { return 0 };
        if !self
            .doc
            .detect
            .required_columns
            .iter()
            .all(|c| table.column(c).is_some())
        {
            return 0;
        }
        match &self.file_name {
            Some(re) if re.is_match(&file.name) => 95,
            Some(_) => 70,
            None => 90,
        }
    }

    fn run(&self, file: &FileInput) -> Result<RunOutput, AdapterError> {
        let table = self
            .read(file)
            .map_err(|e| AdapterError::new(&self.manifest.id, e))?;
        let mut out = RunOutput::default();
        for (line, row) in &table.rows {
            match self.record(&table, *line, row) {
                Ok(Some(r)) => out.records.push(r),
                Ok(None) => out.diagnostics.push(json!({
                    "severity": "info", "message": format!("skipped: {}", self.describe(&table, row)), "locator": format!("row {line}")
                })),
                Err(e) => out.diagnostics.push(json!({ "severity": "warning", "message": e, "locator": format!("row {line}") })),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPING: &str = r#"
[adapter]
id = "test-bank"
version = "0.1.0"
name = "Test bank"
type = "mapping"
files = ["csv"]

[detect]
required_columns = ["Datum", "Omschrijving", "Bedrag"]

[input]
delimiter = ";"
decimal = ","
thousands = "."
date_format = "%d-%m-%Y"

[account]
hint = { const = "DEG-4417" }

[[rules]]
column = "Omschrijving"
matches = "^Koop"
kind = "buy"

[[rules]]
column = "Omschrijving"
equals = "Dividend"
kind = "dividend"

[[rules]]
column = "Omschrijving"
equals = "Saldo"
kind = "skip"

[fields]
date = { column = "Datum" }
external_id = { column = "Id", optional = true }
quantity = { column = "Aantal", transform = ["abs"], optional = true }
instrument.isin = { column = "ISIN", optional = true }
instrument.symbol = { column = "Product", transform = [{ regex = "\\((\\w+)\\)" }], optional = true }
net_cash = { amount = { column = "Bedrag", transform = ["abs"] }, currency = { const = "EUR" } }
memo = { concat = ["Omschrijving", "Product"], separator = " · " }
"#;

    fn adapter() -> MappingAdapter {
        MappingAdapter::from_toml(Manifest::from_toml(MAPPING).unwrap(), MAPPING).unwrap()
    }

    fn csv(body: &str) -> FileInput {
        FileInput::new(
            "export.csv",
            format!("Datum;Omschrijving;Product;ISIN;Aantal;Bedrag;Id\n{body}").into_bytes(),
        )
    }

    #[test]
    fn field_forms_and_transforms() {
        let out = adapter()
            .run(&csv(
                "02-10-2026;Koop 10 @ 98,50;Vanguard (VWCE);IE00BK5BQT80;-10;-1.234,56;T1\n",
            ))
            .unwrap();
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert_eq!(
            out.records[0],
            json!({
                "locator": "row 2", "kind": "buy", "account": {"hint": "DEG-4417"}, "date": "2026-10-02", "external_id": "T1",
                "quantity": "10", "instrument": {"isin": "IE00BK5BQT80", "symbol": "VWCE"},
                "net_cash": {"amount": "1234.56", "currency": "EUR"}, "memo": "Koop 10 @ 98,50 · Vanguard (VWCE)"
            })
        );
    }

    #[test]
    fn optional_fields_are_omitted_and_lookup_works() {
        let doc = MAPPING.replace(
            "memo = { concat",
            "fx.from = { column = \"Omschrijving\", transform = [{ lookup = { \"Dividend\" = \"USD\" }, default = \"EUR\" }] }\nmemo = { concat",
        );
        let a = MappingAdapter::from_toml(Manifest::from_toml(&doc).unwrap(), &doc).unwrap();
        let out = a.run(&csv("03-10-2026;Dividend;;;;12,00;\n")).unwrap();
        let r = &out.records[0];
        assert!(
            r.get("quantity").is_none()
                && r.get("external_id").is_none()
                && r.get("instrument").is_none()
        );
        assert_eq!(r["fx"]["from"], "USD");
    }

    #[test]
    fn locale_numbers_and_dates() {
        assert_eq!(
            parse_number("1.234,56", ",", ".").as_deref(),
            Some("1234.56")
        );
        assert_eq!(
            parse_number("(1.234,56)", ",", ".").as_deref(),
            Some("-1234.56")
        );
        assert_eq!(
            parse_number("1.234,56-", ",", ".").as_deref(),
            Some("-1234.56")
        );
        assert_eq!(
            parse_number("1,234.56", ".", ",").as_deref(),
            Some("1234.56")
        );
        assert_eq!(
            parse_number("12345678901234567890,12", ",", ".").as_deref(),
            Some("12345678901234567890.12"),
            "no float"
        );
        assert_eq!(
            parse_number("1.2.3", ",", "."),
            Some("123".into()),
            "thousands removed first"
        );
        assert_eq!(parse_number("abc", ",", "."), None);
        assert_eq!(
            parse_date("31-12-2026", "%d-%m-%Y").as_deref(),
            Some("2026-12-31")
        );
        assert_eq!(parse_date("31/02/2026", "%d/%m/%Y"), None);
    }

    #[test]
    fn bad_date_becomes_a_row_diagnostic() {
        let out = adapter()
            .run(&csv(
                "31-02-2026;Koop 1;X;;1;-1,00;T9\n02-10-2026;Dividend;;;;1,00;T10\n",
            ))
            .unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.diagnostics[0]["locator"], "row 2");
        assert!(out.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("date format"));
    }

    #[test]
    fn first_matching_rule_and_unmatched_rows() {
        let out = adapter().run(&csv("02-10-2026;Koop 10 @ 98,50;X;;10;-985,00;\n02-10-2026;Rente;;;;0,10;\n02-10-2026;Saldo;;;;5,00;\n")).unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0]["kind"], "buy");
        let msgs: Vec<&str> = out
            .diagnostics
            .iter()
            .map(|d| d["message"].as_str().unwrap())
            .collect();
        assert_eq!(msgs, vec!["no rule matches \"Rente\"", "skipped: Saldo"]);
        assert_eq!(out.diagnostics[1]["severity"], "info");
    }

    #[test]
    fn detection_by_required_columns() {
        let a = adapter();
        assert_eq!(a.detect(&csv("")), 90);
        assert_eq!(
            a.detect(&FileInput::new("x.csv", b"Date;Amount\n".to_vec())),
            0
        );
        assert_eq!(
            a.detect(&FileInput::new("x.pdf", b"%PDF-1.4".to_vec())),
            0,
            "wrong file kind"
        );
    }

    #[test]
    fn invalid_mappings_are_rejected() {
        let m = Manifest::from_toml(MAPPING).unwrap();
        let bad = |from: &str, to: &str| {
            MappingAdapter::from_toml(m.clone(), &MAPPING.replace(from, to))
                .err()
                .unwrap()
        };
        assert!(bad("kind = \"buy\"", "kind = \"purchase\"").contains("not a record kind"));
        assert!(bad("matches = \"^Koop\"", "matches = \"(\"").contains("rule regex"));
        assert!(bad("date = { column = \"Datum\" }", "").contains("`date`"));
        assert!(bad(
            "transform = [\"abs\"] }, currency",
            "transform = [\"double\"] }, currency"
        )
        .contains("unknown transform"));
    }

    #[test]
    fn toml_round_trip() {
        let doc = MappingDoc::from_toml(MAPPING).unwrap();
        let again = MappingDoc::from_toml(&doc.to_toml().unwrap()).unwrap();
        assert_eq!(doc, again);
    }
}
