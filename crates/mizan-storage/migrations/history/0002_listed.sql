-- Daily closes of listed stocks and ETFs, by provider and provider symbol.
-- Stored unadjusted (as traded that day), in the major currency unit.
CREATE TABLE listed_prices (
    source     TEXT NOT NULL,
    symbol     TEXT NOT NULL,
    date       TEXT NOT NULL,
    close      TEXT NOT NULL,
    currency   TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (source, symbol, date)
);
