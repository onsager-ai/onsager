# Changelog

All notable changes to this project are documented here. Format loosely
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project does not yet publish numbered releases.

## [Unreleased]

### Fixed
- **Stiglab + Dashboard**: GitHub App install callback now registers at
  `/api/github-app/callback` (was `/api/github-app/install-callback`),
  matching the Setup URL typically configured on the GitHub App. Post-
  install redirect lands on `/workspaces?github_app_linked=…` instead of
  `/settings` so `WorkspaceCard`'s existing effect can invalidate the
  installations query and the workspace reflects the new installation
  without a manual refresh. The redundant "Install via GitHub App"
  button in `InstallationsSection` is suppressed on the empty state —
  the `NextStepCallout` now owns the sole install CTA, and the section
  only renders an "Add another installation" button once the workspace
  already has one. Fixes the four GitHub-integration bugs reported on
  the Railway deploy (callback 404 → install never recorded, duplicate
  CTAs, no post-install redirect, stale "not installed" state).

### Added
- **Architecture**: ADR 0002 frames design as two loops — inner
  (spec → PR → merge) and outer (observe drift → propose rule → activate
  rule → modify inner loop) — and commits the repo to process ↔ product
  isomorphism. Linked from `CLAUDE.md`. <!-- sha: f5f03d9 -->
- **Ising**: close the feedback loop with gate-override-rate insights.
  `GateOverrideAnalyzer` emits `ising.insight_emitted` on deny+escalate
  rate over a 7-day window; Forge's `InsightCache` tails the stream so
  `WorldState.insights` reflects live priors instead of a hardcoded
  empty vec; Governance dashboard surfaces the signals via a new
  "Ising Insights" card. Part of #36, #40. <!-- sha: aec1c03, 2c9757c -->
- **Ising**: `shape_retry_spike` analyzer flags kinds whose shaping
  rules are under-specified and routes the insight as an `Introduce`
  rule proposal into Synodic's review queue. `parse_forge_event` now
  ingests `forge.shaping_returned` so the analyzer has data in
  production. Part of #40. <!-- sha: 223b821, fe5eb57 -->
- **Ising + Synodic**: second-wave #40 sweep. `ising.rule_proposed`
  gains `signal_kind`, `subject_ref`, `proposed_action`, `class`,
  `rationale`, `confidence`; Synodic gains a `rule_proposals` table
  and listener; new `refract` stream-type events
  (`refract.intent_submitted`, `refract.decomposed`, `refract.failed`);
  new `synodic.gate_resolution_proposed` event; optional `TokenUsage`
  on `stiglab.session_completed` (wire-compatible).
  Closes #35, #37, #38, #39. Part of #36, #40. <!-- sha: b673f21 -->
- **Stiglab + Dashboard**: Phase 0 multi-tenant scaffolding —
  `tenants`, `tenant_members`, `github_app_installations`, `projects`
  tables plus `sessions.project_id`. Webhook secrets encrypted via the
  existing credential key. Dashboard adds a "Workspaces" Settings card
  (members / installations / projects) and a workspace → project
  cascade on session creation. Part of #58, #59. <!-- sha: b714e7d -->
- **Stiglab + Dashboard**: GitHub App install flow closes the Phase 0
  deferred items. New `github_app.rs` mints App JWTs (RS256), exchanges
  installation tokens, and exposes `/api/github-app/config`,
  `/install-start`, `/install-callback`, and per-install
  `accessible-repos`. `add_project` now infers `default_branch` via
  the installation token with fallback to `"main"`. Dashboard surfaces
  an "Install via GitHub App" button and a repo dropdown on Add
  Project; manual-entry path still works when the App is unconfigured.
  Part of #58, #59. <!-- sha: f751330 -->
- **Portal**: new `onsager-portal` crate — a standalone webhook
  deployable that owns GitHub ingress for every factory tenant. HMAC
  signature verification, AES-256-GCM secret decryption shared with
  stiglab, `pull_request.*` → `Kind::PullRequest` + `git.pr_*` events,
  `issues.{opened,labeled}` with `spec` label → `factory_tasks` row +
  `portal.task_materialized`, per-commit `POST /api/gate` with dedup
  and GitHub check-run posting, spec-label sync (`Closes #N` /
  `Part of #N`), and an `onsager-portal backfill` CLI
  (`recent` / `active` / `refract`). Portal owns
  `factory_tasks`, `pr_gate_verdicts`, `pr_branch_links` migrations.
  Closes #60, #61, #62. Part of #58. <!-- sha: 9c893f2 -->
- **Ising**: stream-level analyzers `pr_churn` (≥3 PR opens unmatched
  by merges → `Introduce` rule suggesting a PreDispatch evidence gate)
  and `gate_deny_rate` (≥40% Deny over ≥20 verdicts in 7 days →
  `Rewrite` rule suggesting predicate relaxation). `FactoryModel`
  learns `pr_records` / `pr_activity_by_root` from
  `git.pr_opened` / `git.pr_merged`. <!-- sha: 9c893f2 -->
- **Spine**: session ↔ PR correlation — `StiglabSessionCompleted`
  carries optional `branch` + `pr_number` (wire-compatible); stiglab
  writes `pr_branch_links` at completion; portal's `pr.opened` handler
  resolves the branch and records `vertical_lineage(session →
  pr_artifact)`. <!-- sha: 9c893f2 -->
- **Architecture**: ADR 0001 commits Onsager to the event-bus coordination
  model (Option A), with a concrete migration checklist for the remaining
  sync-RPC call sites. Linked from `CLAUDE.md`. Closes #27.
- **Crates**: split `onsager-spine` into focused crates —
  `onsager-artifact`, `onsager-warehouse`, `onsager-delivery`,
  `onsager-registry`, `onsager-protocol`. Spine now carries only event-bus
  primitives. `cargo tree -p ising` no longer pulls warehouse/delivery/
  registry. Pure refactor, no behavior change. Closes #33.
- **Forge persistence**: tick state transitions now mirror to the
  `artifacts` row after lock release, so `state`, `current_version`, and
  `current_bundle_id` survive a restart. Artifact registration is
  DB-first — the in-memory cache is only populated after the INSERT
  commits, so a failed DB write cannot leave a ghost artifact. New
  `forge::core::persistence` module and DB-backed
  `tests/persistence_restart.rs` cover the round-trip. Closes #30.
- **Spine + forge**: warehouse & delivery v0.1 foundations with the spec
  in `crates/onsager-spine/specs/warehouse-and-delivery-v0.1.md`.
- **Spine**: factory pipeline foundations (issue #14, phases 0–3),
  including run DAG, flow controls, pull request kind, and git event
  scaffolding.
- **Dashboard**: factory pipeline becomes the primary entry point with
  artifact registration; mobile UX redesigned for desktop parity.
- **Dashboard**: shadcn/ui adoption — native form/interactive elements
  replaced with shadcn primitives, enforced by the `dashboard-ui` skill.
- **Dashboard tests**: L1 Playwright e2e scaffold and L2 AI-driven
  web-testing flow (issue #23).
- **Deployment**: Railway config-as-code, deploy-readiness CI, and
  unified container with all subsystems plus the combined dashboard.
- **Skills**: `onsager-pre-push`, `onsager-pr-lifecycle`, `dashboard-ui`,
  consolidated `railway` skill, and updated session-start hook docs.

### Changed
- **Synodic**: cache `InterceptEngine` across `/gate` calls. A new
  `RulesRevision` token (`(count, MAX(updated_at))`) on the `Storage`
  trait (SQLite + Postgres) backs an `EngineCache` that skips the
  per-tick rule reload; steady-state hits are a single aggregate plus
  an `Arc` clone. Closes #32. <!-- sha: 768a25a, 934450d -->
- **Spine**: `events_ext.namespace` now maps to `stream_type` in the
  spine API consumers; `sealed_at` truncated to microseconds for
  PostgreSQL round-trip stability.
- **Stiglab**: spine events query uses static SQL strings.
- **Dashboard**: rebranded from Stiglab to Onsager with integrated logo;
  favicon uses `currentColor` to match the logo convention.
- **CI**: per-crate path filters exclude docs; migrations 003 and 005
  are applied in the Rust workflow; Rust toolchain pinned to 1.95.0.
- **Migrations**: renumbered 003 → 004 after a main merge collision.

### Fixed
- **Forge**: dropped references to removed `onsager-spine` `Kind`
  variants.
- **Spine + forge**: addressed review comments on the warehouse &
  delivery foundations.
- **Sessions**: cleaned up log output — stopped JSON leak, deduplicated
  lines, tagged stderr; `streamLogs` handles `AbortError` cleanly.
- **Container**: drop root privileges via `gosu` so the Claude CLI
  accepts `bypassPermissions`.
- **Build**: commit `Cargo.lock` so Railway builds find it; copy all
  workspace manifests in the stiglab Dockerfile.

### Docs
- Root `README.md` and `apps/dashboard/README.md` rewritten to match the
  current monorepo layout (forge, ising) and `just dev` workflow.
- Outdated specs archived; spec indexes updated.
