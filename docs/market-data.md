# Market data

Mizan values positions from prices and exchange rates stored on your device.
They arrive in one of two ways: a **refresh** asks the built-in providers for
new data, and **manual prices** are entered by you. Nothing is fetched at any
other time.

## Built-in providers

| Provider | Supplies | Sent over the network |
|---|---|---|
| **TEFAS** | Daily unit prices of Turkish investment (`YAT`) and pension (`EMK`) funds, in TRY | Refresh: one request per fund type for the last 7 days (no fund codes, no account data). History: one request per fund code and month, once |
| **TCMB** | Daily exchange rates published by the Turkish central bank | Refresh: one request for `today.xml`. History: one archive file per business day, once |

Both are enabled by default. In **Settings › Market data providers**, each has an
On/Off switch. A disabled provider makes no network requests, and the prices or
rates it stored earlier are still used for valuation. Requests identify
themselves as `Mizan/<version>`; Mizan never imitates a browser.

## Price history cache

Daily fund prices and exchange rates from the past are kept in one cache file
per computer, shared by every Mizan database on it:

| | |
|---|---|
| Location | `~/.mizan/market-history.sqlite`, or `$MIZAN_HOME/market-history.sqlite` when `MIZAN_HOME` is set |
| Contents | TEFAS fund prices by fund code, TCMB exchange rates by currency, the fund type of each code, and a record of every month or day already fetched |
| Never contains | Accounts, statement identifiers, quantities, activities, manual prices, or anything else from your database |

Every refresh also writes the prices and rates it fetched into the cache. Each
fund-month and each TCMB day is downloaded once; months and days that returned
nothing (weekends, holidays, dates before a fund existed) are recorded too, so
they are not asked for again. Only the current month and the last 7 days are
checked again for newly published data.

### History backfill

After every successful refresh, Mizan fills in missing history in the
background:

- **Fund prices** for each asset priced from TEFAS, from its first activity,
  one fund and one month per request, at least 10 seconds apart. The benchmark
  fund (Performance) is downloaded from the portfolio's first activity, even if
  you don't hold it. History is planned from your activities, so nothing is
  downloaded until something is imported; a new benchmark is picked up at the
  next refresh.
- **Exchange rates** from TCMB's daily archive
  (`https://www.tcmb.gov.tr/kurlar/YYYYMM/DDMMYYYY.xml`), one business day per
  request, one a second, from the first activity in a currency other than TRY
  (or from the first activity when the base currency isn't TRY). A
  TRY-only portfolio needs no exchange-rate history.
- **Newest first**, so recent periods are complete within minutes, and both
  providers are interleaved.
- **Resumable:** each month or day is saved as soon as it arrives. Quitting the
  app stops the backfill; the next refresh continues where it stopped.
- **Failures** stop that provider for the run without touching cached data;
  the other provider continues. The error shows in Settings.
- **Five-year limit:** TEFAS serves no prices older than five years. Returns
  value older days at trade prices from your own activities, marked as
  estimates.

**Settings › Price history** shows each provider's state (downloading with
"34 of 120 months", complete, waiting for the next refresh, failed with the
reason, off, or not needed) and the cache location. A disabled provider
downloads no history, and with "refresh on start" off and no manual refresh,
nothing is downloaded at all. The first backfill for a long history takes a
while (about 80 minutes for 8 funds over five years); it happens once per
computer.

The cache holds public data only, so deleting `~/.mizan` is safe: the history
is downloaded again when needed. Your database stays in the app data folder
(see [desktop.md](desktop.md)).

## Refreshing

- **Refresh prices** (on Overview, Holdings, Assets and Settings) runs one refresh.
- **On start:** with "Refresh prices when the app starts, if the last refresh is
  older than a day" checked (the default), a refresh runs in the background
  after the window opens when the last successful refresh is more than 24 hours
  old. Uncheck it to keep the app fully offline.
- **Failures:** a failing provider never stops the other one, and never deletes
  or changes stored data. Settings shows the last refresh report per provider:
  OK, Failed (with the reason), Disabled, or Nothing to fetch.
- **Concurrency:** only one refresh runs at a time.

## Price sources

Each asset has one price source, set on the **Assets** screen with **Edit**:

- **Use TEFAS** with a fund code (for example `AFT`).
- **Manual:** only the prices you enter.
- **None** (the default): the asset shows "no price".

When an asset looks like a TEFAS fund (class `fund` or a TRY currency, with a
three-letter upper-case symbol), Assets shows "Suggested: TEFAS AFT" with an
**Accept** button. Nothing changes until you accept. Changing a source keeps the
prices already stored.

## Manual prices

**Add price** records a price for a date and currency. If a TEFAS price exists
for the same asset and date, the manual price is used; the TEFAS row stays
stored. **Delete** removes a manual price.

## Merging duplicate assets

Statements sometimes name one instrument differently, for example by ISIN in one
file and by symbol in another, which creates two assets. **Merge into…** sends
every position, balance check and future import of one asset to another. Stored
activities are not rewritten, so **Undo merge** restores both assets exactly.

## Valuation rules

- **Price:** a position uses its asset's newest price dated on or before today,
  shown with "as of" and its date. A price older than 7 days gets a **stale**
  badge; the value is still shown.
- **Exchange rates:** TCMB's **forex buying** rate, per one unit (published units
  such as 100 JPY are divided out), dated by TCMB's own date. Weekends and
  holidays use the latest earlier rates. Conversions between two non-TRY
  currencies go through TRY.
- **Base currency:** chosen in **Settings › Valuation** (default TRY). Changing it
  recomputes all totals from stored rates without a refresh.
- **Unrealized gain:** value minus the provisional average cost, only when the
  cost is known and in the price currency. Otherwise it reads "not available"
  with the reason.
- **Totals:** account and portfolio totals add convertible position values and
  cash in the base currency, and say how many positions were excluded for lack
  of a price or exchange rate.

## TEFAS API notes (verified 2026-11-01)

TEFAS publishes no official API documentation. These details were verified on
2026-11-01 from a plain Rust `reqwest` client (rustls) and `curl`, and are
recorded as fixtures in `crates/mizan-market/tests/fixtures/tefas/`.

- **Bot protection:** the website (`https://www.tefas.gov.tr/`) answers plain
  clients with a JavaScript challenge page (TSPD). The JSON API under
  `/api/funds/` does **not**: it answers normal HTTPS requests directly. Mizan
  sends an honest `User-Agent` (`Mizan/<version>`) and never imitates a browser.
- **Legacy endpoint:** `POST /api/DB/BindHistoryInfo` returns HTTP 404,
  "Method not found or disabled!".

### Fund prices

`POST https://www.tefas.gov.tr/api/funds/fonGnlBlgSiraliGetir`, with header
`Content-Type: application/json` and this JSON body:

```json
{
  "fonTipi": "YAT",
  "fonKodu": "AFT",
  "aramaMetni": null, "fonTurKod": null, "fonGrubu": null, "sfonTurKod": null,
  "fonTurAciklama": null, "kurucuKod": null,
  "basTarih": "20261025", "bitTarih": "20261031",
  "basSira": 1, "bitSira": 100000,
  "dil": "TR", "sFonTurKod": "", "fonKod": "", "fonGrup": "", "fonUnvanTip": ""
}
```

| Field | Meaning |
|---|---|
| `fonTipi` | Fund type: `YAT` investment funds, `EMK` pension funds (also `BYF`, `GYF`, `GSYF`) |
| `fonKodu` | Fund code, or `null` for **all funds of that type** |
| `basTarih`, `bitTarih` | Start and end date, `yyyyMMdd` (other formats are rejected) |
| `basSira`, `bitSira` | Row range for paging |

Response:

```json
{"errorCode": null, "errorMessage": null, "resultList": [
  {"fonKodu": "AFT", "fonUnvan": "…", "tarih": "2026-10-30", "fiyat": 0.959756,
   "tedPaySayisi": 16666283447, "kisiSayisi": 127406, "portfoyBuyukluk": 15995571364.17,
   "borsaBultenFiyat": null, "rn": 1}
]}
```

- `tarih` is the price date (`yyyy-MM-dd`) and `fiyat` the unit price in TRY.
  **`fiyat` is a JSON number.** Mizan reads its raw text into a decimal and never
  goes through binary floating point.
- **Rows:** one per fund per business day, newest first. A bulk request
  (`fonKodu: null`) over two days returned 3,958 rows (about 920 KB).
- **No data in range:** `errorMessage` is `"Index 0 out of bounds for length 0"`
  with `resultList: null` (weekends, holidays, ranges with no prices).
- **Bad input:** `errorMessage` holds a parse error, for example for a wrong date format.
- **Limits:** at most one month per request and no start date more than five
  years before today, both verified on 2026-11-02. Longer ranges are refused
  with `"Geçersiz veri: Tarih aralığı 1 ayı aşamaz"`, older start dates with
  `"Geçersiz veri: Baslangıc Tarihi 5 yıldan eski olamaz"`. About 6 requests
  per minute (reported by community clients and consistent with what was
  observed); exceeding the rate made the server close the connection with no
  reply.
- **Single fund:** with `fonKodu` set to one code, the response holds only that
  fund's rows (21 rows for January 2022 for `AFT`, verified 2026-11-02). History
  requests use this; the latest-price refresh keeps using the bulk form.

### Strategy

Each refresh sends one bulk request per fund type (`YAT`, `EMK`) for the last
7 days, then matches the mapped fund codes locally. That is 2 requests per
refresh regardless of how many funds are held. Fund codes found in neither
type are reported per asset as "fund not found".

## TCMB

`GET https://www.tcmb.gov.tr/kurlar/today.xml` returns a `Tarih_Date` element
(`Tarih="30.10.2026"`) with one `Currency` element per currency (`CurrencyCode`,
`Unit`, `ForexBuying`, …). Mizan stores `ForexBuying / Unit` as TRY per unit.
No file is published on weekends or holidays (HTTP 404), and stored rates are
then kept. A recorded file is in `crates/mizan-market/tests/fixtures/tcmb/`.

## Tests

CI uses recorded fixtures only. Live checks are ignored by default:

```sh
cargo test -p mizan-market --test live -- --ignored
```
