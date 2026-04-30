# ADR 0004 — Tighten the seams: HTTP at external boundaries, spine for everything internal

- **Status**: Accepted
- **Date**: 2026-04-26
- **Tracking issues**: #131 (parent spec) with children for each
  lever (A: PR #144 / completed; B–F: opened concurrently with this
  ADR)
- **Supersedes**: none
- **Superseded by**: none

## Context

ADR 0001 committed Onsager to stigmergic coordination over the spine —
no sync HTTP between subsystems, no static linking across them. The
rule is sound. It is also informally stated and informally enforced,
so it drifts. By April 2026 the drift had accumulated in predictable
shapes:

- Two `forge → stiglab/synodic` HTTP RPCs still live in
  `crates/forge/src/cmd/serve.rs` (`HttpStiglabDispatcher` ≈52–304 and
  `HttpSynodicGate` ≈344–397, instantiated in `run` ≈469–490). These
  are the only place in the codebase where the rule is *currently*
  violated; they were left as a known migration debt by ADR 0001 #41.
- A parallel-schema mirror: stiglab's `tenant_workflows` ↔ spine's
  `workflows` (PR #129), with `crates/stiglab/src/server/workflow_spine_mirror.rs`
  as a translator module. The mirror was filed as a bridge with no
  removal date.
- Producer-without-consumer events (PR #127): a `FactoryEventKind`
  variant emitted by one subsystem with no deployed consumer.
- In-memory state caches drifting from the spine (PR #123).
- Half-wired API/UI contracts: backend endpoint shipped without a
  dashboard caller, or vice versa (PR #108).
- Divergent shapes from multiple write paths into the same row (PR #122).
- Compat aliases that ossified (PR #107: `BundleId` → `ArtifactVersionId`
  alias, kept "for one release", still in tree).

The pattern is consistent: the rule is a review-time heuristic, and
review is the wrong place to catch it — by the time a reviewer sees
the bridge, the cost of removing it is higher than the cost of
landing it.

ADR 0002's outer loop argues that durable dev-process patterns
deserve a primitive. This ADR is that primitive for the seam rule.

## Decision

We commit to making the seam rule **explicit, machine-checkable, and
durable across AI sessions**, and to completing the unfinished
structural migrations the rule implies, via six levers landed in
order (A→F). The rule itself, restated:

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> Subsystems (`forge`, `stiglab`, `synodic`, `ising`) coordinate
> **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another
> subsystem. No subsystem imports another subsystem's crate.

The six levers, each landing as a sub-issue under #131:

1. **A — Persist the rule.** Verbatim in root `CLAUDE.md`, in each
   subsystem's `CLAUDE.md`, and restated in the `issue-spec`,
   `onsager-pre-push`, and `onsager-dev-process` skills. Doc-only.
   Landed in PR #144.
2. **B — Mechanical guardrails (no-escape-hatch).** Architecture lint
   (subsystem-A→B Cargo dep, sibling-subsystem HTTP client) and
   bridge-pattern lint (`serde(alias)`, `*_mirror.rs`, legacy type
   alias) hard-fail at CI and pre-push. Producer-without-consumer
   warns from day 1 against the static event registry; hard-fails
   once Lever E lands. **Bridges are not allowed**; cross-subsystem
   migrations land in a single PR. The only exception is database
   schema migrations under `migrations/NNN_*.sql`, which keep their
   existing sequential convention.
3. **C — Complete the ADR 0001 migration.** Delete
   `HttpStiglabDispatcher` and `HttpSynodicGate` from
   `crates/forge/src/cmd/serve.rs`. Replace each with an event
   emit + listener pair: `forge.shaping_dispatched` →
   stiglab listener → `stiglab.session_completed`;
   `forge.gate_requested` → synodic listener → `synodic.gate_verdict`.
   Delete the `onsager-protocol` crate. Removes the only place the
   rule is currently violated.
4. **D — Spine as single source of truth.** Collapse
   `tenant_workflows` / `tenant_workflow_stages` into spine
   `workflows` / `workflow_stages` with a `tenant_id` discriminator.
   Remove `crates/stiglab/src/server/workflow_spine_mirror.rs` in the
   same PR. Removes the `BundleId` → `ArtifactVersionId` alias as part
   of cleanup. Sets the precedent for shared schemas going forward.
5. **E — Registry-backed event types and capability advertisement.**
   Extend `onsager-registry` (per ADR 0003 already the SoT for
   artifact kinds) to event types and subsystem capabilities. Static
   manifest in the registry crate, **checked at CI** — not runtime
   self-report. Each subsystem declares produced/consumed events;
   the registry surfaces "producer with no consumer" so Lever B's
   check can hard-fail.
6. **F — API/UI contract enforcement.** Either OpenAPI emit from
   stiglab/synodic + generated dashboard client, or a CI contract
   test that hits every endpoint the dashboard declares. Either
   approach satisfies the rule. The choice is deferred to the
   Lever F sub-issue.

The execution order — process and enforcement first (A, B), then
structural surgery (C, D), then the registry contract that future
work will hang from (E), then the API/UI contract (F) — is chosen so
that the cheapest-and-soonest items immediately stop new accumulation,
and so that the structural work happens with the guardrails already
in place.

## Rejected alternatives

- **Leave the rule as a review-time heuristic.** Documented as the
  status quo for the last six months; the drift evidence under
  Context demonstrates that this does not scale. Rejected.
- **Allow bridges with `bridge-debt` labels and target removal dates.**
  Considered. Rejected because PR #107's "one release" alias is
  still in tree, and PR #129's mirror module had no removal date at
  all. The label-and-promise pattern relies on the same review-time
  discipline that already failed; the no-escape-hatch posture under
  Lever B replaces it. Database migrations are the only multi-step
  exception, governed separately.
- **Replace the spine with synchronous RPC across subsystems
  (Option B in ADR 0001).** Re-rejected. The naming, ADR 0001, ADR
  0002, and the current dashboard architecture all assume stigmergic
  coordination; reversing that decision is a much larger change than
  finishing the migration to it.
- **Split the registry per-subsystem** (each subsystem owns its event
  catalog). Rejected: defeats the "producer with no consumer" check
  by construction, and creates a new sync point — every subsystem
  has to read every other subsystem's catalog to validate
  cross-subsystem contracts. The static manifest in the central
  registry is one point of authority, and CI is the validator.
- **Defer Lever B until Lever C lands** (no point lint-checking
  something that's about to be deleted). Rejected: B catches *new*
  violations, not the existing one. Landing B before C means the
  Lever C PR is also the last PR that will ever need to add an
  exception, and even that exception is internal to forge's serve.rs
  during the transition (which is one PR by design).

## Consequences

### Positive

- The rule is in two places (CLAUDE.md + skills) where every AI
  session and every human reviewer encounters it before writing
  code. Drift surfaces at draft-and-pre-push time, not at review.
- The two HTTP RPCs from ADR 0001's known-debt list finally go
  away. Forge's tick becomes a pure state machine end-to-end —
  the original ADR 0001 commitment is fulfilled.
- The mirror module pattern (`workflow_spine_mirror.rs`) is removed
  and forbidden going forward. Future shared-schema work uses the
  spine table with a discriminator, set as precedent by Lever D.
- The compat-alias pattern (`BundleId`, `serde(alias)`, "for one
  release" types) is forbidden by CI, not by promise. The renames
  that already failed to land cleanly will land cleanly the next
  time around.
- Half-wired API/UI surfaces (PR #108) become un-mergeable under
  Lever F.

### Negative / trade-offs

- Cross-subsystem migrations get harder. The Lever D PR is, by
  construction, the schema migration *and* the call-site swap *and*
  the mirror-module deletion in one diff. If that exceeds a
  reviewable size, the sub-issue must negotiate the multi-PR plan
  up front — Lever B will not be retroactively waived for a half-
  landed bridge. The trade-off is intentional: the alternative is
  the drift the spec is closing.
- Adding a new cross-subsystem contract now requires a registry
  update (Lever E's manifest) plus producer + consumer in the same
  PR. That's discipline, not burden, but it's real paperwork per
  contract.
- The spine schema accumulates discriminator columns
  (`tenant_id`, etc.) instead of per-subsystem tables. The
  Postgres-side cost is bounded — these are partitioning columns
  with indexes, not new joins — but it does shift schema evolution
  pressure onto the spine crate, which now has to think about
  multi-tenant from the start of every shared concept.
- Lever B's lints have false-positive risk (a legitimate use of
  `serde(alias)` for a wire-format-stable rename outside the
  subsystem-coupling case). The sub-issue handles this by scoping
  the lint to `crates/{forge,stiglab,synodic,ising}/**` — spine and
  registry rename mechanics keep their flexibility.

### Neutral

- No new tables. Lever D collapses two existing tables into the
  spine schema; Lever E extends `onsager-registry`'s existing
  manifest format.
- ADR 0001's runtime invariant (loose coupling, no static linking)
  is preserved end-to-end. This ADR adds *contracts* on the seams,
  not direct calls across them.
- ADR 0003's registry pattern extends naturally — events become a
  registered kind with the same shape as artifacts. Lever E reuses
  the type-definition machinery already in the registry crate.

## Dev-process counterpart

Per ADR 0002, every ADR declares the dev-process analog of the
decision it records.

The decision here *is* a dev-process change as much as it is a code
change. Lever A makes the rule visible at every entry point an AI
session has into the codebase (root `CLAUDE.md`, subsystem
`CLAUDE.md`, the three skills that bracket the inner loop). Lever B
is a process counterpart by construction: a CI-checked rule is a
process artifact, and the `onsager-pre-push` skill restates the same
rule so the local check and the remote check converge.

The product-side analog is the registry (Lever E): the same kind
catalog that ADR 0003 made authoritative for artifacts now becomes
authoritative for events, with the same producer/consumer
contract surface. Process ↔ product isomorphism is preserved: the
review-time pattern ("does this PR have a consumer?") becomes a
registry entry that CI validates.

## Adoption checklist

Execution lives in the child specs of #131. Each lever's checklist
is in its sub-issue. Status as of 2026-04-30:

- [x] **Lever A** — persist the rule in `CLAUDE.md` + skills.
      _Landed: PR #144._
- [x] **Lever B** — mechanical guardrails (no-escape-hatch CI lint).
      _Landed: `xtask/src/lint_seams.rs` enforces arch-deps, sibling-
      subsystem HTTP, `serde(alias)`, `*_mirror.rs`, and legacy type
      aliases; producer-without-consumer is gated on Lever E._
- [x] **Lever C** — complete the ADR 0001 migration; delete
      `onsager-protocol`. _Landed: closed by PR #148. The
      `HttpStiglabDispatcher` / `HttpSynodicGate` RPCs are gone;
      coordination flows through `forge.gate_requested` /
      `synodic.gate_verdict` and `forge.shaping_dispatched` /
      `stiglab.session_completed` (with
      `stiglab.shaping_result_ready` emitted alongside it)._
- [ ] **Lever D** — spine as SoT for workflows; remove
      `workflow_spine_mirror.rs` and the `BundleId` alias.
      _Open: mirror module and alias still in tree._
- [ ] **Lever E** — registry-backed event types + capability
      advertisement (#150). _Open: `lint_seams` carries the producer-
      without-consumer reminder pending the registry manifest._
- [x] **Lever F** — API/UI contract enforcement.
      _Landed: `xtask/src/lint_api_contract.rs` (closed by PR #207)
      asserts every backend route has a dashboard caller (or an
      allowlisted external-only reason) and every dashboard call
      lands on a backend route._

## Out of scope

- **Per-lever implementation detail.** Each lever's sub-issue
  carries its own Design / Plan / Test. This ADR is the umbrella
  decision and execution order.
- **Lever F's enforcement choice** (OpenAPI codegen vs. CI contract
  test). Either satisfies the rule; the sub-issue picks one.
- **Cross-tenant data movement.** Lever D's `tenant_id` discriminator
  is for partitioning shared schemas, not for cross-tenant queries
  or admin tooling. That's a separate concern.
- **Out-of-tree consumers of `onsager-protocol`.** Lever C deletes
  the crate; if any consumer outside this repo emerges, that's a
  bridge-debt issue handled at that point — not pre-allocated here.
- **A retroactive audit of every existing `serde(alias)` and type
  alias in the workspace.** Lever B's lint applies to *new* code;
  pre-existing aliases get triaged as their owning sub-issues land.

## Amendment 2026-04-30 — clause-1 vs clause-2 ownership

Spec #222 (promote `onsager-portal` to a first-class edge subsystem)
surfaced a structural gap in the rule as originally stated: clause 2
(subsystems coordinate via the spine, no sibling-subsystem HTTP) is
machine-checkable and lever-B-enforced, but **clause 1 (the external
HTTP boundary itself) had no concrete owner**. Stiglab carried it by
default — every new external concern (OAuth callback, credential CRUD,
webhook ingest, workflow CRUD, preset registry) landed in stiglab
because stiglab was the only subsystem with a public HTTP surface, and
stiglab consequently grew into a kitchen-sink edge.

The amendment names the owner. **Clause 1 is owned by `portal`.**
Portal becomes a peer of `forge` / `stiglab` / `synodic` / `ising`,
governed by the same seam-rule discipline (clause 2 still applies — it
coordinates with the factory subsystems exclusively via the spine, and
must not import their crates). The distinguishing concern is the
*outer skin*: portal is the only subsystem permitted to host public
HTTP routes that aren't the spine read-API. Future external
integrations (GitLab, Slack, Linear) attach to portal, not to a random
factory subsystem.

The lint-seams check already permits portal HTTP, so no enforcement
change is required. The structural surgery — moving routes, schema, and
the GitHub side-effects of `workflow_activation` — is scoped under
spec #222 and gated on #149 (Lever D, landed) and #161 children A/B
(workspace as first-class scope, landed).
