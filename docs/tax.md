# Tax reports

The **Tax** screen turns your stored activities into working papers for UK, US
and Turkish tax returns: realized gains and losses, then dividends and interest
with any tax withheld, in the country's currency. Pick a country and a tax year;
any country's report can be opened from the same data, whatever your base
currency.

> **A working paper, not tax advice.** Mizan does not work out tax owed,
> allowances, exemption amounts or whether you must file a return. Check every
> figure, and the rules below, before you use them.

Reports are computed from your database and the shared history cache only.
Opening a report never uses the network.

## Accounts

Every account has a tax treatment, set on the Tax screen:

| Treatment | Meaning |
|---|---|
| Taxable | The default. Sales and income appear in reports. |
| ISA | A UK individual savings account: tax-free, left out of every report. |
| Pension | A pension (for example a Turkish BES or a US IRA): left out of every report. |

A US wash sale still counts a repurchase made in an ISA or pension account (see
below).

## Lots

Mizan replays your activities into acquisition lots. Each lot keeps its date,
quantity, cost and currency:

- **Buys** cost what they settled for, including commission and other charges.
  **Sales** bring in their proceeds net of charges.
- **Splits** change quantities, not cost or dates.
- **Spinoffs and exchanges** move cost by the statement's cost-basis share, and
  the new lots keep the original acquisition dates.
- **Returns of capital** reduce cost in proportion to quantity.
- **Transfers between accounts** joined by a transfer-pair link carry their lots
  across with the original dates and costs; a transfer is never a sale. An
  unlinked transfer in uses its own cost basis and acquisition date. Without a
  date the activity date is used and the row says "acquisition date assumed";
  without a cost basis the cost is unknown.
- Merged assets count as one instrument.

## Exchange rates

Each report is in one currency: GBP for the UK, USD for the US, TRY for Turkey.
Amounts are converted on each purchase's and sale's own date, using the TCMB
forex buying rates Mizan already stores, crossed through TRY. The latest rate on
or before the date is used, up to 7 days back; older than that the row reads
"no exchange rate".

- **UK and US** use a statement's own rate instead when it converts directly
  into the report currency. HMRC and the IRS accept a reasonable rate applied
  consistently; if you need official rates, use the CSV export and substitute
  them.
- **Turkey** always uses the TCMB buying rate on the purchase date for the cost
  and on the sale date for the proceeds, as the Turkish rules require, and
  ignores statement rates.

Opening a UK or US report makes the background price history download fetch
TCMB rates from your first activity, so a TRY-only history also gets GBP and
USD rates.

## United Kingdom

Tax years run from 6 April to 5 April and are labelled like `2025-26`.

**Share identification** (TCGA 1992 s.105, s.106A and s.104; HMRC helpsheet
HS284 and Capital Gains Manual CG51500 onwards). Disposals are matched across
all your taxable accounts:

1. **Same day:** acquisitions of the same instrument on the same day.
2. **30 days:** acquisitions in the following 30 days, earliest first ("bed and
   breakfasting").
3. **Pool:** the Section 104 holding of everything else, at average cost.

All acquisitions on one day count as one acquisition, and all disposals on one
day as one disposal. Each row shows the rule that matched it.

**Report:** each disposal with date, instrument, quantity, proceeds, allowable
cost, gain or loss and rule; a summary with the number of disposals, total
proceeds, allowable costs, gains, losses and net gain; dividends and interest
with tax withheld.

**Not covered:** the annual exempt amount and dividend and personal savings
allowances, reporting-fund excess income, accumulation-unit notional
distributions, and the different rules for non-residents.

## United States

Tax years are calendar years.

- **First in, first out** per account (IRS Publication 550). Each row is one
  matched lot.
- **Holding period:** long-term when the sale is more than one year after the
  acquisition, otherwise short-term. Shares bought on 15 March 2024 are
  long-term when sold on 16 March 2025 or later.
- **Wash sales** (Publication 550, "Wash Sales"). A loss is disallowed, in
  proportion to quantity, when the same instrument is bought in any account
  within 30 days before or after the sale and those shares are still held after
  it. Replacement shares are matched in the order they were bought. The row
  shows the disallowed amount with code W. In a taxable account the disallowed
  loss is added to the replacement shares' cost and the sold shares' holding
  period carries over; in an ISA or pension account (for example an IRA) the
  loss is simply lost. "Same instrument" means the same asset in Mizan; options
  and other substantially identical securities are not detected.

**Report:** rows in the layout of Form 8949, columns (a) to (h), grouped into
short-term and long-term with totals; dividends and interest with tax withheld.
Mizan does not choose the Form 8949 box (A to F): that depends on what your
broker reported to the IRS.

**Not covered:** choosing specific lots instead of first in, first out, whether
dividends are qualified, the foreign tax credit calculation, and Form 8938 or
FBAR reporting.

## Turkey

Tax years are calendar years.

- **First in, first out** across all your taxable accounts.
- **Cost and proceeds in TRY:** the cost at the TCMB buying rate on the purchase
  date, the proceeds at the rate on the sale date. The difference therefore
  includes currency gains.
- **Indexation** (GVK Mük. 81): the cost is multiplied by the Yİ-ÜFE value for
  the month before the sale divided by the value for the month before the
  purchase, but only when that ratio is 1.10 or more. Yİ-ÜFE comes from the TCMB
  EVDS service and needs a free API key (see [market-data.md](market-data.md));
  without it, rows say "not indexed: Yİ-ÜFE not available".
- **Where a security is from:**
  - a TR-prefixed ISIN or a TEFAS price source means **taxed at source**
    (Turkish funds and Borsa Istanbul shares are taxed by withholding);
  - any other ISIN means **foreign**, declared in the annual return;
  - no ISIN and no TEFAS price source means **origin unknown**.

  Only foreign securities count towards the totals; the other two groups are
  listed separately so you can see them.

**Report:** foreign securities with dates, quantity, TRY cost, indexed cost
when applied, TRY proceeds and gain or loss; the taxed-at-source and
origin-unknown groups; foreign dividends and interest with tax withheld abroad.

**Not covered:** the annual exemption amounts and declaration thresholds, which
change every year; the two-year rule for shares of Turkish companies held
outside Borsa Istanbul; and crediting foreign tax against Turkish tax.

## Rows that cannot be computed

A row that cannot be computed is shown with its reason and left out of every
total:

| Reason | Cause |
|---|---|
| cost unknown | The units sold came from a transfer or adjustment without a cost basis, or more was sold than Mizan knows was held. |
| no exchange rate | No TCMB rate within 7 days before the date. A refresh downloads missing history. |
| not covered: short sale, option or crypto | This kind of disposal is not handled yet. |

The report says how many rows were left out and why, and marks its totals
incomplete.

## CSV export

**Export CSV** writes two files into a folder you choose, for example
`mizan-uk-2025-26-gains.csv` and `mizan-uk-2025-26-income.csv`. They have the
columns the screen shows, a header row, ISO dates and plain decimals (a dot,
no thousands separators), comma-separated as in RFC 4180. Rows that could not be
computed are included with their reason. Nothing is sent anywhere.

## Sources

- HMRC, *HS284 Shares and Capital Gains Tax*, and the Capital Gains Manual,
  CG51500 onwards (share identification).
- IRS, *Publication 550, Investment Income and Expenses*, "Wash Sales" and
  "Holding Period"; *Instructions for Form 8949*.
- Gelir Vergisi Kanunu, mükerrer madde 80 (değer artışı kazançları) and 81
  (indexation by Yİ-ÜFE).

The tests reproduce HS284's worked examples and Publication 550's wash-sale
examples (`crates/mizan-core/src/tax/uk.rs` and `us.rs`).
