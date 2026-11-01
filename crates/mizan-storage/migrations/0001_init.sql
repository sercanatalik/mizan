-- Initial schema: accounts, assets, import batches, activities and balance assertions.
-- Decimal values live inside the JSON columns as strings, never as REAL.

CREATE TABLE accounts (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Statement account identifiers (e.g. "U1234567") mapped to accounts.
CREATE TABLE account_hints (
    hint       TEXT PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id)
);

CREATE TABLE assets (
    id             INTEGER PRIMARY KEY,
    -- Strongest available identifier, e.g. "isin:US0378331005" or "sym:AAPL@XNAS".
    instrument_key TEXT NOT NULL UNIQUE,
    name           TEXT,
    currency       TEXT,
    hint_json      TEXT NOT NULL
);

-- One committed adapter run over one source file.
CREATE TABLE import_batches (
    id              INTEGER PRIMARY KEY,
    adapter         TEXT NOT NULL,
    adapter_version TEXT NOT NULL,
    file_sha256     TEXT NOT NULL,
    committed_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE activities (
    id              INTEGER PRIMARY KEY,
    batch_id        INTEGER NOT NULL REFERENCES import_batches(id),
    account_id      INTEGER NOT NULL REFERENCES accounts(id),
    asset_id        INTEGER REFERENCES assets(id),
    kind            TEXT NOT NULL,
    date            TEXT NOT NULL,
    -- Position of the record within its source file; orders same-day records.
    source_order    INTEGER NOT NULL,
    fingerprint     TEXT NOT NULL,
    external_id     TEXT,
    -- Provenance.
    adapter         TEXT NOT NULL,
    adapter_version TEXT NOT NULL,
    file_sha256     TEXT NOT NULL,
    locator         TEXT NOT NULL,
    -- 1 when a position arrived without cost basis. Unknown is never stored as zero.
    basis_unknown   INTEGER NOT NULL DEFAULT 0 CHECK (basis_unknown IN (0, 1)),
    -- The canonical record exactly as imported: figures, charges, legs, links, memo, raw.
    record_json     TEXT NOT NULL
);

CREATE INDEX activities_account_date ON activities(account_id, date, source_order);
CREATE INDEX activities_fingerprint ON activities(fingerprint);

CREATE TABLE assertions (
    id             INTEGER PRIMARY KEY,
    batch_id       INTEGER NOT NULL REFERENCES import_batches(id),
    account_id     INTEGER NOT NULL REFERENCES accounts(id),
    as_of          TEXT NOT NULL,
    file_sha256    TEXT NOT NULL,
    locator        TEXT NOT NULL,
    assertion_json TEXT NOT NULL
);

CREATE INDEX assertions_account_date ON assertions(account_id, as_of);

CREATE TABLE assertion_results (
    assertion_id INTEGER PRIMARY KEY REFERENCES assertions(id),
    passed       INTEGER NOT NULL CHECK (passed IN (0, 1)),
    -- Expected vs computed values per balance line.
    result_json  TEXT NOT NULL
);
