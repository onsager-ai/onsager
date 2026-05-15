# ADR 0010 — Provenance as substrate first-class

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: yes
- **Tracking issues**: #347 (ADR-01), #348 (SUB-01), #350 (SUB-03), #356 (EXE-04 Verify)
- **Supersedes**: none
- **Superseded by**: none

This ADR carries `Identity impact: yes` because it introduces a new
kernel-level type that every artifact carries and every node respects.
Provenance is not metadata — it is part of what makes an Onsager
artifact an Onsager artifact.

## Context

The 0.1 substrate has no type-level distinction between "this row was
produced by a script that pinned its inputs" and "this row was produced
by an LLM completing a prompt." Both are `Artifact` rows. Reviewers and
runtime gates infer trustworthiness from context: who emitted it, was
there a test, did a human approve. The inference is unreliable and
unenforceable.

The substrate needs a first-class answer to one question: **given an
artifact, what must be true for a downstream consumer to trust it as
deterministic?** Without that, the "Verify executor" pattern that 0.2
relies on (EXE-04) has no truth to certify.

## Decision

Add two types to `onsager-artifact`:

```rust
pub enum Provenance {
    Deterministic { source: SourceTag },
    Uncertain     { source: SourceTag },
}

pub enum SourceTag {
    Agent,     // LLM / model output
    Script,    // deterministic code (sandboxed)
    External,  // upstream system (GitHub, etc.)
    Human,     // human edit
    Composed,  // derived from multiple parents
}
```

Every `Artifact` carries `provenance: Provenance` and
`produced_by_node: Option<NodeId>` (filled when produced inside a
workflow; `None` for legacy / externally-ingested rows).

**Propagation rule** (the kernel of invariant 2, formalized in
ADR 0018): a node's emitted provenance is the *maximum uncertainty*
of its declared output provenance and all input provenances. Uncertain
is contagious: any Uncertain input poisons the output unless the node
is a Verify executor.

**Verify is the only upgrade path.** An Uncertain artifact can become
Deterministic only by passing through a node whose executor is
`Verify` (EXE-04) — a script-backed check that asserts a downstream-
checkable property (tests pass, schema matches, signature valid).
Verify is a kernel-recognized executor type; no other executor may
upgrade provenance.

**Requires-deterministic edges.** Workflow edges may carry
`requires_deterministic: bool`. If true and the upstream output is
Uncertain, the workflow fails to load (invariant 1).

## Rejected alternatives

- **Free-form trust scores.** A numeric "trust level" between 0 and
  1. Rejected: forces every site to pick a threshold; meaningless
  arithmetic; no clear escalation path. The two-valued enum is the
  decidable shape.
- **Provenance as an out-of-band metadata table.** Same row, separate
  table joined for trust decisions. Rejected: makes the kernel
  invariants joinful and lazy; the substrate needs to validate at
  Execution Plan compile time, not at scheduling time.
- **Per-node "I trust this" override.** A node could mark itself as
  upgrading provenance without being a Verify executor. Rejected:
  silently re-introduces the 0.1 problem (trust by convention). The
  Verify monopoly is the point.

## Consequences

### Positive

- **Kernel invariants are decidable.** Provenance is a closed enum;
  invariants 1 and 2 check at workflow-load time with no I/O.
- **The Verify executor has work to do.** It is the only structurally
  meaningful gate in 0.2; everything else is data flow.
- **Reviewers stop guessing.** The artifact tells you whether it has
  been verified.

### Negative

- **Schema migration.** Existing artifact rows backfill to
  `Deterministic { source: External }` (SUB-01). Pre-launch lets us
  do this without choreography.
- **Every executor declares its emitted provenance.** A small new
  trait method (`declared_provenance(&self, inputs: &[Provenance])
  -> Provenance`). For most executors this is a constant.

### Neutral

- **Spine event shape unchanged.** Provenance is a field on the
  artifact row, not on the event envelope. Events still carry
  `artifact_id`.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: a PR's CI status is the
dev-process Verify. "Tests pass" upgrades the PR's trustworthiness;
"green CI but uncovered new code" is the *theater coverage* failure
mode named in root CLAUDE.md — exactly the substrate's Verify-without-
checking-property defect at the dev-process scale.

## Adoption checklist

- [ ] Add `Provenance` + `SourceTag` to `onsager-artifact` (SUB-01).
- [ ] Add `provenance` and `produced_by_node` columns to the
      `artifacts` table; backfill existing rows.
- [ ] `Executor::declared_provenance(&self, inputs)` on the trait
      (EXE-01).
- [ ] Verify executor (EXE-04) is the sole upgrade path.
- [ ] Static validators (SUB-03, ADR 0018) enforce invariants 1 and 2.

## Out of scope

- **Multi-level provenance** (e.g. "Uncertain-from-agent" vs
  "Uncertain-from-human"). `SourceTag` captures the producer kind but
  the upstream/downstream rules treat all Uncertain alike. Fine-
  grained policies are post-launch.
- **Cross-workspace provenance flow.** An artifact crossing workspace
  boundaries is a separate concern (and currently disallowed).
