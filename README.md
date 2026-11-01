# Mizan

Mizan is a local-first, open-source portfolio tracker. Your data stays on your
device, in a SQLite file you own.

What sets Mizan apart is **free and open imports**: broker statements (CSV,
XLSX, PDF, OFX, IBKR Flex and more) come in through community-maintained
adapters, not a paid sync service. Every import is reviewed before it is
saved, checked against the balances printed on the statement, and can be
undone.

> Status: early development. The import foundation (`mizan.import/1` schema,
> validation and review pipeline) is being built first.

## How imports work

```
statement file --> adapter --> mizan.import/1 batch --> review --> commit
                   (mapping file,   (records, balance      (duplicates,
                    TS script or     assertions,            warnings,
                    native Rust)     diagnostics)           balance checks)
```

- **Adapters report; the core interprets.** Adapters emit what the statement
  says, with account and instrument *hints*. Mizan resolves, deduplicates and
  verifies.
- **Balance assertions.** Closing balances on a statement are checked against
  what Mizan computes, so a missing transaction is caught at import time.
- **Safe re-imports.** Overlapping statements are deduplicated by fingerprint.

The schema is documented in [docs/import-schema.md](docs/import-schema.md) and
published as JSON Schema in [`schema/`](schema/).

## Building

Requires a recent stable Rust toolchain.

```sh
cargo build --workspace
cargo test --workspace
```
