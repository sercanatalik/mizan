-- Market data: settings, price sources, prices, exchange rates, asset merges, refresh log.
-- Decimal values are TEXT, as in 0001.

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT INTO settings (key, value) VALUES
    ('base_currency', 'TRY'),
    ('refresh_on_start', 'true'),
    ('provider.tefas.enabled', 'true'),
    ('provider.tcmb.enabled', 'true');

-- One merge sends every key of `from_asset` to `into_asset`. Undo sets undone_at.
CREATE TABLE asset_merges (
    id         INTEGER PRIMARY KEY,
    from_asset INTEGER NOT NULL REFERENCES assets(id),
    into_asset INTEGER NOT NULL REFERENCES assets(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    undone_at  TEXT,
    CHECK (from_asset <> into_asset)
);

-- At most one active merge per merged asset.
CREATE UNIQUE INDEX asset_merges_active ON asset_merges(from_asset) WHERE undone_at IS NULL;

-- Absent row = no price source. provider: 'tefas' (symbol = fund code) or 'manual'.
CREATE TABLE price_sources (
    asset_id INTEGER PRIMARY KEY REFERENCES assets(id),
    provider TEXT NOT NULL CHECK (provider IN ('tefas', 'manual')),
    symbol   TEXT
);

CREATE TABLE prices (
    asset_id   INTEGER NOT NULL REFERENCES assets(id),
    date       TEXT NOT NULL,
    source     TEXT NOT NULL,
    value      TEXT NOT NULL,
    currency   TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (asset_id, date, source)
);

-- Rates are TRY per one unit of `currency`.
CREATE TABLE fx_rates (
    currency     TEXT NOT NULL,
    date         TEXT NOT NULL,
    source       TEXT NOT NULL,
    try_per_unit TEXT NOT NULL,
    fetched_at   TEXT NOT NULL,
    PRIMARY KEY (currency, date, source)
);

CREATE TABLE refresh_log (
    id          INTEGER PRIMARY KEY,
    started_at  TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    report_json TEXT NOT NULL
);
