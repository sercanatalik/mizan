# Desktop app

The Mizan desktop app is a Tauri v2 application with a React + TypeScript
frontend. Everything stays on your machine: there is no server and no account.

```
apps/desktop/              React + TypeScript + Vite frontend (npm)
apps/desktop/src-tauri/    Rust host (crate mizan-desktop): commands, menu, database
```

## Prerequisites

- A recent stable Rust toolchain (see `rust-version` in `Cargo.toml`)
- Node.js 24 and npm
- The Tauri CLI: `cargo install tauri-cli --version "^2"`
- On Linux, WebKitGTK and friends:
  `sudo apt-get install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`

## Run and build

```sh
cd apps/desktop
npm install

# Development: starts Vite on :5173 and opens the app with hot reload.
cargo tauri dev

# Release build: an app bundle and installer under target/release/bundle/.
cargo tauri build

# Only the app bundle (skips installers such as the macOS .dmg).
cargo tauri build --bundles app
```

On macOS the `.dmg` step drives Finder through AppleScript. If it fails with
"Finder got an error", allow your terminal to control Finder (System Settings ›
Privacy & Security › Automation) or build with `--bundles app`.

### Release size

Measured with `cargo tauri build --bundles app` on macOS (Apple silicon), from
a clean checkout:

| Version | `mizan-desktop` binary | `Mizan.app` | Clean release build |
|---|---|---|---|
| Market data (before the adapter runtime) | 20.4 MB | 19.5 MiB | 39 s |
| Adapter runtime (QuickJS, TypeScript stripping, XLSX/PDF/OFX readers) | 30.0 MB | 28.7 MiB | 72 s |
| Returns engine (history cache, backfill, returns, charts) | 30.6 MB | 29.3 MiB | not measured clean |

The adapter runtime adds about 9.6 MB; the returns engine about 0.6 MB. Polars was evaluated for the returns
engine and not adopted: a minimal program using it was 57 MB stripped, and its
decimal type rounds products to a fixed scale.

Frontend checks, as CI runs them:

```sh
cd apps/desktop
npm run typecheck
npm run lint
npm test
```

## Network

The app works offline. It goes online only to refresh prices (Refresh prices,
or once on start when the last refresh is over a day old), and to download
missing price history in the background after such a refresh, and only to the
providers enabled in Settings: `www.tefas.gov.tr`, `www.tcmb.gov.tr`, and for
assets you assigned to them, `query1.finance.yahoo.com` and
`api.twelvedata.com`. For real returns it asks only the inflation provider of
your base currency, at most once a day: `ec.europa.eu` (TRY, EUR),
`api.bls.gov` (USD) or `www.ons.gov.uk` (GBP). See
[market-data.md](market-data.md).

## Where your data lives

The app creates one SQLite database, `mizan.db`, in the per-user app data
folder on first launch and migrates it automatically:

| Platform | Location |
|---|---|
| macOS | `~/Library/Application Support/org.mizan.desktop/mizan.db` |
| Windows | `%APPDATA%\org.mizan.desktop\mizan.db` |
| Linux | `~/.local/share/org.mizan.desktop/mizan.db` |

Column mappings you save are TOML files in the `adapters` folder beside it.
Past fund prices and exchange rates are kept separately in
`~/.mizan/market-history.sqlite` (or under `$MIZAN_HOME`), shared by every
database on the computer; it holds public market data only and can be deleted
safely.
Back up or move `mizan.db` (and the `adapters` folder) to back up or move your data. Databases created with
`mizan-cli` open in the app unchanged.

## Importing

The app opens statement files (CSV, XLSX, PDF, OFX/QFX, XML) and
`mizan.import/1` batch files (see [import-schema.md](import-schema.md)).

1. Open **Imports** and choose **Open file** (or **File › Import file…**, Cmd/Ctrl+O).
   - A batch file opens directly.
   - A statement goes through the adapters (see [adapters.md](adapters.md)).
     The best match reads it; the review header shows which one
     ("ibkr-flex 0.1.0"). When several adapters match, pick another under
     **Adapter**; the file is read again and earlier decisions are cleared.
   - A file no adapter recognizes is reported. For CSV and XLSX, **Open column
     mapper** lets you map its columns with a live preview and save the mapping
     under a name; the next file with the same columns is recognized
     automatically. Saved mappings live in the `adapters` folder next to the
     database and can be exported to share.
2. Review what the file would change. Nothing is saved yet.
   - **Unresolved:** create an account or map the statement's account identifier
     to an existing account; create assets for unknown instruments.
   - **Need your decision:** a record matches one imported from a different file
     and has no transaction id. Choose **Second trade** or **Duplicate**.
     Balance checks update after every choice.
   - **Balance checks:** closing balances printed on the statement compared with
     what Mizan computes. A failed check usually means a missing record.
3. **Commit.** Records still awaiting a decision are skipped.
4. **Import history** lists every import. **Undo** removes one import as a whole
   (its records and balance checks); accounts and assets are kept.

## Screens and what they show

Every number comes from your database. Where something cannot be computed yet,
the screen says what is missing instead of showing a number.

| Screen | Shows now | Waiting for |
|---|---|---|
| Overview | Portfolio total and per-account totals in the base currency, value over time with net contributions (estimated stretches shaded), accounts with their latest balance check, items needing attention | Price history for days not downloaded yet |
| Holdings | Positions and cash per account; price (as of), value, weight, unrealized gain; provisional average cost; flags | |
| Performance | Time- and money-weighted returns (portfolio and per account) for 1M, 3M, YTD, 1Y and since start, benchmark return and growth chart, net contributions; a Nominal / Real switch for inflation-adjusted figures; estimates and missing exchange rates are labelled (see [returns.md](returns.md)) | |
| Imports | Statement files through adapters, column mapper, full review, commit, history and undo | AI-assisted column mapper |
| Assets | Price source per asset from a provider menu (TEFAS, Yahoo Finance, Twelve Data, manual), TEFAS and Yahoo suggestions, manual prices, merging duplicates | |
| Settings | Base currency and the inflation index used for real returns, market data providers (on/off, Twelve Data API key), refresh on start, last refresh report, price history download state and cache location | |
| Activity, Income, Accounts | Placeholder | Later changes |

Prices come from the built-in TEFAS, Yahoo Finance, Twelve Data and TCMB
providers or from manual entry, and inflation indices from Eurostat, BLS and ONS; see
[market-data.md](market-data.md) for what is sent where, and how values are computed.

Average cost is provisional: acquisitions add the cash they settled for,
disposals remove cost pro rata, splits keep cost, spinoffs move their cost-basis
share. It shows "not available" (with the reason) for units without a known
basis, purchases in more than one currency, and short positions.

## Typed bridge

Types shared with the frontend are generated from Rust with ts-rs into
`apps/desktop/src/bridge/generated/`. Tests fail when they drift; regenerate with:

```sh
MIZAN_BLESS=1 cargo test -p mizan-core --features ts --test bindings
MIZAN_BLESS=1 cargo test -p mizan-desktop bindings
```
