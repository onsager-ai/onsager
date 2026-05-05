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

## How to add an ADR

1. Pick the next sequential number.
2. Write `NNNN-short-slug.md` following the existing template:
   Status / Date / Tracking issues / Context / Decision / Rejected alternatives /
   Consequences / **Dev-process counterpart** (per ADR 0002) /
   Adoption checklist / Out of scope.
3. Link it from `CLAUDE.md` and from this index.
4. If the ADR introduces an architectural invariant that should be enforced,
   open a follow-up spec for the lint or contract test that makes it
   machine-checkable (per ADR 0004's no-escape-hatch posture).
