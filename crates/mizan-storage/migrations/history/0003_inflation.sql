-- Monthly consumer price index values, by provider, series and month (YYYY-MM).
-- Stored as published (TEXT decimals); the base year is part of the series id.
CREATE TABLE inflation_index (
    source     TEXT NOT NULL,
    series     TEXT NOT NULL,
    month      TEXT NOT NULL,
    value      TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (source, series, month)
);

-- The last time each series was requested, and whether that request succeeded.
CREATE TABLE inflation_checks (
    source     TEXT NOT NULL,
    series     TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    status     TEXT NOT NULL CHECK (status IN ('ok', 'failed')),
    PRIMARY KEY (source, series)
);
