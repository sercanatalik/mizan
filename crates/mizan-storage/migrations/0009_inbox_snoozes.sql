-- Inbox items the user snoozed. An item is hidden while `until` is in the
-- future and its figures still hash to `fingerprint`; otherwise it returns.
CREATE TABLE inbox_snoozes (
    key         TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    until       TEXT NOT NULL
);
