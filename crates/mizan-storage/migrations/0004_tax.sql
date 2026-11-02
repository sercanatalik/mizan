-- Tax treatment per account: taxable (default), isa or pension.
-- ISA and pension accounts are left out of tax reports.

ALTER TABLE accounts ADD COLUMN tax_treatment TEXT NOT NULL DEFAULT 'taxable'
    CHECK (tax_treatment IN ('taxable', 'isa', 'pension'));
