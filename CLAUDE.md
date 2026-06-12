# Onsager

Workflow automation that runs AI agent sessions and turns their output into trusted artifacts. One process, one database, one dashboard.

## What makes Onsager Onsager

These commitments define the system's identity. Changes to them carry an `Identity impact: yes` flag in the modifying ADR ([`docs/adr/`](docs/adr/); [`docs/adr/README.md`](docs/adr/README.md) is the index) and require explicit rationale.

- **Postgres is the spine; events are the record, not the protocol** ([ADR 0029](docs/adr/0029-one-process-progressive-rebuild.md), amending ADR 0001). All durable truth — workflows, runs, sessions, artifacts, credentials — lives in Postgres, and every run writes an append-only audit trail to the `events` table that the dashboard reads back. Coordination inside the factory is function calls; there is no event-RPC, no listener dispatch, no second process. Re-splitting along the Postgres seam stays possible if scale ever demands it — that is the load-bearing half of the original event-bus commitment, kept.
- **Artifacts are the unit of meaning.** A run's purpose is its artifacts. Every artifact carries first-class **provenance** (`uncertain` / `deterministic` + source, ADR 0010's concept carried forward) from the moment it's written; agent output is `Uncertain` until something verifies it. Verification machinery returns when a verifier has a consumer.
- **Specs are ground truth, code is downstream.** When spec and code conflict, the code is wrong. Specs are amended deliberately (recorded on the issue before the deviation ships); code is amended to match. This governs the dev process itself, including every porting decision of the 0.5 reset.
- **No producer without a consumer.** A new table, event kind, route, concept, or config field lands in the same PR as the thing that reads it — or it doesn't land. This single review-time ratchet replaces the v1 lint suite; it is how the schema stays 11 tables instead of growing back to 25.

### Claim-honesty checks

Before claiming a Plan item done — in a session message, checkbox tick, or PR description:

- **"The main path works, so it's essentially done."** Plan items are atomic. Either complete, or split into `N.1` (done) / `N.2` (deferred, with reason) and amend the spec before merge.
- **"Tests pass, so the change is correct."** Mutate one line of the new path locally; a test (or a live e2e) must catch it. Green CI plus uncovered logic is theater.
- **"I'll handle the edge case as a follow-up."** A follow-up with no issue doesn't exist. Open one (`Part of #N`) or do it now.
- **"The workaround is fine."** Maybe — but amend the Plan item to say "workaround, because X", or open the follow-up issue. Shipping a workaround under unchanged Plan wording is silent scope reduction.

Named failure modes for review shorthand: *claim ≠ reality*, *silent scope reduction*, *theater coverage*, *narrative-as-state*, *untracked defer*.

## Operating posture: pre-launch

Onsager is not yet live to users. That removes user-protection scaffolding only — never the discipline that produces well-built work. Skip: compat aliases, deprecation windows, migration choreography for data nobody has, preview banners. Keep: spec discipline, claim-honesty, tests incl. the mutation check, review, root-cause fixes, the ratchet. Spend the window on structural moves cheap now and expensive later. Flipping the posture at launch is an ADR-worthy moment.

## Architecture

One binary (`onsager`) serves everything:

- **HTTP edge** — dashboard API, auth (dev-login + GitHub OAuth), credentials CRUD, the GitHub App webhook, and the agent-facing MCP endpoint (`POST /mcp/messages`).
- **Scheduler** — in-process: a fire is a function call that inserts a `runs` row and spawns a supervised tokio task which iterates the workflow's stage list. Abort = abort the task + SIGKILL the session's process group.
- **Agent sessions** — one spawn path (`sessions.rs`): the `claude` CLI with `--mcp-config` pointing back at this process. Sessions deliver via the MCP `emit_artifact` tool (writes an `artifacts` row) or `declare_no_output`; a session that does neither fails its run (liveness, plain).
- **Static dashboard** — in production the same process serves the built Vite app (`ONSAGER_STATIC_DIR`); one container, no edge dispatcher.

In dev, the dashboard is Vite with an `/api` + `/mcp` proxy.

### Schema (11 tables, fresh migrations, `schema_migrations` ledger)

`users`, `auth_sessions`, `workspaces`, `workspace_members`, `credentials` (001) · `workflows`, `runs`, `sessions`, `session_logs`*, `artifacts`, `events` (002) · `github_app_installations` (003). Migrations are immutable after merge; fixes are new migrations. (*`session_logs` is in 002 awaiting its consumer — first candidate for the ratchet if streaming logs don't land.)

Key shapes:

- `workflows.trigger` / `.definition` are jsonb serde enums (`TriggerKind`, `Vec<StageKind>`) — closed enums, no trait objects; the executable form is derived at fire time, never persisted twice.
- `artifacts`: `content_uri` XOR `external_ref` (CHECK-enforced), `provenance` jsonb, backrefs to run + session.
- `events`: append-only audit; the run timeline and Activity feed read it; nothing dispatches off it.

## User-facing vocabulary (4 nouns)

**Workflow** (trigger + stages) · **Run** (one execution; the center of gravity) · **Artifact** (what a run produces; carries provenance) · **Stage** (a step in a definition; surface-internal). Internal terms (sessions, pgids, tokens) stay internal. New nav nouns default to "no" and need an ADR.

## GitHub integration

- **One App-level webhook** (`/api/webhooks/github`, `GITHUB_WEBHOOK_SECRET`, fail closed). `installation` events maintain `github_app_installations`; `issues.labeled` matches active workflows by `repo`+`label` and fires them with the issue prepended to the first agent stage's prompt.
- **Repo access**: a write-scoped installation token minted per run for the trigger repo, surfaced as `ONSAGER_REPOS` (clone URLs with `x-access-token`). The agent pushes branches and opens PRs itself.
- `crates/onsager-github` is the only other workspace crate — the typed GitHub client (App JWT, token mints, OAuth, signature verification). No other code talks to github.com.

## Workspace layout

```
crates/
  onsager/          <- the factory: http/, scheduler, sessions, mcp, github, auth, domain, events
  onsager-github/   <- typed GitHub client (App, OAuth, webhook signatures)
apps/
  dashboard/        <- React UI (workflows, runs, artifacts, activity, credentials)
deploy/Dockerfile   <- single-container build (binary + dashboard dist)
```

## Getting started

Prerequisites: Docker, Rust toolchain, pnpm.

```bash
cp .env.example .env     # set ONSAGER_CREDENTIAL_KEY (openssl rand -hex 32)
just setup               # git hooks + pnpm install
just dev                 # Postgres + the onsager process + Vite
```

Then dev-login in the dashboard and add `CLAUDE_CODE_OAUTH_TOKEN` under Credentials — agent sessions read workspace credentials as env vars. The server applies migrations at boot.

Useful env: `ONSAGER_BIND` (default `127.0.0.1:3002`), `ONSAGER_CLAUDE_BIN` (shim the agent CLI in tests), `ONSAGER_MCP_URL` (defaults to the bind address), `GITHUB_APP_ID/SLUG/PRIVATE_KEY`, `GITHUB_WEBHOOK_SECRET`, `GITHUB_CLIENT_ID/SECRET`.

### Parallel dev environments (per-worktree)

Git worktrees + the devproxy: each worktree gets its own `COMPOSE_PROJECT_NAME` / `DEV_HOST` via direnv and is reachable through the machine Traefik at `http://<branch>.onsager.localhost:8000` when `~/dev-proxy/dynamic` exists (`just dev` self-registers). Manage worktrees with `wt new <branch>` / `wt rm <branch>` / `wt ls`.

## Build & test

```bash
just build   # cargo + dashboard
just test    # cargo test + vitest
just lint    # fmt + clippy -D warnings + eslint
```

The e2e pattern: boot the binary against a fresh database, drive the API with curl, and shim the agent with `ONSAGER_CLAUDE_BIN` (a script that prints a stream-JSON `result` event; see the M2/M3 PRs for working shims). The canonical loop check is the M1 e2e: create workflow → activate → fire → artifact row.

## Conventions

- Rust edition 2024; rustfmt; clippy warnings-as-errors; thiserror for library errors, anyhow at the edges; unit tests co-located.
- Small focused commits, imperative mood, under 72 characters.
- **Merge policy**: PR → main squash only; update branches by rebase (`git pull --rebase origin main`).
- Migrations: `NNN_name.sql`, contiguous (pre-commit + deploy-ready enforce), immutable after merge.
- Prefer `Edit` over `Write` for existing files; split big rewrites.

## Session defaults (Claude Code cloud)

If the current branch starts with `claude/`: push, open a PR (spec issue linked via `Closes #N` / `Part of #N`, or `labels: ["trivial"]` for genuinely trivial changes — default is spec), pass `body` as a plain inline string, and subscribe to PR activity for CI auto-fix.

## History

v1 — the multi-subsystem event-bus factory (16 crates at peak) — was consolidated by ADR 0027 (0.3), simplified by ADR 0028 (0.4), and rebuilt as this one-process system by ADR 0029 (0.5). The v1 tree lived in `legacy/` during the rebuild and was deleted at the M4 flip; it remains in git history (PR #627 moved it in, the M4 PR removed it). The ADRs under `docs/adr/` carry the full reasoning chain.
