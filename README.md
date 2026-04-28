# Onsager

AI factory stack — monorepo for the Onsager event bus and its subsystems.

## Architecture

Onsager is a **factory event bus** architecture. Subsystems are runtime-decoupled
via a shared PostgreSQL `events` / `events_ext` table + `pg_notify` channel.
They coordinate through stigmergy (indirect signals via shared medium), not
direct calls.

```
         onsager-spine (event bus lib)
        /       |        |        \
   forge    stiglab   synodic    ising
```

Subsystems must NOT import each other and must NOT be statically linked into
the same binary. The `onsager` dispatcher has zero business dependencies — it
discovers subsystem binaries on `PATH`.

## Subsystems

| Crate           | Role                                                          |
|-----------------|---------------------------------------------------------------|
| `onsager-spine` | Shared event bus library (PostgreSQL + `pg_notify`)           |
| `onsager`       | Unified CLI dispatcher (`onsager <subsystem> ...`)            |
| `forge`         | Production line — drives artifacts through their lifecycle    |
| `stiglab`       | Distributed AI agent session orchestration                    |
| `synodic`       | AI agent governance (hooks + spine integration)               |
| `ising`         | Continuous improvement engine — observes and surfaces insights|
| `onsager-portal`| GitHub webhook ingress — verifies HMAC, materializes factory tasks, posts check-run verdicts |

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

## Per-crate context

Each subsystem has its own `CLAUDE.md` or `.claude/` directory with
subsystem-specific instructions:

- `crates/onsager-spine/CLAUDE.md`
- `crates/stiglab/.claude/`
- `crates/synodic/.claude/`

## License

AGPL-3.0
