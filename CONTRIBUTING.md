# Contributing to Mizan

Thanks for helping. This document covers the rules every contribution must
follow.

## Clean room

Mizan is written from scratch under the MIT license. Write adapters and code
from public format documentation and your own files; do not copy code from
other projects.

## Adapters and fixtures

[docs/adapters.md](docs/adapters.md) explains how to write an adapter: start
with a mapping file, use a TypeScript script when a mapping can't express the
format, and native Rust only for widely used standards.

Every adapter ships with fixtures: a synthetic or anonymized sample statement
and the exact `mizan.import/1` batch it must produce. CI runs every adapter
against its fixtures and type-checks the scripts.

```text
adapters/<adapter-id>/
    adapter.toml
    script.ts            (script adapters)
    fixtures/<case>/
        input.<ext>      synthetic or anonymized source file
        expected.json    the exact batch the adapter must produce
```

```sh
cargo test -p mizan-adapters --test fixtures_gate                 # check
MIZAN_BLESS=1 cargo test -p mizan-adapters --test fixtures_gate   # regenerate, then review git diff
```

Before submitting a fixture, remove personal data. Prefer a synthetic file
with the same structure over an edited real one:

- account numbers, names, addresses, IBANs and tax IDs
- order and transaction IDs that could identify you (replace with fake values
  that keep the same format)
- amounts may stay realistic, but nothing should link back to a real person
- never commit a real statement, even temporarily: git history keeps it

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Planned work and its specifications live under `openspec/`.
