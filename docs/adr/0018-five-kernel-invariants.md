# ADR 0018 — Five kernel invariants: static validation on workflow load

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: yes
- **Tracking issues**: #347 (ADR-01), #350 (SUB-03)
- **Supersedes**: ADR 0004 (tighten the seams) — partial; the seam
  rule's six-lever mechanical enforcement remains for cross-
  subsystem code-level discipline. For substrate-internal
  correctness, the five kernel invariants below are the new
  hard-fail surface
- **Superseded by**: none

This ADR carries `Identity impact: yes` because it names what the
substrate refuses to load. The invariants are the substrate's
constitutional check; a workflow that violates one is not a
workflow.

## Context

ADR 0010 (provenance), ADR 0011 (SubWorkflow), ADR 0012 (executor
catalog), ADR 0015 (Spec Plan IO contract), and ADR 0017 (Plan
Compiler) each introduce constraints. Stated as prose in the relevant
ADR, those constraints are individually unenforceable. To be load-
bearing, they must be (a) named in one place, (b) static — checked
at compile, not at runtime, (c) hard-fail with offending IDs, (d) the
*only* substrate-internal correctness contract (anything else is
either guidance or executor-internal).

ADR 0004's six levers established this discipline for cross-
subsystem code (`lint-seams`, `check-events`, `check-api-contract`,
`check-file-budget`). The substrate now needs its own.

## Decision

Five invariants, each a function in `crates/onsager-substrate/src/
validate.rs`, all called by `validate_workflow(w) -> Result<(),
Vec<InvariantViolation>>` and run at Execution Plan compile time
(ADR 0017 step 4):

**Invariant 1 — requires_deterministic edges reject Uncertain.** If
an edge has `requires_deterministic: true`, its upstream node's
emitted provenance must be `Deterministic`. Verify executors are the
only nodes allowed to flip the upstream from Uncertain to
Deterministic (ADR 0010).

**Invariant 2 — Uncertain is contagious via emits_provenance.** A
node's emitted provenance is the max-uncertainty of its declared
output provenance and all input provenances. Implementations cannot
silently downgrade Uncertain to Deterministic; only Verify can.

**Invariant 3 — Workflow OutputSpec matches actual provenance.** A
workflow declares the provenance of its outputs (via `OutputSpec`).
The actual provenance flowing on the exit path must equal the
declared one. Catches workflows that promise Deterministic but
contain an unverified Uncertain path.

**Invariant 4 — SubWorkflow workflow_ref resolves.** Every
`SubWorkflow` executor's `workflow_ref` must look up in the
`WorkflowLibrary`. Cycles in the resolution graph are forbidden.
(Covers ADR 0011 cycle detection.)

**Invariant 5 — Single writer per artifact.** No two nodes in an
Execution Plan share an output `ArtifactId`. Each artifact has
exactly one producer.

Violations carry: invariant number, offending node/edge IDs, and a
human-readable message. Multiple violations are collected and
returned together so authors fix them in one pass.

## Rejected alternatives

- **Runtime checks instead of static.** Cheaper to implement,
  catastrophic in practice: a structurally-invalid workflow surfaces
  as a mid-execution failure with partial state. Static is the
  bedrock.
- **More invariants** (e.g. "no orphan nodes," "DAG is connected").
  Reasonable polish, but the five above are *correctness*; everything
  else is *style*. Style checks can land as lints, not invariants.
- **Fewer invariants** (drop the OutputSpec check, drop the single-
  writer check). Each of the five blocks a specific class of bug
  reviewers cannot reliably catch. Removing any of them re-introduces
  the bug class.
- **Soft invariants (warnings instead of errors).** Soft constraints
  drift. Hard-fail is the only signal that survives velocity.

## Consequences

### Positive

- **Substrate refuses to load broken workflows.** Authors fix shapes
  before any node runs. Mid-run "this can't be right" surprises go
  away.
- **Provenance has teeth.** ADR 0010's enum is meaningful because
  invariants 1, 2, 3 enforce it.
- **The compiler has work to do at validate time** (ADR 0017 step 4).
  The pipeline ends with a real check, not a rubber stamp.

### Negative

- **Authoring friction.** A workflow that "almost works" doesn't
  load. For v1 this is the desired loud-failure mode; in practice
  the error messages need to be excellent.
- **Five is the right count, but each adds a maintenance surface.**
  As provenance / SubWorkflow / executor semantics evolve, the
  invariants evolve. Each change is an ADR amendment, not a quiet
  edit.

### Neutral

- **Invariant numbering is permanent.** If we add a 6th invariant,
  it is invariant 6. We do not renumber.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: the named failure modes in
root CLAUDE.md (*claim ≠ reality*, *silent scope reduction*, *theater
coverage*, *narrative-as-state*, *untracked defer*) are the dev-
process kernel invariants. Each names a specific structural defect
that bypasses claim-honesty. The substrate invariants and the
dev-process named failure modes share the same shape: name the
defect class, check at the right moment, fail loudly.

## Adoption checklist

- [ ] `validate_workflow` in `crates/onsager-substrate/src/validate.
      rs` (SUB-03, #350).
- [ ] One check function per invariant.
- [ ] One positive + one negative test per invariant (10 tests
      minimum).
- [ ] Plan Compiler (SUB-05) calls `validate_workflow` as its final
      step.
- [ ] Error messages include offending node/edge IDs.

## Out of scope

- **Cross-Execution-Plan invariants** (consistency across two
  Plans). The substrate validates one Plan at a time; cross-Plan
  consistency is a separate concern (and arguably not desirable —
  Plans are independent units).
- **Performance invariants** (max nodes, max depth). Style /
  resource limits, not correctness. Addressable as lints.
- **Runtime re-validation.** Plans are immutable post-compile; the
  scheduler trusts them. If we ever need editable Plans, re-
  validate after edit is the obvious answer.
