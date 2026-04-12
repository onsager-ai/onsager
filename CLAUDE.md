# Onsager

Single-crate client library for the Onsager event spine.

## Architecture

This is a **library crate** (no binaries). The public surface is:

- **`EventStore`** — read/write access to `events` / `events_ext` tables + `pg_notify` subscription.
- **`Listener`** — high-level consumer that filters by `Namespace` and dispatches to an `EventHandler`.
- **`Namespace`** — validated newtype partitioning `events_ext` between components.

The library does **not** manage schema. The SQL contract lives in `migrations/001_initial.sql`; downstream services apply it themselves.

## Polyrepo

Sibling repos under `onsager-ai/` are the consumers:

- `stiglab` — AI agent orchestration
- `synodic` — policy enforcement
- `ising` — evaluation framework
- `telegramable` — Telegram integration

Each lives in its own repo with its own specs, CI, and codebase.

## Build & Test

```bash
cargo build              # Build
cargo test               # Run all tests
cargo clippy -- -D warnings  # Lint
cargo fmt --check        # Format check
```

## Conventions

- Rust edition 2021, rustfmt formatting, clippy with warnings-as-errors
- thiserror for library errors, anyhow for application errors
- Small focused commits, imperative mood, under 72 characters
- Unit tests co-located in `#[cfg(test)]` modules
