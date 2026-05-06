# Stiglab

Distributed AI agent session orchestration. Lives behind a public-ish
HTTP surface on port 3000 (sessions, nodes, WebSocket) and listens on
the spine for cross-subsystem coordination.

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
  called by the dashboard. Routes that have moved to portal (spec
  #222) stay live as reverse proxies via `routes::portal::proxy` so
  the dashboard's API_BASE cutover in Slice 6 can land independently:
  - GitHub webhook ingestion (`/webhooks/github`,
    `/api/webhooks/github`, `/api/github-app/webhook`) — Slice 1.
  - Auth / OAuth / SSO (`/api/auth/github`,
    `/api/auth/github/callback`, `/api/auth/me`, `/api/auth/logout`,
    `/api/auth/sso/redeem`, `/api/auth/sso/finish`,
    `/api/auth/dev-login` in debug builds) — Slice 5. Stiglab keeps
    cookie + PAT validation (`AuthUser` extractor reads the shared
    `auth_sessions` and `user_pats` tables) but no longer mints
    sessions or PATs.
  - Personal Access Tokens (`GET/POST /api/pats`,
    `DELETE /api/pats/{id}`) — Slice 2b. Portal mints/lists/revokes;
    stiglab still verifies presented PATs in its own `AuthUser`
    extractor for the credentials/workspaces/projects/workflows
    routes that haven't moved yet.
- **Forbidden HTTP surfaces.** Anything called from `forge`, `synodic`,
  or `ising`. **Lever C status (#148): no remaining violation** —
  `HttpStiglabDispatcher` and the `POST /api/shaping` route it
  called are gone as of phase 5. Do not add new internal routes to
  satisfy a sibling subsystem — emit/consume an event instead.
- **Coordinating with forge or synodic.** Listen on the spine for the
  event you care about, write your response as a new event. Concrete
  pattern in production today: stiglab's `shaping_listener` consumes
  `forge.shaping_dispatched`, spawns the agent session, and emits
  `stiglab.session_completed` + `stiglab.shaping_result_ready` when
  the session reaches a terminal state (or `stiglab.session_failed`
  on the error path).
- **Cargo deps.** `stiglab` may depend on `onsager-{artifact,
  registry, spine}` (the protocol DTOs now live in
  `onsager_spine::protocol` per #131 Lever C; the standalone
  `onsager-protocol` crate is gone). It must NOT depend on `forge`,
  `synodic`, or `ising`. CI will hard-fail this once Lever B's
  architecture lint lands.
- **Spine as single source of truth.** Lever D (#149) is done. Stiglab
  no longer keeps a private `workspace_workflows` schema —
  `src/server/workflow_db.rs` writes the spine `workflows` /
  `workflow_stages` tables directly, the `workflow_spine_mirror.rs`
  translator is gone, and the source tables are dropped by the spine
  migration. New code: write to spine tables directly via the spine
  pool (`state.spine.as_ref().pool()`); do not introduce a
  per-subsystem mirror table for spine-owned data.
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
                            + reverse proxies to portal for GitHub webhook
                            ingress and `/api/auth/*`
                            (`routes::portal::proxy`)
    spine.rs            <- spine read/write helpers (preferred path
                            for cross-subsystem coordination)
    workflow_db.rs      <- workflow CRUD against the spine pool
                            (post-Lever D)
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
