# ADR 0016 — Workflow Library: N isomorphic islands

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: no
- **Tracking issues**: #347 (ADR-01), #351 (SUB-04 Workflow Library)
- **Supersedes**: none
- **Superseded by**: none

## Context

ADR 0015 made the Spec Plan a DAG of `SpecRef`s with a `kind` field.
ADR 0009 made `Workflow` the reusable template. The connection — how
a `kind` resolves to a `Workflow` — was deferred. Without an explicit
shape for that mapping, every spec would either embed its workflow
(violating ADR 0015's no-inline rule) or rely on hidden defaults.

The empirical observation that motivates the chosen shape: in every
real-world Spec Plan, specs of the same kind have *structurally
identical* execution — same nodes, same edges, same provenance flow.
Two `impl` specs share the same shape; two `design` specs share a
different one. The variation is across kinds, not within them. This
is "N isomorphic islands": same-kind specs are internally isomorphic;
across kinds, shapes differ freely.

## Decision

The **Workflow Library** is a flat catalog: one `Workflow` per spec
`kind`. The compiler resolves `kind → Workflow` via library lookup
(no priority lists, no defaults, no fallback chain).

```rust
pub struct WorkflowLibrary {
    workflows: HashMap<String, Workflow>,  // key: spec kind
}

impl WorkflowLibrary {
    pub fn lookup(&self, kind: &str) -> Result<&Workflow, LookupError>;
}
```

**Internal symmetry (intra-island).** Two specs of kind `impl` share
the same `Workflow`. Their Execution Plan subgraphs are identical in
shape — same nodes, same edges, same executor kinds — differing only
in namespace and in the `Artifact`s flowing through.

**Cross-island asymmetry.** Workflows for different kinds may differ
arbitrarily. A `design` workflow may be `research → sketch → review →
final`; an `impl` workflow may be `prep → code → test → release`. The
only common shape is the entry/exit contract (ADR 0015).

**Library is content, not code.** The library is a data structure
loaded at startup (from `crates/onsager-substrate/workflows/*.toml`
or equivalent). Adding a workflow is editing a file, not a compile.

## Rejected alternatives

- **One workflow per spec (no library).** Every SpecRef embeds its
  own node graph. Defeats reuse; conflates ADR 0009's logical and
  physical layers. Authors converge on copy-paste.
- **Multiple workflows per kind, selected at compile time.** Adds a
  selection mechanism (priority, predicate, …) that has no clear
  axis. If two `impl` shapes are needed, that is two kinds, not one
  kind with two workflows.
- **Workflow inheritance / mixins.** "design_with_review extends
  design." Premature OO; v1's flat catalog is simpler and covers all
  current cases. SubWorkflow (ADR 0011) handles composition where
  needed.

## Consequences

### Positive

- **Compiler is trivially deterministic.** `library[spec.kind]` is
  a hashmap lookup. Same input → same output, by construction.
- **Internal-symmetry property is structural.** Same-kind specs
  cannot drift apart; the library is the only source of shape.
- **Adding capability is adding a kind.** A new spec kind = a new
  workflow in the library = a new entry in the kind taxonomy. No
  code change outside the library file.

### Negative

- **Kind taxonomy is the scaling axis.** A library with 50 kinds
  needs governance on what counts as a new kind. For v1 we have ~5
  (design, impl, review, ops, ising-derived); growth is bounded.
- **One workflow per kind is a real constraint.** If two teams want
  different `impl` shapes, they need two kinds (`impl_rust`,
  `impl_ts`) or one kind that branches inside the workflow. The
  constraint is honest about the tradeoff.

### Neutral

- **The "review_wf" diagram pattern** (in the 0.2 design doc — a
  library entry with no current Spec Plan reference) is normal: the
  library may have unused entries.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: skills in `onsager-skills` are
the dev-process workflow library. One skill per recurring task class
(`issue-spec`, `onsager-pre-push`, `onsager-pr-lifecycle`); skills
are content (markdown), not code; adding one is a PR to the sibling
repo. The "internal-symmetry" property maps: two PRs invoking the
same skill should follow the same shape.

## Adoption checklist

- [ ] `WorkflowLibrary` type in `onsager-substrate` (SUB-04, #351).
- [ ] Initial library content (workflows for the existing spec
      kinds) under `crates/onsager-substrate/workflows/`.
- [ ] Library loading at startup; lookup-by-kind used by the
      compiler (SUB-05).
- [ ] Document the kind taxonomy in root `CLAUDE.md` (post-
      adoption).

## Out of scope

- **Per-workspace library overrides.** All workspaces share one
  library in v1. If a workspace needs custom shapes, that is a
  separate ADR.
- **Library versioning.** Pre-launch we ship one version; post-
  launch versioning of workflow shapes is a separate concern.
- **Dynamic library editing at runtime.** Library reload on file
  change is a developer ergonomics concern, not an architectural
  one; addressed by tooling.
