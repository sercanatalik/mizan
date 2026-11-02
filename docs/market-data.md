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
| **Yahoo Finance** | Daily closes of US and European stocks and ETFs, no key needed | Refresh: one request per assigned symbol for the last month. History: one request per symbol for its missing years |
| **Twelve Data** | Daily closes of stocks and ETFs with your free API key (the free plan covers US listings) | As Yahoo, with the key in a request header |
| **Eurostat** | Monthly consumer price indices for TRY (HICP Türkiye) and EUR (euro-area HICP) | Only when the base currency is TRY or EUR: one request for that series, at most once a day |
| **BLS** | Monthly US consumer price index (USD) from the US Bureau of Labor Statistics | Only when the base currency is USD: one request per 10 years of missing history, at most once a day |
| **ONS** | Monthly UK consumer price index (GBP) from the UK Office for National Statistics | Only when the base currency is GBP: one request, at most once a day |
| **EVDS** | Monthly Turkish domestic producer price index (Yİ-ÜFE) from the TCMB, with your free API key | Only once a key is saved: one request for the missing months, at most once a day, with the key in a request header |

All eight are enabled by default; the inflation providers are described under
[Inflation indices](#inflation-indices), and EVDS under
[Producer price index for Turkish tax reports](#producer-price-index-for-turkish-tax-reports).
 Yahoo Finance and Twelve Data are only asked
for the symbols you assign to them on the Assets screen, so they make no
requests until you do; Twelve Data also waits until you save a key. In
**Market data › Providers**, each has an On/Off switch. A disabled provider makes no network requests, and the prices or
rates it stored earlier are still used for valuation. Requests identify
themselves as `Mizan/<version>`; Mizan never imitates a browser.

## Price history cache

Daily fund prices and exchange rates from the past are kept in one cache file
per computer, shared by every Mizan database on it:

| | |
|---|---|
| Location | `~/.mizan/market-history.sqlite`, or `$MIZAN_HOME/market-history.sqlite` when `MIZAN_HOME` is set |
| Contents | TEFAS fund prices by fund code, stock and ETF closes by provider and symbol, TCMB exchange rates by currency, monthly inflation index values by series, the fund type of each code, and a record of every month, year or day already fetched and when each inflation series was last checked |
| Never contains | Accounts, statement identifiers, quantities, activities, manual prices, API keys, or anything else from your database |

Every refresh also writes the prices and rates it fetched into the cache. Each
fund-month and each TCMB day is downloaded once; months and days that returned
nothing (weekends, holidays, dates before a fund existed) are recorded too, so
they are not asked for again. Stock and ETF history is recorded per symbol and
calendar year. Only the current month (TEFAS), the current year (stocks and
ETFs) and the last 7 days are checked again for newly published data.

### History backfill

After every successful refresh, Mizan fills in missing history in the
background:

- **Fund prices** for each asset priced from TEFAS, from its first activity,
  one fund and one month per request, at least 10 seconds apart. The benchmark
  fund (Performance) and every asset marked as a benchmark on Assets are
  downloaded from the first activity of the account that holds them, or from
  the portfolio's first activity when you don't hold them.
  History is planned from your activities, so nothing is downloaded until
  something is imported; marking a benchmark starts its download at once.
- **Stock and ETF closes** for each asset priced from Yahoo Finance or Twelve
  Data, from its first activity (benchmarks from the first activity of the
  account holding them, or the portfolio's first activity when not held). One request per symbol covers every missing year through today:
  Yahoo requests at least 2 seconds apart, Twelve Data at least 8 (its free
  plan allows 8 a minute; it serves at most 5,000 days per request, so a span
  over 19 years takes more than one). There is no five-year limit. An unknown
  symbol is noted on the Providers tab and skipped.
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
  the other provider continues. The error shows on the Providers tab.
- **Five-year limit:** TEFAS serves no prices older than five years. Returns
  value older days at trade prices from your own activities, marked as
  estimates.

**Market data › Providers › Price history** shows each provider's state (downloading with
"34 of 120 months" or "6 of 14 years", complete, waiting for the next refresh,
waiting for an API key, failed with the reason, off, or not needed) and the
cache location. A disabled provider
downloads no history, and with "refresh on start" off and no manual refresh,
nothing is downloaded at all. The first backfill for a long history takes a
while (about 80 minutes for 8 funds over five years); it happens once per
computer.

The cache holds public data only, so deleting `~/.mizan` is safe: the history
is downloaded again when needed. Your database stays in the app data folder
(see [desktop.md](desktop.md)).

## Refreshing

- **Refresh prices** (on Overview, Holdings, Assets and Market data › Providers) runs one refresh.
- **On start:** with "Refresh prices when the app starts, if the last refresh is
  older than a day" checked (the default), a refresh runs in the background
  after the window opens when the last successful refresh is more than 24 hours
  old. Uncheck it to keep the app fully offline.
- **Failures:** a failing provider never stops the other one, and never deletes
  or changes stored data. The Providers tab shows the last refresh report per provider:
  OK, Failed (with the reason), Disabled, Nothing to fetch, or Needs an API key.
  An unknown stock symbol is reported for that asset only ("NOSUCHXYZ: symbol
  not found"). A rejected key or a provider's request limit stops that
  provider for the rest of the refresh.
- **Concurrency:** only one refresh runs at a time.

## Price sources

Each asset has one price source, chosen on the **Assets** screen with **Edit**
from the price-source menu, then **Save source**:

- **TEFAS** with a fund code (for example `AFT`).
- **Yahoo Finance** with a Yahoo symbol: `AAPL` for US listings, the symbol
  plus an exchange suffix elsewhere (`VWCE.DE` for XETRA, `VUSA.L` for London,
  `IWDA.AS` for Amsterdam).
- **Twelve Data** with a Twelve Data symbol (for example `MSFT`).
- **Manual:** only the prices you enter.
- **None** (the default): the asset shows "no price".

The provider sources need a code or symbol. If a provider is off on the Providers tab,
or Twelve Data has no key, the asset's source says so.

Suggestions appear with an **Accept** button, and nothing changes until you
accept:

- **TEFAS** when an asset looks like a TEFAS fund (class `fund` or a TRY
  currency, with a three-character upper-case code of letters and digits, such as
  AFT or AH1): "TEFAS: AFT".
- **Yahoo** when an imported symbol has a known exchange code (MIC), or no
  exchange code and a USD currency: `VWCE` on `XETR` gives "Yahoo: VWCE.DE",
  `BRK.B` on `XNYS` gives "Yahoo: BRK-B". Suffixes are known for the US
  exchanges and XETRA, Frankfurt, Amsterdam, Paris, Brussels, Lisbon, Milan,
  Madrid, London, SIX Swiss, Vienna, Stockholm, Copenhagen, Helsinki, Oslo and
  Dublin. Suggestions are worked out on your computer; no request is made.

Changing a source keeps the prices already stored.

## Stock and ETF prices

**Prices as traded.** Mizan stores each day's close as it was traded, not
adjusted for later splits or dividends. Your imported quantities, splits and
dividends already describe what happened, so adjusted prices would count a
split twice.

- **Yahoo** returns closes adjusted for later splits. Each request runs through
  today and includes the split events, so Mizan multiplies each close by the
  splits after it. Example: Yahoo shows AAPL at 124.8075 on 2020-08-28, before
  the 4:1 split of 2020-08-31; Mizan stores 499.23.
- **Twelve Data** is asked for unadjusted prices (`adjust=none`).

**Decimals.** Closes are read from the response text, never through binary
floating point. Yahoo closes are rounded to the decimals Yahoo declares for the
symbol (`priceHint`), and Twelve Data closes to 4 decimals.

**Minor units.** Prices quoted in pence or cents are converted: GBp and GBX to
GBP, ILA to ILS, ZAc to ZAR, divided by 100.

**Dates.** Each close is dated on the exchange's own trading day. During
trading hours Yahoo's newest bar is the live price; the next refresh within 7
days replaces it with the close.

**The Twelve Data key.** Save it in **Market data › Providers**. It is
stored in your database only and sent in the `Authorization` header, never in a
URL. The Providers tab shows only its last four characters, and it never appears in
refresh reports or the history cache. **Clear key** removes it.

**Yahoo is unofficial.** The chart endpoint is not a documented API and may
change or stop working. A Yahoo failure never affects the other providers; you
can switch it off and use Twelve Data or manual prices instead.

**Privacy.** Asking a provider for a symbol tells it that someone at your
network address is interested in it, as asking TEFAS for a fund code does.
Mizan only asks for symbols you assigned.

## Inflation indices

Real (inflation-adjusted) returns divide by the consumer price index of the
**base currency**. Mizan knows one index per supported base currency:

| Base currency | Index | Provider | Series |
|---|---|---|---|
| TRY | HICP Türkiye (harmonised index of consumer prices) | Eurostat | `prc_hicp_minr`, geo `TR`, 2015=100 |
| EUR | Euro-area HICP, changing composition | Eurostat | `prc_hicp_minr`, geo `EA`, 2015=100 |
| USD | CPI for all urban consumers (CPI-U), not seasonally adjusted | BLS | `CUUR0000SA0` |
| GBP | CPI index | ONS | `D7BT`, 2015=100 |

Other base currencies have no index, and real returns say so.

- **Only the base currency's series is fetched**, during a refresh, from the
  month before your first activity onward. Months already cached are not
  requested again, except the last three, which are re-read for revisions.
- **At most once a day:** a series is not requested again within 24 hours,
  and not at all while it is complete up to last month.
- **Changing the base currency** fetches nothing by itself; the next refresh
  fetches the new series. Until then real returns say the index has not been
  downloaded yet.
- **No key, honest requests:** all three providers are public and keyless.
  A failed request keeps the cached values and is reported in the last
  refresh; the other providers carry on.

**HICP Türkiye is not TÜFE.** The figure usually quoted for Turkish inflation
is TÜİK's TÜFE, which is only published through keyed or less machine-readable
channels. HICP Türkiye is the harmonised index TÜİK compiles for Eurostat from
the same price collection; it follows TÜFE closely but is not identical.
The Providers tab and Performance always label it "HICP Türkiye".

How monthly values become a daily index is described in
[returns.md](returns.md#real-returns).

## Producer price index for Turkish tax reports

Turkish tax reports raise the cost of a foreign security by the domestic
producer price index (Yİ-ÜFE) when it rose 10% or more between the month
before the purchase and the month before the sale (see [tax.md](tax.md)).
The index comes from the TCMB's EVDS service, which needs a free API key
(register at `evds3.tcmb.gov.tr`). Enter it in **Market data › Providers ›
EVDS**.

- **No key, no request.** The refresh lists EVDS as "needs an API key", and
  Turkish reports show unindexed costs with a note on each row.
- **The key stays in the user database.** It is sent only in the `key`
  request header, never in a URL, and is never written to the history cache,
  logs or refresh reports. The Providers tab shows it masked.
- **Caching and timing** follow the inflation indices: from the month before
  your first activity, cached by month in the history cache, re-reading the
  last three months, at most once a day.


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
- **Base currency:** chosen in **Settings › Valuation** or the view bar (default TRY). Changing it
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

## Yahoo Finance and Twelve Data API notes (verified 2026-11-02)

**Yahoo:**
- `GET https://query1.finance.yahoo.com/v8/finance/chart/{symbol}`:
  - latest: `range=1mo&interval=1d&events=split`
  - history: `period1=<from>&period2=<tomorrow>&interval=1d&events=split`, in
    Unix seconds at midnight UTC
- The answer is `chart.result[0]`, with:
  - `meta.currency` (`USD`, `EUR`, `GBp`, …), `meta.priceHint` and
    `meta.gmtoffset`
  - `timestamp[]`, the bar opening times
  - `indicators.quote[0].close[]`, split-adjusted, with `null` for missing bars
  - `events.splits{}`, each with `date`, `numerator` and `denominator`
- An unknown symbol answers HTTP 404 with
  `chart.error.description = "No data found, symbol may be delisted"`.

**Twelve Data:**
- `GET https://api.twelvedata.com/time_series?symbol=…&interval=1day&adjust=none&order=asc`,
  with either `outputsize=30` or `start_date`/`end_date` and
  `outputsize=5000`.
- The key goes in `Authorization: apikey <key>`.
- The answer has `meta.currency` and `values[]`, each with `datetime` and
  `close` as text.
- Errors are `{"code", "message", "status": "error"}`:
  - 401: invalid key
  - 429: request limit
  - 400/404: unknown symbol

Recorded responses are in `crates/mizan-market/tests/fixtures/yahoo/` and
`crates/mizan-market/tests/fixtures/twelvedata/`. Stooq, a common keyless
source, now answers with a browser check and is not used.

## Inflation API notes (verified 2026-11-02)

**Eurostat:**
- `GET https://ec.europa.eu/eurostat/api/dissemination/statistics/1.0/data/prc_hicp_minr?format=JSON&geo={EA|TR}&unit=I15&coicop18=TOTAL&sinceTimePeriod=YYYY-MM`.
- The answer is JSON-stat 2.0: `id[]` and `size[]` list the dimensions,
  `dimension.time.category.index` maps `YYYY-MM` to a position, and `value{}`
  holds numbers keyed by the flat position (the last dimension varies
  fastest). Months without data are absent.
- Errors are `{"error":[{"status", "label"}]}`. An unknown geo returns no values.
- The older dataset `prc_hicp_midx` stopped at December 2025, when Eurostat
  moved to the 2018 classification of consumption (`coicop18`).

**BLS (public API v1):**
- `POST https://api.bls.gov/publicAPI/v1/timeseries/data/` with
  `{"seriesid":["CUUR0000SA0"],"startyear":"YYYY","endyear":"YYYY"}`; at most 10
  years per request and 25 requests a day without a key.
- `Results.series[0].data[]` holds `{year, period, value}`, newest first.
  `M01`–`M12` are months, `M13` the annual average (skipped). A value of `-`
  means nothing was published (October 2025, when US CPI was not collected);
  that month is skipped and the daily index interpolates across it.
- Failures come back as `status: "REQUEST_NOT_PROCESSED"` with a `message[]`.

**ONS:**
- `GET https://www.ons.gov.uk/economy/inflationandpriceindices/timeseries/d7bt/mm23/data`
  returns the whole series; `months[]` holds `{date: "2026 SEP", value: "144.1"}`.
- This is the website's own data endpoint rather than a versioned API; if its
  format changes, the refresh reports ONS as failed and cached values stay.

Recorded responses are in `crates/mizan-market/tests/fixtures/eurostat/`,
`bls/` and `ons/`.

## EVDS API notes (checked 2026-11-02)

- `GET https://evds3.tcmb.gov.tr/igmevdsms-dis/series=TP.TUFE1YI.T1&startDate=DD-MM-YYYY&endDate=DD-MM-YYYY&type=json`.
  Parameters are written after the slash, without a `?`.
- The key goes in the `key` request header. Since 5 April 2024 a key in the
  URL is not accepted; without the header the service answers
  `403 {"status":"403","message":"Required request header 'key' is not present"}`
  (checked live). The older `evds2.tcmb.gov.tr/service/evds` endpoint now
  redirects.
- `TP.TUFE1YI.T1` is the Yİ-ÜFE series (2003=100), monthly.
- Responses list `items[]` with `Tarih` as `YYYY-M` and the value under the
  series code with dots replaced by underscores (`TP_TUFE1YI_T1`); months not
  yet published are `null`.
- The series code and the response shape come from the EVDS usage guide and
  public client libraries, and were confirmed in the app with a key on
  2026-11-02 (Turkish tax rows showed indexed costs). The live test, run with
  `MIZAN_EVDS_KEY` set, checks them again.

## Tests

CI uses recorded fixtures only. Live checks are ignored by default:

```sh
cargo test -p mizan-market --test live -- --ignored
```

The EVDS check runs only when `MIZAN_EVDS_KEY` holds your key, and is skipped otherwise.
