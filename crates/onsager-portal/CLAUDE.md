# Portal

The **edge** subsystem — one of the factory's two processes (ADR
0027). Portal owns clause 1 of the seam rule (external HTTP
boundaries) and, since #583, runs agent sessions in-process via
`src/session_runner.rs` (no agent WebSocket surface remains).

## The seam rule (canonical)

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> The external HTTP boundary is owned by `portal` (the edge subsystem).
> Factory processes coordinate **exclusively** via the spine: events
> on the bus + reads against shared spine tables. No process makes
> HTTP calls to another process. No subsystem imports another
> subsystem's crate.

What this means for portal specifically:

- **Allowed HTTP surfaces.** Public dashboard API, OAuth callbacks,
  webhook receivers, and the MCP server. Anything an external client
  (browser, GitHub, future GitLab/Slack/Linear integrations)
  connects to. Portal owns 100% of the external HTTP surface.
- **Forbidden HTTP surfaces.** Routes that exist only to be called by
  another factory process. When portal needs the substrate scheduler
  to act, it emits a spine intent, not an HTTP request.
- **Cargo deps.** Portal may depend on `onsager-{artifact, github,
  spine, registry, substrate, agent-spawn}`. It must NOT depend on a
  sibling subsystem crate.
- **Spine is the coordination medium.** When a portal route handler
  needs the scheduler to act, emit a spine event (e.g. the dashboard
  "Run" button → `trigger.fired` / `plan.run_requested`).
- **Credentials live in portal.** `user_credentials`,
  `github_app_installations`, `user_pats`,
  `portal_webhook_secrets` — anything that decrypts to a
  `Credential` for `onsager-github` is portal-shaped. Workspace and
  workspace-membership tables live in the spine (cross-cutting).
- **Agent sessions are in-process.** `POST /api/tasks` inserts the
  session row and hands it to `SessionRunner` (semaphore-bounded
  Claude CLI spawn via `onsager-agent-spawn`); progress persists to
  `sessions` / `session_logs`; lifecycle emits `session.completed` /
  `session.failed` on the spine. Cancel kills the child process.

See [ADR 0004 — Tighten the seams](../../docs/adr/0004-tighten-the-seams.md),
[ADR 0027 — two-process consolidation](../../docs/adr/0027-two-process-consolidation.md),
and spec [#222](https://github.com/onsager-ai/onsager/issues/222)
(the staged promotion that made portal the edge; all slices landed —
see the issue for the slice-by-slice history).

## Reconciliation poller (spec #121)

Webhooks miss deliveries — GitHub's docs are explicit that delivery
is best-effort. The reconciliation poller is the backstop. One
tokio task per project ticks at the mode-derived interval, calls
the [`Adapter::poll_since`] trait, and persists the cursor in
`adapter_reconciliation_state`. Idempotency between the webhook
path and the poll path is enforced by the partial unique index
on `events_ext (adapter_id, external_ref)` (spine migration 032)
— whichever side arrives first wins; the loser is a silent
no-op via DB constraint, not coordination.

Three ingestion modes (column `projects.ingestion_mode`, default
`webhook+reconciler`):

- `webhook+reconciler` — webhooks for low latency, reconciler at
  300 s as a backstop.
- `polling-only` — local-dev / webhook-less; the poller at 60 s
  is the only ingest path.
- `webhook-only` — opt out of the reconciler (not recommended;
  silent drops become permanent).

Interval floor is 30 s (`MIN_POLL_INTERVAL`) — protects against
runaway configs hammering GitHub.

Code lives at `crates/onsager-portal/src/reconciliation/`:

- `mode.rs` — `IngestionMode` enum + interval policy.
- `state.rs` — `adapter_reconciliation_state` read/write helpers.
- `scheduler.rs` — boot-time scan + per-project poll loop.

The GitHub `Adapter` impl lives at
`crates/onsager-github/src/polling.rs`. v1 resource scope is
`issue` + `pull_request` (closed+merged) per #121 § "v1 resource
scope"; `check_*` polling is explicitly out of scope (webhook
remains the only path) until a v2 spec adds GraphQL bulk fetch.

Open follow-ups under #121:

- Spine-emit wiring **landed** via spec #430 — the scheduler now
  routes each observed update through `reconciliation::translator`,
  emits via `reconciliation::emit`, and advances the cursor only on
  a no-failure batch. Dedup against webhook deliveries happens at the
  `(adapter_id, external_ref)` partial unique index on `events_ext`.
- ETag / `If-None-Match` round-trip in `GitHubAdapter` so a 304
  skips work and doesn't count against the 5000/hr rate limit.
- Per-project / per-installation credential resolution in the
  scheduler — v1 polls unauthenticated.

## Build & Test

```bash
cargo build -p onsager-portal
cargo test  -p onsager-portal --lib
cargo clippy -p onsager-portal --all-targets -- -D warnings
```
