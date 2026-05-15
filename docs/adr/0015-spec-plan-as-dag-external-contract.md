# ADR 0015 — Spec Plan as DAG-shaped external contract

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: no
- **Tracking issues**: #347 (ADR-01), #349 (SUB-02)
- **Supersedes**: none
- **Superseded by**: none

## Context

ADR 0009 named the Spec Plan as the external input to the three-layer
pipeline. The shape of that input was deferred. Without a fixed
shape, every Spec Plan author (humans via GitHub issues, Refract,
future MCP clients) would converge on a different convention, and the
compiler would have to accept a moving target.

The pipeline's correctness rests on the Spec Plan being a *DAG with
typed IO at its boundary*. The compiler needs to (a) look up workflows
by spec kind and (b) connect spec-level edges via exit/entry — both
require an external contract, not an implementation detail.

## Decision

A Spec Plan is a DAG:

```rust
pub struct SpecPlan {
    pub specs: Vec<SpecRef>,
    pub deps:  Vec<SpecDep>,
}

pub struct SpecRef {
    pub id:     SpecId,         // stable, externally-assigned
    pub kind:   String,         // looked up against the Workflow Library
    pub inputs: SpecInputs,     // entry-side artifact references
}

pub struct SpecDep {
    pub from: SpecId,           // upstream spec
    pub to:   SpecId,           // downstream spec
}
```

**Unified entry/exit IO.** Every workflow declares an `EntrySpec` and
an `OutputSpec` (typed artifact slots). A Spec Plan dependency
`from → to` connects `from`'s workflow exit to `to`'s workflow entry
— like a function call. The compiler validates types at compile time
(invariants 3 and 5 in ADR 0018).

**DAG, strictly.** Cycles in `deps` are a Spec Plan validation
error. The Workflow Library's internal node graphs can have any DAG
shape they want; the Spec Plan layer is *also* a DAG. Two DAGs
nested via Plan Compiler instantiation produce a DAG.

**Stable spec IDs.** `SpecId` is externally-assigned (GitHub issue
number, Refract-allocated UUID, etc.). The compiler uses it as the
namespace key when instantiating workflows; renumbering breaks
identity.

**No inline node definitions.** A SpecRef does not embed its
workflow; it references one by kind. This is what makes the "N
isomorphic islands" property of ADR 0016 expressible.

## Rejected alternatives

- **Spec Plan as a freeform tree of "tasks."** No typed IO, no
  kind-based dispatch, no compile-time validation. Becomes a JIRA
  clone, not a substrate input.
- **Spec Plan with inline workflows.** Each SpecRef embeds its own
  node graph. Defeats the Workflow Library's reuse story (ADR 0016);
  authors converge on copy-paste.
- **Multiple entry/exit slots per workflow** (variadic IO). Premature
  generalization; v1 picks the simplest shape (one entry, one exit)
  that works for the cases we have. Extending later is cheap.

## Consequences

### Positive

- **Two-layer DAG composes.** Spec Plan DAG × Workflow DAG = a flat
  Execution Plan DAG. Plan Compiler's job (ADR 0017) is small.
- **Authoring is bounded.** Humans/agents author the *outer*
  skeleton (5–20 specs); they do not author node graphs (the
  library does).
- **Refract has a clean output target** (ADR 0014). Its job is to
  emit a Spec Plan; the workflows are not its concern.

### Negative

- **Workflow Library must cover the kinds Spec Plans use.** A
  SpecRef with an unknown kind fails to compile. For v1 this is the
  desired loud-failure mode; later we can add a "no-op" fallback if
  warranted.
- **Cross-spec IO type-matching adds compile complexity.** The
  compiler validates that `from.exit_type` matches `to.entry_type`.
  Mismatches surface as compile errors.

### Neutral

- **GitHub issues are the canonical SpecRef surface today.** The
  `issue-spec` skill in `onsager-skills` produces a GitHub-issue
  Spec Plan. Other authors (Refract, MCP `submit_spec_plan`) emit
  the same shape.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: a `Part of #N` / `Closes #N`
reference in a PR description is the dev-process SpecDep. The bot
that enforces "every PR links a spec issue" (`pr-spec-sync.yml`) is
the dev-process compiler check.

## Adoption checklist

- [x] `SpecPlan`, `SpecRef`, `SpecDep` types in `onsager-substrate`
      (`crate::spec_plan`). Originally scoped to SUB-02 (#349); the
      types landed alongside the Plan Compiler in SUB-05 (#352)
      because the compiler is their first consumer.
- [x] DAG / cycle validation on Spec Plan load —
      `SpecPlan::validate` reports duplicate ids, dangling refs,
      and cycles before any library lookup.
- [x] Unified `EntrySpec` / `OutputSpec` on `Workflow` — `EntrySpec`
      added in SUB-05 (#352) alongside the existing `OutputSpec`
      from SUB-02 (#349).
- [x] Structural IO check at compile time — `NoExit` / `NoEntry`
      errors when a spec dep targets a workflow slot that doesn't
      exist. Artifact-level type matching deferred until the kind
      taxonomy stabilizes.
- [ ] MCP tool `submit_spec_plan` derives schema from `SpecPlan` via
      schemars (per ADR 0007 SSOT principle).

## Out of scope

- **Variadic Spec Plan IO** (multiple entries, multiple exits per
  spec). v1 is single-entry / single-exit. Extending later is a
  type-system widening, not a redesign.
- **Spec Plan mutation mid-run.** A running Spec Plan is immutable;
  edits produce a new compile.
