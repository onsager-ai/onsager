# ADR 0003 — Deliverable as the workflow-run output; registry-backed artifact kinds

- **Status**: Accepted
- **Date**: 2026-04-22
- **Tracking issues**: #100 (parent spec) with children #101–#105
- **Supersedes**: none
- **Superseded by**: none

## Context

Onsager's workflow model conflates process and product. Three concrete
conflations motivate this ADR:

1. **`BundleId` name overload.** `crates/onsager-artifact/src/artifact.rs:64`
   defines `BundleId` as a SHA-256 content hash of a single artifact's
   state. The name reads as "workflow output container" but means
   "per-artifact version snapshot." There is no type for a workflow's
   aggregate output.
2. **Typed but still code-coupled kinds in two places.** `Kind` in
   `onsager-artifact` has a small built-in set
   (`Code | Document | PullRequest`) plus `Custom(String)`, so it is
   typed but not actually closed. `WorkflowArtifactKind` in
   `apps/dashboard/src/lib/api.ts:253` is still a hardcoded TS union
   (`'github-issue' | 'github-pr'`). In practice, adding or fully
   supporting a kind still requires synchronized edits across the
   artifact crate, the dashboard, forge, and registry-facing code.
   `onsager-registry` already carries `TypeDefinition`,
   `RegisteredType`, `ArtifactAdapter`, `GateEvaluator`, and seed
   catalogs at `crates/onsager-registry/src/catalog.rs:21-68`, but
   those facts are not the source of truth for the dashboard or forge.
3. **Flow visualization duplication.** The Governed pipeline preset
   renders `Issue → PR → PR → PR → PR` because `ArtifactFlowOverview.tsx`
   maps one pill per stage's input artifact, conflating "where are we in
   the process" with "what artifact is flowing."

ADR 0001 commits us to stigmergic coordination over the spine. The
aggregate output of a workflow run is first-class in that model —
subsystems consume each other's outputs via events — but it has no
typed shape in the code. Adding a `Deployment` or `Session` workflow
today requires the cross-cutting synchronized edit described above.

## Decision

We adopt a two-level model with **process separated from product**, and
push artifact-kind definitions into the already-existing registry.

### Deliverable vs. WorkflowRun

- **`WorkflowRun`** — process, transient metadata: which graph, current
  node, transition history. Lives with the workflow engine.
- **`Deliverable`** — product, durable: an open map keyed by artifact
  kind, carrying lineage. Emitted and updated as spine events; rehydrated
  by replay. Becomes the workflow's typed output and can be consumed as
  input to downstream workflows.

```rust
pub struct Deliverable {
    pub id: DeliverableId,
    pub workflow_run_id: WorkflowRunId,
    pub entries: BTreeMap<KindId, Vec<ArtifactId>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### Registry-backed artifact kinds + merge rules

`TypeDefinition` in `onsager-registry` gains two fields: `intrinsic_schema`
(the artifact's shape) and `merge_rule` (how partial updates from gates
combine). `MergeRule` is a first-class registry concept with four variants:

- `Overwrite` — latest wins (default)
- `MergeByKey` — maps keyed by identity (e.g., `verdicts` by source)
- `Append` — list concatenation
- `DeepMerge` — recursive per-field merge (PR uses this)

Gates emit partial updates; the registry-declared `MergeRule` folds them
into the Deliverable. No per-kind merge code escapes into the artifact
crate.

### Intrinsic vs. referential links

Two kinds of link between artifacts, both already scaffolded in the
`Artifact` struct's `vertical_lineage` / `horizontal_lineage`:

- **Intrinsic** — fields inside a single artifact (e.g., `PR.commits`,
  `PR.checks`, `PR.reviews`, `PR.merged`). Cannot exist without their
  container.
- **Referential** — typed id pointers between peer artifacts (e.g.,
  `PR.closes_issue → Issue.id`). Peers exist independently.

Rule of thumb: if the thing can exist standalone, it's a peer kind with
a referential link; if it cannot, it's an intrinsic field.

### Renames and scope

- `BundleId` → `ArtifactVersionId` (freeing the "Bundle" namespace from
  the single-artifact-version meaning; the new aggregate takes the name
  **Deliverable** rather than reusing "Bundle" to avoid retread).
- Seed-catalog `Spec` kind → `Issue` (GitHub-projection name).
- **MVP is GitHub-only.** Canonical kind names follow GitHub's
  vocabulary (`Issue`, `PR`). Non-GitHub artifact sources are deferred.

### Built-in kinds

Ship in two waves:

- **v1**: `Issue`, `PR` (existing, `Spec` renamed), plus `Deployment`
  and `Session` (new/promoted).
- **v1.1**: `Release`, `Rule`, `Observation`, `Proposal` (outer-loop
  governance kinds — matches ADR 0002's two-loop framing).

## Rejected alternatives

- **LangGraph-style `State`** — a single object fusing process
  scratchpad and product output. Rejected: the vocabulary imports wrong
  connotations (scratchpad, transient) for what is actually a durable,
  signed deliverable. The mechanics (typed channels, reducers, nodes
  emitting partial updates) are kept under native vocabulary.
- **Keeping built-in `Kind` tags code-defined in `onsager-artifact`** —
  doesn't scale to new kinds without synchronized edits across
  subsystems. Moved to registry.
- **Decomposing PR into peer artifacts** (`Commit`, `Check`, `Review`,
  `Merge`) — rejected: these cannot exist without a PR and should be
  intrinsic fields, matching GitHub's actual resource model.
- **Fixed-schema Deliverable with named fields** — rejected: forecloses
  on future kinds without a schema migration. Open map keyed by kind
  keeps the registry as the single place new kinds are registered.
- **Keeping `BundleId` for the new aggregate** — rejected: the crate
  already uses `BundleId` for per-artifact version snapshots; reusing
  the name would overload it. Two concepts, two names.

## Consequences

### Positive

- Adding `Deployment`, `Session`, `Release`, etc. is a registry change,
  not a cross-subsystem refactor. The dashboard's kind picker becomes
  runtime data from `GET /api/workflow/kinds`.
- Flow visualization separates cleanly: the strip shows
  gates (process); a Deliverable panel shows typed artifact state
  (product). The Governed-pipeline duplication disappears by
  construction.
- Lineage stays on existing infrastructure — `vertical_lineage` /
  `horizontal_lineage` on the `Artifact` struct — so referential links
  are queryable end-to-end without new tables.
- `MergeRule` prevents the class of bugs where two gates writing the
  same channel overwrite each other's partial updates (e.g., CI verdict
  clobbered by Synodic verdict under "latest wins").
- Deliverables are chainable: one workflow's Deliverable can trigger
  another by declaring an artifact-kind dependency on it. This is how
  ADR 0002's outer loop lands as code, not just as process.

### Negative / trade-offs

- The `BundleId` → `ArtifactVersionId` rename touches many sites
  (struct fields, DB columns, wire-format prefix `bnd_` → `ver_`). A
  one-release Serde-compat shim keeps the blast radius manageable; the
  detail is in #101.
- Every new artifact kind must now declare an `intrinsic_schema` and a
  `merge_rule`. That's discipline, not burden, but it's real paperwork
  per kind.
- Dashboard gains one API round-trip at boot to fetch registered kinds.
  Poll-on-load for v1; SSE-based invalidation deferred.

### Neutral

- No new tables. `onsager-registry` already carries the scaffolding;
  this ADR extends `TypeDefinition` rather than introducing a parallel
  system.
- ADR 0001's stigmergic invariant is preserved. Deliverable updates are
  spine events; subsystems observe each other's Deliverables via the
  event stream, never via direct calls.

## Dev-process counterpart

Per ADR 0002, every ADR declares the dev-process analog of the decision
it records.

An umbrella GitHub issue is already a Deliverable container: the body's
Plan checklist is the typed record of what has been produced in service
of the umbrella, and sub-issues are artifacts-by-kind attached to it.
#40 and this ADR's own parent #100 are concrete instances — each has a
typed aggregate (sub-issues + labels + linked PRs) that accumulates as
work advances.

The spec label taxonomy (`area:*`, `priority:*`, `feat|fix|refactor`,
`draft|planned|in-progress`) is the dev-process analog of a
registry-backed kind system: labels are the open schema and the
`draft → planned → in-progress → closed` lifecycle is the per-kind
state machine. Adding `area:ising` was a registry add, not a repo
refactor — because labels are already open-schema and runtime.

Process-product isomorphism is intact: what this ADR does for artifact
kinds in code, GitHub labels already do for spec issues in the repo.

## Adoption checklist

Execution lives in the child specs. This ADR is the decision record
they point at. Each checklist item is a PR under #100:

- [ ] #101 — Introduce `Deliverable`; rename `BundleId` →
      `ArtifactVersionId` in `onsager-artifact`.
- [ ] #102 — Extend `TypeDefinition` with `intrinsic_schema` +
      `merge_rule`; dashboard reads kinds from registry; rename
      `Spec` kind → `Issue` in the seed catalog.
- [ ] #103 — Consolidate PR as a rich artifact with intrinsic
      commits/checks/reviews/merged; `closes_issue` as referential
      link.
- [ ] #104 — Split WorkflowRun flow strip from Deliverable panel in
      the dashboard; remove the hardcoded artifact-pill mapping.
- [ ] #105 — Register `Deployment` kind; promote stiglab's `Session`
      to a registered artifact.
- [ ] `CLAUDE.md` gains a one-line summary of this ADR alongside
      0001 and 0002.

## Out of scope

- **Non-GitHub artifact sources** — v1 is GitHub-only; later
  projections (GitLab, Linear, internal systems) get their own ADR if
  and when they land.
- **v1.1 kinds** (`Release`, `Rule`, `Observation`, `Proposal`) — own
  tracking when v1 stabilizes; kinds ship with no schema change under
  this ADR.
- **Conditional edges and subgraphs in `WorkflowRun`** — linear
  workflows only for v1. Branching is a future ADR once real
  forcing-functions (retry loops, CI-failure routing) accumulate.
- **Serde compat policy** for the `BundleId` rename and the `bnd_` →
  `ver_` prefix — lives in #101.
- **Cache invalidation strategy** for the dashboard's registry
  kind-list — lives in #102.
- **Dashboard cache-invalidation for Deliverable state views** — own
  issue once the Deliverable panel lands in #104.
