-- Corrections to market data series, per user database. The shared history
-- cache is never written to by edits. Prices are keyed like `tefas:AFT` or
-- `yahoo:VWCE.DE`, rates by currency, indices like `eurostat:TR:I15`.

CREATE TABLE series_edits (
    kind      TEXT NOT NULL CHECK (kind IN ('price', 'rate', 'index')),
    series    TEXT NOT NULL,
    -- YYYY-MM-DD, or YYYY-MM for indices.
    period    TEXT NOT NULL,
    -- NULL when the stored value is ignored.
    value     TEXT,
    currency  TEXT,
    ignored   INTEGER NOT NULL DEFAULT 0 CHECK (ignored IN (0, 1)),
    edited_at TEXT NOT NULL,
    PRIMARY KEY (kind, series, period),
    CHECK ((ignored = 1 AND value IS NULL) OR (ignored = 0 AND value IS NOT NULL))
);
