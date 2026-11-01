//! Primitive value types used by the schema: decimals, money, codes, dates, hashes.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use schemars::gen::SchemaGenerator;
use schemars::schema::{InstanceType, Schema, SchemaObject, StringValidation};
use schemars::JsonSchema;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

fn string_schema(pattern: Option<&str>, format: Option<&str>, description: &str) -> Schema {
    let mut obj = SchemaObject {
        instance_type: Some(InstanceType::String.into()),
        format: format.map(str::to_owned),
        ..Default::default()
    };
    if let Some(p) = pattern {
        obj.string = Some(Box::new(StringValidation {
            pattern: Some(p.to_owned()),
            ..Default::default()
        }));
    }
    obj.metadata().description = Some(description.to_owned());
    Schema::Object(obj)
}

/// Visitor that accepts only JSON strings and parses them with `parse`.
struct StrVisitor<T> {
    expecting: &'static str,
    parse: fn(&str) -> Result<T, String>,
}

impl<T> Visitor<'_> for StrVisitor<T> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.expecting)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<T, E> {
        (self.parse)(v).map_err(E::custom)
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<T, E> {
        Err(E::custom(format!(
            "expected {}, got the number {v}; numbers must be written as strings",
            self.expecting
        )))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<T, E> {
        Err(E::custom(format!(
            "expected {}, got the number {v}; numbers must be written as strings",
            self.expecting
        )))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<T, E> {
        Err(E::custom(format!(
            "expected {}, got the number {v}; numbers must be written as strings",
            self.expecting
        )))
    }
}

/// A decimal number carried as a JSON string, parsed without loss of precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dec(pub Decimal);

impl Dec {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if t != s || t.is_empty() {
            return Err(format!("invalid decimal string {s:?}"));
        }
        let valid = {
            let body = t.strip_prefix('-').unwrap_or(t);
            let mut parts = body.splitn(2, '.');
            let int = parts.next().unwrap_or("");
            let frac = parts.next();
            !int.is_empty()
                && int.bytes().all(|b| b.is_ascii_digit())
                && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
        };
        if !valid {
            return Err(format!("invalid decimal string {s:?}"));
        }
        Decimal::from_str_exact(t)
            .map(Dec)
            .map_err(|e| format!("invalid decimal string {s:?}: {e}"))
    }

    pub fn value(&self) -> Decimal {
        self.0
    }
}

impl FromStr for Dec {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Dec::parse(s)
    }
}

impl fmt::Display for Dec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for Dec {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Dec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StrVisitor {
            expecting: "a decimal string such as \"20.40\"",
            parse: Dec::parse,
        })
    }
}

impl JsonSchema for Dec {
    fn schema_name() -> String {
        "Decimal".into()
    }
    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("mizan::Decimal")
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        string_schema(
            Some(r"^-?[0-9]+(\.[0-9]+)?$"),
            None,
            "Decimal number written as a string. Binary floating-point JSON numbers are rejected.",
        )
    }
}

/// An ISO 4217 currency code, or an upper-case asset code for non-fiat money.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code(String);

impl Code {
    pub fn parse(s: &str) -> Result<Self, String> {
        let ok = (2..=12).contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if ok {
            Ok(Code(s.to_owned()))
        } else {
            Err(format!(
                "invalid currency or asset code {s:?}; expected 2-12 upper-case letters or digits"
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Code {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Code {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StrVisitor {
            expecting: "a currency or asset code such as \"EUR\"",
            parse: Code::parse,
        })
    }
}

impl JsonSchema for Code {
    fn schema_name() -> String {
        "Code".into()
    }
    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("mizan::Code")
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        string_schema(
            Some("^[A-Z0-9]{2,12}$"),
            None,
            "ISO 4217 currency code (e.g. EUR), or an upper-case asset code for non-fiat money.",
        )
    }
}

/// A monetary amount: a decimal paired with a currency or asset code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub amount: Dec,
    pub currency: Code,
}

/// A calendar date with no time component (`YYYY-MM-DD`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(pub NaiveDate);

impl Date {
    pub fn parse(s: &str) -> Result<Self, String> {
        let err = || format!("invalid date {s:?}; expected YYYY-MM-DD with no time component");
        if s.len() != 10 {
            return Err(err());
        }
        NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map(Date)
            .map_err(|_| err())
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.format("%Y-%m-%d"))
    }
}

impl Serialize for Date {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StrVisitor {
            expecting: "a date string YYYY-MM-DD",
            parse: Date::parse,
        })
    }
}

impl JsonSchema for Date {
    fn schema_name() -> String {
        "Date".into()
    }
    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("mizan::Date")
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        string_schema(
            Some("^[0-9]{4}-[0-9]{2}-[0-9]{2}$"),
            Some("date"),
            "Calendar date YYYY-MM-DD in the account's local calendar. No time component.",
        )
    }
}

/// Lower-case hex SHA-256 digest of a source file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Hex(String);

impl Sha256Hex {
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Sha256Hex(s.to_owned()))
        } else {
            Err(format!(
                "invalid file_sha256 {s:?}; expected 64 lower-case hex characters"
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Sha256Hex {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha256Hex {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StrVisitor {
            expecting: "a SHA-256 hex digest",
            parse: Sha256Hex::parse,
        })
    }
}

impl JsonSchema for Sha256Hex {
    fn schema_name() -> String {
        "Sha256Hex".into()
    }
    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("mizan::Sha256Hex")
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        string_schema(
            Some("^[0-9a-f]{64}$"),
            None,
            "Lower-case hex SHA-256 digest of the source file.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decimal_string_keeps_precision() {
        let d: Dec = serde_json::from_value(json!("0.125")).unwrap();
        assert_eq!(d.to_string(), "0.125");
        let m: Money =
            serde_json::from_value(json!({"amount": "20.40", "currency": "USD"})).unwrap();
        assert_eq!(m.amount.to_string(), "20.40");
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({"amount": "20.40", "currency": "USD"})
        );
    }

    #[test]
    fn float_is_rejected() {
        let err = serde_json::from_value::<Dec>(json!(20.4)).unwrap_err();
        assert!(
            err.to_string().contains("must be written as strings"),
            "{err}"
        );
        assert!(serde_json::from_value::<Dec>(json!(20)).is_err());
    }

    #[test]
    fn malformed_decimal_strings_are_rejected() {
        for s in ["", " 1", "1e5", "1.", ".5", "+1", "abc", "1,5"] {
            assert!(Dec::parse(s).is_err(), "{s:?} should be rejected");
        }
        assert!(Dec::parse("-3.60").is_ok());
    }

    #[test]
    fn codes() {
        assert!(Code::parse("EUR").is_ok());
        assert!(Code::parse("USDC").is_ok());
        assert!(Code::parse("eur").is_err());
        assert!(Code::parse("E").is_err());
    }

    #[test]
    fn date_only_accepted_datetime_rejected() {
        let d: Date = serde_json::from_value(json!("2026-03-14")).unwrap();
        assert_eq!(d.to_string(), "2026-03-14");
        assert!(serde_json::from_value::<Date>(json!("2026-03-14T10:00:00")).is_err());
        assert!(serde_json::from_value::<Date>(json!("2026-03-14 10:00")).is_err());
        assert!(serde_json::from_value::<Date>(json!("2026-02-30")).is_err());
    }

    #[test]
    fn sha256_format() {
        assert!(Sha256Hex::parse(&"a".repeat(64)).is_ok());
        assert!(Sha256Hex::parse("9f2c").is_err());
        assert!(Sha256Hex::parse(&"A".repeat(64)).is_err());
    }
}
