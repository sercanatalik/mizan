# Adapters

An adapter reads a statement file (CSV, XLSX, PDF, OFX/QFX, XML) and writes
down what it says as a `mizan.import/1` batch: records, balance assertions and
diagnostics (see [import-schema.md](import-schema.md)). Adapters report; the
core interprets. Mizan resolves accounts and instruments, removes duplicates,
checks balances and shows everything for review before anything is saved.

```
statement file --> detection --> adapter --> batch --> validation --> review --> commit
```

## Choosing a type

| Type | Write it when | You write |
|---|---|---|
| **Mapping** | The file is a table (CSV or XLSX) with one row per transaction | A TOML file; no programming |
| **Script** | Rows must be grouped or combined, the layout is irregular, or the file is a PDF | TypeScript, run in a sandbox |
| **Native** | The format is a published standard used by many institutions (OFX, Flex Query) | Rust, inside `mizan-adapters` |

Start with a mapping. Move to a script only when a mapping cannot express the
file; the [Anadolu Hayat Emeklilik adapter](../adapters/anadolu-hayat/script.ts)
is a script because a contribution day becomes a deposit plus several fund
purchases.

## Built-in adapters

| Id | Reads | Type |
|---|---|---|
| `ofx` | OFX 1.x (SGML) and 2.x (XML) bank and investment statements, `.ofx`/`.qfx` | native |
| `ibkr-flex` | Interactive Brokers Flex Query XML: trades, cash transactions, open positions, ending cash | native |
| `anadolu-hayat` | Anadolu Hayat Emeklilik "Geçmiş Fon İşlemleri" XLSX (private pension fund history) | script |
| `generic-bank-csv-tr` | A Turkish bank account CSV (`;`, windows-1254, `1.234,56`); a starting point to copy | mapping |

Adapters emit balance assertions only when the statement itself states closing
balances or holdings. A transaction history without holdings produces none.

## Package layout

Bundled adapters live in this repository and ship with the app:

```text
adapters/<id>/
    adapter.toml          manifest (and, for mappings, the whole mapping)
    script.ts             script adapters only
    fixtures/<case>/
        input.<ext>       anonymized or synthetic sample statement
        expected.json     the exact batch the adapter must produce
```

## The manifest

Every adapter starts with an `[adapter]` table:

```toml
[adapter]
id = "my-bank-csv"        # lowercase letters, digits and hyphens; unique
version = "1.0.0"         # semantic version; bump it when the output changes
name = "My Bank CSV"      # shown in the app
type = "mapping"          # "mapping", "script" or "native"
files = ["csv"]           # csv, xlsx, pdf, xml, ofx, text
description = "Account history export"   # optional
# script = "script.ts"    # script adapters
# native = "ofx"          # native adapters: the built-in parser's name
```

The id and version are recorded on every imported record, so you can always
tell which adapter (and which version of it) produced a transaction.

## Detection

When a file is opened, every adapter whose `files` include the file's type
scores it from 0 (not mine) to 100. The best match is preselected and the
review lets you switch to any other match. A file nobody recognizes is
reported as such; for CSV and XLSX the app then offers the column mapper.

- Mappings score 90 when all `required_columns` are present, 95 when a
  `file_name` pattern also matches (70 when it doesn't), 0 otherwise.
- Scripts and native adapters compute their own score.
- On a tie, your own mappings win over bundled adapters.

A `mizan.import/1` JSON batch never goes through an adapter.

## Mapping files

A mapping maps columns to record fields and decides each row's kind with
ordered rules. This example reads a CSV like the one below it.

<!-- mapping-example -->
```toml
[adapter]
id = "example-broker-csv"
version = "1.0.0"
name = "Example broker CSV"
type = "mapping"
files = ["csv"]

[detect]
required_columns = ["Datum", "Omschrijving", "Aantal", "Bedrag", "Valuta", "ISIN", "Order"]
# file_name = "^transactions-.*\\.csv$"   # optional regular expression

[input]
delimiter = ","            # guessed from the header when omitted
# encoding = "windows-1252" # UTF-8 (with or without BOM) when omitted
skip_rows = 0              # rows before the header row
# sheet = "Transactions"   # XLSX: the first sheet when omitted
decimal = ","
thousands = "."
date_format = "%d-%m-%Y"   # chrono/strftime syntax

[account]
hint = { const = "BROKER-1" }

# The first matching rule wins. A row matching no rule is not imported and
# shows up in the review as a diagnostic.
[[rules]]
column = "Omschrijving"
matches = "^Koop"
kind = "buy"

[[rules]]
column = "Omschrijving"
matches = "^Verkoop"
kind = "sell"

[[rules]]
column = "Omschrijving"
equals = "Dividend"
kind = "dividend"

[[rules]]
column = "Omschrijving"
matches = "(?i)^saldo"
kind = "skip"

[fields]
date = { column = "Datum" }
external_id = { column = "Order", optional = true }
quantity = { column = "Aantal", transform = ["abs"], optional = true }
net_cash = { amount = { column = "Bedrag", transform = ["abs"] }, currency = { column = "Valuta" } }
instrument.isin = { column = "ISIN", optional = true }
memo = { column = "Omschrijving" }
```

<!-- mapping-example-input -->
```csv
Datum,Omschrijving,Aantal,Bedrag,Valuta,ISIN,Order
02-10-2026,Koop 10 @ 98,"10","-985,00",EUR,IE00B4L5Y983,A-1
15-10-2026,Dividend,,"12,40",EUR,IE00B4L5Y983,
31-10-2026,Saldo,,,EUR,,
```

### Field sources

Each field takes one source:

| Source | Meaning |
|---|---|
| `{ column = "Datum" }` | The cell in that column |
| `{ const = "TRY" }` | A fixed value |
| `{ concat = ["Datum", "Tijd"], separator = " " }` | Cells joined (default separator: a space) |

Add `optional = true` to omit the field when the cell is empty instead of
rejecting the row, and `transform = [...]` to change the value, in order:

| Transform | Effect |
|---|---|
| `"trim"`, `"upper"` | Trim whitespace, upper-case |
| `"abs"`, `"negate"` | Absolute value, flip the sign (numbers) |
| `{ regex = "\\((\\w+)\\)" }` | The first capture group (or the whole match) |
| `{ lookup = { "Koop" = "buy" }, default = "other" }` | Replace by table; without `default`, an unknown value rejects the row |

### Field names

Field paths follow the record schema: `date`, `settle_date`, `external_id`,
`quantity`, `memo`, `net_cash.amount` and `net_cash.currency`, `price.*`,
`gross.*`, `fx.rate`/`fx.from`/`fx.to`, `instrument.isin`, `instrument.symbol`,
`instrument.name`, `instrument.currency`, `instrument.class_hint` and so on.
`date` is required. Fields ending in `amount`, `quantity`, `rate` or
`multiplier` are read as numbers with the declared separators; `date`,
`settle_date` and other dates with `date_format`. A cell that does not parse
produces a diagnostic for its row and no record. Amounts are stored as
magnitudes; the kind says which way the money moved.

### Rules

A rule tests one column with `equals` (exact text) or `matches` (a regular
expression) and gives a record kind (`deposit`, `withdrawal`, `buy`, `sell`,
`dividend`, `interest`, `fee`, `tax`, … see the schema) or `"skip"` for rows
that carry no transaction (opening balances, subtotals).

### The column mapper

For an unrecognized CSV or XLSX file, **Imports › Open column mapper** builds
a mapping interactively: file options, a column or fixed value for each field,
kind rules and a live preview. **Save mapping** writes it to the `adapters`
folder next to your database (for example
`~/Library/Application Support/org.mizan.desktop/adapters/` on macOS); it is
loaded at start and recognized for future files with the same columns.
**Export…** copies the file elsewhere, for example to contribute it.

User mappings are plain TOML: edit them by hand, then restart the app. A user
mapping cannot take the id of a built-in adapter.

### The Adapters screen

**Records › Adapters** lists every adapter the app can use: the built-in ones
and the mappings you saved, with type (native, script or mapping), the file
types each reads, and how many stored imports used it. A saved file that did
not load is listed with the reason (for example an id that a built-in adapter
already uses).

- **New mapping from file…** picks a CSV or XLSX file and opens the column
  mapper for it, even when an adapter already recognizes the file.
- **Rename** changes a saved mapping's display name; its id and file stay, so
  detection and earlier imports are unaffected.
- **Export** writes the manifest to a `.toml` file you choose. Built-in
  adapters can be exported too (their `adapter.toml`), as a starting point.
- **Delete** removes a saved mapping after you confirm. It is no longer
  offered for new files; imports already made with it keep their records.

Built-in adapters cannot be renamed or deleted.

## Scripts

A script adapter is `adapters/<id>/script.ts`. It exports `detect` and `run`;
types are stripped at load time and the code runs in an embedded QuickJS
engine. The full API is in [`adapters/mizan-adapter.d.ts`](../adapters/mizan-adapter.d.ts).

<!-- script-example -->
```ts
export function detect(input: MizanInput): number {
  return input.rows?.[0]?.cells[0] === "Trade Date" ? 80 : 0;
}

export function run(input: MizanInput): MizanOutput {
  const records: MizanRecord[] = [];
  for (const { row, cells } of (input.rows ?? []).slice(1)) {
    const [date, action, amount] = cells;
    if (action !== "Deposit") {
      throw Object.assign(new Error(`unknown action "${action}"`), { locator: `row ${row}` });
    }
    records.push({
      locator: `row ${row}`,
      account: { hint: "EXAMPLE" },
      kind: "deposit",
      date: mizan.date(date, "%m/%d/%Y"),
      net_cash: { amount: mizan.decimal(amount, { decimal: ".", thousands: "," }), currency: "USD" },
    });
  }
  return { records };
}
```

What a script receives:

- **CSV and XLSX:** `input.rows`, the first row of the file first (no header
  handling), each with its 1-based row number. Blank rows are skipped.
- **PDF:** `input.pages`, the text of each page. Only PDFs with a text layer
  work; scanned images do not.
- **Other text** (XML, OFX, text): `input.text`.

What it returns: records without `source` (Mizan fills in provenance from each
record's `locator`, the adapter id and version, and the file's SHA-256),
optional assertions and diagnostics. Throwing an `Error` fails the import with
the message; give it a `locator` property to point at the row.

Host helpers, all pure: `mizan.decimal(text, { decimal, thousands })`,
`mizan.date(text, format)` and `mizan.log(message)` (an info line in the review).
Do decimal arithmetic on integers (for example kuruş or cents), not floats.

### The sandbox

- No network, no file system, no modules: everything comes in through `input`.
- No clock or randomness: `Date.now()`, `new Date()` and `Math.random()` throw,
  so the same file always gives the same batch. `new Date(y, m, d)` works.
- 64 MB of memory and 10 seconds per call; exceeding either fails the import
  with "memory limit exceeded" or "time limit exceeded".

Type-check scripts with:

```sh
cd apps/desktop && npx tsc --noEmit -p ../../adapters/tsconfig.json
```

Only bundled scripts run for now; users can add mappings, not scripts.

## Native adapters

Native adapters are Rust modules in `crates/mizan-adapters/src/native/`,
registered by name in `native::build`. Their package in `adapters/<id>/` holds
only the manifest (`type = "native"`, `native = "<name>"`) and fixtures. Keep
them tolerant: unknown sections become info diagnostics naming the section,
never a crash.

## Errors and validation

Every adapter's output is validated as a `mizan.import/1` batch before review.
Invalid items appear as invalid items in the review; they don't abort the
import. An adapter failure (a parse error, a script exception, a sandbox limit)
is shown as `adapter <id> failed at <locator>: <message>` and opens no review.
Nothing is written to the database until you commit.

## Fixtures and the CI gate

Each bundled adapter needs at least one fixture case. CI runs every adapter
against its fixtures with the real runtime and fails on any difference:

```sh
cargo test -p mizan-adapters --test fixtures_gate
```

After an intended change, regenerate the expected batches and review the diff
before committing:

```sh
MIZAN_BLESS=1 cargo test -p mizan-adapters --test fixtures_gate
git diff adapters/
```

A new case only needs `input.<ext>`; blessing creates `expected.json`. To see
what a batch would do to a database, run
`cargo run -p mizan-cli -- import preview adapters/<id>/fixtures/<case>/expected.json --db scratch.db --create-missing`.

## Privacy rules for samples

Fixtures are public. Never commit a real statement, even briefly: git history
keeps it.

- Prefer a **synthetic** file with the same structure (sheet names, headers,
  number and date formats, the kinds of rows) to an edited real one.
- Replace account and contract numbers, names, addresses, IBANs, tax ids and
  transaction ids with fake values in the same format.
- Change amounts and dates; nothing should link back to a real person.
- Check the file itself, not only what a spreadsheet app shows: XLSX files
  keep shared strings, metadata and hidden sheets.
- Check your real file locally with the app or a scratch test, and keep it out
  of the repository.
