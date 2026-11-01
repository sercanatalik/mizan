# Contributing to Mizan

Thanks for helping. This document covers the rules every contribution must
follow.

## Adapters and fixtures

Every adapter ships with fixtures: an anonymized sample statement and the exact
`mizan.import/1` batch it must produce. CI runs every adapter against its
fixtures.

```text
fixtures/<adapter-id>/<case>/
    input.<ext>      anonymized source file
    expected.json    the exact batch the adapter must produce
```

See [docs/import-schema.md](docs/import-schema.md#adapter-fixtures) for the
harness, and `mizan-cli import validate` to check a batch by hand.

Before submitting a fixture, remove personal data:

- account numbers, names, addresses, IBANs and tax IDs
- order and transaction IDs that could identify you (replace with fake values
  that keep the same format)
- amounts may stay realistic, but nothing should link back to a real person

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Planned work and its specifications live under `openspec/`.
