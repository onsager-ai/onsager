# Onsager

AI factory stack — monorepo for the Onsager event bus and its subsystems.

## Architecture

Onsager is an **AI factory event bus**. Subsystems are runtime-decoupled
via a shared PostgreSQL `events` / `events_ext` table + `pg_notify` channel.
They coordinate through stigmergy (indirect signals via shared medium), not
direct calls.

```
                       onsager-spine  (event bus library)
                              │
        ┌─────────┬───────────┼───────────┬──────────┬──────────┐
        │         │           │           │          │          │
     portal    forge       stiglab     synodic     ising     refract
     (edge)                                                  (decomposer)
```

The seam rule has two clauses (see [ADR 0004](docs/adr/0004-tighten-the-seams.md)):

1. **External boundary.** HTTP routes exist only at external boundaries —
   the dashboard API and external webhooks (GitHub, etc.). The external
   boundary is owned by `portal` (the edge subsystem).
2. **Internal coordination.** Factory subsystems (`forge`, `stiglab`,
   `synodic`, `ising`, `refract`) coordinate **exclusively** via the spine —
   no sibling-subsystem HTTP, no cross-subsystem Cargo deps. The `onsager`
   dispatcher has zero business deps and discovers subsystem binaries on
   `PATH`.

Both clauses are mechanically enforced by `xtask lint-seams`,
`xtask check-events`, and `xtask check-api-contract`.

For the navigable map of how everything fits together, what's enforced,
and what's still in flight, see [`docs/architecture.md`](docs/architecture.md)
and the ADRs under [`docs/adr/`](docs/adr/).

## Subsystems

| Crate            | Role                                                                             |
|------------------|----------------------------------------------------------------------------------|
| `onsager-spine`  | Shared event bus library (PostgreSQL + `pg_notify`); SoT for shared workflow tables |
| `onsager`        | Unified CLI dispatcher (`onsager <subsystem> ...`)                               |
| `onsager-portal` | Edge subsystem — public HTTP, GitHub webhooks, OAuth, credentials                |
| `forge`          | Production line — drives artifacts through their lifecycle                       |
| `stiglab`        | Distributed AI agent session orchestration                                       |
| `synodic`        | AI agent governance — gates, verdicts, escalations                               |
| `ising`          | Continuous improvement — observes the spine and surfaces insights                |
| `refract`        | Intent decomposer — expands a high-level intent into an artifact tree            |

Library crates (`onsager-{artifact, warehouse, delivery, registry, github}`)
are typed shared building blocks consumed by the subsystems above.

A single React app at `apps/dashboard/` surfaces sessions, nodes, governance,
and factory views.

## Getting Started

Prerequisites: Docker, Rust toolchain (via rustup), pnpm.

```bash
cp .env.example .env       # configure environment
just dev                   # Postgres, migrations, and all services
just smoke-test            # verify everything works (in another terminal)
```

Open the dashboard at http://localhost:5173 and click **Dev Login** —
debug builds (the default `cargo build` / `just dev` profile) seed a
`${USER}@local` user plus a default workspace and expose a one-click
login button on the LoginPage. A persistent banner reminds you you're
in dev mode. Release builds (`cargo build --release`) strip the
seeder + the `/api/auth/dev-login` route entirely; production deploys
must use real GitHub OAuth.

To use your real GitHub identity locally instead, set
`GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET` in `.env` and click
**Sign in with GitHub** on the LoginPage.

To run agent sessions, add your `CLAUDE_CODE_OAUTH_TOKEN` via
**Dashboard → Settings → Credentials** (encrypted at rest, passed to agents
as env vars).

Services:
- **Dashboard** — http://localhost:5173 (Vite dev server with HMR)
- **Stiglab API** — http://localhost:3000 (sessions, nodes, WebSocket)
- **Synodic API** — http://localhost:3001 (governance)
- **Postgres** — `postgres://onsager:onsager@localhost:5432/onsager`

To stop: `Ctrl+C` for services, `just dev-down` for Postgres.

## Build & Test

```bash
just build           # Rust workspace + dashboard
just test            # All tests
just test-all        # Includes spine integration tests
just lint            # fmt + clippy + eslint
```

Or directly:

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Install

```bash
just install         # installs onsager dispatcher + subsystem binaries
```

After install, both forms work:

```bash
onsager stiglab serve
stiglab serve
```

## Conventions

- Rust edition 2021, rustfmt formatting, clippy with warnings-as-errors
- `thiserror` for library errors, `anyhow` for application errors
- Small focused commits, imperative mood, under 72 characters
- Unit tests co-located in `#[cfg(test)]` modules
- All internal deps use `path = "../..."` — no git deps, no crates.io

## Preview environments

Every open PR gets an ephemeral Railway deploy at
`https://onsager-pr-<number>.up.railway.app` with a fresh Postgres plugin.
See [`docs/preview-environments.md`](docs/preview-environments.md) for
setup and troubleshooting.

## Documentation

- [`docs/architecture.md`](docs/architecture.md) — top-level architecture
  overview: subsystems, the seam rule, lever status, what's in flight.
- [`docs/adr/`](docs/adr/) — architecture decision records (start with the
  [index](docs/adr/README.md)).
- [`docs/events.md`](docs/events.md) — event catalog (auto-generated from
  `FactoryEventKind`; regenerate with `just gen-event-docs`).
- [`docs/preview-environments.md`](docs/preview-environments.md) — per-PR
  Railway previews.

Each subsystem has its own `CLAUDE.md` or `.claude/` directory with
subsystem-specific instructions:

- `crates/onsager-spine/CLAUDE.md`
- `crates/onsager-portal/CLAUDE.md`
- `crates/onsager-registry/CLAUDE.md`
- `crates/stiglab/.claude/`
- `crates/synodic/.claude/`

## License

AGPL-3.0
