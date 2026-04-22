# Changelog

All notable changes to this project are documented here. Format loosely
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project does not yet publish numbered releases.

## [Unreleased]

### Added
- **Artifact + Registry + Dashboard**: Deliverable + registry-backed kinds
  (issue #100, closes #101–#105). New `Deliverable` / `DeliverableId` /
  `WorkflowRunId` / `KindId` value objects plus `DeliverableCreated` /
  `DeliverableUpdated` spine events separate the run-scoped deliverable
  from the workflow blueprint. `BundleId` → `ArtifactVersionId` (type
  alias retained for one release); artifact columns
  `current_bundle_id` / `bundle_history` → `current_version_id` /
  `version_history` via migration 007, with serde aliases on the old
  keys. `TypeDefinition` grows `intrinsic_schema` + `merge_rule`
  (`Overwrite` / `MergeByKey` / `Append` / `DeepMerge`); canonical
  kinds renamed to `Issue` + `PR` (legacy ids ship as `aliases`); `PR`
  registers with `DeepMerge` and a schema covering commits / checks /
  reviews / merged / closes_issue; `Deployment` + `Session` seeded with
  their intrinsic schemas. Stiglab exposes `GET /api/workflow/kinds`;
  the dashboard drops its hardcoded kind union, normalizes legacy ids
  on read, ships a `DeliverablePanel` for typed PR cards, rewrites
  `ArtifactFlowOverview` to render one pill per gate (fixing the
  duplicate-PR pill on the Governed preset), and adds a `Merge → Deploy
  staging` preset chaining `PR → Deployment`.
  <!-- sha: 6268185, e0042e4 -->
- **Deploy**: per-PR Railway preview environments. Every open PR gets
  an ephemeral deploy at `onsager-pr-<N>.up.railway.app` with a forked
  (empty-by-default) Postgres plugin, torn down automatically on PR
  close. The workflow listens to Railway's `deployment_status` GitHub
  App events — no Railway API token in CI — extracts PR number + URL
  from the payload, smoke-tests the deploy, and upserts a sticky PR
  comment with the result. `railway.toml` declares the
  `[environments.preview]` overrides (smaller quota, `ONSAGER_PREVIEW`
  flag); `docs/preview-environments.md` covers setup and failure
  modes; fork PRs skip both jobs (read-only token, no secrets).
  <!-- sha: 2c72757, 151d54e, c08d077, d8856c1, cc4cf50, d959492 -->
- **Architecture**: ADR 0003 records the Deliverable + registry-backed
  kinds decisions driving spec #100 and children #101–#105 —
  `WorkflowRun` separated from `Deliverable`, artifact kinds live in
  `onsager-registry` with `intrinsic_schema` + `merge_rule`,
  `BundleId` renamed to `ArtifactVersionId`, MVP is GitHub-only.
  <!-- sha: d3012d8, 8cc45a7 -->
- **Forge + Stiglab + Dashboard**: workflow v1 — declarative, stage-driven
  factory runtime. Forge ships a pure `stage_runner` with strict
  declared-order enforcement across all four gate kinds (agent-session,
  external-check, governance, manual-approval), a `LiveGateEvaluator`
  wired to the signal cache + synodic gate + stiglab dispatcher, a
  trigger subscriber that registers `github-issue` artifacts on
  `trigger.fired`, and a `workflow_signal_listener` that classifies
  `git.ci_completed` / `git.pr_merged` / `git.pr_closed` /
  `stiglab.session_completed` into `SignalCache` entries. Spine gains
  `TriggerFired` + `StageEntered` / `GatePassed` / `GateFailed` /
  `StageAdvanced` variants on a new `workflow` namespace, plus
  `workflow_id` / `current_stage_index` / `workflow_parked_reason`
  persisted on `artifacts`. Migration 006 adds `workflows` +
  `workflow_stages`. Stiglab adds `POST/GET/PATCH /api/workflows` with
  tenant/auth guards, a `github-issue-to-pr` preset registry, and
  `POST /api/webhooks/github` with HMAC-SHA256 verification,
  1 MiB `DefaultBodyLimit`, label-match filtering, and idempotent
  label + repo-webhook registration on activation (with dedup
  deregister on deactivation). `onsager-registry` learns built-in
  `github-issue` / `github-pr` kinds. Dashboard ships `/workflows`,
  `/workflows/start` (60-second post-install card), and
  `/workflows/:id` detail, a card-stack + chat builder
  (`WorkflowBuilder`, `CardStackEditor`, `ChatBuilder`), OAuth-only
  install/repo/label pickers (`LabelCombobox` reuses cmdk with an
  inline create affordance), a constrained `ArtifactKindSelect`
  (github-issue/github-pr), and a `GateKindToggle` with `aria-pressed`
  semantics. Sidebar adds a Workflows entry under Factory; first-run
  users are redirected to `/workflows`; the Register Artifact escape
  hatch is gone from primary nav. Closes #80, #81, #82.
  <!-- sha: 427ff64, 6c0741b, c4e891b -->
- **Dev**: `just setup` one-time recipe + `.githooks/pre-commit` that
  enforces sequential `NNN_` naming on every spine and synodic
  migration file (no gaps, no duplicates). Claude Code sessions
  auto-activate via a `UserPromptSubmit` hook that runs
  `git config core.hooksPath .githooks` before every message.
  <!-- sha: 890b3e5, bce6d0c -->
- **CI**: `deploy-ready` gains a sequential-migration-naming check that
  validates every `*.sql` under the spine + synodic migration dirs has
  a leading `NNN_` prefix and that the numbers are consecutive from 1.
  <!-- sha: dfb156e -->

### Changed
- **Dashboard + Stiglab**: compressed, code-split, skeleton-first
  initial load (closes #95). Stiglab wraps static serving in gzip+br
  and sets status-aware `Cache-Control` — `immutable` on hashed
  `/assets/*` (2xx only, so a 404 during a bad deploy can't be cached
  for a year), `no-cache` on `index.html`. Dashboard lazy-loads every
  page route (login stays eager), isolates react / react-router /
  react-query into stable vendor chunks via `manualChunks`, and
  renders shimmer `AppShellSkeleton` + `PageSkeleton`
  (list / detail / default) as Suspense fallbacks instead of the
  three "Loading..." text placeholders. Onboarding-gate queries run
  in parallel; `tower` promoted to a workspace dependency.
  <!-- sha: ab6a083, be6f4d3 -->

### Fixed
- **Stiglab**: workflow activate surfaces the missing GitHub App
  permission as a `400` with an actionable message instead of a
  dead-end `502 "github api error"`. A new
  `ActivationError::MissingGithubPermission` variant carries the
  required permission name per call site (`Issues` for labels,
  `Repository webhooks` for hook endpoints) plus the upstream status
  + body; any other non-2xx keeps the `502` but stops swallowing the
  response so the failure is diagnosable from the client too.
  <!-- sha: bdf7c93, c34cfb4 -->
- **Stiglab**: `list_repo_labels` was never wired up — selecting a
  label in the workflow trigger config's `LabelCombobox` always
  showed "Couldn't load labels" even when the install was healthy.
  Added a paginated wrapper in `github_app.rs`, a handler that
  validates tenant membership + install ownership, and route
  registration for
  `/tenants/:id/github-installations/:install_id/repos/:owner/:repo/labels`.
  <!-- sha: 9c611e6, 80f9b09 -->
- **Stiglab**: renamed stiglab's tenant-scoped `workflows` /
  `workflow_stages` to `tenant_workflows` / `tenant_workflow_stages`
  so they no longer collide with spine migration `006_workflows.sql`
  — stiglab's in-process `run_migrations()` was hitting
  `IF NOT EXISTS` on the spine-created `workflows` table, skipping
  silently, then dying at `CREATE INDEX ON workflows (tenant_id)`
  with "column tenant_id does not exist" and taking the API down
  on boot. <!-- sha: cd9c3e3 -->
- **Dashboard**: workflow-create now matches stiglab's contract.
  `createWorkflowRequestToBackend` + `workflowFromBackend` translate
  between the UI draft and the wire shape stiglab validates (flat
  `trigger_kind` / `repo_owner` / `repo_name` / `trigger_label` /
  `install_id`, `active: bool`, stages as `{ gate_kind, params }`);
  `draftToCreateRequest(draft, installations, tenantId, activate)`
  resolves the numeric GitHub install id from the loaded
  installations list instead of `parseInt`-ing the `inst_abc…` record
  id. `useNodes` gates its 5s poll on resolved auth so logged-out
  users stop seeing `/api/nodes` 401 spam. The placeholder
  `ChatBuilder` is hidden from the workflow builder. A
  `tests/smoke/workflow-create-contract.test.ts` pins the adapter
  against stiglab's expected keys. <!-- sha: 27a118d, b6f5384 -->
- **Dashboard**: workflow-builder mobile + flow-strip polish. Mobile
  `Sheet` caps at `max-h-[90dvh]` with `min-h-0` so long content
  scrolls instead of overflowing off-screen; a new `PresetPicker`
  (issue → PR, agent only, CI → merge, governed pipeline) lets users
  start from a template; `ArtifactBadge` + `ArtifactFlowOverview`
  show each stage's input/output kind with an arrow separator,
  labelling both sides when a stage transforms the kind (e.g.
  agent-session issue → PR). Typed `ARTIFACT_META_BY_VALUE` as
  `Partial<Record<…>>`, decorative icons get `aria-hidden` +
  `focusable={false}`, and `GITHUB_ISSUE_TO_PR_PRESET` is shared
  between the preset constant and the `WORKFLOW_PRESETS` entry.
  <!-- sha: 88d755d, 48fd66d -->
- **Deploy**: migration ordering now uses `sort -V` (GNU version sort)
  in both the e2e workflow and the container entrypoint so numeric
  segments compare naturally (`001 < 002 < … < 009 < 010`) and new
  migrations without zero-padding still land in sequence. The
  entrypoint also switches from `ls | sort -V | while read` to
  `for f in $(…)` so `set -e` actually propagates `psql` failures —
  previously the subshell swallowed non-zero exits and the container
  appeared to migrate silently while the health check died.
  <!-- sha: 86e3486, eff83fa -->
- **Deploy**: portal's `gosu` invocation in `entrypoint.sh` is now on a
  single line with pre-expanded env vars so the trailing `&` actually
  backgrounds the process under dash — multi-line `\` continuation
  inside `sh -c` was running portal in the foreground and blocking
  stiglab startup. <!-- sha: 7b12756 -->
- **Stiglab + e2e**: `run_migrations` no longer calls SQLite-only
  `pragma_table_info()` to probe for `sessions.user_id` — that errored
  immediately against PostgreSQL. Switched to the unconditional
  `ALTER` + swallow-error pattern already used for the other additive
  column migrations. e2e also builds synodic with
  `--features synodic/postgres` (the default `sqlite` feature crashed
  on a `postgres://` URL with "PostgreSQL support not compiled in")
  and the spine migration step now applies every `*.sql` file instead
  of hard-coding the first two. <!-- sha: bfe9057 -->

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
  <!-- sha: cf6ee7b -->
- **Dashboard**: `WorkspaceCard` no longer surfaces raw UUIDs. Members
  render as avatar + `@github_login` chips backed by a new
  `list_tenant_members_with_users` query + `TenantMemberWithUser` DTO
  that LEFT JOINs `users` (falls back to the raw `user_id` when the join
  row is null). The Add Project installation `Select` trigger renders
  `{account_login} ({account_type})` via an explicit `<Select.Value>`
  children function instead of echoing the installation UUID.
  <!-- sha: 69da8eb -->
- **Dashboard**: `DropdownMenuLabel` renders as a styled `div` instead
  of Base UI's `Menu.GroupLabel` — using the label outside a
  `Menu.Group` parent threw Base UI error #31 and blanked the page on
  user-avatar click. <!-- sha: 43811c2 -->

### Added
- **Dashboard**: `/workspaces` as a first-class onboarding route —
  replaces the Settings → Workspaces card with a stepped welcome hero
  for zero-workspace users, `OnboardingGate` redirect on first visit,
  a top-level "Organization → Workspaces" sidebar entry, and a
  `QuickCreateMenu` "New Workspace" action backed by a dialog with
  auto-derived slug. Factory Overview renders a workspace-setup CTA
  banner when the user has no workspaces; Settings keeps a link-out to
  `/workspaces` for discoverability. <!-- sha: 4c3f5b7 -->
- **Dashboard**: sidebar `SetupChecklist` + progressive nav disclosure.
  Authenticated users with zero workspaces only see Organization +
  System in the sidebar; Factory/Governance/Infrastructure groups and
  the Register Artifact CTA unlock once the first workspace is created.
  A compact three-step checklist (workspace → GitHub → project) is
  pinned under the nav, session-dismissible, and auto-hides on
  completion. Sidebar + checklist share one React Query cache via a
  new `useSetupProgress` hook. <!-- sha: dedd788 -->
- **Dashboard**: `NextStepCallout` in the `WorkspaceCard` header
  surfaces the single next onboarding action — Install GitHub App, Add
  project, or Start a session — as a primary CTA with step-of-3
  framing and a distinct success state. When the GitHub App is
  unavailable server-side, an amber "Setup blocked" callout names the
  unblocking action instead of leaving the user in dead silence.
  <!-- sha: eb54ffd -->
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
- **Dashboard**: Add Project and `InstallationsSection` switch to an
  OAuth-only repo onboarding flow — a Popover + cmdk (`Command`)
  combobox with typeahead replaces manual owner/name entry, and the
  "Link manually" installation form is gone. When an install has no
  accessible repos, the empty state deep-links to the GitHub
  installation settings instead of falling back to paste. The
  underlying UX principle — linkable fields (repo owner/name,
  installation IDs, project slugs) must be solved with OAuth pickers
  or deep-links-out, never typed inputs — is persisted in the
  `dashboard-ui` skill. <!-- sha: 2d788f1 -->
- **Dashboard**: account menu consolidated into the top-right header.
  `ThemeToggle` and the account info block are removed from the
  sidebar footer; desktop now mirrors mobile with `QuickCreateMenu` +
  `UserMenu` on the right, and the sidebar footer is down to just the
  version string. <!-- sha: 82dff89 -->
- **Deploy**: `onsager-portal` is co-deployed inside the unified
  stiglab container instead of running as its own Railway service. The
  entrypoint supervises `onsager-portal serve` on `PORTAL_PORT`
  (default `3002`) and shares `STIGLAB_CREDENTIAL_KEY`; stiglab
  reverse-proxies `/webhooks/github` through to it so HMAC-SHA256
  verification still works against the raw body. `railway.toml`
  documents the three-process layout, adds `PORTAL_PORT` +
  `GITHUB_TOKEN`, and extends `watchPatterns` to trigger redeploys on
  portal / artifact / refract changes. <!-- sha: 8620d71 -->
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
