//! OFX / QFX statements (OFX 1.x SGML and 2.x XML): bank and investment accounts.

use std::collections::HashMap;

use mizan_import::Dec;
use regex::Regex;
use serde_json::{json, Map, Value};

use super::xml::{parse, Node};
use crate::manifest::{FileKind, Manifest};
use crate::run::{AdapterError, RunOutput};
use crate::{Adapter, FileInput};

pub struct OfxAdapter {
    pub manifest: Manifest,
}

/// Turn OFX 1.x SGML (leaf elements without end tags) into XML.
pub fn normalize(text: &str) -> Result<String, String> {
    let start = text.find("<OFX>").ok_or("no <OFX> element found")?;
    let body = &text[start..];
    if text[..start].contains("<?xml") || text[..start].contains("<?OFX") {
        return Ok(body.to_string());
    }
    // `<NAME>value` not followed by `</NAME>` becomes `<NAME>value</NAME>`.
    let tag = Regex::new(r"<([A-Za-z0-9._]+)>([^<]*)").expect("valid regex");
    let mut out = String::with_capacity(body.len() + body.len() / 4);
    let mut last = 0;
    for c in tag.captures_iter(body) {
        let m = c.get(0).expect("match");
        out.push_str(&body[last..m.start()]);
        let name = &c[1];
        let value = c[2].trim();
        let rest = &body[m.end()..];
        let closed = rest.trim_start().starts_with(&format!("</{name}>"));
        if value.is_empty() || closed {
            out.push_str(&format!("<{name}>{value}"));
        } else {
            out.push_str(&format!("<{name}>{}</{name}>", escape(value)));
        }
        last = m.end();
    }
    out.push_str(&body[last..]);
    Ok(out)
}

/// Escape a bare `&`; SGML files usually carry `&amp;` already.
fn escape(s: &str) -> String {
    let entity =
        Regex::new(r"^&(amp|lt|gt|quot|apos|#[0-9]+|#x[0-9a-fA-F]+);").expect("valid regex");
    let mut out = String::with_capacity(s.len());
    for (i, ch) in s.char_indices() {
        if ch == '&' && !entity.is_match(&s[i..]) {
            out.push_str("&amp;");
        } else {
            out.push(ch);
        }
    }
    out
}

/// `20261002120000[-5:EST]` → `2026-10-02`.
fn date(s: &str) -> Option<String> {
    let d: String = s.chars().take(8).collect();
    (d.len() == 8 && d.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8]))
}

fn amount(s: &str) -> Option<rust_decimal::Decimal> {
    let t = s.trim().trim_start_matches('+');
    let t = if t.contains(',') && !t.contains('.') {
        t.replace(',', ".")
    } else {
        t.to_string()
    };
    Dec::parse(&t).ok().map(|d| d.value())
}

fn money(v: rust_decimal::Decimal, ccy: &str) -> Value {
    json!({ "amount": v.abs().normalize().to_string(), "currency": ccy })
}

struct Ctx<'a> {
    account: Value,
    currency: String,
    securities: &'a HashMap<String, Value>,
    out: RunOutput,
}

fn security_key(secid: Option<&Node>) -> Option<String> {
    let s = secid?;
    Some(format!(
        "{}:{}",
        s.text_at(&["UNIQUEIDTYPE"]).unwrap_or(""),
        s.text_at(&["UNIQUEID"])?
    ))
}

fn instrument(secid: Option<&Node>, securities: &HashMap<String, Value>) -> Option<Value> {
    let key = security_key(secid)?;
    securities.get(&key).cloned().or_else(|| {
        let s = secid?;
        let id = s.text_at(&["UNIQUEID"])?;
        Some(match s.text_at(&["UNIQUEIDTYPE"]).unwrap_or("") {
            "ISIN" => json!({ "isin": id }),
            "CUSIP" => json!({ "cusip": id }),
            _ => json!({ "name": id }),
        })
    })
}

fn read_securities(root: &Node) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    let mut infos = Vec::new();
    root.find_all("SECINFO", &mut infos);
    for info in infos {
        let Some(key) = security_key(info.child("SECID")) else {
            continue;
        };
        let secid = info.child("SECID").expect("checked");
        let mut h = Map::new();
        let id = secid.text_at(&["UNIQUEID"]).unwrap_or("");
        match secid.text_at(&["UNIQUEIDTYPE"]).unwrap_or("") {
            "ISIN" => {
                h.insert("isin".into(), json!(id));
            }
            "CUSIP" => {
                h.insert("cusip".into(), json!(id));
            }
            _ => {}
        }
        if let Some(t) = info.text_at(&["TICKER"]) {
            h.insert("symbol".into(), json!(t));
        }
        if let Some(n) = info.text_at(&["SECNAME"]) {
            h.insert("name".into(), json!(n));
        }
        map.insert(key, Value::Object(h));
    }
    map
}

impl Ctx<'_> {
    fn diag(&mut self, severity: &str, message: String, locator: String) {
        self.out
            .diagnostics
            .push(json!({ "severity": severity, "message": message, "locator": locator }));
    }

    fn bank_txn(&mut self, t: &Node, n: usize) {
        let fitid = t.text_at(&["FITID"]).unwrap_or("");
        let locator = if fitid.is_empty() {
            format!("STMTTRN #{n}")
        } else {
            format!("FITID {fitid}")
        };
        let (Some(d), Some(amt)) = (
            t.text_at(&["DTPOSTED"]).and_then(date),
            t.text_at(&["TRNAMT"]).and_then(amount),
        ) else {
            return self.diag(
                "warning",
                "transaction without a date or amount".into(),
                locator,
            );
        };
        let ty = t.text_at(&["TRNTYPE"]).unwrap_or("OTHER");
        let kind = match ty {
            "FEE" | "SRVCHG" => "fee",
            "INT" => "interest",
            "DIV" => "dividend",
            "CREDIT" | "DEP" | "DIRECTDEP" => "deposit",
            "DEBIT" | "PAYMENT" | "ATM" | "POS" | "CHECK" | "DIRECTDEBIT" | "REPEATPMT"
            | "CASH" => "withdrawal",
            _ if amt.is_sign_negative() => "withdrawal",
            _ => "deposit",
        };
        let memo: Vec<&str> = [t.text_at(&["NAME"]), t.text_at(&["MEMO"])]
            .into_iter()
            .flatten()
            .collect();
        let mut r = json!({
            "locator": locator, "account": self.account, "kind": kind, "date": d,
            "net_cash": money(amt, t.text_at(&["CURRENCY", "CURSYM"]).unwrap_or(&self.currency)),
        });
        if !fitid.is_empty() {
            r["external_id"] = json!(fitid);
        }
        if !memo.is_empty() {
            r["memo"] = json!(memo.join(" · "));
        }
        self.out.records.push(r);
    }

    fn inv_txn(&mut self, t: &Node, n: usize) {
        if t.name == "INVBANKTRAN" {
            if let Some(s) = t.child("STMTTRN") {
                self.bank_txn(s, n);
            }
            return;
        }
        let tran = t.find("INVTRAN");
        let fitid = tran
            .and_then(|x| x.text_at(&["FITID"]))
            .unwrap_or("")
            .to_string();
        let locator = if fitid.is_empty() {
            format!("{} #{n}", t.name)
        } else {
            format!("FITID {fitid}")
        };
        let Some(d) = tran.and_then(|x| x.text_at(&["DTTRADE"])).and_then(date) else {
            return self.diag(
                "warning",
                format!("{} without a trade date", t.name),
                locator,
            );
        };
        let settle = tran.and_then(|x| x.text_at(&["DTSETTLE"])).and_then(date);
        let inst = instrument(t.find("SECID"), self.securities);
        let ccy = t
            .text_at(&["CURRENCY", "CURSYM"])
            .unwrap_or(&self.currency)
            .to_string();
        let total = t.find("TOTAL").and_then(|x| amount(&x.text));
        let mut base = json!({ "locator": locator, "account": self.account, "date": d });
        if let Some(s) = settle {
            base["settle_date"] = json!(s);
        }
        if let Some(i) = inst {
            base["instrument"] = i;
        }
        if let Some(m) = tran.and_then(|x| x.text_at(&["MEMO"])) {
            base["memo"] = json!(m);
        }
        let with_ext = |mut r: Value, suffix: &str| {
            if !fitid.is_empty() {
                r["external_id"] = json!(format!("{fitid}{suffix}"));
            }
            r
        };
        match t.name.as_str() {
            "BUYSTOCK" | "BUYMF" | "BUYOTHER" | "BUYDEBT" | "BUYOPT" | "SELLSTOCK" | "SELLMF"
            | "SELLOTHER" | "SELLDEBT" | "SELLOPT" => {
                let buy = t.name.starts_with("BUY");
                let kind = match (
                    buy,
                    t.text_at(&["BUYTYPE"])
                        .or(t.text_at(&["SELLTYPE"]))
                        .or(t.text_at(&["OPTBUYTYPE"])),
                ) {
                    (true, Some("BUYTOCOVER")) | (true, Some("BUYTOCLOSE")) => "buy_to_cover",
                    (false, Some("SELLSHORT")) | (false, Some("SELLTOOPEN")) => "short_sell",
                    (true, _) => "buy",
                    (false, _) => "sell",
                };
                let mut r = base.clone();
                r["kind"] = json!(kind);
                if let Some(u) = t.find("UNITS").and_then(|x| amount(&x.text)) {
                    r["quantity"] = json!(u.abs().normalize().to_string());
                }
                if let Some(p) = t.find("UNITPRICE").and_then(|x| amount(&x.text)) {
                    r["price"] = money(p, &ccy);
                }
                let mut charges = Vec::new();
                for (tag, ty) in [
                    ("COMMISSION", "commission"),
                    ("FEES", "other"),
                    ("TAXES", "transaction_tax"),
                ] {
                    if let Some(v) = t
                        .find(tag)
                        .and_then(|x| amount(&x.text))
                        .filter(|v| !v.is_zero())
                    {
                        charges.push(json!({ "type": ty, "amount": money(v, &ccy) }));
                    }
                }
                if !charges.is_empty() {
                    r["charges"] = json!(charges);
                }
                if let Some(tot) = total {
                    r["net_cash"] = money(tot, &ccy);
                }
                self.out.records.push(with_ext(r, ""));
            }
            "INCOME" => {
                let kind = match t.text_at(&["INCOMETYPE"]) {
                    Some("INTEREST") => "interest",
                    Some("DIV") => "dividend",
                    _ => "distribution",
                };
                let mut r = base.clone();
                r["kind"] = json!(kind);
                if let Some(tot) = total {
                    r["net_cash"] = money(tot, &ccy);
                }
                self.out.records.push(with_ext(r, ""));
            }
            "REINVEST" => {
                // Income paid and immediately used to buy more units.
                let mut inc = base.clone();
                inc["kind"] = json!(if t.text_at(&["INCOMETYPE"]) == Some("INTEREST") {
                    "interest"
                } else {
                    "dividend"
                });
                if let Some(tot) = total {
                    inc["net_cash"] = money(tot, &ccy);
                }
                self.out.records.push(with_ext(inc, ":income"));
                let mut buy = base.clone();
                buy["kind"] = json!("buy");
                if let Some(u) = t.find("UNITS").and_then(|x| amount(&x.text)) {
                    buy["quantity"] = json!(u.abs().normalize().to_string());
                }
                if let Some(p) = t.find("UNITPRICE").and_then(|x| amount(&x.text)) {
                    buy["price"] = money(p, &ccy);
                }
                if let Some(tot) = total {
                    buy["net_cash"] = money(tot, &ccy);
                }
                self.out.records.push(with_ext(buy, ":buy"));
            }
            "TRANSFER" => {
                let mut r = base.clone();
                let inbound = t.text_at(&["TFERACTION"]) != Some("OUT");
                r["kind"] = json!(if inbound {
                    "position_in"
                } else {
                    "position_out"
                });
                match t.find("UNITS").and_then(|x| amount(&x.text)) {
                    Some(u) => r["quantity"] = json!(u.abs().normalize().to_string()),
                    None => {
                        return self.diag(
                            "warning",
                            "transfer without units".into(),
                            r["locator"].as_str().unwrap_or("").to_string(),
                        )
                    }
                }
                self.out.records.push(with_ext(r, ""));
            }
            other => self.diag(
                "info",
                format!("{other} transactions are not imported yet"),
                locator,
            ),
        }
    }
}

impl OfxAdapter {
    fn convert(&self, file: &FileInput) -> Result<RunOutput, String> {
        let text = file
            .text(None)
            .or_else(|_| file.text(Some("windows-1252")))?;
        let root = parse(&normalize(&text)?)?;
        let securities = read_securities(&root);
        let mut out = RunOutput::default();

        let mut stmts = Vec::new();
        root.find_all("STMTRS", &mut stmts);
        root.find_all("CCSTMTRS", &mut stmts);
        for st in stmts {
            let acct = st
                .find("ACCTID")
                .map(|a| a.text.trim().to_string())
                .unwrap_or_else(|| "OFX".into());
            let mut ctx = Ctx {
                account: json!({ "hint": acct }),
                currency: st.text_at(&["CURDEF"]).unwrap_or("USD").to_string(),
                securities: &securities,
                out: RunOutput::default(),
            };
            let mut txns = Vec::new();
            st.find_all("STMTTRN", &mut txns);
            for (i, t) in txns.into_iter().enumerate() {
                ctx.bank_txn(t, i + 1);
            }
            if let (Some(bal), Some(asof)) = (
                st.find("LEDGERBAL")
                    .and_then(|b| b.text_at(&["BALAMT"]))
                    .and_then(amount),
                st.find("LEDGERBAL")
                    .and_then(|b| b.text_at(&["DTASOF"]))
                    .and_then(date),
            ) {
                ctx.out.assertions.push(json!({
                    "locator": "LEDGERBAL", "account": ctx.account, "as_of": asof,
                    "cash": [{ "amount": bal.normalize().to_string(), "currency": ctx.currency }],
                }));
            }
            merge(&mut out, ctx.out);
        }

        let mut inv = Vec::new();
        root.find_all("INVSTMTRS", &mut inv);
        for st in inv {
            let acct = st
                .find("ACCTID")
                .map(|a| a.text.trim().to_string())
                .unwrap_or_else(|| "OFX".into());
            let mut ctx = Ctx {
                account: json!({ "hint": acct }),
                currency: st.text_at(&["CURDEF"]).unwrap_or("USD").to_string(),
                securities: &securities,
                out: RunOutput::default(),
            };
            if let Some(list) = st.child("INVTRANLIST") {
                for (i, t) in list
                    .children
                    .iter()
                    .filter(|c| !matches!(c.name.as_str(), "DTSTART" | "DTEND"))
                    .enumerate()
                {
                    ctx.inv_txn(t, i + 1);
                }
            }
            let asof = st.text_at(&["DTASOF"]).and_then(date);
            if let Some(asof) = asof {
                let mut positions = Vec::new();
                if let Some(pl) = st.child("INVPOSLIST") {
                    for p in &pl.children {
                        let pos = p.find("INVPOS");
                        let (Some(inst), Some(units)) = (
                            instrument(pos.and_then(|x| x.find("SECID")), &securities),
                            pos.and_then(|x| x.text_at(&["UNITS"])).and_then(amount),
                        ) else {
                            continue;
                        };
                        positions.push(json!({ "instrument": inst, "quantity": units.normalize().to_string() }));
                    }
                }
                let cash: Vec<Value> = st
                    .find("INVBAL")
                    .and_then(|b| b.text_at(&["AVAILCASH"]))
                    .and_then(amount)
                    .map(|c| vec![json!({ "amount": c.normalize().to_string(), "currency": ctx.currency })])
                    .unwrap_or_default();
                if !positions.is_empty() || !cash.is_empty() {
                    let mut a =
                        json!({ "locator": "INVPOSLIST", "account": ctx.account, "as_of": asof });
                    if !positions.is_empty() {
                        a["positions"] = json!(positions);
                    }
                    if !cash.is_empty() {
                        a["cash"] = json!(cash);
                    }
                    ctx.out.assertions.push(a);
                }
            }
            merge(&mut out, ctx.out);
        }
        Ok(out)
    }
}

fn merge(into: &mut RunOutput, from: RunOutput) {
    into.records.extend(from.records);
    into.assertions.extend(from.assertions);
    into.diagnostics.extend(from.diagnostics);
}

impl Adapter for OfxAdapter {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn detect(&self, file: &FileInput) -> u8 {
        if file.kind != FileKind::Ofx {
            return 0;
        }
        let head = String::from_utf8_lossy(&file.bytes[..file.bytes.len().min(2048)]);
        if head.contains("<OFX>") || head.contains("OFXHEADER") {
            90
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

    fn adapter() -> OfxAdapter {
        OfxAdapter {
            manifest: Manifest {
                id: "ofx".into(),
                version: "1.0.0".into(),
                name: "OFX".into(),
                kind: AdapterKind::Native,
                files: vec![FileKind::Ofx],
                native: Some("ofx".into()),
                script: None,
                description: None,
            },
        }
    }

    const SGML: &str = "OFXHEADER:100\nDATA:OFXSGML\nVERSION:102\n\n<OFX>\n<BANKMSGSRSV1><STMTTRNRS><STMTRS>\n<CURDEF>EUR\n<BANKACCTFROM><BANKID>B1<ACCTID>NL01TEST<ACCTTYPE>CHECKING</BANKACCTFROM>\n<BANKTRANLIST><DTSTART>20261001<DTEND>20261031\n<STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20261005120000[+1:CET]<TRNAMT>-12.50<FITID>T1<NAME>Cafe &amp; Co</STMTTRN>\n<STMTTRN><TRNTYPE>SRVCHG<DTPOSTED>20261015<TRNAMT>-2.00<FITID>T2</STMTTRN>\n</BANKTRANLIST>\n<LEDGERBAL><BALAMT>985.50<DTASOF>20261031</LEDGERBAL>\n</STMTRS></STMTTRNRS></BANKMSGSRSV1>\n</OFX>\n";

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<?OFX OFXHEADER="200" VERSION="220" SECURITY="NONE" OLDFILEUID="NONE" NEWFILEUID="NONE"?>
<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>EUR</CURDEF>
<BANKACCTFROM><BANKID>B1</BANKID><ACCTID>NL01TEST</ACCTID><ACCTTYPE>CHECKING</ACCTTYPE></BANKACCTFROM>
<BANKTRANLIST><DTSTART>20261001</DTSTART><DTEND>20261031</DTEND>
<STMTTRN><TRNTYPE>DEBIT</TRNTYPE><DTPOSTED>20261005120000[+1:CET]</DTPOSTED><TRNAMT>-12.50</TRNAMT><FITID>T1</FITID><NAME>Cafe &amp; Co</NAME></STMTTRN>
<STMTTRN><TRNTYPE>SRVCHG</TRNTYPE><DTPOSTED>20261015</DTPOSTED><TRNAMT>-2.00</TRNAMT><FITID>T2</FITID></STMTTRN>
</BANKTRANLIST><LEDGERBAL><BALAMT>985.50</BALAMT><DTASOF>20261031</DTASOF></LEDGERBAL>
</STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>"#;

    fn run(text: &str) -> RunOutput {
        adapter()
            .run(&FileInput::new("s.ofx", text.as_bytes().to_vec()))
            .unwrap()
    }

    #[test]
    fn sgml_and_xml_are_equivalent() {
        let a = run(SGML);
        let b = run(XML);
        assert_eq!(a, b);
        assert_eq!(a.records.len(), 2);
        assert_eq!(a.records[0]["kind"], "withdrawal");
        assert_eq!(a.records[0]["memo"], "Cafe & Co");
        assert_eq!(a.records[0]["date"], "2026-10-05");
        assert_eq!(a.records[1]["kind"], "fee");
        assert_eq!(a.assertions[0]["cash"][0]["amount"], "985.5");
    }

    const INV: &str = r#"<?xml version="1.0"?><?OFX OFXHEADER="200" VERSION="220"?>
<OFX><INVSTMTMSGSRSV1><INVSTMTTRNRS><INVSTMTRS><DTASOF>20261031</DTASOF><CURDEF>USD</CURDEF>
<INVACCTFROM><BROKERID>broker.example</BROKERID><ACCTID>INV-1</ACCTID></INVACCTFROM>
<INVTRANLIST><DTSTART>20261001</DTSTART><DTEND>20261031</DTEND>
<BUYSTOCK><INVBUY><INVTRAN><FITID>B100</FITID><DTTRADE>20261002</DTTRADE><DTSETTLE>20261006</DTSETTLE></INVTRAN>
<SECID><UNIQUEID>037833100</UNIQUEID><UNIQUEIDTYPE>CUSIP</UNIQUEIDTYPE></SECID>
<UNITS>10</UNITS><UNITPRICE>150.00</UNITPRICE><COMMISSION>1.00</COMMISSION><TOTAL>-1501.00</TOTAL>
<SUBACCTSEC>CASH</SUBACCTSEC><SUBACCTFUND>CASH</SUBACCTFUND></INVBUY><BUYTYPE>BUY</BUYTYPE></BUYSTOCK>
<CLOSUREOPT><INVTRAN><FITID>X1</FITID><DTTRADE>20261010</DTTRADE></INVTRAN></CLOSUREOPT>
</INVTRANLIST>
<INVPOSLIST><POSSTOCK><INVPOS><SECID><UNIQUEID>037833100</UNIQUEID><UNIQUEIDTYPE>CUSIP</UNIQUEIDTYPE></SECID>
<HELDINACCT>CASH</HELDINACCT><POSTYPE>LONG</POSTYPE><UNITS>100</UNITS><UNITPRICE>155</UNITPRICE><MKTVAL>15500</MKTVAL><DTPRICEASOF>20261031</DTPRICEASOF></INVPOS></POSSTOCK></INVPOSLIST>
<INVBAL><AVAILCASH>2500.00</AVAILCASH><MARGINBALANCE>0</MARGINBALANCE><SHORTBALANCE>0</SHORTBALANCE></INVBAL>
</INVSTMTRS></INVSTMTTRNRS></INVSTMTMSGSRSV1>
<SECLISTMSGSRSV1><SECLIST><STOCKINFO><SECINFO><SECID><UNIQUEID>037833100</UNIQUEID><UNIQUEIDTYPE>CUSIP</UNIQUEIDTYPE></SECID>
<SECNAME>Example Corp</SECNAME><TICKER>EXMP</TICKER></SECINFO></STOCKINFO></SECLIST></SECLISTMSGSRSV1></OFX>"#;

    #[test]
    fn investment_buy_and_positions() {
        let out = run(INV);
        let b = &out.records[0];
        assert_eq!(b["kind"], "buy");
        assert_eq!(b["quantity"], "10");
        assert_eq!(b["price"], json!({"amount": "150", "currency": "USD"}));
        assert_eq!(
            b["charges"],
            json!([{"type": "commission", "amount": {"amount": "1", "currency": "USD"}}])
        );
        assert_eq!(b["net_cash"], json!({"amount": "1501", "currency": "USD"}));
        assert_eq!(
            b["instrument"],
            json!({"cusip": "037833100", "symbol": "EXMP", "name": "Example Corp"})
        );
        assert_eq!(
            (b["external_id"].as_str(), b["settle_date"].as_str()),
            (Some("B100"), Some("2026-10-06"))
        );
        let a = &out.assertions[0];
        assert_eq!(a["as_of"], "2026-10-31");
        assert_eq!(a["positions"][0]["quantity"], "100");
        assert_eq!(a["cash"][0]["amount"], "2500");
        assert_eq!(
            out.diagnostics[0]["message"],
            "CLOSUREOPT transactions are not imported yet"
        );
    }

    #[test]
    fn detection() {
        assert_eq!(
            adapter().detect(&FileInput::new("s.qfx", SGML.as_bytes().to_vec())),
            90
        );
        assert_eq!(
            adapter().detect(&FileInput::new("s.csv", b"a,b".to_vec())),
            0
        );
    }
}
