# Import schema `mizan.import/1`

Every Mizan adapter, however it is written (mapping file, script or native
Rust), produces one **batch** in this format. Mizan validates the batch,
resolves the hints, removes duplicates, checks balances and shows a review
before anything is saved.

The machine-readable JSON Schema is
[`schema/mizan.import.1.schema.json`](../schema/mizan.import.1.schema.json).
It is generated from the Rust types in `crates/mizan-import`; a test fails if
the committed file falls out of date.

## Principles

- **Adapters report; the core interprets.** Write down what the statement says.
  Do not look up internal ids, compute missing figures or apply exchange rates.
- **Keep redundant figures.** If the statement shows quantity, price, fees and
  net cash, emit all of them. Mizan checks that they add up and warns if they
  don't, without changing any of them.
- **Every item is traceable** to an adapter, a file and a position in it.
- **Amounts are magnitudes.** The kind gives the direction (a `buy` takes cash
  out, a `dividend` brings it in). Signed values are used only for
  `legs[].quantity_delta`, assertion balances, and the `net_cash` and
  `quantity` of `internal_transfer` and `adjustment` (positive means into the
  account).
- **`fx_conversion`:** `gross` is the amount that leaves the account in one
  currency, `net_cash` the amount that arrives in the other.
- **Derivative kinds** (`option_exercise`, `option_assignment`,
  `option_expiry`) move only the option position. Delivery of the underlying
  and any cash come as their own trade records.

## Value types

| Type | Format | Example |
|---|---|---|
| Decimal | JSON **string** matching `-?digits(.digits)?`. JSON numbers are rejected so no precision is lost. | `"20.40"` |
| Code | ISO 4217 currency, or an upper-case asset code (2-12 letters/digits) | `"EUR"`, `"USDC"` |
| Money | `{ "amount": Decimal, "currency": Code }` | `{"amount": "20.40", "currency": "USD"}` |
| Date | `YYYY-MM-DD`, no time component, in the account's local calendar | `"2026-03-14"` |
| Sha256Hex | 64 lower-case hex characters | |

## Batch

| Field | Required | Description |
|---|---|---|
| `schema` | yes | Must be `"mizan.import/1"`. Any other value rejects the whole batch and nothing is saved. |
| `records` | no | Array of import records. |
| `assertions` | no | Array of balance assertions. |
| `diagnostics` | no | Array of diagnostics. |

A batch with a valid version is validated item by item: an invalid record or
assertion is reported with its position and locator and left out; the rest of
the batch is still processed.

## Provenance (`source`)

Required on every record and assertion.

| Field | Description |
|---|---|
| `adapter` | Adapter identifier, e.g. `degiro-csv`. |
| `adapter_version` | Adapter version, e.g. `0.4.0`. |
| `file_sha256` | SHA-256 of the source file. |
| `locator` | Position in the source: `row 184`, `page 3, line 12`. Must not be empty. |

## Import record

| Field | Required | Description |
|---|---|---|
| `source` | yes | Provenance. |
| `external_id` | no | Broker transaction id, when the statement has one. Used for links and duplicate detection. |
| `account` | yes | Account hint. |
| `kind` | yes | One of the kinds below. |
| `date` | yes | Date of the economic event. |
| `settle_date` | no | Settlement date. |
| `instrument` | depends | Instrument hint. Required for `position_in` / `position_out`. |
| `quantity` | depends | Units. Required (positive) for `position_in` / `position_out`. |
| `price` | no | Unit price, as Money in the price currency. |
| `gross` | no | Gross amount before charges. |
| `charges` | no | Typed fees and taxes that belong to this record. |
| `net_cash` | no | Amount that actually moved in or out of cash. |
| `fx` | no | `{ "rate": Decimal, "from": Code, "to": Code }`, only when the statement shows a conversion. `rate` is units of `to` per one `from`. |
| `legs` | depends | Only on `exchange` and `spinoff`, where at least one is required. |
| `split` | depends | `{ "new": Decimal, "old": Decimal }`, both positive. Required on `split`, not allowed elsewhere. |
| `cost_basis` | no | Only on `position_in`. When absent, the position's basis is **unknown**, never zero. |
| `acquisition_date` | no | Only on `position_in`. |
| `links` | no | Links to other records by external id. |
| `memo` | depends | Free text from the statement. Required on `adjustment`. |
| `raw` | no | Original source fields, for debugging. Any JSON value. |

Fields not listed here are rejected. In particular, fields that look like
internal Mizan ids (`id`, `account_id`, `asset_id`, `activity_id`,
`instrument_id`, `batch_id`) are rejected anywhere in a record: adapters
reference accounts and instruments only by hints.

### Kinds

| Family | Kinds |
|---|---|
| Cash | `deposit`, `withdrawal`, `fx_conversion`, `internal_transfer` |
| Trade | `buy`, `sell`, `short_sell`, `buy_to_cover` |
| Income | `dividend`, `interest`, `distribution`, `reward` (staking, airdrops, broker bonuses) |
| Charge | `fee`, `tax`: only for charges with no parent record |
| Position move | `position_in`, `position_out`: securities moving between brokers |
| Corporate action | `split`, `exchange` (mergers, symbol changes), `spinoff`, `return_of_capital` |
| Derivative | `option_exercise`, `option_assignment`, `option_expiry` |
| Correction | `adjustment`: always flagged, memo required |

There is no "unknown" kind. Input an adapter cannot classify becomes a
diagnostic.

Short positions use `short_sell` and `buy_to_cover` explicitly. Mizan never
infers a short from missing history.

### Account hint (`account`)

| Field | Required | Description |
|---|---|---|
| `hint` | yes | Identifier as printed, e.g. `U1234567`. |
| `name_hint` | no | Human-readable name. |

### Instrument hint (`instrument`)

At least one of `isin`, `cusip`, `figi`, `symbol` or `name` is required.

| Field | Description |
|---|---|
| `isin` | ISIN. |
| `cusip` | CUSIP. |
| `figi` | OpenFIGI identifier. |
| `symbol` | Ticker as printed. |
| `mic` | ISO 10383 market identifier, e.g. `XNAS`. |
| `name` | Name as printed. |
| `class_hint` | `equity`, `etf`, `fund`, `bond`, `option`, `crypto`, `commodity`, `cash`, `other`. |
| `currency` | Quote currency. |
| `multiplier` | Contract multiplier (Decimal), e.g. `"100"`. |
| `option` | `{ "underlying": string, "expiry": Date, "strike": Decimal, "right": "call" \| "put" }`. |

### Charge (`charges[]`)

| Field | Description |
|---|---|
| `type` | `commission`, `exchange_fee`, `stamp_duty`, `transaction_tax`, `withholding_tax`, `other`. |
| `amount` | Money, not negative. |

### Leg (`legs[]`)

| Field | Required | Description |
|---|---|---|
| `instrument` | yes | Instrument hint. |
| `quantity_delta` | yes | Signed change in quantity of that instrument. |
| `cost_basis_share` | no | Fraction 0-1 of the original cost basis allocated to this leg. |

### Link (`links[]`)

| Field | Description |
|---|---|
| `role` | `reverses` (a cancellation; the original stays in history), `transfer_pair`, `related`. |
| `target` | External id of the other record. |

## Balance assertion

Closing balances printed on the statement. Mizan replays activities to the end
of `as_of` and reports each assertion as passed or failed.

| Field | Required | Description |
|---|---|---|
| `source` | yes | Provenance. |
| `account` | yes | Account hint. |
| `as_of` | yes | Balances hold at the end of this date. |
| `cash` | * | Money per currency. |
| `positions` | * | `{ "instrument": InstrumentHint, "quantity": Decimal }` per instrument. |

\* At least one cash balance or position is required.

## Diagnostic

| Field | Required | Description |
|---|---|---|
| `severity` | yes | `info`, `warning` or `error`. |
| `message` | yes | What went wrong. |
| `locator` | yes | Position in the source. |
| `raw` | no | Original source fields. |

Diagnostics are shown during review and are never saved as activities.

## Example

```json
{
  "schema": "mizan.import/1",
  "records": [
    {
      "source": {
        "adapter": "ibkr-flex",
        "adapter_version": "1.2.0",
        "file_sha256": "9f2c00000000000000000000000000000000000000000000000000000000a71e",
        "locator": "row 184"
      },
      "external_id": "1234567890",
      "account": { "hint": "U1234567", "name_hint": "IBKR Margin" },
      "kind": "dividend",
      "date": "2026-03-14",
      "instrument": { "isin": "US0378331005", "symbol": "AAPL", "mic": "XNAS", "currency": "USD" },
      "quantity": "100",
      "gross": { "amount": "24.00", "currency": "USD" },
      "charges": [
        { "type": "withholding_tax", "amount": { "amount": "3.60", "currency": "USD" } }
      ],
      "net_cash": { "amount": "20.40", "currency": "USD" }
    }
  ],
  "assertions": [
    {
      "source": {
        "adapter": "ibkr-flex",
        "adapter_version": "1.2.0",
        "file_sha256": "9f2c00000000000000000000000000000000000000000000000000000000a71e",
        "locator": "page 4"
      },
      "account": { "hint": "U1234567" },
      "as_of": "2026-03-31",
      "cash": [{ "amount": "1520.33", "currency": "USD" }],
      "positions": [{ "instrument": { "isin": "US0378331005" }, "quantity": "100" }]
    }
  ],
  "diagnostics": [
    { "severity": "warning", "message": "No rule for description \"Zinsgutschrift Korrektur\"", "locator": "row 87" }
  ]
}
```

## Command-line tools

`mizan-cli` checks batches and previews imports without touching your data.

```sh
cargo build -p mizan-cli

# Is this a valid mizan.import/1 batch? Exits 1 if any item is invalid.
mizan-cli import validate batch.json

# What would importing this batch do to the database at mizan.db?
# Nothing is written; if mizan.db does not exist, an empty database is assumed.
mizan-cli import preview batch.json --db mizan.db

# Treat unknown accounts and instruments as new instead of unresolved,
# and print the full review report as JSON.
mizan-cli import preview batch.json --db mizan.db --create-missing --json
```

`preview` prints the review: new records, duplicates, records that need your
decision, figure warnings, ordering questions, adapter diagnostics, invalid
items, unresolved hints, balance checks, flagged adjustments and positions with
unknown cost basis.

## Adapter fixtures

Every adapter ships fixtures, one directory per case:

```text
fixtures/<adapter-id>/<case>/
    input.<ext>      anonymized source file
    expected.json    the exact batch the adapter must produce
```

The harness in `mizan_core::fixture` passes the input bytes and their SHA-256
to the adapter, checks that the output is a clean batch, and compares it with
`expected.json`. A mismatch fails with a line diff.
