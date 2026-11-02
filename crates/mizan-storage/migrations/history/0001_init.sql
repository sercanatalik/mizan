-- Market-history cache, shared by every Mizan database on the machine.
-- Public market data only: fund codes, currencies, dates, prices and rates.
-- Decimal values are TEXT.

-- Unit prices of TEFAS funds, in TRY.
CREATE TABLE fund_prices (
    code       TEXT NOT NULL,
    date       TEXT NOT NULL,
    price      TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (code, date)
);

-- TRY per one unit of `currency`.
CREATE TABLE fx_rates (
    currency     TEXT NOT NULL,
    date         TEXT NOT NULL,
    try_per_unit TEXT NOT NULL,
    source       TEXT NOT NULL,
    fetched_at   TEXT NOT NULL,
    PRIMARY KEY (currency, date)
);

-- One row per fetched unit: (tefas, fund code, YYYY-MM) or (tcmb, *, YYYY-MM-DD).
-- status: ok (data stored), none (no data published), too_old (provider refuses the range).
CREATE TABLE fetched (
    source     TEXT NOT NULL,
    key        TEXT NOT NULL,
    period     TEXT NOT NULL,
    status     TEXT NOT NULL CHECK (status IN ('ok', 'none', 'too_old')),
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (source, key, period)
);

-- TEFAS fund type (YAT, EMK, ...) per fund code, learned from refreshes.
CREATE TABLE fund_types (
    code      TEXT PRIMARY KEY,
    fund_type TEXT NOT NULL
);
