# Refract

Refract is the **Spec Plan author** — given a high-level intent, it
produces a Spec Plan (the artifact tree the factory executes against).
It is no longer a sibling crate inside this monorepo.

## Where it lives

- Repository: [`onsager-ai/onsager-refract`](https://github.com/onsager-ai/onsager-refract)
- Decision: [ADR 0014 — Onsager–Refract boundary](../adr/0014-onsager-refract-boundary.md)
- Migration spec: [#364](https://github.com/onsager-ai/onsager/issues/364) (MIG-02)

## Boundary

Refract talks to Onsager **only** through the portal's public MCP
surface (ADR 0007). It is one Spec Plan producer among several —
humans authoring GitHub issues with the `issue-spec` skill are another;
the dashboard chat driving `submit_spec_plan` is another.

The Onsager monorepo retains:

- The Spec Plan type definitions in `onsager-substrate`.
- The MCP tools Refract calls (`submit_spec_plan`, `update_spec`, …).
- The `refract.*` event variants on the spine (`refract.intent_submitted`,
  `refract.decomposed`, `refract.failed`) — re-emitted by the sibling
  repo via the MCP tools.

The Onsager monorepo no longer carries:

- `crates/refract/` (deleted in MIG-02).
- A `refract` dependency on any in-tree crate.

## Why the split

See ADR 0014 for the full rationale. In one sentence: Onsager is the
factory + substrate; Refract is a Spec Plan author. Conflating the two
under one repo (and one lockfile) blurred the seam ADR 0009
established between *what produces Spec Plans* and *what executes
them*.
