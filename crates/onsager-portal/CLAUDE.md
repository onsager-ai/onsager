# Portal

The **edge** subsystem. Portal owns clause 1 of the seam rule —
external HTTP boundaries — so the factory subsystems (`forge`,
`stiglab`, `synodic`, `ising`) can stay behind the seam and
coordinate exclusively via the spine.

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

What this means for portal specifically:

- **Allowed HTTP surfaces.** Public dashboard API, OAuth callbacks,
  webhook receivers. Anything an external client (browser, GitHub,
  future GitLab/Slack/Linear integrations) calls into.
- **Forbidden HTTP surfaces.** Routes that exist only to be called by
  a sibling subsystem. Clause 2 still applies to portal — when portal
  needs `forge`/`stiglab`/`synodic`/`ising` to do work, it emits a
  spine intent, not an HTTP request.
- **Cargo deps.** Portal may depend on `onsager-{artifact, github,
  spine, registry}`. It must NOT depend on `forge`, `stiglab`,
  `synodic`, or `ising`.
- **Spine is the coordination medium.** When a portal route handler
  needs another subsystem to act, emit a spine event (e.g. portal
  receives `PATCH /api/workflows/:id active=true`, does the GitHub
  side-effects it owns, then emits `workflow.activate_requested`
  for stiglab to consume).
- **Credentials live in portal.** `user_credentials`,
  `github_app_installations`, `user_pats`,
  `portal_webhook_secrets` — anything that decrypts to a
  `Credential` for `onsager-github` is portal-shaped. Workspace and
  workspace-membership tables live in the spine (cross-cutting).

See [ADR 0004 — Tighten the seams](../../docs/adr/0004-tighten-the-seams.md)
(amendment 2026-04-30 names portal as clause-1's owner) and spec
[#222](https://github.com/onsager-ai/onsager/issues/222) for the
promotion plan.

## Status (in flight)

Spec #222 promotes portal from a thin webhook+proxy service to a
first-class edge subsystem. The migration is staged:

- **Foundation (landed).** ADR 0004 amendment, `area:portal`
  label, root `CLAUDE.md` topology update, this file.
- **Operational shell (landed).** Portal runs alongside the factory
  subsystems via `just dev` on `:3002` (matches the `PORTAL_PORT`
  default stiglab's transitional proxy expects). `just dev-portal`
  starts it standalone. The `crates/onsager-portal/migrations/`
  directory is live and applied at startup (alongside the legacy
  inline `CREATE TABLE` calls that haven't been migrated to `.sql`
  files yet); first table to land via the new path is
  `portal_webhook_secrets` (open question 3 of #222).
- **Slice 1 — webhook ingestion (landed, PR #248).** Portal owns the
  live `POST /webhooks/github` handler; stiglab keeps the legacy URLs
  as reverse proxies via `routes::portal::proxy`.
- **Slice 5 — auth / OAuth / SSO (landed).** Portal owns
  `/api/auth/github`, `/api/auth/github/callback`, `/api/auth/me`,
  `/api/auth/logout`, `/api/auth/sso/redeem`, `/api/auth/sso/finish`,
  and (debug-only) `/api/auth/dev-login`. Three new portal-owned
  tables — `users` / `auth_sessions` / `sso_exchange_codes` — live in
  `crates/onsager-portal/migrations/{002,003,004}`. Stiglab proxies
  the same URLs through `routes::portal::proxy` (which now forwards
  `Set-Cookie` and `Location` so the OAuth dance round-trips
  unchanged) and keeps a cookie-only read against the spine-shared
  `auth_sessions` table for its own `AuthUser` extractor. PAT
  validation still lives in stiglab — Slice 2 moves it.
- **Routes (follow-ups).** Slices 2/3/4: move
  `/api/credentials/*`, `/api/pats/*`, `/api/workspaces/*`,
  `/api/installations/*`, `/api/workflows/*`, and the preset
  registry into portal. Each route group lands atomically (portal
  handler live + stiglab handler deleted in the same PR).
- **Schema split (follow-ups).** `workspaces` /
  `workspace_members` / `projects` move into
  `crates/onsager-spine/migrations/`; `user_credentials`,
  `github_app_installations`, `user_pats` move into
  `crates/onsager-portal/migrations/` next to the auth migrations.
  Atomic per-PR per Lever B.

While the migration is in flight, stiglab still hosts most of the
external HTTP surface — that is the drift #222 closes, not a pattern
to extend. New external concerns attach to portal.

## Build & Test

```bash
cargo build -p onsager-portal
cargo test  -p onsager-portal --lib
cargo clippy -p onsager-portal --all-targets -- -D warnings
```
