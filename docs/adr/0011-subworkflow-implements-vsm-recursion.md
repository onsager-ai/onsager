# ADR 0011 — SubWorkflow implements VSM recursion

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: no
- **Tracking issues**: #347 (ADR-01), #349 (SUB-02), #358 (EXE-06 SubWorkflow executor)
- **Supersedes**: none
- **Superseded by**: none

## Context

Stafford Beer's Viable System Model is recursive: every viable system
"contains, and is contained in" other viable systems. ADR 0005 named
S5 governance at the factory scale; the kernel needs the same recursion
expressed in code, not as a review-time conceit.

The 0.1 architecture has no native nesting. A "review" inside a
"feature workflow" is hand-coded ad-hoc as a sequence of nodes, with
no way to substitute one review shape for another or to compose
deeper levels. The result is that reusable sub-flows live as
copy-paste or as ill-defined helpers.

## Decision

Workflow nesting is a property of the executor type, not a separate
construct:

- A node's `executor` may be a `SubWorkflow` executor (EXE-06) that
  carries a `workflow_ref: WorkflowId`.
- At Execution Plan compile time, every `SubWorkflow` node is
  expanded recursively: the referenced workflow is instantiated into
  the parent's namespace, its entry connects to the SubWorkflow
  node's inputs, its exit connects to the SubWorkflow node's
  outputs.
- Workflow IO is **uniform**: a workflow's entry takes inputs as
  `Vec<(ArtifactId, Artifact)>`, its exit produces outputs of the
  same shape. Nesting depth is invisible from the outside — a
  SubWorkflow node has the same IO surface as any other node.

The uniform IO contract is what makes recursion cheap. A nested
workflow is *the same kind of thing* as the outer one; no special-
casing in the scheduler, the compiler, the kernel invariants, or the
spine event shape.

**Provenance flows through naturally.** A SubWorkflow exit's
provenance is the provenance of its terminal node's output, computed
by the same propagation rule (ADR 0010 invariant 2). Verify inside a
SubWorkflow upgrades just as Verify at the outer level does.

## Rejected alternatives

- **Inline-only sub-flows.** A "subroutine" syntax that expands at
  authoring time instead of compile time. Rejected: bloats workflow
  definitions; loses the "library shape" framing of ADR 0016 (one
  workflow per kind, reusable across specs).
- **Separate `Subflow` type distinct from `Workflow`.** A second-
  class abstraction with its own IO contract. Rejected: violates the
  uniform IO principle; introduces special-cases in compiler and
  scheduler; Beer's recursion is unitary, not hierarchical.
- **Runtime expansion (lazy SubWorkflow).** Expand only when the
  SubWorkflow node is scheduled. Rejected: defers the kernel-
  invariant check past compile time; a structurally-invalid
  SubWorkflow would surface as a mid-run failure instead of a
  compile-time error.

## Consequences

### Positive

- **Recursion is free.** No new construct, no new scheduler logic.
  SubWorkflow is an executor like any other.
- **Workflow Library composes.** A `code_review` workflow can be
  referenced from `feature_wf`, `bugfix_wf`, `refactor_wf` without
  duplication.
- **Kernel invariants validate the full tree.** Compile-time
  expansion means invariant checks (ADR 0018) see the fully-resolved
  graph; nested invariants fail loudly, not silently.

### Negative

- **Cycle detection required.** A workflow referencing itself
  (directly or transitively) must be rejected at compile time.
  Invariant 4 (ADR 0018) covers `workflow_ref` resolution; cycle
  detection is part of it.
- **Compile output size grows.** Deeply-nested Execution Plans can
  be larger than their Spec Plan + Library inputs by a multiplier.
  For v1 this is negligible (workflows have small node counts);
  later if it matters, the compiler can emit a compact form.

### Neutral

- **Substrate scheduler is unaware of nesting.** It walks a flat
  Execution Plan; the recursion was already resolved.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: a sub-issue (`Part of #N`) is
the SubWorkflow node in the dev-process. The parent spec's plan
references the sub-issue; the sub-issue has its own plan; closure
propagates upward the same way SubWorkflow exit provenance does.

## Adoption checklist

- [ ] `WorkflowId` newtype in `onsager-substrate` (SUB-02).
- [ ] `SubWorkflow` executor (EXE-06) — registry-backed, expands at
      Execution Plan compile time.
- [ ] Cycle-detection in the compiler (SUB-05) for `workflow_ref`.
- [ ] Invariant 4 (ADR 0018) — `workflow_ref` must resolve.

## Out of scope

- **Mutual recursion across workspaces.** A SubWorkflow's
  `workflow_ref` resolves within the same Workflow Library; cross-
  library references are a separate concern.
- **Dynamic SubWorkflow selection at runtime** (choose which
  workflow to expand based on input). Pre-launch we do not need
  this; the compile-time expansion is sufficient for known patterns.
