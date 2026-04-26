# Onsager

AI factory stack — monorepo for the Onsager event bus and its subsystems.

## Architecture

Onsager is a **factory event bus** architecture. Subsystems are runtime-decoupled
via a shared PostgreSQL `events` / `events_ext` table + `pg_notify` channel.
They coordinate through stigmergy (indirect signals via shared medium), not
direct calls.

See [ADR 0001](docs/adr/0001-event-bus-coordination-model.md) for the
decision and migration checklist.

[ADR 0002](docs/adr/0002-process-product-isomorphism.md) frames design as
two loops — the **inner loop** (spec → PR → merge) and the **outer loop**
(observe drift → propose rule → activate rule → modify inner loop) — and
commits us to process ↔ product isomorphism: every factory primitive
ships with its dev-process counterpart enabled, and every durable
dev-process pattern is filed as evidence for a future primitive.

```
         onsager-spine (event bus lib)
        /       |        |        \
   forge    stiglab   synodic    ising    <- do NOT depend on each other
```

**Architectural invariant**: subsystems (`forge`, `stiglab`, `synodic`, `ising`)
must NOT import each other, and must NOT be statically linked into the same
binary. The `onsager` dispatcher has zero business dependencies -- it discovers
subsystem binaries on PATH.

## The seam rule (canonical)

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> Subsystems (`forge`, `stiglab`, `synodic`, `ising`) coordinate
> **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another
> subsystem. No subsystem imports another subsystem's crate.

This is the rule. ADR 0001 set it; spec #131 is collapsing the
remaining places it is informally stated or informally enforced into
six levers (A–F: persisted rule → mechanical guardrails → finish ADR
0001 migration → spine as SoT → registry-backed event types →
API/UI contract enforcement). Until the levers all land, the rule is
review-time discipline; treat the drift patterns below as the working
heuristics.

Live violation (Lever C target): `crates/forge/src/cmd/serve.rs:65–180`
still constructs `HttpStiglabDispatcher` and `HttpSynodicGate` against
sibling subsystem ports. New code must not add to that pattern.

## Architectural drift patterns to watch

Loose runtime coupling is correct and stays — but the seams it creates are
informal, and recent PRs show drift accumulating in predictable shapes. When
designing or reviewing a change, watch for these and prefer **unification at
the seam** over a bridge. If a bridge is the right call for now, file a
follow-up issue with a `bridge-debt` label and a target removal date.

- **Parallel schemas across subsystems.** If two subsystems each persist their
  own version of the same concept (e.g. stiglab `tenant_workflows` vs spine
  `workflows`, PR #129), the spine wins — the private table should be
  collapsed into the spine table with a discriminator column (e.g. `tenant_id`).
  The mirror/translator pattern is a bridge, not a destination.
- **Producer with no consumer.** A subsystem can emit events that nothing
  consumes if a consumer is coded but undeployed (PR #127). Treat new event
  types as a contract: producer + consumer + deploy manifest land together,
  or the producer waits.
- **In-memory caches drifting from the bus.** If a subsystem caches state
  that the spine owns, it will drift the moment something changes that state
  out-of-band (PR #123). Default to reading from the spine; only cache with
  an explicit invalidation path tied to a spine event.
- **Half-wired API/UI contracts.** Endpoint shipped without a UI caller, or
  client method shipped without a backend handler (PR #108). Backend and
  dashboard changes for the same surface should land in one PR (or two PRs
  with a contract test that fails until both sides exist).
- **Divergent state shapes from multiple write paths.** If a row can be
  created via two paths (e.g. OAuth callback vs. manual install, PR #122),
  both paths must produce the same shape — or the read side has to be
  defensive in a single, named place, not at every call site.
- **Compat aliases that ossify.** Renames with `serde(alias=...)` or type
  aliases "for one release" (PR #107 `BundleId` → `ArtifactVersionId`) tend
  to outlive their intended window. File a `bridge-debt` issue at rename
  time; remove the alias on the target date, not "eventually".

The strategy spec #131 captures the full reasoning and the six-lever plan
to make these contracts enforced rather than informal. Until that lands,
treat the patterns above as review-time heuristics.

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

## Session defaults (Claude Code cloud)

If the current branch name starts with `claude/` (the prefix cloud sessions
create), treat PR creation and CI auto-fix as part of finishing the task —
do not wait to be asked:

1. Push the branch.
2. Open a pull request.
3. Subscribe to PR activity so CI failures and review comments are auto-fixed.

Skip this for branches that don't start with `claude/` (local/manual work).

## Per-crate context

Each subsystem has its own CLAUDE.md or `.claude/` directory with
subsystem-specific instructions:

- `crates/onsager-spine/CLAUDE.md`
- `crates/stiglab/.claude/`
- `crates/synodic/.claude/`
