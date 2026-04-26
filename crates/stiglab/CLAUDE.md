# Stiglab

Distributed AI agent session orchestration. Lives behind a public-ish
HTTP surface on port 3000 (sessions, nodes, WebSocket, OAuth/SSO,
GitHub webhooks) and listens on the spine for cross-subsystem
coordination.

## The seam rule (canonical)

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> Subsystems (`forge`, `stiglab`, `synodic`, `ising`) coordinate
> **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another
> subsystem. No subsystem imports another subsystem's crate.

What this means for stiglab specifically:

- **Allowed HTTP surfaces.** Routes under `src/server/routes/` that are
  called by the dashboard, and webhook receivers (e.g. the GitHub
  webhook router at `src/server/webhook_router.rs`) called by external
  services. Both are "external boundary" by definition.
- **Forbidden HTTP surfaces.** Anything called from `forge`, `synodic`,
  or `ising`. The legacy `HttpStiglabDispatcher` path
  (`crates/forge/src/cmd/serve.rs` ≈52–304, instantiated in `run`
  ≈469–490) is the one remaining violation, and Lever C of spec #131
  deletes it. Do not add new internal routes to satisfy a sibling
  subsystem — emit/consume an event instead.
- **Coordinating with forge or synodic.** Listen on the spine for the
  event you care about, write your response as a new event. Concrete
  pattern: forge will emit `forge.shaping_dispatched` once Lever C
  lands; stiglab consumes it and emits `stiglab.session_completed` (or
  the equivalent error event) when the session resolves.
- **Cargo deps.** `stiglab` may depend on `onsager-{artifact,
  protocol, registry, spine}` (and on `onsager-protocol` only until
  Lever C deletes that crate; matches the current
  `crates/stiglab/Cargo.toml`). It must NOT depend on `forge`,
  `synodic`, or `ising`. CI will hard-fail this once Lever B's
  architecture lint lands.
- **Spine as single source of truth.** `src/server/workflow_db.rs` +
  `src/server/workflow_spine_mirror.rs` are the live drift example —
  stiglab owns `tenant_workflows` while the spine owns `workflows`.
  Lever D collapses these into the spine schema with a `tenant_id`
  discriminator, in one PR, alongside removal of the mirror module.
  New code: write to spine tables directly; do not extend the mirror.
- **In-memory caches.** State the spine owns is read from the spine.
  Cache only with an explicit invalidation path tied to a spine event
  (the PR #123 drift pattern is what happens otherwise).

See [ADR 0001](../../docs/adr/0001-event-bus-coordination-model.md) for
the original decision and spec #131 for the six-lever enforcement plan.

## Architecture quick map

```
src/
  agent/                <- agent connection + session execution
  core/                 <- session lifecycle, queue, drain logic
  server/
    routes/             <- dashboard-facing HTTP (allowed seam)
    webhook_router.rs   <- GitHub webhook (allowed seam)
    spine.rs            <- spine read/write helpers (preferred path
                            for cross-subsystem coordination)
    workflow_db.rs      <- Lever D target: collapse into spine
    workflow_spine_mirror.rs  <- Lever D target: delete with the migration
    ws/                 <- agent WebSocket (allowed seam)
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
