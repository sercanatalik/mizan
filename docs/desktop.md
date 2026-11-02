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
providers enabled on Market data › Providers: `www.tefas.gov.tr`, `www.tcmb.gov.tr`, and for
assets you assigned to them, `query1.finance.yahoo.com` and
`api.twelvedata.com`. For real returns it asks only the inflation provider of
your base currency, at most once a day: `ec.europa.eu` (TRY, EUR),
`api.bls.gov` (USD) or `www.ons.gov.uk` (GBP). Once you save an EVDS key, it
also asks `evds3.tcmb.gov.tr` for the Turkish producer price index, at most once
a day. Tax reports never use the network. See [market-data.md](market-data.md).

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

## Reset data

**Settings › Danger zone › Reset data** starts over. Choose what to reset:

| Scope | Clears | Keeps |
|---|---|---|
| Portfolio data | Accounts and their statement identifiers, assets and price sources, merges, imports, activities, balance checks, amendments, manual prices, prices and rates stored in this database, series edits | Settings, API keys, saved column mappings, the shared market history in `~/.mizan` |
| Everything | All of the above, settings and API keys (back to their defaults), and the shared market history, which is downloaded again on the next refresh | Saved column mappings |

The screen first shows what will be cleared, and for **Everything** warns that
other Mizan databases on this computer share the market history. You type
`RESET` to confirm. Before anything is cleared, Mizan:

- stops a running refresh or history download;
- closes an open import review;
- writes backups next to the originals: `mizan-backup-<YYYYMMDD-HHMMSS>.db` next
  to `mizan.db`, and with **Everything** also
  `market-history-backup-<YYYYMMDD-HHMMSS>.sqlite` next to the cache.

If a backup can't be written, nothing is cleared. Each file is cleared in one
transaction, and every screen refreshes afterwards.

**Restoring a backup:** quit Mizan, rename the backup to `mizan.db` (or
`market-history.sqlite`) in place of the current file, and start Mizan again.

## Screens and what they show

Every number comes from your database. Where something cannot be computed yet,
the screen says what is missing instead of showing a number.

| Screen | Shows now | Waiting for |
|---|---|---|
| Overview | Portfolio total and per-account totals in the base currency, value over time with net contributions (estimated stretches shaded), accounts with their latest balance check (each opens its account page), items needing attention | Price history for days not downloaded yet |
| Inbox | One queue of everything that needs you before the numbers can be trusted, with a type filter, a detail pane, the action that fixes each item and a snooze (see [Inbox](#inbox)); the sidebar badge counts open items | |
| Holdings | Positions and cash per account; price (as of), value, weight, unrealized gain; provisional average cost; flags | |
| Activity | Every stored activity, newest first, 200 at a time: date, account, kind, instrument, quantity, price, net cash, charges and source (import file and row, or manual). Filters by account, kind, instrument, dates and source, with cash in, out and net per currency; each row opens the full record. **Add activity** enters buys, sells, deposits, withdrawals, dividends, interest, fees, taxes, transfers and splits by hand; manual entries can be edited and deleted; imported ones can be amended (the imported values are kept and can be restored) and link to their import; changed activities carry an Edited tag | |
| Performance | Time- and money-weighted returns (portfolio and per account) for 1M, 3M, YTD, 1Y and since start, benchmark return and growth chart, net contributions; a Nominal / Real switch for inflation-adjusted figures; estimates and missing exchange rates are labelled (see [returns.md](returns.md)) | |
| Imports | Statement files through adapters, column mapper, full review, commit, history and undo | AI-assisted column mapper |
| Assets | Price source per asset from a provider menu (TEFAS, Yahoo Finance, Twelve Data, manual), TEFAS and Yahoo suggestions, manual prices, merging duplicates; a Benchmark switch per asset and an Add benchmark form for instruments you don't hold, whose history is downloaded from the holding account's first activity (or your first activity when not held) | |
| Tax | UK, US and Turkish working papers for a tax year: realized gains with each country's matching rules, dividends and interest with tax withheld, rows that could not be computed and why, account tax treatment (taxable, ISA, pension) and CSV export (see [tax.md](tax.md)) | |
| Market data | Every stored price, rate and index series (Prices, Rates, Indices tabs) with latest value, history, staleness, gaps while held and series gaps; filters; a chart and table per series; edit, add and ignore values per database, with Edited and Ignored tags and Revert; CSV export (see [market-data.md](market-data.md#browsing-and-correcting-market-data)); the Providers tab holds the providers (on/off, Twelve Data and EVDS API keys), refresh on start, the last refresh report, the inflation index used for real returns, and price history download state with the cache location | |
| Settings | Base currency and data reset; a link to Market data › Providers, where providers now live | |
| Accounts | Every account with its value, activity count, statement identifiers, tax treatment and latest balance check; merged accounts with Undo. Each account opens its own page (see [Accounts](#accounts)) | |
| Income | Dividends, distributions and interest for the selected accounts and period: gross, withheld and net in the base currency against the previous period, net income by month (empty months shown), income by instrument (each opens its payments in Activity), every payment with those that could not be converted for lack of a rate, and CSV export. Uses the same gross and withholding rules as the tax reports, and includes tax-free accounts | |
| Adapters | Built-in adapters and saved mappings with type, file types, origin and use count; new mapping from a file, rename, export and delete for saved mappings (see [adapters.md](adapters.md#the-adapters-screen)) | |
| Exports | Database backup to a folder you choose (`mizan-backup-<date-time>.db`, written in full or not at all), and CSV files of every activity, holdings as of today and income, for the account and period chosen in the view bar; links to the tax report and market data exports | |

Prices come from the built-in TEFAS, Yahoo Finance, Twelve Data and TCMB
providers or from manual entry, inflation indices from Eurostat, BLS and ONS,
and the Turkish producer price index from EVDS; see
[market-data.md](market-data.md) for what is sent where, and how values are computed.

Average cost is provisional: acquisitions add the cash they settled for,
disposals remove cost pro rata, splits keep cost, spinoffs move their cost-basis
share. It shows "not available" (with the reason) for units without a known
basis, purchases in more than one currency, and short positions.

## Inbox

The Inbox lists every condition that needs a decision, worked out from your
data each time it opens; nothing is stored except snoozes. Items disappear on
their own once the condition no longer holds.

| Type | Item | Action |
|---|---|---|
| Imports | An open import review with records awaiting a duplicate decision | Opens the review |
| Imports | An open review naming accounts or assets that are not linked yet | Opens the review |
| Balance checks | An account whose latest balance check failed, with the statement figure, the computed figure and the difference for each failing line | Opens the account page at its balance checks |
| Market data | A held asset whose latest price is more than 7 days old | Opens the asset's price source and manual price controls |
| Market data | A held asset with no price at all | Opens the asset's price source and manual price controls |
| Cost basis | Units that arrived without a cost (for example a transfer in) | Opens the activity form for a `position_in` with the account and asset filled |

Items are listed newest condition first. **Snooze 1 day** or **Snooze 7 days**
hides an item from the list and the badge until the snooze ends or its figures
change (for example a new statement with a different difference), whichever
comes first. Snoozed items are listed under **Snoozed** with their end date and
can be brought back with **Unsnooze**. Resetting portfolio data clears snoozes.

## Accounts

The account page (`/accounts/<id>`) shows one account:

- **Value tiles:** the value in the base currency and in a second currency (TRY,
  USD, EUR or GBP, chosen on the page and saved per database), with the rate
  used; the last-day change against the previous price date; the 1-year return
  (also in the second currency) and year to date; and positions and cash
  apart, so a negative total explains itself.
- **Value at current units:** today's units at each day's prices, stacked per
  position, for 1M to 5Y in either currency. It shows the price effect only;
  the separate value-over-time chart uses the actual units and net
  contributions. Past eight positions, the smallest fold into "Other".
- **Allocation donut, holdings table** (units, price, value in both currencies,
  weight of the priced positions, and day, 1-month and 1-year price returns,
  with positions total, cash and account value rows), **returns** per period, **income** by year with tax
  withheld, the 20 most recent activities (**All activity** opens Activity
  filtered to the account) and the balance-check history.

Managing an account:

- **Rename** and **tax treatment** (the same setting the Tax screen uses).
- **Price source** (Per asset, TEFAS, Yahoo Finance or Twelve Data). With a
  provider chosen, every asset in the account that has no price source gets
  one from its symbol (the symbol is the TEFAS fund code, Yahoo adds the
  exchange suffix). This happens right away and again after each import or
  manual entry into the account. Sources set on the Assets screen are never
  replaced, and assets without a usable symbol are listed and left alone.
  Refresh prices afterwards to fetch them.
- **Statement identifiers** match imported records to the account. Adding or
  moving one affects future imports only; records already stored stay where
  they are. An identifier can belong to only one account.
- **Delete** is offered only for an account with no activities, no balance
  checks and no merge history; otherwise the page says why.
- **Merge…** moves every activity, balance check and identifier into another
  account in one step, after showing how many will move. Transfers between the
  two become internal. Re-importing the same statement is still recognised as
  a duplicate. The merged account disappears from every screen and is listed
  under **Merged accounts** on the Accounts screen, where **Undo merge** moves
  back exactly what the merge moved; anything imported into the target since
  stays there, and identifiers moved elsewhere since are reported and left in
  place. A merge whose target was itself merged later must wait until that
  later merge is undone.

## Typed bridge

Types shared with the frontend are generated from Rust with ts-rs into
`apps/desktop/src/bridge/generated/`. Tests fail when they drift; regenerate with:

```sh
MIZAN_BLESS=1 cargo test -p mizan-core --features ts --test bindings
MIZAN_BLESS=1 cargo test -p mizan-desktop bindings
```
