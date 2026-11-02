-- Listed stock and ETF providers: widen the price-source check.
-- SQLite cannot alter a CHECK, so the table is rebuilt.

CREATE TABLE price_sources_new (
    asset_id INTEGER PRIMARY KEY REFERENCES assets(id),
    provider TEXT NOT NULL CHECK (provider IN ('tefas', 'yahoo', 'twelvedata', 'manual')),
    symbol   TEXT
);

INSERT INTO price_sources_new (asset_id, provider, symbol)
    SELECT asset_id, provider, symbol FROM price_sources;

DROP TABLE price_sources;
ALTER TABLE price_sources_new RENAME TO price_sources;
