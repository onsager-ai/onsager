# Architecture decision records

Onsager's architectural decisions live as numbered ADRs. Each ADR records
the context, the decision, the rejected alternatives, and the consequences
— and (per ADR 0002) the dev-process analog of the decision it records.

For the navigable overview that ties these together with current status,
see [`../architecture.md`](../architecture.md).

## Index

| # | Title | Status |
|---|---|---|
| [0001](0001-event-bus-coordination-model.md) | Event bus is the coordination medium (Option A) | Superseded (2026-05-15) by ADR 0009 + ADR 0017 — migration completed via ADR 0004 Lever C |
| [0002](0002-process-product-isomorphism.md) | Process ↔ product isomorphism as design discipline | Superseded (2026-05-15) by ADR 0013 — framing kept as design principle |
| [0003](0003-deliverable-and-registry-backed-kinds.md) | Deliverable as the workflow-run output; registry-backed artifact kinds | Accepted (2026-04-22) — v1 partially landed under #100 |
| [0004](0004-tighten-the-seams.md) | Tighten the seams: HTTP at external boundaries, spine for everything internal | Superseded (2026-05-15) by ADR 0018 — code-level seam levers remain in force |
| [0005](0005-s5-governance-scales-with-scale.md) | S5 governance scales with scale | Accepted (partial-keep 2026-05-15) — `Identity impact: yes` |
| [0006](0006-edge-dispatcher-as-the-public-boundary.md) | Edge dispatcher as the public boundary | Accepted (2026-05-11, originally proposed 2026-05-09; amended 2026-05-09 by ADR 0008) — `Identity impact: no` |
| [0007](0007-tools-and-skills-as-the-public-contract.md) | Tools and skills as the public contract | Accepted (2026-05-11, originally proposed 2026-05-09) — `Identity impact: no` — MCP backend slice landed |
| [0008](0008-portal-owns-the-agent-control-plane.md) | Portal owns the agent control plane | Accepted (2026-05-11, originally proposed 2026-05-09) — `Identity impact: no` |
| [0009](0009-three-layer-pipeline.md) | Three-layer pipeline: Spec Plan + Workflow + Execution Plan | Accepted (2026-05-15) — `Identity impact: yes` |
| [0010](0010-provenance-as-substrate-first-class.md) | Provenance as substrate first-class | Accepted (2026-05-15) — `Identity impact: yes` |
| [0011](0011-subworkflow-implements-vsm-recursion.md) | SubWorkflow implements VSM recursion | Accepted (2026-05-15) — `Identity impact: no` |
| [0012](0012-executor-catalog-replaces-nodekind.md) | Executor catalog replaces NodeKind | Accepted (2026-05-15) — `Identity impact: no` |
| [0013](0013-observer-as-second-substrate-citizen.md) | Observer as second substrate citizen | Superseded (2026-06-11) by ADR 0027 — `Identity impact: yes` |
| [0014](0014-onsager-refract-boundary.md) | Onsager–Refract boundary: Refract leaves the monorepo | Accepted (2026-05-15) — `Identity impact: no` |
| [0015](0015-spec-plan-as-dag-external-contract.md) | Spec Plan as DAG-shaped external contract | Accepted (2026-05-15) — `Identity impact: no` |
| [0016](0016-workflow-library-n-isomorphic-islands.md) | Workflow Library: N isomorphic islands | Accepted (2026-05-15) — `Identity impact: no` |
| [0017](0017-plan-compiler-three-step-algorithm.md) | Plan Compiler: three-step algorithm | Accepted (2026-05-15) — `Identity impact: yes` |
| [0018](0018-five-kernel-invariants.md) | Five kernel invariants: static validation on workflow load | Accepted (2026-05-15) — `Identity impact: yes` |
| [0019](0019-dashboard-ia-pipeline-centric-three-tab-surface.md) | Dashboard IA: pipeline-centric three-tab surface | Accepted (2026-05-22) — `Identity impact: no` |
| [0020](0020-chat-as-portable-global-component-with-contextual-auto-scope.md) | Chat as portable global component with contextual auto-scope | Accepted (2026-05-22) — `Identity impact: no` |
| [0021](0021-detail-page-ia-node-as-section.md) | Detail page IA: node-as-section, attribute-as-expanded-detail | Accepted (2026-05-28) — `Identity impact: no` |
| [0022](0022-provenance-source-tag-first-class-ui.md) | Provenance source tag as first-class UI | Proposed (2026-05-27) — `Identity impact: no` |
| [0023](0023-spec-plan-run-orchestration-and-dashboard-placement.md) | Spec-plan run orchestration and dashboard placement | Accepted (2026-05-29) — `Identity impact: no` |
| [0024](0024-workflow-lifecycle-orthogonal-readiness-autofire-axes.md) | Workflow lifecycle: orthogonal readiness/autofire axes + trigger-neutral binding | Accepted (2026-05-29, adoption ongoing) — `Identity impact: no` |
| [0025](0025-chat-as-design-and-maintenance-surface.md) | Chat as design + maintenance surface | Accepted (2026-05-29, adoption ongoing) — `Identity impact: no` — amends ADR 0020 |
| [0026](0026-nav-ia-sidebar-and-mobile-drawer.md) | Nav IA: desktop sidebar + mobile drawer | Accepted (2026-05-31) — `Identity impact: no` — supersedes ADR 0019's top-bar chrome |
| [0027](0027-two-process-consolidation.md) | 0.3 consolidation: two-process factory (portal + engine) | Accepted (2026-06-11, **enforced**) — `Identity impact: yes` — supersedes ADR 0013 |
| [0028](0028-one-workflow-model-one-run-path.md) | 0.4 simplification: one workflow model, one run path | Accepted (2026-06-11, enforced 2026-06-12) — `Identity impact: no` |
| [0029](0029-one-process-progressive-rebuild.md) | 0.5 reset: one factory process, events as record, progressive rebuild | Accepted (2026-06-12, enforced same day) — `Identity impact: yes` |
| [0030](0030-scaling-sessions-across-machines.md) | Scaling agent sessions across machines | Accepted (2026-06-15, building — owner override of the defer; M1–M3 landed) — `Identity impact: yes` — amends ADR 0029's one-process commitment |

## How to add an ADR

1. Pick the next sequential number.
2. Write `NNNN-short-slug.md` following the existing template. Header
   metadata block (in this order):
   - `Status` — `Accepted` / `Proposed` / `Superseded`.
   - `Date` — ISO date.
   - `Identity impact` — `yes` or `no` (per ADR 0005; required for
     ADRs from 0005 onward).
   - `Tracking issues` — links to the spec and any sub-issues.
   - `Supersedes` — prior ADR number, or `none`.
   - `Superseded by` — `none` at creation; updated if a later ADR
     replaces this one.

   Body sections (in this order):
   Context / Decision / Rejected alternatives / Consequences /
   **Dev-process counterpart** (per ADR 0002) / Adoption checklist /
   Out of scope.
3. Link it from `CLAUDE.md` and from this index.
4. If the ADR introduces an architectural invariant that should be enforced,
   open a follow-up spec for the lint or contract test that makes it
   machine-checkable (per ADR 0004's no-escape-hatch posture).

## The `Identity impact` field

Per [ADR 0005](0005-s5-governance-scales-with-scale.md), every new ADR
declares whether it changes any of the four identity commitments named
in root `CLAUDE.md` ("What makes Onsager Onsager"). The value is exactly
`yes` or `no` — keep it mechanically scannable. Any rationale belongs
in Context / Decision (or a one-sentence note immediately below the
metadata block), not inside the field value.

- **`Identity impact: no`** (the common case) — the ADR records a
  decision that operates within the existing identity commitments. No
  extra rationale needed beyond the usual ADR sections.
- **`Identity impact: yes`** — the ADR amends or extends one of the
  four identity commitments, or establishes a new one. The ADR must
  include explicit rationale for which commitment(s) change and why.
  Identity changes are sticky by design and carry a higher review bar.

The flag applies to ADRs from 0005 onward. ADRs 0001–0004 are not
retroactively flagged.
