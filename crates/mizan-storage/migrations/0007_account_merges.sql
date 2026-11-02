-- Account merges: what moved from one account into another, so a merge can be
-- undone exactly. Accounts with an active merge are hidden from list_accounts.

CREATE TABLE account_merges (
    id           INTEGER PRIMARY KEY,
    from_account INTEGER NOT NULL REFERENCES accounts(id),
    into_account INTEGER NOT NULL REFERENCES accounts(id),
    merged_at    TEXT NOT NULL,
    undone_at    TEXT,
    -- {"activities": [ids], "assertions": [ids], "hints": [text]}
    moved_json   TEXT NOT NULL,
    CHECK (from_account <> into_account)
);

CREATE UNIQUE INDEX account_merges_active ON account_merges(from_account) WHERE undone_at IS NULL;
