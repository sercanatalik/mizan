-- The price source an account gives its assets that have none yet:
-- 'tefas', 'yahoo' or 'twelvedata'; NULL leaves each asset to the user.
ALTER TABLE accounts ADD COLUMN price_source TEXT
    CHECK (price_source IN ('tefas', 'yahoo', 'twelvedata'));
