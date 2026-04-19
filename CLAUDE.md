# Onsager

AI factory stack — monorepo for the Onsager event bus and its subsystems.

## Architecture

Onsager is a **factory event bus** architecture. Subsystems are runtime-decoupled
via a shared PostgreSQL `events` / `events_ext` table + `pg_notify` channel.
They coordinate through stigmergy (indirect signals via shared medium), not
direct calls.

See [ADR 0001](docs/adr/0001-event-bus-coordination-model.md) for the
decision and migration checklist.

```
         onsager-spine (event bus lib)
        /       |        |        \
   forge    stiglab   synodic    ising    <- do NOT depend on each other
```

**Architectural invariant**: subsystems (`forge`, `stiglab`, `synodic`, `ising`)
must NOT import each other, and must NOT be statically linked into the same
binary. The `onsager` dispatcher has zero business dependencies -- it discovers
subsystem binaries on PATH.

## Workspace layout

```
crates/
  onsager-artifact/    <- domain value objects (Artifact, ArtifactId, BundleId, Kind, lineage, quality)
  onsager-spine/       <- event bus client (EventStore, Listener, Namespace, FactoryEvent)
  onsager-warehouse/   <- bundle sealing + Warehouse trait (depends on artifact)
  onsager-delivery/    <- consumer routing (depends on artifact, warehouse)
  onsager-registry/    <- type registry, seed catalog, evaluators (depends on artifact, spine)
  onsager-protocol/    <- sync-RPC DTOs; deleted when ADR 0001 migration completes
  onsager/             <- dispatcher CLI (~100 LOC, no business deps)
  forge/               <- production line — drives artifacts through their lifecycle (lib + bin)
  ising/               <- continuous improvement engine — observes and surfaces insights (lib + bin)
  stiglab/             <- distributed AI agent session orchestration (lib + bin)
  synodic/             <- AI agent governance (lib + bin)
apps/
  dashboard/           <- React UI (sessions, nodes, governance, factory views)
```

Subsystem → support-crate dependencies (as of #33):

- `forge`   → `onsager-{artifact, warehouse, protocol, spine}`
- `stiglab` → `onsager-{artifact, protocol, spine}`
- `synodic` → `onsager-{artifact, protocol, spine}`
- `ising`   → `onsager-{artifact, protocol, spine}` (no warehouse/delivery/registry)

## Getting Started

Prerequisites: Docker, Rust toolchain (via rustup), pnpm.

```bash
cp .env.example .env            # configure environment (reference for docker-compose)
just dev                        # start Postgres, run migrations, and launch services
just smoke-test                 # verify everything works (in another terminal)
```

To run agent sessions, add your `CLAUDE_CODE_OAUTH_TOKEN` via
Dashboard > Settings > Credentials (encrypted at rest, passed to agents as env vars).

Services:
- **Dashboard**: http://localhost:5173 (Vite dev server with HMR)
- **Stiglab API**: http://localhost:3000 (sessions, nodes, WebSocket)
- **Synodic API**: http://localhost:3001 (governance)
- **Postgres**: localhost:5432 (event spine)

To stop: `Ctrl+C` for services, `just dev-down` for Postgres.

## Build & Test

```bash
just build           # Rust workspace + dashboard
just test            # All tests
just test-all        # All tests including spine integration tests
just lint            # fmt + clippy + eslint
```

Or directly:

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Conventions

- Rust edition 2021, rustfmt formatting, clippy with warnings-as-errors
- thiserror for library errors, anyhow for application errors
- Small focused commits, imperative mood, under 72 characters
- Unit tests co-located in `#[cfg(test)]` modules
- All internal deps use `path = "../..."` -- no git deps, no crates.io

## Environment variables

Subsystem-specific env vars worth calling out:

- `SYNODIC_FAIL_POLICY` (forge, default `escalate`) — what verdict the Forge
  side returns when the Synodic gate is unreachable, returns 5xx, or its
  response cannot be parsed. One of `escalate` | `deny` | `allow`.
  `escalate` parks the decision non-blockingly (forge invariant #5);
  `deny` keeps the artifact in its current state; `allow` is the legacy
  fail-open behavior and must be opted into explicitly. 4xx responses and
  parse errors always deny regardless of policy — those are protocol bugs
  that should surface loudly.

## Per-crate context

Each subsystem has its own CLAUDE.md or `.claude/` directory with
subsystem-specific instructions:

- `crates/onsager-spine/CLAUDE.md`
- `crates/stiglab/.claude/`
- `crates/synodic/.claude/`
