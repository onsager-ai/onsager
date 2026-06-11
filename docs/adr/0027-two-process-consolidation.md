# ADR 0027 — 0.3 consolidation: two-process factory (portal + engine)

- **Status**: Accepted
- **Date**: 2026-06-11
- **Identity impact**: yes
- **Adoption**: enforced
- **Tracking issues**: #581 (umbrella), #582 (synodic), #583 (stiglab), #584 (ising + observers), #585 (executors), #586 (crate merge), #587 (lint diet)
- **Supersedes**: ADR 0013 (observer as second substrate citizen) — the observer tier retires with the citizen taxonomy; observers return only when a concrete consumer exists. ADR 0009 / 0017 / 0018 (three-layer pipeline, plan compiler, kernel invariants) are explicitly **not** superseded.
- **Superseded by**: none

This ADR carries `Identity impact: yes` because it amends commitment 1 ("Event-bus factory, not service mesh"). The spine stays the runtime coordination medium; what retires is the plurality of factory subsystems coordinating across it. The factory becomes two processes — **portal** (edge) and **engine** (trigger → compile → execute → emit) — and the "substrate citizen" taxonomy (workflow citizen + observer citizen, ADR 0013) collapses to one.

## Context

A 2026-06-11 audit traced the live product path against the workspace. The path that works — trigger fires → scheduler compiles a single-spec Execution Plan → the `agent` executor spawns a Claude Code session → artifact lands on the spine → dashboard renders the run — spans roughly 45k LOC. Around it sits roughly 25k LOC (a quarter of the Rust workspace) that is dormant, duplicated, or speculative:

- **synodic** (11.6k LOC) — nothing emits `forge.gate_requested` post-0.2 (forge retired), so `gate_listener` idles; the Verify executor that would call the gate is unregistered; nothing reads the tables its 5 migrations create.
- **onsager-observers** (3.5k LOC) — zero callers of `ObserverRuntime::new()`; the scheduler never spawns it (OBS-01 #361 unlanded); nothing reads `observer_outputs`. Its own header admits "designed-for-future-use library with no in-tree reverse dep".
- **stiglab** (7.8k LOC, ~half dormant) — its session spawning (`agent/session/process.rs`) is superseded by the scheduler's `ClaudeCliRunner`; its `shaping_listener` consumes events nothing emits. Only live role: `/agent/ws-internal` on loopback behind portal (ADR 0008).
- **ising** — deprecated shell plus manifest rows.
- **SubWorkflow / Human executors** (~1.8k LOC) — structurally complete, unregistered, never exercised by a real run.
- **Event manifest** — 28 of 54 `FactoryEventKind` rows are diagnostic-only.
- **xtask** (8.5k LOC) — larger than scheduler + substrate combined; much of it polices seams between subsystems that are dormant.

The architecture was sized for an organization of many independent teams operating a governed agent fleet. The operating reality is one person plus agents, pre-launch, zero users. Scale of *execution* (more concurrent runs, workflows, repos) comes from the scheduler + Postgres queue, which the slim core already has; it does not require scale of *architecture*. Root CLAUDE.md names pre-launch as precisely the window for "replacing whole subsystems when the original guess was wrong" — this is that move, at its cheapest.

## Decision

Consolidate to a **two-process factory**:

1. **portal** — the edge: HTTP, auth, OAuth, webhooks, credentials, dashboard API, MCP server, and (newly) the agent control-plane WS it already terminates.
2. **engine** — the factory: trigger handling → plan compilation → executor dispatch → event + artifact emission. One crate (`onsager-engine`) merging onsager-substrate, onsager-nodes, onsager-scheduler, onsager-trigger, and onsager-agent-spawn, with module boundaries mirroring today's crates.

The Postgres spine (events, `pg_notify`, shared tables) remains the only coupling between the two — commitment 1's mechanism survives; its subsystem plurality does not.

Deletions (each recoverable from git, each its own spec/PR): synodic (#582), stiglab after folding the WS control plane into portal (#583), ising + onsager-observers (#584), the SubWorkflow and Human executor impls plus orphaned diagnostic manifest rows (#585). The lint apparatus then shrinks to the surviving boundary (#587): `lint-seams` re-scopes to portal/engine, `check-single-impl-traits` and `check-orphan-crates` retire, the high-value lints (`check-events`, `check-api-contract`, `check-generated-types`, `check-file-budget`, `check-hitl-coverage`, `check-tools-and-skills`) stay.

Explicitly **kept**: the three-layer pipeline and plan-compiler generality (ADR 0009/0017 — small, pure, already built, and "Plans" is a live dashboard surface), the five kernel invariants (ADR 0018), provenance as substrate first-class (ADR 0010), the Script and Verify executors (near-term: RUN-01 #359; MIG-01 #363 re-scoped to in-process verify checks), and the four-noun user vocabulary. Verify gating, when it becomes real, is an in-process module of the Verify executor — not a subsystem behind events.

## Rejected alternatives

- **Freeze the dormant subsystems instead of deleting.** Frozen code still carries event contracts, migrations, lint scope, compose entries, and CLAUDE.md surface that every future change must navigate. The carrying cost is the problem; git already provides the archive.
- **Finish wiring them instead (land OBS-01, register Verify against synodic, …).** That doubles down on architecture sized for a scale of organization that doesn't exist, and defers the features that matter (script + verify stages) behind a gate protocol nobody needs yet.
- **Also delete the plan-compiler generality (multi-spec DAGs, invariants).** Its cost was sunk at authoring time; it is pure, statically validated, and has a live dashboard surface. The expensive complexity is subsystems with runtime contracts, not pure compile-time code.
- **Full rewrite into a single binary.** Portal/engine is a real boundary (public surface vs factory execution, different failure and deploy profiles). One process would re-couple edge concerns into the execution loop for no measured win.

## Consequences

- ~20–25k LOC and roughly half the conceptual surface removed with zero loss of working functionality; 15 crates become ~8; the dispatcher CLI likely retires with only one factory binary left (#586 open question).
- Root CLAUDE.md's commitment 1 is amended to the two-process framing (this ADR's PR updates the bullet's references; the Architecture prose updates land with each deletion PR). Commitments 2–4 are untouched.
- ADR 0013 is superseded. ADR 0005's meta-rule (governance scales with scale) is *applied*, not changed: this is governance scaling **down** to match operational scale.
- Script + Verify enablement gets cheaper: no synodic gate protocol to honor, no observer tier to notify.
- Re-introducing a governance or observer tier post-launch is deliberate new work with its own ADR, not a revert.

## Dev-process counterpart

Per ADR 0002: the product move (delete subsystems with no consumer) has the same shape as the process move shipped alongside it (#587 — retire meta-lints whose upkeep exceeds their catch rate). Both are the Occam's-razor checklist applied at subsystem altitude: a wire connected at one end is the same defect whether it's an event with no consumer or a lint with no violations to catch. The process apparatus shrinks in the same wave as the architecture it polices.

## Adoption checklist

- [x] Decision recorded (this ADR), CLAUDE.md commitment 1 amended with the 0027 reference, index + ADR 0013 metadata updated. (PR #588)
- [x] #582 — synodic retired (crate, migrations, manifest rows, dashboard stubs, lint lists). (PR #591)
- [x] #583 — agent control plane folded into portal (in-process `session_runner`); stiglab deleted. (PR #592)
- [x] #584 — ising + onsager-observers deleted; OBS-01/OBS-02 closed. (PR #593)
- [x] #585 — SubWorkflow + Human executors deleted; manifest swept (`node.awaiting_human` kept — live Agent-executor emitter; `stage.advanced` producer gap tracked by #594). (PR #595)
- [x] #586 — onsager-nodes/scheduler/trigger merged into `onsager-engine`; dispatcher CLI deleted; substrate + agent-spawn stay shared below-seam (amendment on the issue — portal depends on both). (PR #596)
- [x] #587 — lint diet landed (`lint-seams` re-scoped to portal/engine; orphan-crates + single-impl-traits retired); CLAUDE.md Occam section updated. (PR #597)
- [x] Flip `Adoption` to `enforced`; one end-to-end run verified on the consolidated stack 2026-06-11 (manual trigger fire → engine compiled the plan → agent executor ran a live Claude session → artifact registered on the spine).

## Out of scope

- **Script executor registration and real Verify checks** — tracked by RUN-01 #359 and re-scoped MIG-01 #363; this ADR only makes them cheaper.
- **Exercising multi-spec DAGs / subworkflow plans** — the generality is kept but proving it out is feature work, not consolidation.
- **Post-launch governance/observer reintroduction** — its own ADR when a concrete consumer exists.
- **The four-noun vocabulary, dashboard IA, and MCP/skills contract** (ADRs 0007, 0019–0026) — untouched.
