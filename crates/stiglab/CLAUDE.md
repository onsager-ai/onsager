# Stiglab

Distributed AI agent session orchestration. Post-ADR 0006 / ADR 0008
stiglab is fully **internal**: it binds to `127.0.0.1:3000` (loopback
only), exposes a single route (`/agent/ws-internal`) that portal's
`/agent/ws` proxies bytes to, and listens on the spine for
cross-subsystem coordination.

## The seam rule (canonical)

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> The external HTTP boundary is owned by `portal` (the edge subsystem).
> Factory subsystems (`forge`, `stiglab`, `synodic`, `ising`) coordinate
> **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another
> subsystem. No subsystem imports another subsystem's crate.

What this means for stiglab specifically:

- **No external HTTP surfaces.** After [ADR 0008](../../docs/adr/0008-portal-owns-the-agent-control-plane.md)
  (spec #291) stiglab hosts exactly one HTTP route —
  `/agent/ws-internal` — and it is bound to loopback only. Portal
  terminates the externally-reachable `/agent/ws` upgrade and
  forwards bytes here over `127.0.0.1:3000`. Every dashboard-facing
  `/api/*` route lives on portal; stiglab no longer carries any
  reverse proxies. `xtask check-api-contract` enforces this:
  adding a non-loopback-only route here is a hard CI failure.
- **Coordinating with forge or synodic.** Listen on the spine for the
  event you care about, write your response as a new event. Concrete
  pattern in production today: stiglab's `shaping_listener` consumes
  `forge.shaping_dispatched`, spawns the agent session, and emits
  `stiglab.session_completed` + `stiglab.session_result_ready` when
  the session reaches a terminal state (or `stiglab.session_failed`
  on the error path).
- **Reads of portal-owned tables.** The route surfaces moved to
  portal in spec #222 (`workspaces` / `workspace_members` /
  `projects`, `github_app_installations`, `workflows` /
  `workflow_stages`, PATs, credentials, auth sessions, spine API).
  Portal is the only writer. Stiglab keeps narrowly-scoped reads of
  the same tables via its own connection pool when the agent runtime
  needs them — same database, separate pool, never a write.
- **Cargo deps.** `stiglab` may depend on `onsager-{artifact,
  github, spine}`. It must NOT depend on `forge`, `synodic`, or
  `ising`. CI hard-fails this via `lint-seams`.
- **Spine as single source of truth.** Lever D (#149) is done. Stiglab
  no longer keeps a private `workspace_workflows` schema; the
  `workflow_spine_mirror.rs` translator is gone and the source tables
  are dropped by the spine migration. New reads of spine tables use
  the spine pool (`state.spine.as_ref().pool()`); new writes go
  through the appropriate edge subsystem (portal owns workflow
  writes), not a per-subsystem mirror table.
- **In-memory caches.** State the spine owns is read from the spine.
  Cache only with an explicit invalidation path tied to a spine event
  (the PR #123 drift pattern is what happens otherwise).

See [ADR 0001](../../docs/adr/0001-event-bus-coordination-model.md) for
the original decision, spec #131 for the six-lever enforcement plan,
and ADR 0006 / ADR 0008 for the process-level move that closed the
last clause-1 carve-out.

## Architecture quick map

```
src/
  agent/                <- agent connection + session execution
  core/                 <- session lifecycle, queue, drain logic
  server/
    routes/             <- shaping dispatch core called by the
                            spine listener (no HTTP routes
                            registered post-ADR 0008)
    spine.rs            <- spine read/write helpers (preferred path
                            for cross-subsystem coordination)
    workflow_db.rs      <- read-only workflow lookup against the
                            spine pool (writer surface lives in
                            portal per #222 Slice 4)
    ws/                 <- /agent/ws-internal handler (loopback only;
                            portal proxies /agent/ws here)
```

## Build & Test

Run from repo root:

```bash
cargo build -p stiglab
cargo test  -p stiglab --lib
cargo clippy -p stiglab --all-targets -- -D warnings
```

CI runs the workspace pass with `RUSTFLAGS="-D warnings"` against a
merge preview of `origin/main`; reproduce that locally via the
`onsager-pre-push` skill before pushing.
