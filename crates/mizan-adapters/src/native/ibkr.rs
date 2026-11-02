//! Interactive Brokers Flex Query XML: trades, cash transactions, open
//! positions and ending cash.

use std::collections::BTreeMap;

use mizan_import::Dec;
use rust_decimal::Decimal;
use serde_json::{json, Map, Value};

use super::xml::{parse, Node};
use crate::manifest::{FileKind, Manifest};
use crate::run::{AdapterError, RunOutput};
use crate::{Adapter, FileInput};

pub struct IbkrFlexAdapter {
    pub manifest: Manifest,
}

/// Sections whose rows this adapter reads, or that carry no rows.
const HANDLED: &[&str] = &[
    "Trades",
    "CashTransactions",
    "OpenPositions",
    "CashReport",
    "AccountInformation",
];

/// `20261002`, `2026-10-02`, `20261002;093000` or `2026-10-02, 09:30:00` → `2026-10-02`.
fn date(s: &str) -> Option<String> {
    let s = s.trim();
    let d: String = if s.len() >= 10 && s.as_bytes()[4] == b'-' {
        s[..10].replace('-', "")
    } else {
        s.chars().take(8).collect()
    };
    (d.len() == 8 && d.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8]))
}

fn num(n: &Node, attr: &str) -> Option<Decimal> {
    n.attr(attr)
        .and_then(|v| Dec::parse(v.trim()).ok())
        .map(|d| d.value())
}

fn money(v: Decimal, ccy: &str) -> Value {
    json!({ "amount": v.abs().normalize().to_string(), "currency": ccy })
}

fn instrument(n: &Node) -> Option<Value> {
    let mut h = Map::new();
    for (attr, key) in [
        ("isin", "isin"),
        ("cusip", "cusip"),
        ("figi", "figi"),
        ("symbol", "symbol"),
        ("description", "name"),
    ] {
        if let Some(v) = n.attr(attr) {
            h.insert(key.into(), json!(v.trim()));
        }
    }
    if h.is_empty() {
        return None;
    }
    if let Some(c) = n.attr("currency") {
        h.insert("currency".into(), json!(c));
    }
    let class = match n.attr("assetCategory") {
        Some("STK") => Some(if n.attr("subCategory") == Some("ETF") {
            "etf"
        } else {
            "equity"
        }),
        Some("FUND") => Some("fund"),
        Some("BOND") | Some("BILL") => Some("bond"),
        Some("OPT") | Some("FOP") => Some("option"),
        Some("CRYPTO") => Some("crypto"),
        Some("CMDTY") => Some("commodity"),
        _ => None,
    };
    if let Some(c) = class {
        h.insert("class_hint".into(), json!(c));
    }
    if let Some(m) = num(n, "multiplier").filter(|m| *m != Decimal::ONE) {
        h.insert("multiplier".into(), json!(m.normalize().to_string()));
    }
    Some(Value::Object(h))
}

/// Key used to pair a withholding tax with its dividend.
fn income_key(n: &Node, d: &str) -> String {
    format!(
        "{}|{}|{}",
        n.attr("isin")
            .or(n.attr("conid"))
            .or(n.attr("symbol"))
            .unwrap_or(""),
        d,
        n.attr("currency").unwrap_or("")
    )
}

/// Keep execution-level rows when the query has them; never summaries.
fn detail_rows<'a>(rows: &[&'a Node], detail: &str) -> Vec<&'a Node> {
    let has_detail = rows.iter().any(|r| r.attr("levelOfDetail") == Some(detail));
    rows.iter()
        .copied()
        .filter(|r| match r.attr("levelOfDetail") {
            None => true,
            Some(l) if has_detail => l == detail,
            Some(l) => !l.contains("SUMMARY"),
        })
        .collect()
}

struct Statement<'a> {
    node: &'a Node,
    account: String,
    out: RunOutput,
    /// (account, as_of) → (positions, cash)
    balances: BTreeMap<(String, String), (Vec<Value>, Vec<Value>)>,
}

impl<'a> Statement<'a> {
    fn account_of(&self, n: &Node) -> Value {
        json!({ "hint": n.attr("accountId").unwrap_or(&self.account) })
    }

    fn diag(&mut self, severity: &str, message: String, locator: String) {
        self.out
            .diagnostics
            .push(json!({ "severity": severity, "message": message, "locator": locator }));
    }

    fn rows(&self, section: &str, row: &str) -> Vec<&'a Node> {
        self.node
            .child(section)
            .map(|s| s.children.iter().filter(|c| c.name == row).collect())
            .unwrap_or_default()
    }

    fn trades(&mut self) {
        let rows = self.rows("Trades", "Trade");
        for (i, t) in detail_rows(&rows, "EXECUTION").into_iter().enumerate() {
            let locator = match t.attr("tradeID") {
                Some(id) => format!("Trade {id}"),
                None => format!("Trades row {}", i + 1),
            };
            let Some(d) = t.attr("dateTime").or(t.attr("tradeDate")).and_then(date) else {
                self.diag("warning", "trade without a date".into(), locator);
                continue;
            };
            let side = t.attr("buySell").unwrap_or("");
            if side.contains("(Ca.)") {
                self.diag(
                    "warning",
                    "cancelled trade skipped; check its replacement in the same file".into(),
                    locator,
                );
                continue;
            }
            if t.attr("assetCategory") == Some("CASH") {
                self.diag(
                    "warning",
                    format!(
                        "currency conversion {} is not imported yet",
                        t.attr("symbol").unwrap_or("")
                    ),
                    locator,
                );
                continue;
            }
            let qty = num(t, "quantity").unwrap_or_default();
            let buy = if side.is_empty() {
                qty.is_sign_positive()
            } else {
                side.starts_with("BUY")
            };
            let oc = t.attr("openCloseIndicator").unwrap_or("");
            let kind = match (buy, oc.contains('C'), oc.contains('O')) {
                (true, true, _) => "buy_to_cover",
                (false, false, true) => "short_sell",
                (true, _, _) => "buy",
                (false, _, _) => "sell",
            };
            let ccy = t.attr("currency").unwrap_or("USD");
            let mut r = json!({ "locator": locator, "account": self.account_of(t), "kind": kind, "date": d, "quantity": qty.abs().normalize().to_string() });
            if let Some(s) = t.attr("settleDateTarget").and_then(date) {
                r["settle_date"] = json!(s);
            }
            if let Some(i) = instrument(t) {
                r["instrument"] = i;
            }
            if let Some(p) = num(t, "tradePrice") {
                r["price"] = money(p, ccy);
            }
            let mut charges = Vec::new();
            if let Some(c) = num(t, "ibCommission").filter(|c| !c.is_zero()) {
                charges.push(json!({ "type": "commission", "amount": money(c, t.attr("ibCommissionCurrency").unwrap_or(ccy)) }));
            }
            if let Some(c) = num(t, "taxes").filter(|c| !c.is_zero()) {
                charges.push(json!({ "type": "transaction_tax", "amount": money(c, ccy) }));
            }
            if !charges.is_empty() {
                r["charges"] = json!(charges);
            }
            if let Some(n) = num(t, "netCash") {
                r["net_cash"] = money(n, ccy);
            }
            if let Some(id) = t.attr("tradeID") {
                r["external_id"] = json!(id);
            }
            self.out.records.push(r);
        }
    }

    fn cash(&mut self) {
        let rows = self.rows("CashTransactions", "CashTransaction");
        let rows = detail_rows(&rows, "DETAIL");
        // Withholding taxes by (security, date, currency), consumed by dividends.
        let mut withholding: BTreeMap<String, Vec<&Node>> = BTreeMap::new();
        for c in &rows {
            if c.attr("type") == Some("Withholding Tax") {
                if let Some(d) = c.attr("dateTime").or(c.attr("reportDate")).and_then(date) {
                    withholding.entry(income_key(c, &d)).or_default().push(c);
                }
            }
        }
        for (i, c) in rows.iter().enumerate() {
            let locator = match c.attr("transactionID") {
                Some(id) => format!("CashTransaction {id}"),
                None => format!("CashTransactions row {}", i + 1),
            };
            let ty = c.attr("type").unwrap_or("");
            if ty == "Withholding Tax" {
                continue; // emitted below if no dividend took it
            }
            let (Some(d), Some(amt)) = (
                c.attr("dateTime").or(c.attr("reportDate")).and_then(date),
                num(c, "amount"),
            ) else {
                self.diag(
                    "warning",
                    "cash transaction without a date or amount".into(),
                    locator,
                );
                continue;
            };
            let ccy = c.attr("currency").unwrap_or("USD");
            let kind = match ty {
                "Dividends" | "Payment In Lieu Of Dividends" => "dividend",
                "Deposits/Withdrawals" | "Deposits & Withdrawals" => {
                    if amt.is_sign_negative() {
                        "withdrawal"
                    } else {
                        "deposit"
                    }
                }
                "Broker Interest Received" | "Bond Interest Received" => "interest",
                "Broker Interest Paid"
                | "Bond Interest Paid"
                | "Other Fees"
                | "Advisor Fees"
                | "Commission Adjustments" => {
                    if amt.is_sign_negative() {
                        "fee"
                    } else {
                        "adjustment"
                    }
                }
                other => {
                    self.diag(
                        "warning",
                        format!("cash transaction type {other:?} is not imported yet"),
                        locator,
                    );
                    continue;
                }
            };
            let mut r = json!({ "locator": locator, "account": self.account_of(c), "kind": kind, "date": d, "net_cash": money(amt, ccy) });
            if let Some(desc) = c.attr("description") {
                r["memo"] = json!(desc);
            }
            if kind == "dividend" {
                if let Some(i) = instrument(c) {
                    r["instrument"] = i;
                }
                if let Some(taxes) = withholding.remove(&income_key(c, &d)) {
                    let tax: Decimal = taxes.iter().filter_map(|t| num(t, "amount")).sum();
                    r["gross"] = money(amt, ccy);
                    r["charges"] =
                        json!([{ "type": "withholding_tax", "amount": money(tax, ccy) }]);
                    r["net_cash"] = money(amt + tax, ccy);
                }
            }
            if let Some(id) = c.attr("transactionID") {
                r["external_id"] = json!(id);
            }
            self.out.records.push(r);
        }
        // Withholding without a dividend in this file: a tax of its own, or a refund.
        for c in withholding.into_values().flatten() {
            let locator = match c.attr("transactionID") {
                Some(id) => format!("CashTransaction {id}"),
                None => "CashTransactions withholding".into(),
            };
            let (Some(d), Some(amt)) = (
                c.attr("dateTime").or(c.attr("reportDate")).and_then(date),
                num(c, "amount"),
            ) else {
                continue;
            };
            let ccy = c.attr("currency").unwrap_or("USD");
            let kind = if amt.is_sign_negative() {
                "tax"
            } else {
                "adjustment"
            };
            let mut r = json!({
                "locator": locator, "account": self.account_of(c), "kind": kind, "date": d, "net_cash": money(amt, ccy),
                "memo": c.attr("description").unwrap_or("Withholding tax"),
            });
            if let Some(i) = instrument(c) {
                r["instrument"] = i;
            }
            if let Some(id) = c.attr("transactionID") {
                r["external_id"] = json!(id);
            }
            self.out.records.push(r);
        }
    }

    fn positions(&mut self) {
        let to = self.node.attr("toDate").and_then(date);
        let rows = self.rows("OpenPositions", "OpenPosition");
        for p in detail_rows(&rows, "SUMMARY") {
            let (Some(asof), Some(qty), Some(inst)) = (
                p.attr("reportDate").and_then(date).or(to.clone()),
                num(p, "position"),
                instrument(p),
            ) else {
                continue;
            };
            let acct = p.attr("accountId").unwrap_or(&self.account).to_string();
            self.balances
                .entry((acct, asof))
                .or_default()
                .0
                .push(json!({ "instrument": inst, "quantity": qty.normalize().to_string() }));
        }
        let cash: Vec<&Node> = self.rows("CashReport", "CashReportCurrency");
        for c in cash {
            let ccy = c.attr("currency").unwrap_or("");
            if ccy.is_empty()
                || ccy == "BASE_SUMMARY"
                || c.attr("levelOfDetail").is_some_and(|l| l != "Currency")
            {
                continue;
            }
            let (Some(asof), Some(end)) = (
                c.attr("toDate").and_then(date).or(to.clone()),
                num(c, "endingCash"),
            ) else {
                continue;
            };
            let acct = c.attr("accountId").unwrap_or(&self.account).to_string();
            self.balances
                .entry((acct, asof))
                .or_default()
                .1
                .push(json!({ "amount": end.normalize().to_string(), "currency": ccy }));
        }
        for ((acct, asof), (positions, cash)) in std::mem::take(&mut self.balances) {
            let mut a = json!({ "locator": format!("OpenPositions {asof}"), "account": { "hint": acct }, "as_of": asof });
            if !positions.is_empty() {
                a["positions"] = json!(positions);
            }
            if !cash.is_empty() {
                a["cash"] = json!(cash);
            }
            self.out.assertions.push(a);
        }
    }

    fn unhandled(&mut self) {
        let node = self.node;
        for section in node
            .children
            .iter()
            .filter(|s| !HANDLED.contains(&s.name.as_str()))
        {
            for (i, _) in section.children.iter().enumerate() {
                self.diag(
                    "info",
                    format!("{} rows are not imported yet", section.name),
                    format!("{} row {}", section.name, i + 1),
                );
            }
        }
    }
}

impl IbkrFlexAdapter {
    fn convert(&self, file: &FileInput) -> Result<RunOutput, String> {
        let root = parse(&file.text(None)?)?;
        if root.find("FlexQueryResponse").is_none() {
            return Err("not a Flex Query response (no <FlexQueryResponse> element)".into());
        }
        let mut statements = Vec::new();
        root.find_all("FlexStatement", &mut statements);
        if statements.is_empty() {
            return Err("the Flex Query response holds no statements".into());
        }
        let mut out = RunOutput::default();
        for node in statements {
            let mut st = Statement {
                node,
                account: node.attr("accountId").unwrap_or("IBKR").to_string(),
                out: RunOutput::default(),
                balances: BTreeMap::new(),
            };
            st.trades();
            st.cash();
            st.positions();
            st.unhandled();
            out.records.extend(st.out.records);
            out.assertions.extend(st.out.assertions);
            out.diagnostics.extend(st.out.diagnostics);
        }
        Ok(out)
    }
}

impl Adapter for IbkrFlexAdapter {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn detect(&self, file: &FileInput) -> u8 {
        if file.kind != FileKind::Xml {
            return 0;
        }
        let head = String::from_utf8_lossy(&file.bytes[..file.bytes.len().min(1024)]);
        if head.contains("<FlexQueryResponse") {
            95
        } else {
            0
        }
    }

    fn run(&self, file: &FileInput) -> Result<RunOutput, AdapterError> {
        self.convert(file)
            .map_err(|e| AdapterError::new(&self.manifest.id, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::AdapterKind;

    fn run(body: &str) -> RunOutput {
        let a = IbkrFlexAdapter {
            manifest: Manifest {
                id: "ibkr-flex".into(),
                version: "1.0.0".into(),
                name: "IBKR".into(),
                kind: AdapterKind::Native,
                files: vec![FileKind::Xml],
                native: Some("ibkr".into()),
                script: None,
                description: None,
            },
        };
        let xml = format!(
            r#"<FlexQueryResponse queryName="q" type="AF"><FlexStatements count="1"><FlexStatement accountId="U0000001" fromDate="20261001" toDate="20261031">{body}</FlexStatement></FlexStatements></FlexQueryResponse>"#
        );
        let f = FileInput::new("flex.xml", xml.into_bytes());
        assert_eq!(a.detect(&f), 95);
        a.run(&f).unwrap()
    }

    #[test]
    fn dividend_with_withholding() {
        let out = run(r#"<CashTransactions>
            <CashTransaction currency="USD" symbol="EXMP" isin="US0000000AA1" description="EXMP CASH DIVIDEND" dateTime="20261015;202000" amount="24.00" type="Dividends" transactionID="900" levelOfDetail="DETAIL"/>
            <CashTransaction currency="USD" symbol="EXMP" isin="US0000000AA1" description="EXMP US TAX" dateTime="20261015;202000" amount="-3.60" type="Withholding Tax" transactionID="901" levelOfDetail="DETAIL"/>
            </CashTransactions>"#);
        assert_eq!(out.records.len(), 1);
        let r = &out.records[0];
        assert_eq!(r["kind"], "dividend");
        assert_eq!(r["gross"], json!({"amount": "24", "currency": "USD"}));
        assert_eq!(
            r["charges"],
            json!([{"type": "withholding_tax", "amount": {"amount": "3.6", "currency": "USD"}}])
        );
        assert_eq!(r["net_cash"], json!({"amount": "20.4", "currency": "USD"}));
    }

    #[test]
    fn closing_a_short() {
        let out = run(r#"<Trades>
            <Trade accountId="U0000001" currency="USD" assetCategory="STK" symbol="EXMP" isin="US0000000AA1" tradeID="11" dateTime="20261003;100000" quantity="-8" tradePrice="50" ibCommission="-1" ibCommissionCurrency="USD" netCash="399" buySell="SELL" openCloseIndicator="O" levelOfDetail="EXECUTION"/>
            <Trade accountId="U0000001" currency="USD" assetCategory="STK" symbol="EXMP" isin="US0000000AA1" tradeID="12" dateTime="20261010;100000" quantity="8" tradePrice="45" ibCommission="-1" ibCommissionCurrency="USD" netCash="-361" buySell="BUY" openCloseIndicator="C" levelOfDetail="EXECUTION"/>
            <Trade accountId="U0000001" currency="USD" assetCategory="STK" symbol="EXMP" quantity="0" levelOfDetail="SYMBOL_SUMMARY"/>
            </Trades>"#);
        let kinds: Vec<&str> = out
            .records
            .iter()
            .map(|r| r["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["short_sell", "buy_to_cover"]);
        assert_eq!(out.records[1]["quantity"], "8");
        assert_eq!(out.records[1]["external_id"], "12");
        assert_eq!(out.records[1]["charges"][0]["amount"]["amount"], "1");
    }

    #[test]
    fn open_positions_and_ending_cash() {
        let out = run(r#"<OpenPositions>
            <OpenPosition accountId="U0000001" currency="USD" assetCategory="STK" symbol="EXMP" isin="US0000000AA1" reportDate="20261031" position="100" levelOfDetail="SUMMARY"/>
            </OpenPositions>
            <CashReport><CashReportCurrency accountId="U0000001" currency="BASE_SUMMARY" endingCash="999" toDate="20261031"/>
            <CashReportCurrency accountId="U0000001" currency="USD" endingCash="1234.50" toDate="20261031"/></CashReport>"#);
        assert_eq!(out.assertions.len(), 1);
        let a = &out.assertions[0];
        assert_eq!(a["as_of"], "2026-10-31");
        assert_eq!(a["positions"][0]["quantity"], "100");
        assert_eq!(a["positions"][0]["instrument"]["isin"], "US0000000AA1");
        assert_eq!(a["cash"], json!([{"amount": "1234.5", "currency": "USD"}]));
    }

    #[test]
    fn unsupported_section() {
        let out = run(r#"<AccountInformation accountId="U0000001" currency="USD"/>
            <Transfers><Transfer symbol="A"/><Transfer symbol="B"/></Transfers>
            <CashTransactions><CashTransaction currency="USD" dateTime="20261001" amount="100" type="Deposits/Withdrawals" transactionID="1"/></CashTransactions>"#);
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0]["kind"], "deposit");
        assert_eq!(out.diagnostics.len(), 2);
        assert_eq!(
            out.diagnostics[0]["message"],
            "Transfers rows are not imported yet"
        );
        assert_eq!(out.diagnostics[1]["locator"], "Transfers row 2");
    }

    #[test]
    fn dates() {
        assert_eq!(date("2026-10-02, 09:30:00").as_deref(), Some("2026-10-02"));
        assert_eq!(date("20261002;093000").as_deref(), Some("2026-10-02"));
        assert_eq!(date("x"), None);
    }
}
