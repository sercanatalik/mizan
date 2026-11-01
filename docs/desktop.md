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

Frontend checks, as CI runs them:

```sh
cd apps/desktop
npm run typecheck
npm run lint
npm test
```

## Network

The app works offline. It goes online only to refresh prices (Refresh prices,
or once on start when the last refresh is over a day old), and only to the
providers enabled in Settings. See [market-data.md](market-data.md).

## Where your data lives

The app creates one SQLite database, `mizan.db`, in the per-user app data
folder on first launch and migrates it automatically:

| Platform | Location |
|---|---|
| macOS | `~/Library/Application Support/org.mizan.desktop/mizan.db` |
| Windows | `%APPDATA%\org.mizan.desktop\mizan.db` |
| Linux | `~/.local/share/org.mizan.desktop/mizan.db` |

Back up or move that file to back up or move your data. Databases created with
`mizan-cli` open in the app unchanged.

## Importing

Until adapters for raw statements (CSV, PDF, …) are available, the app imports
`mizan.import/1` batch files (see [import-schema.md](import-schema.md)).

1. Open **Imports** and choose **Open batch file** (or **File › Import batch…**, Cmd/Ctrl+O).
2. Review what the batch would change. Nothing is saved yet.
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
| Overview | Portfolio total and per-account totals in the base currency, accounts with their latest balance check, items needing attention | Returns engine for the value chart |
| Holdings | Positions and cash per account; price (as of), value, weight, unrealized gain; provisional average cost; flags | |
| Performance | Net contributions (deposits minus withdrawals) by period | Returns engine for TWR, MWR, benchmark |
| Imports | Full review, commit, history and undo | Adapters for raw statement files |
| Assets | Price source per asset, TEFAS suggestions, manual prices, merging duplicates | |
| Settings | Base currency, market data providers (on/off), refresh on start, last refresh report | |
| Activity, Income, Accounts | Placeholder | Later changes |

Prices come from the built-in TEFAS and TCMB providers or from manual entry; see
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
