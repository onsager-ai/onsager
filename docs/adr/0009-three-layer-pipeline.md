# ADR 0009 — Three-layer pipeline: Spec Plan, Workflow, Execution Plan

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: yes
- **Tracking issues**: #347 (ADR-01), #352 (SUB-05 Plan Compiler), #359 (RUN-01)
- **Supersedes**: ADR 0001 (event bus is the coordination medium) —
  partial, jointly with ADR 0017. The spine remains the runtime
  medium, but the *coordination shape* is now the three-layer
  pipeline below, not direct stigmergic emission from feature-specific
  subsystems. ADR 0009 names the layers; ADR 0017 names the algorithm
  that compiles between them.
- **Superseded by**: none

This ADR carries `Identity impact: yes` because it reframes commitment 1
(event-bus factory): the spine is still the medium but it is no longer
authored directly by feature subsystems. Authorship now flows from
Spec Plan → Workflow → Execution Plan → substrate scheduler, and only
the scheduler emits on the spine.

## Context

The 0.1 architecture coordinates through the spine, with feature
subsystems (forge, stiglab, synodic, ising, refract) each emitting and
consuming events directly. ADR 0001 named this the right substrate.
ADR 0004's six levers mechanically enforced *how* subsystems talk
across the spine. Neither addressed *what they are computing*.

By spring 2026, three patterns kept reappearing:

- Workflow definitions and their runtime instances were mixed in one
  table (`workspace_workflows`, then spine `workflows`). Editing a
  template silently changed in-flight runs.
- Cross-spec coordination (one spec's output gating another) had no
  first-class shape; it was hand-written ad-hoc per feature.
- The path from "user intent" to "what the substrate will execute"
  had no named intermediate forms, so reasoning about determinism,
  validation, and reuse happened at whatever layer the author chose
  that day.

Database query compilers solved this decades ago: separate logical
plan, physical plan, and the compiler that maps between them. Onsager
needs the same separation.

## Decision

Authorship splits into three named layers, isomorphic to query
compilation:

| Layer | Role | Analog |
|---|---|---|
| **Spec Plan** | The external contract. A DAG of specs (`SpecRef`) and dependencies (`SpecDep`). One per project. Authored by humans / Refract. | Logical plan |
| **Workflow** | A reusable template — node graph + IO contract — for a single spec `kind`. Many specs share one workflow. Authored once per kind, library-resident. | Template |
| **Execution Plan** | An immutable, fully-resolved node graph produced by compiling a Spec Plan against the Workflow Library. The scheduler runs this. | Physical plan |

The compiler (ADR 0017) is the only thing that produces an Execution
Plan from a Spec Plan + Workflow Library. The substrate scheduler is
the only thing that executes an Execution Plan. Both are
deterministic; neither involves an LLM.

The spine remains the runtime medium for everything the scheduler
emits (artifact state, node state, observer outputs). Subsystem-direct
emission of business events disappears as feature subsystems are
deprecated (MIG-01..03).

## Rejected alternatives

- **One layer (workflow-only).** Conflates template and instance.
  Edits to the template mutate in-flight runs; reuse is impossible.
  This is the 0.1 state we are exiting.
- **Two layers (Spec Plan + Execution Plan, no Workflow Library).**
  Each spec embeds its own node graph inline. Same shape repeats
  across N specs of the same kind; the "N isomorphic islands"
  observation (ADR 0016) becomes impossible to express.
- **Four layers (add a "Resolved Workflow" between Workflow and
  Execution Plan).** No use case demands it; the compiler's three
  steps (lookup → instantiate → connect) are simple enough that an
  intermediate form would just add ceremony.

## Consequences

### Positive

- **Templates are reusable.** Two `impl` specs share one workflow
  shape; editing it once changes both, future runs only.
- **Determinism is bounded.** The compiler is pure; the scheduler is
  deterministic given an Execution Plan. LLM uncertainty lives inside
  nodes (executor outputs), not in the pipeline shape.
- **Validation has a home.** Kernel invariants (ADR 0018) run when
  the Execution Plan is produced, before any node executes.

### Negative

- **Three new types in the public vocabulary.** `SpecPlan`,
  `Workflow`, `ExecutionPlan`. The dashboard, MCP tools, and skills
  all learn the distinction.
- **One more compile step.** Spec Plan edits do not immediately reach
  the scheduler — they go through the compiler first. For v1 this is
  always cheap (the compiler runs in milliseconds).

### Neutral

- **Spine remains the runtime medium.** ADR 0001's stigmergic
  coordination model still holds for what the scheduler emits.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: the issue → spec → PR loop
maps directly. The GitHub issue is the SpecRef; the `kind` label
selects the workflow; the PR is one execution. CI's spec-vs-trivial
gate is the kernel invariant check at the dev-process scale.

## Adoption checklist

- [ ] Land `onsager-substrate` crate types: `SpecPlan`, `Workflow`,
      `ExecutionPlan` (SUB-02).
- [ ] Land the Plan Compiler (SUB-05, #352).
- [ ] Update dashboard vocabulary to surface Spec Plan / Execution
      Plan distinction (post-MIG).
- [ ] Migration MIG-01..03 retire forge/stiglab/synodic/ising/refract
      crates; the three-layer pipeline replaces their coordination
      shape.

## Out of scope

- **LLM in the compiler.** The compiler is deterministic by design;
  intent decomposition that requires an LLM happens upstream (Refract
  produces a Spec Plan, then exits — see ADR 0014).
- **Dynamic re-compilation mid-run.** v1 compiles once per Spec Plan
  edit; mid-run editing is not supported.
