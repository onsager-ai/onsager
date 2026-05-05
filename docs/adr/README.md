# Architecture decision records

Onsager's architectural decisions live as numbered ADRs. Each ADR records
the context, the decision, the rejected alternatives, and the consequences
— and (per ADR 0002) the dev-process analog of the decision it records.

For the navigable overview that ties these together with current status,
see [`../architecture.md`](../architecture.md).

## Index

| # | Title | Status |
|---|---|---|
| [0001](0001-event-bus-coordination-model.md) | Event bus is the coordination medium (Option A) | Accepted (2026-04-19) — migration completed via ADR 0004 Lever C |
| [0002](0002-process-product-isomorphism.md) | Process ↔ product isomorphism as design discipline | Accepted (2026-04-19, amended 2026-05-01) |
| [0003](0003-deliverable-and-registry-backed-kinds.md) | Deliverable as the workflow-run output; registry-backed artifact kinds | Accepted (2026-04-22) — v1 partially landed under #100 |
| [0004](0004-tighten-the-seams.md) | Tighten the seams: HTTP at external boundaries, spine for everything internal | Accepted (2026-04-26, amended 2026-04-30) — all six levers landed (2026-04-30) |
| [0005](0005-s5-governance-scales-with-scale.md) | S5 governance scales with scale | Accepted (2026-05-05) — `Identity impact: yes` |

## How to add an ADR

1. Pick the next sequential number.
2. Write `NNNN-short-slug.md` following the existing template:
   Status / Date / **Identity impact** (per ADR 0005) / Tracking issues /
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
in root `CLAUDE.md` ("What makes Onsager Onsager"):

- **`Identity impact: no`** (the common case) — the ADR records a
  decision that operates within the existing identity commitments. No
  extra rationale needed beyond the usual ADR sections.
- **`Identity impact: yes`** — the ADR amends or extends one of the
  four identity commitments, or establishes a new one. The ADR must
  include explicit rationale for which commitment(s) change and why.
  Identity changes are sticky by design and carry a higher review bar.

The flag applies to ADRs from 0005 onward. ADRs 0001–0004 are not
retroactively flagged.
