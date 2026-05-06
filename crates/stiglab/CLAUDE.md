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
    extractor for the workspaces/projects/workflows routes that
    haven't moved yet.
  - Per-workspace credentials
    (`GET /api/workspaces/:id/credentials`,
    `PUT/DELETE /api/workspaces/:id/credentials/:name`) — Slice 2a.
    Portal owns the read/write surface and the AES-256-GCM helpers;
    stiglab still owns the in-process decrypt-and-launch path used
    when spawning agent sessions (`decrypt_credential` in
    `tasks.rs`/`workflows.rs`).
  - Workspace + member + project CRUD (`GET/POST /api/workspaces`,
    `GET /api/workspaces/:id`, `GET /api/workspaces/:id/members`,
    `GET/POST /api/workspaces/:id/projects`, `GET /api/projects`,
    `GET/DELETE /api/projects/:id`) — Slice 3a. Portal is the only
    writer; the `workspaces` / `workspace_members` / `projects`
    schema lives in `crates/onsager-spine/migrations/020_workspaces_to_spine.sql`
    post-Slice 3a. Stiglab still reads the same Postgres tables for
    the in-process session/task/workflow lookups (`db::is_workspace_member`,
    `db::get_project`, `db::list_workspaces_for_user`, etc.) — same
    database, separate connection pool.
  - GitHub App installation routes
    (`GET/POST /api/workspaces/:id/github-installations*`,
    `GET /api/github-app/{config,install-start,callback}`) — Slice 3b.
    Portal is the only writer; the `github_app_installations` schema
    lives in `crates/onsager-portal/migrations/007_github_app_installations.sql`
    post-Slice 3b. Stiglab still reads the same Postgres table for
    the in-process session/task/project lookups
    (`db::get_github_app_installation`,
    `db::get_install_webhook_secret_cipher`) — same database,
    separate connection pool.
  - Workflow CRUD + GitHub side-effects
    (`GET/POST /api/workflows`, `GET/PATCH/DELETE /api/workflows/:id`,
    `GET /api/workflows/:id/runs`, `GET /api/workflow/kinds`) — Slice 4.
    Portal is the only writer of `workflows` / `workflow_stages` and
    the only process that talks to GitHub for label create + webhook
    register/deregister. Stiglab keeps `src/server/workflow_db.rs`
    slimmed to a single read function
    (`find_active_github_workflows_for_workspace_repo`) used by
    `routes/projects.rs` replay-trigger — same database, separate
    connection pool, portal is the only writer. The
    `core/preset.rs` and `core/workflow.rs` writer surface (and
    `server/workflow_activation.rs`) moved to portal in the same PR.
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
  no longer keeps a private `workspace_workflows` schema; the
  `workflow_spine_mirror.rs` translator is gone and the source tables
  are dropped by the spine migration. Spec #222 Slice 4 then moved
  the writer surface (`server/workflow_activation.rs` and the full
  `server/workflow_db.rs` CRUD) to portal — the slim
  `server/workflow_db.rs` that remains in stiglab is read-only,
  serving the `routes/projects.rs` replay-trigger. New code that
  reads spine tables uses the spine pool
  (`state.spine.as_ref().pool()`); new writes go through the
  appropriate edge subsystem (portal owns workflow writes), not a
  per-subsystem mirror table.
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
    workflow_db.rs      <- read-only workflow lookup against the
                            spine pool (writer surface moved to
                            portal in #222 Slice 4)
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
