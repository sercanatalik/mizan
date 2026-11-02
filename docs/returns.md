# Returns

Mizan computes how your portfolio has done from what is stored on your
computer: imported activities, prices and exchange rates (including the price
history cache, see [market-data.md](market-data.md)). Nothing is fetched to
compute a return, so the figures are the same offline.

## Daily value

For every account and every calendar day from its first activity to today,
Mizan replays the activities up to that day and values what the account held
at the end of it:

- **Positions** at the newest price dated on or before that day. A manual price
  wins over a provider price on the same date. Weekends and holidays use the
  last earlier price.
- **Cash** balances as they are.
- **Conversion** to the base currency with the newest TCMB rate on or before
  that day, through TRY.

The portfolio value is the sum of the account values.

### Estimated values

When a position has no price on or before a day (for example a fund before
TEFAS history begins five years back, or a stock without a price provider), it
is valued at:

1. the most recent **trade price** from that asset's activities on or before the
   day: the record's price, otherwise its cash amount divided by quantity, then
2. its **provisional average cost**.

Such days are **estimated**. Every return that uses an estimated day shows an
estimate mark with the number of days, for example
"estimate · 40 days at trade price or cost". Exact figures never carry the
mark. A position with none of these is left out and counted.

### Missing exchange rates

A day whose value needs a rate that isn't stored (for example while exchange
rate history is still downloading) has no value. Any such day in a period makes
that account's returns for the period "not available", with the reason, for
example "exchange rates before 2025-01-02 are missing". The portfolio figures
then leave that account out and say so.

## Flows

Returns separate what you put in or took out from what your investments
earned:

| Counts as a flow | Not a flow (part of the return) |
|---|---|
| Deposits and withdrawals | Dividends, interest, distributions |
| Securities transferred in or out, at that day's value | Fees and taxes |
| Transfers between your own accounts (for each account only) | Buying and selling, currency conversions |

A transfer between two of your accounts is a flow for each account, and not a
flow for the portfolio as a whole. If only one side of such a transfer is
imported, the portfolio sees money appear or disappear without a flow, which
shows up as a gain or loss. Import both statements to avoid this.

## Time-weighted return (TWR)

The time-weighted return measures the investments, independent of when money
was added or withdrawn. Each day's flows count as arriving at the start of the
day:

```
daily return  r_d = V_d / (V_{d-1} + F_d) - 1
period TWR        = (1 + r_1) x (1 + r_2) x ... x (1 + r_n) - 1
```

`V_d` is the value at the end of day `d` and `F_d` that day's flows. Days with
nothing invested are skipped. For periods longer than a year, the annualized
rate `(1 + TWR)^(365 / days) - 1` is shown as well.

## Money-weighted return (MWR)

The money-weighted return is the internal rate of return of your own cash
flows: the rate at which the value at the start, every flow, and the value at
the end discount to zero, compounding daily over actual days / 365. It reflects
the timing of your deposits and withdrawals.

- Periods of one year or less show the rate for the period itself, not
  annualized.
- Longer periods show both the cumulative and the annualized rate.
- When the flows have no unique rate (or nothing was invested), it reads
  "not available" with the reason.

The rate is solved numerically (Newton's method with a bisection fallback).
This is the only place Mizan uses floating point; money amounts are always
exact decimals.

## Worked example

<!-- returns-example -->
One account and one fund, base currency TRY:

| Day | What happens | Fund price | Value at end of day |
|---|---|---|---|
| 2 Mar | Deposit 10000.00 TRY, buy 1000 units for 10000.00 | 10.00 | 10000.00 |
| 3 Mar | | 10.10 | 10100.00 |
| 4 Mar | Deposit 10100.00 TRY, buy 1000 units for 10100.00 | 10.201 | 20402.00 |

- TWR: 2 Mar `10000 / (0 + 10000) = 1`, 3 Mar `10100 / 10100 = 1.01`,
  4 Mar `20402 / (10100 + 10100) = 1.01`, so TWR = 1.01 × 1.01 − 1 =
  **+2.01%**.
- MWR: flows of −10000 on day 0 and −10100 on day 2, then +20402 at the end
  of day 3. The annual rate that discounts these to zero, taken over 3 days,
  is **+2.26%**. It is higher than the TWR because more money was invested
  during the stronger days.

## Periods

1M, 3M, YTD, 1Y and since start, ending today. A period that starts before an
account's first activity starts at its first activity instead, and the screen
says "since" that date.

## Benchmark

On **Performance**, choose any asset with stored prices as the benchmark (for
example a TEFAS fund), or none. Its return for a period is its price on or
before today divided by its price on or before the period start, minus one,
both converted to the base currency. Without a price at the start it reads
"not available: no price before" that date. The growth chart shows the
portfolio's time-weighted growth and the benchmark's, both starting at 100.

## Base currency

All values and returns are in the base currency (Settings › Valuation).
Changing it, a price source, a manual price, or imported activities recomputes
everything from stored data, without a network request.
