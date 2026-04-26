# Onsager spine

Event-bus client library for the Onsager factory stack. Scope is strictly
event-stream coordination — `store`, `listener`, `namespace`,
`factory_event`, `extension_event`. Domain types (artifact, bundle,
registry, protocol DTOs) live in their own crates after the #33 split.

## Architecture

This is a **library crate** (no binaries). The public surface is:

- **`EventStore`** — read/write access to `events` / `events_ext` tables + `pg_notify` subscription.
- **`Listener`** — high-level consumer that filters by `Namespace` and dispatches to an `EventHandler`.
- **`Namespace`** — validated newtype partitioning `events_ext` between components.
- **`FactoryEvent` / `FactoryEventKind`** — the typed event vocabulary used by every subsystem. Payloads that reference artifact types pull them from `onsager-artifact`. The wire-level catalog at `docs/events.md` is auto-generated from this enum — run `just gen-event-docs` after adding/editing variants. CI runs `--check` and fails if the catalog is stale.

The crate depends on `onsager-artifact` (for `ArtifactId`, `BundleId`, etc. in event payloads) and nothing else in the workspace.

The library does **not** manage schema. The SQL contract lives in `migrations/001_initial.sql`; downstream services apply it themselves.

## Monorepo

All subsystems live under `crates/` in the workspace root:

- `forge` — production line (artifact lifecycle, scheduling kernel)
- `stiglab` — AI agent session orchestration
- `synodic` — AI agent governance
- `ising` — continuous improvement engine (factory observation)

Each depends on `onsager-spine` via `path = "../onsager-spine"`.
Subsystems must NOT import each other.

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
