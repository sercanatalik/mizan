//! Import records, balance assertions, diagnostics and the batch that carries them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::{Code, Date, Dec, Money, Sha256Hex};

/// The only schema version this crate accepts.
pub const SCHEMA_V1: &str = "mizan.import/1";

/// Where a record, assertion or diagnostic came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// Adapter identifier, e.g. `degiro-csv`.
    pub adapter: String,
    /// Adapter version, e.g. `0.4.0`.
    pub adapter_version: String,
    /// SHA-256 of the source file.
    pub file_sha256: Sha256Hex,
    /// Position in the source, e.g. `row 184` or `page 3, line 12`.
    pub locator: String,
}

/// Closed set of record kinds.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    // Cash
    Deposit,
    Withdrawal,
    FxConversion,
    InternalTransfer,
    // Trade
    Buy,
    Sell,
    ShortSell,
    BuyToCover,
    // Income
    Dividend,
    Interest,
    Distribution,
    Reward,
    // Charge
    Fee,
    Tax,
    // Position move
    PositionIn,
    PositionOut,
    // Corporate action
    Split,
    Exchange,
    Spinoff,
    ReturnOfCapital,
    // Derivative
    OptionExercise,
    OptionAssignment,
    OptionExpiry,
    // Correction
    Adjustment,
}

impl Kind {
    pub const ALL: [Kind; 24] = [
        Kind::Deposit,
        Kind::Withdrawal,
        Kind::FxConversion,
        Kind::InternalTransfer,
        Kind::Buy,
        Kind::Sell,
        Kind::ShortSell,
        Kind::BuyToCover,
        Kind::Dividend,
        Kind::Interest,
        Kind::Distribution,
        Kind::Reward,
        Kind::Fee,
        Kind::Tax,
        Kind::PositionIn,
        Kind::PositionOut,
        Kind::Split,
        Kind::Exchange,
        Kind::Spinoff,
        Kind::ReturnOfCapital,
        Kind::OptionExercise,
        Kind::OptionAssignment,
        Kind::OptionExpiry,
        Kind::Adjustment,
    ];

    /// The `snake_case` name used in JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Deposit => "deposit",
            Kind::Withdrawal => "withdrawal",
            Kind::FxConversion => "fx_conversion",
            Kind::InternalTransfer => "internal_transfer",
            Kind::Buy => "buy",
            Kind::Sell => "sell",
            Kind::ShortSell => "short_sell",
            Kind::BuyToCover => "buy_to_cover",
            Kind::Dividend => "dividend",
            Kind::Interest => "interest",
            Kind::Distribution => "distribution",
            Kind::Reward => "reward",
            Kind::Fee => "fee",
            Kind::Tax => "tax",
            Kind::PositionIn => "position_in",
            Kind::PositionOut => "position_out",
            Kind::Split => "split",
            Kind::Exchange => "exchange",
            Kind::Spinoff => "spinoff",
            Kind::ReturnOfCapital => "return_of_capital",
            Kind::OptionExercise => "option_exercise",
            Kind::OptionAssignment => "option_assignment",
            Kind::OptionExpiry => "option_expiry",
            Kind::Adjustment => "adjustment",
        }
    }

    pub fn carries_legs(&self) -> bool {
        matches!(self, Kind::Exchange | Kind::Spinoff)
    }
}

/// Reference to an account as the statement names it. Never an internal id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountHint {
    /// Identifier as printed on the statement, e.g. `U1234567`.
    pub hint: String,
    /// Optional human-readable account name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_hint: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClassHint {
    Equity,
    Etf,
    Fund,
    Bond,
    Option,
    Crypto,
    Commodity,
    Cash,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptionRight {
    Call,
    Put,
}

/// Contract terms for an option instrument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OptionTerms {
    /// Underlying symbol as printed on the statement.
    pub underlying: String,
    pub expiry: Date,
    pub strike: Dec,
    pub right: OptionRight,
}

/// Reference to an instrument by whatever identifiers the statement shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstrumentHint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cusip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub figi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// ISO 10383 market identifier code, e.g. `XNAS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_hint: Option<ClassHint>,
    /// Currency the instrument is quoted in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<Code>,
    /// Contract multiplier, e.g. `100` for most equity options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<Dec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<OptionTerms>,
}

impl InstrumentHint {
    /// True when at least one identifying field is present.
    pub fn has_identifier(&self) -> bool {
        [
            &self.isin,
            &self.cusip,
            &self.figi,
            &self.symbol,
            &self.name,
        ]
        .iter()
        .any(|f| f.as_deref().is_some_and(|s| !s.trim().is_empty()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChargeType {
    Commission,
    ExchangeFee,
    StampDuty,
    TransactionTax,
    WithholdingTax,
    Other,
}

/// A fee or tax incurred as part of its parent record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Charge {
    #[serde(rename = "type")]
    pub charge_type: ChargeType,
    pub amount: Money,
}

/// A currency conversion shown on the statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fx {
    /// Units of `to` per one unit of `from`.
    pub rate: Dec,
    pub from: Code,
    pub to: Code,
}

/// One instrument's change in a multi-asset corporate action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Leg {
    pub instrument: InstrumentHint,
    /// Signed change in quantity of this instrument.
    pub quantity_delta: Dec,
    /// Fraction (0-1) of the original cost basis allocated to this leg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_basis_share: Option<Dec>,
}

/// New units per old units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SplitRatio {
    pub new: Dec,
    pub old: Dec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkRole {
    Reverses,
    TransferPair,
    Related,
}

/// A link to another record, by its external identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub role: LinkRole,
    /// External identifier of the target record.
    pub target: String,
}

/// One economic event as a statement reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportRecord {
    pub source: Provenance,
    /// Broker transaction identifier, if the statement has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub account: AccountHint,
    pub kind: Kind,
    pub date: Date,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle_date: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<InstrumentHint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Dec>,
    /// Unit price in the instrument's price currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<Money>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gross: Option<Money>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub charges: Vec<Charge>,
    /// Amount that actually moved in or out of cash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net_cash: Option<Money>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<Fx>,
    /// Only on `exchange` and `spinoff`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legs: Vec<Leg>,
    /// Only on `split`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<SplitRatio>,
    /// Only on `position_in`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_basis: Option<Money>,
    /// Only on `position_in`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquisition_date: Option<Date>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    /// Original source fields, kept for debugging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

/// Expected quantity of one instrument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PositionBalance {
    pub instrument: InstrumentHint,
    pub quantity: Dec,
}

/// Closing balances printed on a statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    pub source: Provenance,
    pub account: AccountHint,
    /// Balances are as of the end of this date.
    pub as_of: Date,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cash: Vec<Money>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<PositionBalance>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Input an adapter could not turn into a record. Shown in review, never persisted as an activity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Position in the source, e.g. `row 87`.
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

/// Everything one adapter run produces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    /// Must be `mizan.import/1`.
    pub schema: String,
    #[serde(default)]
    pub records: Vec<ImportRecord>,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}
