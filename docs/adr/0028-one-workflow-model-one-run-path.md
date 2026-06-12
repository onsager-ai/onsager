# ADR 0028 — 0.4 simplification: one workflow model, one run path

- **Status**: Accepted
- **Date**: 2026-06-11
- **Identity impact**: no
- **Adoption**: enforced (2026-06-12 — all six sub-specs landed; wave-close e2e green, evidence on #599)
- **Tracking issues**: #599 (umbrella; sub-specs listed there)
- **Supersedes**: the Lever D legacy workflow storage model (ADR 0004's
  `workflows`/`workflow_stages` stage-chain as the authored form); the
  `trigger.fired → bridge` dispatch contract of RUN-03 (#386). ADR 0009 /
  0017 / 0018 / 0027 remain in force — this ADR *completes* 0027's
  consolidation at the data-model altitude.
- **Superseded by**: none

## Context

The 0.3 wave (ADR 0027) deleted the dormant quarter of the workspace.
A follow-up audit of what *remains* found the debt is now duplicated
live structure, not dead code:

1. **Two workflow models.** The dashboard builder edits the legacy
   stage-chain (`workflows` + `workflow_stages`, `gates` jsonb,
   `translate_stage`) — ~3.7k LOC in portal plus ~4.2k LOC of builder
   UI — while the engine executes substrate DAGs from
   `workflow_library`. They are joined only by a `spec_kind` string
   resolved by a "best-effort v1" bridge (payload field, else preset
   fallback, else drop). The 0.3 wave-close e2e made the split
   concrete: the workflow visible in the dashboard was not the thing
   that ran; a library entry had to be registered separately.
2. **Two run semantics.** `plan.run_requested` (the dashboard
   Plans→Run path) carries a portal-minted, plan-scoped MCP token;
   bare `trigger.fired` does not — so any trigger-fired agent session
   that emits is parked by the liveness gate *by design*, and an
   input-less agent node dies on a swallowed CLI usage error. The
   FTUE activation rung also lost its producer in this gap (#594).
3. **Process apparatus sized for the old workspace.** A second
   parallel dev-env system (the slot system, #194) superseded by the
   devproxy/host-run flow; a CI changes-filter matrix built for 16
   crates; boutique lints whose upkeep now exceeds their catch rate;
   a stale `docs/architecture.md` that root CLAUDE.md itself warns
   against trusting.

## Decision

1. **One workflow model.** The user-facing **Workflow** (canonical
   noun) *is* a workflow-library entry: the builder edits the
   substrate DAG directly (a linear chain rendered exactly as today is
   simply a path-shaped DAG), and saving registers a new library
   version keyed by the workflow's own id — no `spec_kind` string
   join, no preset-to-stage-chain expansion, no `translate_stage`.
   The `workflows` row slims to identity + trigger binding +
   lifecycle axes (ADR 0024 readiness/autofire are untouched).
   `workflow_stages` and the gates-jsonb vocabulary retire.
2. **One run path.** Portal — which already owns every fire entry
   point (webhooks, the manual fire endpoint, MCP) — converts every
   fire into a tokenized `plan.run_requested` (single-spec plan for
   plain workflow runs, exactly what `run_spec_plan` does today).
   The engine consumes one event kind; the `trigger.fired` bridge,
   its spec_kind/preset fallback, and the tokenless degraded mode
   retire. `trigger.fired` remains as a diagnostic audit record of
   the fire itself. The run-terminal signal (`plan.run_completed`)
   becomes the FTUE activation source (#594).
3. **Process diet.** The slot system deletes (one dev-env flow:
   worktree devproxy + host-run `just dev`); the CI changes-filter
   matrix collapses (8 crates — run the workspace check always);
   `check-adr-adoption` is kept (it polices this ADR) but
   `check-metaphor-leakage` / `check-detail-page-tabs` /
   `check-deferred-todos` are re-judged against their catch rate;
   `docs/architecture.md` is deleted in favor of CLAUDE.md + ADRs;
   the registry crate sheds its dead 0.1 machinery (evaluators has
   zero external users today).

## Rejected alternatives

- **Keep both models, harden the bridge.** The bridge is the bug: two
  authored forms of one noun guarantee drift, and every feature
  (builder, presets, runs view, FTUE) pays the translation tax twice.
- **Engine-side token minting for `trigger.fired`.** Moves credential
  custody across the seam; portal is the edge and owns credentials —
  converting at portal keeps the key in one process.
- **Delete the legacy model without a builder migration.** The builder
  is the product's main authoring surface; it moves in the same wave
  (pre-launch, no users to migrate — the cheap window again).

## Consequences

- The four-noun vocabulary is *strengthened*: Workflow/Run/Artifact/
  Stage each map to exactly one storage + execution form (Stage = DAG
  node). The "Plans" carve-out is unchanged.
- ~4–6k LOC of live-but-duplicated structure retires across portal,
  dashboard, and engine; the dev DB schema drops two tables
  (pre-launch: no migration choreography).
- The liveness-gate park for trigger-fired emitting agents disappears
  as a failure class, along with the empty-prompt CLI error.
- Dev-process surface shrinks: one dev-env system, fewer CI jobs,
  fewer lints — same hard-fail core (`check-events`,
  `check-api-contract`, `check-generated-types`, `check-file-budget`,
  `check-hitl-coverage`, `check-tools-and-skills`, `lint-seams`).

## Adoption checklist

- [x] Decision recorded; index updated. (PR #600)
- [x] Run-path unification landed (portal converts fires; bridge
      retired; #594 closed by re-pointing FTUE to the run-terminal
      signal). (#601 / PR #607)
- [x] One workflow model landed (builder edits the DAG; stage-chain
      storage retired; e2e: a builder-authored workflow runs with no
      manual library registration). (#602 / PR #608; follow-up fixes
      #613, #614, #616)
- [x] Registry diet landed (dead evaluators/registry machinery gone).
      (#603 / PR #609; the orphaned `adapter::register` boot write
      removed at this flip)
- [x] Spine vocabulary sweep landed (legacy gate protocol types,
      `forge` stream namespace). (#604 / PR #610)
- [x] Dev-env consolidation landed (slot system deleted). (#605 /
      PR #611)
- [x] CI + lint diet 2 landed (filter matrix collapsed; lint set
      re-judged; stale architecture doc removed). (#606 / PR #612)
- [x] Flip `Adoption` to `enforced`; e2e on the unified model. (this
      PR; e2e evidence on #599 — builder-created manual workflow →
      fire → tokenized plan run → live agent session → MCP
      `emit_artifact` → `plan.run_completed`, zero manual seeding)

## Out of scope

- Builder UX beyond parity (DAG *editing* affordances beyond the
  linear chain ship separately if wanted — the human decides the
  shape on the umbrella).
- Script/Verify executor enablement (RUN-01 #359, MIG-01 #363).
- Post-launch concerns (the pre-launch posture still applies).
