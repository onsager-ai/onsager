# Onsager architecture

A navigable map of how Onsager is built today, what shape it is moving toward,
and which ADR or spec issue carries each decision. The canonical statements
live in `CLAUDE.md` and the four ADRs under [`docs/adr/`](adr/) — this doc is
the index that ties them together.

If you are an AI session, the operational rules you need are in root
`CLAUDE.md` and the per-subsystem `CLAUDE.md` files. Read this doc when you
need the *why* behind a rule or want to see what's still in flight.

## At a glance

Onsager is an **AI factory event bus**. A shared PostgreSQL `events` /
`events_ext` table plus `pg_notify` is the single coordination medium;
subsystems coordinate through *stigmergy* (indirect signals via the shared
medium), not direct calls. The `onsager` dispatcher is a ~100-line CLI with
zero business deps that discovers subsystem binaries on `PATH`.

```
                       onsager-spine  (event bus library)
                              │
        ┌─────────┬───────────┼───────────┬──────────┬──────────┐
        │         │           │           │          │          │
     portal    forge       stiglab     synodic     ising     refract
     (edge)                                                  (decomposer)
```

- **Edge subsystem:** `portal` — the only subsystem permitted to host public
  HTTP routes (dashboard API, GitHub webhooks, OAuth, credentials).
- **Factory subsystems:** `forge`, `stiglab`, `synodic`, `ising`, `refract` —
  AI-native concerns. Coordinate exclusively via the spine.
- **Library plane:** `onsager-{spine, artifact, warehouse, delivery, registry,
  github}` — typed shared building blocks. No runtime of their own.

The dashboard (`apps/dashboard/`) is a single React app surfacing every
subsystem.

## The two-clause seam rule

The seam rule is the canonical architectural invariant. It has two clauses:

> **Clause 1 — external boundary.**
> HTTP APIs exist only at external boundaries: user-facing endpoints called
> by the dashboard, and webhooks called by external services (GitHub, etc.).
> The external HTTP boundary is owned by `portal` (the edge subsystem).
>
> **Clause 2 — internal coordination.**
> Factory subsystems (`forge`, `stiglab`, `synodic`, `ising`, `refract`)
> coordinate **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another subsystem.
> No subsystem imports another subsystem's crate.

Clause 2 is fully enforced (ADR 0004). Clause 1 is being concretized
(spec [#220](https://github.com/onsager-ai/onsager/issues/220)) — see "In
flight" below.

## Architecture decision records

| ADR | Decision | Status |
|---|---|---|
| [0001](adr/0001-event-bus-coordination-model.md) | Event bus is the coordination medium (stigmergy over sync RPC) | Accepted; migration complete via ADR 0004 Lever C |
| [0002](adr/0002-process-product-isomorphism.md) | Process ↔ product isomorphism as design discipline | Accepted (amended 2026-05-01 to scope down the explicit five-primitive outer-loop programme) |
| [0003](adr/0003-deliverable-and-registry-backed-kinds.md) | Deliverable as workflow-run output; registry-backed artifact kinds | Accepted; v1 landed (children #101–#105 partial) |
| [0004](adr/0004-tighten-the-seams.md) | Tighten the seams: HTTP at external boundaries, spine for everything internal | Accepted; all six levers landed (2026-04-30) |

[`docs/adr/README.md`](adr/README.md) carries one-line summaries for navigation.

## Subsystems

| Subsystem | Role | `CLAUDE.md` |
|---|---|---|
| [`portal`](../crates/onsager-portal/) | **Edge** — public HTTP, GitHub webhooks, OAuth, credentials, workflow CRUD (in-flight handover from stiglab) | [crates/onsager-portal/CLAUDE.md](../crates/onsager-portal/CLAUDE.md) |
| [`forge`](../crates/forge/) | Production line — drives artifacts through their lifecycle; pure state machine over the spine | — |
| [`stiglab`](../crates/stiglab/) | Distributed AI agent session orchestration; today still hosts most external HTTP | [crates/stiglab/.claude/](../crates/stiglab/.claude/) |
| [`synodic`](../crates/synodic/) | AI agent governance (gates, verdicts, escalations) | [crates/synodic/.claude/](../crates/synodic/.claude/) |
| [`ising`](../crates/ising/) | Continuous improvement — observes spine, surfaces insights, proposes rules | — |
| [`refract`](../crates/refract/) | Intent decomposer — expands a high-level intent into an artifact tree | — |

Library crates (no runtime; consumed by subsystems):

| Crate | Role |
|---|---|
| [`onsager-spine`](../crates/onsager-spine/) | Event bus client: `EventStore`, `Listener`, `Namespace`, `FactoryEvent`. Single source of truth for events and shared workflow tables. |
| [`onsager-artifact`](../crates/onsager-artifact/) | Domain value objects: `Artifact`, `ArtifactId`, `ArtifactVersionId`, `Kind`, lineage, quality. |
| [`onsager-warehouse`](../crates/onsager-warehouse/) | Bundle sealing + `Warehouse` trait. |
| [`onsager-delivery`](../crates/onsager-delivery/) | Consumer routing for sealed bundles. |
| [`onsager-registry`](../crates/onsager-registry/) | Type registry, seed catalog, evaluators; SoT for artifact kinds (ADR 0003) and event manifest (ADR 0004 Lever E). |
| [`onsager-github`](../crates/onsager-github/) | Typed GitHub API + webhook verification + OAuth; library plane (spec [#220](https://github.com/onsager-ai/onsager/issues/220) Sub-A landed). |
| [`onsager`](../crates/onsager/) | Dispatcher CLI (~100 LOC, no business deps). |

Subsystem ↔ support-crate dependencies (canonical):

- `forge`   → `onsager-{artifact, warehouse, spine}` (DTOs in spine since #131 Lever C; no separate `protocol` crate)
- `stiglab` → `onsager-{artifact, spine}`
- `synodic` → `onsager-{artifact, spine}`
- `ising`   → `onsager-{artifact, spine}`
- `refract` → `onsager-{artifact, spine}`
- `portal`  → `onsager-{artifact, spine, github, registry}` — never depends on a sibling subsystem

## What's enforced, mechanically

The seam rule is no longer review-time discipline. Three xtask lints
hard-fail at CI and pre-push:

| Lint | Catches |
|---|---|
| [`xtask lint-seams`](../xtask/src/lint_seams.rs) | Subsystem-to-subsystem Cargo deps; sibling-subsystem HTTP (well-known ports / `*_URL` env vars / `localhost:<port>` literals); new `serde(alias)`; new `*_mirror.rs`; legacy type aliases. |
| [`xtask check-events`](../xtask/src/check_events.rs) | Coverage of [`crates/onsager-registry/src/events.rs`](../crates/onsager-registry/src/events.rs) (75 rows, one per `FactoryEventKind`); both-ends declared; emit-call-sites match producers; listener-call-sites match consumers. Producer-without-consumer hard-fails (PR [#229](https://github.com/onsager-ai/onsager/pull/229)). |
| [`xtask check-api-contract`](../xtask/src/lint_api_contract.rs) | Every backend route has a dashboard caller (or an allowlisted external-only reason) and every dashboard call lands on a backend route. |

The event manifest is exposed at `GET /api/registry/events` for runtime
introspection, and the human-readable event catalog is regenerated from
`FactoryEventKind` into [`docs/events.md`](events.md).

## ADR 0004 lever status

ADR 0004's six levers are the canonical project-wide checklist. As of
2026-04-30 all six have landed.

| Lever | What it does | Status |
|---|---|---|
| **A** — Persist the rule | Verbatim in root + subsystem `CLAUDE.md` and three skills (`issue-spec`, `onsager-pre-push`, `onsager-dev-process`) | Landed (PR [#144](https://github.com/onsager-ai/onsager/pull/144)) |
| **B** — Mechanical guardrails | `lint-seams` arch-deps, sibling-subsystem HTTP, `serde(alias)`, `*_mirror.rs`, legacy type aliases | Landed (PR [#155](https://github.com/onsager-ai/onsager/pull/155)) |
| **C** — Complete the ADR 0001 migration | Delete `HttpStiglabDispatcher` + `HttpSynodicGate`; coordinate via `forge.gate_requested` / `synodic.gate_verdict` and `forge.shaping_dispatched` / `stiglab.shaping_result_ready`; delete `onsager-protocol` | Landed (PR [#148](https://github.com/onsager-ai/onsager/pull/148)) |
| **D** — Spine as SoT | Collapse `workspace_workflows` / `workspace_workflow_stages` into spine `workflows` / `workflow_stages`; remove `workflow_spine_mirror.rs` and `BundleId` alias | Landed (PR [#225](https://github.com/onsager-ai/onsager/pull/225), [#219](https://github.com/onsager-ai/onsager/pull/219)) |
| **E** — Registry-backed event types | Static manifest in `crates/onsager-registry/src/events.rs`; CI enforces coverage and call-site agreement; manifest at `GET /api/registry/events` | Landed (PR [#227](https://github.com/onsager-ai/onsager/pull/227)) |
| **F** — API/UI contract | `lint-api-contract` asserts every route has a caller and every call has a route | Landed (PR [#207](https://github.com/onsager-ai/onsager/pull/207)) |

## Where we are now vs. where we want to be

Active migrations the project is steering toward. Each links the ADR or spec
that owns the target state.

### Edge subsystem promotion — `onsager-portal` becomes clause-1's owner

> Spec: [#220](https://github.com/onsager-ai/onsager/issues/220) (umbrella) /
> [#222](https://github.com/onsager-ai/onsager/issues/222) (portal promotion) /
> [#223](https://github.com/onsager-ai/onsager/issues/223) (feedback patterns)
> · ADR 0004 amendment 2026-04-30

**Now.** Portal exists and owns GitHub webhook ingest, the new
`correlation_id` column, dispatch/await helpers, and the (foundational) edge
posture. Stiglab still hosts most of the public HTTP — workflow CRUD,
credential CRUD, OAuth callbacks, the preset registry — and is doing two
jobs (agent-session execution *plus* edge).

**Target.** Portal is a first-class peer of forge / stiglab / synodic /
ising. The full public-HTTP surface lives in portal:
`/api/webhooks/github`, `/api/workflows/*`, `/api/credentials/*`,
`/api/installations/*`, `/api/workspaces/*`, `/auth/github/*`, preset
registry. Stiglab shrinks to its real role — agent session orchestration —
and `cargo build -p stiglab` no longer pulls in `axum`'s server feature.
Schema split: `workspaces` / `workspace_members` / `projects` live in spine
(landed via #161); `user_credentials` / `github_app_installations` /
`user_pats` / `portal_webhook_secrets` live in portal.

**Glue.** Portal coordinates with factory subsystems via the spine only:
when portal needs stiglab to act (e.g. activate a workflow → flip state +
dispatch), portal emits `workflow.activate_requested`; stiglab consumes.
The ≤2s synchronous-wait + 202+SSE feedback contract is owned by #223 and
rides on Lever E's manifest.

### Open-schema artifact kinds — ADR 0003 wave-2

> Spec: [#100](https://github.com/onsager-ai/onsager/issues/100) (umbrella) ·
> ADR [0003](adr/0003-deliverable-and-registry-backed-kinds.md)

**Now.** `Deliverable`, `DeliverableId`, `WorkflowRunId`, `KindId` exist;
`DeliverableCreated` / `DeliverableUpdated` events are on the bus; the
dashboard reads kinds from the registry; the seed-catalog `Spec` kind is
renamed `Issue`; `BundleId` is renamed `ArtifactVersionId` and the alias
is gone. Parts of #101 + #102 + #105 are landed.

**Target.** Per-kind merge rules (`Overwrite` / `MergeByKey` / `Append` /
`DeepMerge`) on `TypeDefinition` so gates emit partial updates that fold
deterministically into the Deliverable; PR consolidated as a rich artifact
with intrinsic `commits` / `checks` / `reviews` / `merged` and a
referential `closes_issue` link; the dashboard's flow strip splits cleanly
from the Deliverable panel; `Deployment` and `Session` registered as
first-class kinds.

**Open children.** [#102](https://github.com/onsager-ai/onsager/issues/102)
(merge rules) · [#103](https://github.com/onsager-ai/onsager/issues/103)
(PR consolidation) · [#104](https://github.com/onsager-ai/onsager/issues/104)
(WorkflowRun / Deliverable split) ·
[#105](https://github.com/onsager-ai/onsager/issues/105) (Deployment +
Session kinds).

### Reference-only external artifacts

> Spec: [#170](https://github.com/onsager-ai/onsager/issues/170) (mechanism) /
> [#171](https://github.com/onsager-ai/onsager/issues/171) (Kind::PullRequest
> migration)

**Now.** External items (PRs, issues) are projected into Onsager's artifact
tables with full state copies, which means Onsager's row drifts from
GitHub's row whenever GitHub changes out-of-band — the
*divergent-state-shapes-from-multiple-write-paths* drift pattern.

**Target.** A reference-only artifact kind that stores `(provider, ref)`
and reads through to the source of truth on demand, eliminating the local
shadow row. `Kind::PullRequest` becomes the first reference-only kind.

### Trigger taxonomy v2

> Spec: [#236](https://github.com/onsager-ai/onsager/issues/236) (umbrella) /
> [#237](https://github.com/onsager-ai/onsager/issues/237) (foundation:
> unify `TriggerKind` / `TriggerSpec`, registry-backed)

**Now.** Trigger kinds live in two divergent types (`stiglab::TriggerKind`
kebab-case, no params; `forge::TriggerSpec` snake_case, carries
`{repo, label}`). `workflows.trigger_kind` is locked to a single value via
a `CHECK` constraint, and the dashboard hardcodes `'github-issue-webhook'`
in four places. The internal-symmetry defect from `CLAUDE.md`.

**Target.** Single `TriggerKind` in `onsager-spine`, registry-backed, with
v2 categories: schedule / event / request / manual. Per-kind config rides
in `workflows.trigger_config` JSONB. Adding a new trigger kind becomes a
registry add, not a cross-subsystem refactor.

### Internal-aesthetic / interior cleanup

> Spec: [#153](https://github.com/onsager-ai/onsager/issues/153)
> (internal-aesthetic refactor) /
> [#218](https://github.com/onsager-ai/onsager/issues/218)
> (synodic orchestration TODOs)

**Now.** A handful of large modules and dangling-wire patterns survive
inside subsystems — the kind of asymmetry the *internal aesthetic* section
of `CLAUDE.md` calls out (files >~500 LOC, equivalent concepts under
different names, `#[allow(dead_code)]` "for later"). Several have been
addressed in PRs [#212](https://github.com/onsager-ai/onsager/pull/212),
[#213](https://github.com/onsager-ai/onsager/pull/213), [#231](https://github.com/onsager-ai/onsager/pull/231).

**Target.** Interior parity with the seam rule — symmetric naming across
subsystems where two things are the same concept; no `#[allow(dead_code)]`
without an open consumer ticket; no module mixing unrelated concerns.

## Drift patterns the architecture is designed against

`CLAUDE.md` lists six drift patterns that recur in PRs. They are now caught
mechanically by the lints above; the list below is the glossary of failure
modes those checks were designed against.

1. **Parallel schemas across subsystems** (e.g. former
   `workspace_workflows` ↔ spine `workflows`). Resolved by Lever D; future
   shared concepts collapse into the spine table with a discriminator.
2. **Producer with no consumer** (PR #127). Caught by `check-events`.
3. **In-memory caches drifting from the bus** (PR #123). Default to spine
   reads; cache only with an explicit invalidation tied to a spine event.
4. **Half-wired API/UI contracts** (PR #108). Caught by `check-api-contract`.
5. **Divergent shapes from multiple write paths** (PR #122). Either both
   paths produce the same shape, or the read side is defensive in one
   named place — never at every call site.
6. **Compat aliases that ossify** (PR #107: `BundleId` → `ArtifactVersionId`
   "for one release"). Hard-failed by `lint-seams`.

The seam rule and the six lints are operational projections of the
*internal aesthetic* value stated in `CLAUDE.md`: care about the inside the
same way you'd care about the outside.

## Process ↔ product isomorphism

Per ADR [0002](adr/0002-process-product-isomorphism.md): every factory
primitive ships with its dev-process counterpart enabled, and every durable
dev-process pattern is filed as evidence for a future primitive. The
two-loop framing — *inner loop* (spec → PR → merge), *outer loop* (observe
drift → propose rule → activate rule → modify inner loop) — describes how
the system operates today.

The 2026-05-01 amendment scoped down the explicit five-primitive outer-loop
programme (#35 / #36 / #37 / #38 / #39); the dev-process surfaces those
primitives would have automated (umbrella trackers, manual tracker refresh,
skills + CLAUDE.md, weekly comments, `claude/*` session tokens) continue to
fulfil their roles. The principle and the "Dev-process counterpart" section
in ADR / skill templates remain.

## Pointers

- Event vocabulary: [`docs/events.md`](events.md) — auto-generated from
  `FactoryEventKind`; regenerate with `just gen-event-docs`.
- API surface: [`crates/stiglab/src/server/`](../crates/stiglab/src/server/)
  (today) → [`crates/onsager-portal/src/server/`](../crates/onsager-portal/src/)
  (target).
- Preview environments: [`docs/preview-environments.md`](preview-environments.md).
- Worktree-based parallel dev environments: see `just worktree-new` (root
  `CLAUDE.md` § Parallel dev environments).
- Per-subsystem operational notes: `crates/<subsystem>/CLAUDE.md` or
  `crates/<subsystem>/.claude/`.
