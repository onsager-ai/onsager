# ADR 0023 — Spec-plan run orchestration and dashboard placement

- **Status**: Accepted
- **Date**: 2026-05-29 (accepted 2026-05-29)
- **Identity impact**: no
- **Tracking issues**: #498 (umbrella spec) and sub-issues #499 (B1),
  #500 (B2), #501 (B3), #502 (B4), #503 (F1), #504 (F2).
- **Supersedes**: none. Builds on ADR 0009 (three-layer pipeline),
  ADR 0015 (Spec Plan as DAG external contract), and ADR 0017 (plan
  compiler).
- **Superseded by**: none
- **Amended by**: ADR 0025 (chat is no longer the sole launch+monitor
  surface for spec-plan runs) and #542 (the noun surface gains a guided
  authoring form, so F1's in-chat `SpecPlanDAG` authoring is complemented
  by a structured "Create Plan" surface, not replaced).

## Context

The `Spec Plan → Workflow → Execution Plan → run` pipeline (ADR 0009 /
0015 / 0017) is complete on the **authoring + compile** side but never
closes into **execution** for a persisted multi-spec Spec Plan. As of
the issue audit on `main`:

- `submit_spec_plan` persists a multi-spec `SpecPlan`
  (`crates/onsager-portal/src/spec_plan_db.rs`); `get_execution_plan`
  (`crates/onsager-portal/src/mcp/tools/substrate_specs.rs`) reads it
  back and `compile()`s it — but stops there. No reader compiles **and
  runs** a persisted plan.
- The only non-test `Scheduler::run` on a deployed path is
  `crates/onsager-scheduler/src/bridge.rs` (`handle_payload`). Per
  `trigger.fired` it hardcodes a **single-spec** plan (`specs: [one],
  deps: []`) and uses an ephemeral `InMemoryPlanStore::new()` — despite
  RUN-01 (#359) speccing the scheduler as "spine-persisted,
  restart-safe."
- No `run_spec_plan` MCP tool and no plan-level trigger exist.
- The dashboard has no surface for an authored spec-plan graph or for
  plan-run progress; chat renders spec-plan tool calls as text-only
  HitlCards.

Two decisions in closing this gap are architectural, not mechanical,
and the umbrella spec (#498) flagged both for an ADR.

## Decision 1 — Run orchestration

**The existing `onsager-scheduler` host (RUN-03, #386) drives persisted
multi-spec plans. Portal does not run plans; it emits an intent on the
spine and the scheduler host consumes it.**

- **Invocation is an event, not an HTTP call.** The `run_spec_plan` MCP
  tool (B3, on portal — the edge) emits a new spine event
  `plan.run_requested` (`FactoryEventKind::SpecPlanRunRequested`,
  producer = `Portal`, consumer = `Substrate`). Portal does not import
  the scheduler and does not run the plan itself — this is the seam
  rule: portal is the edge, the scheduler host is the runtime. The
  event mirrors how `trigger.fired` already crosses the same seam.
- **One run entry, two front doors.** The single-spec `trigger.fired`
  flow and the multi-spec `plan.run_requested` flow share **one**
  `PlanRunner::run` entry inside the scheduler host (compile a
  `SpecPlan` against a library snapshot, generate/derive a `PlanId`,
  drive `Scheduler::run` against the durable store). `bridge.rs`
  becomes a thin adapter that builds the single-element `SpecPlan` and
  delegates to that shared entry — no parallel run loops.
- **The store is durable and restart-safe.** The deployed run path
  binds a spine-backed `PlanStore` (B1) over `execution_plan_nodes` /
  `execution_plan_artifacts`, replacing the ephemeral
  `InMemoryPlanStore`. A small `execution_plans` registry maps a
  `PlanId` to the `SpecPlan` JSON it compiled from, so a host restart
  reloads any plan with non-terminal nodes, recompiles it against the
  current library, and re-drives `Scheduler::run` — which already
  resets `Ready`/`Running` to `Pending` and re-dispatches.
- **`PlanId` is derived from the source, not random, on the
  invocation path.** A `run_spec_plan` for `(workspace_id,
  spec_plan_id)` derives a stable `PlanId` so a re-invocation resumes
  the same run rather than forking a second one. The `trigger.fired`
  path keeps its workflow-derived id.

**Why the scheduler host, not portal or a new process.** The scheduler
host already owns the spine listener loop, the executor registry, and
the `Scheduler`. Portal is the edge (ADR 0006) and must stay free of
runtime concerns; the `onsager` dispatcher commits to zero business
deps. Adding a second listener (`plan.run_requested`) to the existing
host reuses all of that wiring and keeps one process responsible for
"compile a SpecPlan and run it."

`InMemoryPlanStore` remains for tests and is the seam at which the
orchestration logic is unit-tested without a database; the
`SqlxPlanStore` round-trip is covered by a `DATABASE_URL`-gated
integration test in the same style as the portal reconciliation tests.

## Decision 2 — Dashboard placement (IA)

**No `/plans` top-level noun. The authored spec-plan graph and the
plan-run progress both live in chat, the portable global component
(ADR 0020). A spec-plan run is a *Run* (one of the canonical four
nouns); folding plan runs into the Runs surface is a deliberate
follow-up, not part of this slice.**

The canonical user-facing vocabulary (spec #286) is exactly four
top-level nouns — Workflow / Run / Artifact / Stage. A `/plans` route
would introduce a fifth, which the vocabulary commitment forbids
without a deliberate decision. We decline to add it.

- **F1 — authored graph: in-chat, attached to spec-plan tool cards.**
  When a spec-plan tool (`submit_spec_plan` / `get_spec_plan` /
  `run_spec_plan`) is rendered in chat, a `SpecPlanDAG` preview renders
  the `{ specs, deps }` shape as a graph beneath the HitlCard — the
  same place `WorkflowDAGPreview` renders a single workflow's stage
  chain. This is a graph-of-specs, distinct from the stage-chain
  component. Chat is reachable from every workspace surface (ADR 0020),
  so the view is reachable without a new route.
- **F2 — run progress: the same in-chat graph, with live node state.**
  Once `run_spec_plan` (B3) launches a stored plan, the same
  `SpecPlanDAG` overlays per-spec/per-node state (pending / running /
  done / failed) sourced from the scheduler's `node.*` spine events
  (read same-origin through portal's `/api/spine/events`, keyed by the
  `substrate:<plan_id>:` stream prefix). The run is launched from chat
  and its progress shows in chat — the "reachable from where a run is
  launched" requirement is met without a new noun.
- **Future fold into Runs.** A spec-plan run *is* a Run conceptually.
  Listing plan runs alongside workflow runs on the Runs surface
  requires the backend to expose plan runs in the run-listing read
  path (today `WorkflowRun`-shaped). That is filed as a follow-up; this
  ADR fixes the placement so F1/F2 are not dangling routes in the
  meantime.

**Why in-chat rather than a detail page.** Spec plans are authored in
chat (the `issue-spec` / spec-plan authoring loop) and launched from
chat. ADR 0019 / 0021 organize detail pages around substrate primitives
that already have list views (Workflows / Runs); a spec-plan run has no
list view yet, so a `RunDetailPage`-style page would be a dangling
surface. Rendering on the chat card the plan was authored/launched from
keeps the graph reachable and avoids inventing navigation for a noun we
deliberately are not adding.

## Consequences

- A new `FactoryEventKind::SpecPlanRunRequested` variant + manifest row
  (`plan.run_requested`, producer Portal, consumer Substrate) — both
  ends declared, `check-events` satisfied.
- A new `run_spec_plan` mutation tool routes through a HitlCard
  (`check-hitl-coverage`), like `run_workflow`.
- New spine tables `execution_plans`, `execution_plan_nodes`,
  `execution_plan_artifacts` (the names RUN-01 / RUN-03 reserved). The
  scheduler host ensures them idempotently at startup and they ship as
  a canonical spine migration.
- `SpecInputs.artifacts` (B4) becomes load-bearing: the compiler
  carries each spec's declared input artifacts onto its entry node and
  the scheduler materializes them from the `PlanStore` at execution.
- The dashboard gains a `SpecPlanDAG` component and the typed
  `SpecPlan` / `SpecRef` / `SpecDep` shapes it consumes.

## Adoption checklist

- [x] Decision recorded (this ADR), `Identity impact: no`.
- [x] B1 — durable `PlanStore` + recovery (#499). `SqlxPlanStore`
  (`crates/onsager-scheduler/src/plan_store.rs`) + recovery in
  `service.rs`.
- [x] B2 — run a persisted multi-spec `SpecPlan` via the shared entry
  (#500). `PlanRunner::run` (`crates/onsager-scheduler/src/plan_runner.rs`).
- [x] B3 — `run_spec_plan` tool + `plan.run_requested` trigger (#501).
  `FactoryEventKind::SpecPlanRunRequested` + the MCP tool.
- [x] B4 — wire `SpecInputs` at run (#502).
- [x] F1 — `SpecPlanDAG` in-chat (#503)
  (`apps/dashboard/src/components/chat/SpecPlanDAG.tsx`).
- [x] F2 — plan-run progress overlay in-chat (#504)
  (`apps/dashboard/src/components/chat/SpecPlanRunView.tsx`).

All landed via #505. The future fold of plan runs into the Runs surface
remains the deliberate follow-up named in Decision 2.
