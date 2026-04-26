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

## The seam rule (canonical)

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> Subsystems (`forge`, `stiglab`, `synodic`, `ising`) coordinate
> **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another
> subsystem. No subsystem imports another subsystem's crate.

What this means for the spine specifically:

- **Spine = mechanism, not a subsystem.** It does not have a sibling
  HTTP surface and is not addressable on a port. Subsystems link the
  library; they coordinate by writing/reading `events` / `events_ext`
  rows and listening on `pg_notify`.
- **New event types are the cross-subsystem contract.** When a new
  `FactoryEventKind` variant is added here, the producer and at least
  one consumer must land in the same PR (or two PRs gated by a contract
  test). A producer with no consumer is the drift pattern from PR #127;
  Lever E will make this CI-enforceable via a registry manifest.
- **Don't grow a sync RPC API on the spine.** If a question feels like
  "stiglab needs to ask synodic *now*", the answer is an event +
  listener pair, not a request/response surface. ADR 0001 documents
  why; spec #131 is closing the last place this is still violated.
- **Spine tables are the single source of truth.** Subsystem-private
  tables that mirror a spine concept (e.g. `tenant_workflows` ↔
  `workflows`, PR #129) are the Lever D drift pattern: collapse into
  the spine table with a discriminator column rather than building a
  mirror.

See [ADR 0001](../../docs/adr/0001-event-bus-coordination-model.md) for
the original decision and spec #131 for the six-lever enforcement plan.

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
