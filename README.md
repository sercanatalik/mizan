# Mizan

Mizan is a local-first, open-source portfolio tracker. Your data stays on your
device, in a SQLite file you own. No account, no server, no subscription.

What sets Mizan apart is **free and open imports**: broker statements come in
through community-maintained adapters, not a paid sync service. Every import is
reviewed before anything is saved, checked against the balances printed on the
statement, and can be undone.

> **Status: early development.** Imports from statement files, the desktop app,
> market data (TEFAS fund prices, US and European stock and ETF prices, TCMB
> exchange rates, with their history) and
> time- and money-weighted returns work today.

## What works today

| Area | Status |
|---|---|
| **Import pipeline** | `mizan.import/1` schema and validation, duplicate detection across overlapping statements, figure consistency checks, balance checks against statement closing balances, review report, all-or-nothing commit, undo |
| **Desktop app** | Tauri app with a sidebar grouped into Portfolio, Records, Reports and Reference; a view bar on every portfolio screen for account scope, period, base currency and Nominal / Real, kept across restarts; an Add menu; a ⌘K command palette for screens, accounts, assets and actions; File, Go, View and Data native menus; local SQLite database |
| **Adapters** | OFX/QFX bank and investment statements, Interactive Brokers Flex Query XML, Anadolu Hayat Emeklilik pension fund history (XLSX); any CSV or XLSX through the in-app column mapper, saved as a reusable mapping; sandboxed TypeScript scripts for irregular formats |
| **Imports screen** | Open a statement or batch, see which adapter read it and switch to another match, review, map accounts and create assets, decide on possible duplicates, commit, browse history, undo an import |
| **Inbox** | One queue of everything that needs you: import decisions, unlinked accounts and assets, failed balance checks, unknown cost basis, stale and missing prices; each with its figures and the action that fixes it; snooze for a day or a week; the sidebar badge counts open items |
| **Holdings** | Positions and cash per account with price, value, weight and unrealized gain; provisional average cost; flags for unknown cost basis and failed balance checks; an inspector with the position's figures, open lots and a one-year price chart |
| **Activity** | Every stored activity with filters, cash totals and the full record behind each row; add buys, sells, deposits, dividends, transfers, splits and more by hand, with the same checks as imports; every activity can be edited, and amended imports keep their imported values for one-click revert |
| **Overview** | Net worth and its change, time- and money-weighted returns with the benchmark, and income for the chosen period; allocation by asset class; value-over-time chart with net contributions; latest balance checks; the newest Inbox items |
| **Income** | Dividends, distributions and interest for the chosen accounts and period: gross, withheld and net in the base currency against the previous period, by month and by instrument, every payment, CSV export; same rules as the tax reports |
| **Performance** | Time- and money-weighted returns per account and in total for 1M, 3M, YTD, 1Y and since start; a benchmark of your choice; growth chart; net contributions. A Nominal / Real switch shows every figure after inflation in your base currency (TRY, USD, EUR or GBP). Days without a market price use trade prices or cost and are marked as estimates |
| **Market data** | Built-in TEFAS (Turkish fund prices), Yahoo Finance (US and European stocks and ETFs, no key), Twelve Data (stocks and ETFs with a free key) and TCMB (exchange rates) providers, plus Eurostat, BLS and ONS inflation indices and the EVDS producer price index (free key), each switchable on Market data › Providers; prices stored as traded, with splits undone; manual prices; refresh on demand or on start; price history downloaded once in the background into a shared cache in `~/.mizan` |
| **Tax** | Working papers for UK, US and Turkish returns: realized gains with UK share matching (same day, 30 days, pool), US first in, first out with short and long terms and wash sales, Turkish first in, first out with TCMB rates and Yİ-ÜFE indexation; dividends and interest with tax withheld; ISA and pension accounts left out; CSV export. No tax owed is computed |
| **Market data** | Every stored price, exchange rate and index series with its history, staleness and gaps; edit, add or ignore values (kept per database, never in the shared cache), and every calculation uses the corrections; CSV export |
| **Assets** | Price source per asset chosen from a provider menu, TEFAS and Yahoo symbol suggestions, manual prices, merging duplicate assets |
| **Adapters screen** | Built-in adapters and saved mappings in one list with use counts; new mapping from a file, rename, export and delete |
| **Exports** | Database backup to a folder you choose, and CSV files of activities, holdings and income |
| **CLI** | `mizan-cli import validate` and `mizan-cli import preview` |

Not yet: prices for Borsa Istanbul and other markets beyond the US and Europe
(use manual prices; returns value them at trade prices meanwhile),
and the AI-assisted column mapper. Every screen shows real data only;
where something can't be computed yet, it says what is missing instead of
showing a number.

## Design direction

Mockups of the target design ("Ledger" direction). They use sample data and
include features that aren't built yet, such as market value and returns.

| Overview | Holdings |
| --- | --- |
| ![Overview mockup: portfolio value, returns and accounts needing attention](docs/images/overview.png) | ![Holdings mockup: positions grouped by account with weights and unrealized gains](docs/images/holdings.png) |
| **Import review** | **Performance** |
| ![Import review mockup: balance checks, duplicate decisions and commit bar](docs/images/imports-review.png) | ![Performance mockup: time- and money-weighted returns against a benchmark](docs/images/performance.png) |

## How imports work

```
statement file --> adapter --> mizan.import/1 batch --> review --> commit
                   (mapping file,   (records, balance      (duplicates,
                    TS script or     assertions,            warnings,
                    native Rust)     diagnostics)           balance checks)
```

- **Adapters report; the core interprets.** Adapters write down what the
  statement says, with account and instrument *hints*. Mizan resolves,
  deduplicates and verifies.
- **Balance checks.** Closing balances on a statement are compared with what
  Mizan computes, so a missing transaction is caught at import time.
- **Safe re-imports.** Overlapping statements are deduplicated by fingerprint.
  Matches that can't be decided automatically are left to you.
- **Nothing is final.** Every import can be undone as a whole.

The schema is documented in [docs/import-schema.md](docs/import-schema.md) and
published as JSON Schema in [`schema/`](schema/). Writing an adapter (a
mapping file, a script or native code) is covered in
[docs/adapters.md](docs/adapters.md).

## Getting started

### Desktop app

Requires stable Rust, Node.js 24 and the Tauri CLI
(`cargo install tauri-cli --version "^2"`). On Linux, also install WebKitGTK;
see [docs/desktop.md](docs/desktop.md).

```sh
cd apps/desktop
npm install
cargo tauri dev                  # run with hot reload
cargo tauri build --bundles app  # release app bundle under target/release/bundle/
```

To try it, open **Imports** and load a sample statement from
`adapters/*/fixtures/` (for example
`adapters/ibkr-flex/fixtures/monthly/input.xml`), or the example batches in
`crates/mizan-core/tests/data/e2e/` (`seed.json`, then `batch.json`).

Your data is one SQLite file in the app data folder. See
[docs/desktop.md](docs/desktop.md) for its location, the import flow and what
each screen shows.

### Command line

```sh
cargo run -p mizan-cli -- import validate batch.json
cargo run -p mizan-cli -- import preview batch.json --db mizan.db
```

`preview` prints the review report and writes nothing.

### Tests

```sh
cargo test --workspace
cd apps/desktop && npm test
```

## Project layout

```
crates/mizan-import    mizan.import/1 schema, validation, JSON Schema
crates/mizan-storage   SQLite store and migrations
crates/mizan-core      import pipeline, balance replay, read models, fixture harness
crates/mizan-market    market data providers (TEFAS, TCMB, Yahoo Finance, Twelve Data) and refresh
crates/mizan-adapters  adapter runtime: detection, mapping engine, script sandbox, OFX and IBKR Flex
adapters/              bundled adapters with their fixtures, and the script API types
crates/mizan-cli       command-line tools
apps/desktop           React + TypeScript frontend
apps/desktop/src-tauri Tauri host: commands, menu, database location
docs/                  schema, adapter, market data, returns and desktop documentation
```

## Roadmap

1. AI-assisted column mapper that writes mapping files; more bundled adapters
2. Prices for more markets (Borsa Istanbul)
3. Holdings cost from tax lots instead of provisional average cost
4. Official TÜFE through the EVDS service for real returns

## Contributing

Contributions are welcome, especially broker adapters. Read
[CONTRIBUTING.md](CONTRIBUTING.md) first: it explains the clean-room rule and
how adapter fixtures work, and [docs/adapters.md](docs/adapters.md) for how to
write one.

## License

[MIT](LICENSE)
