# Refract

Refract is the **Spec Plan author** — given a high-level intent, it
produces a Spec Plan (the artifact tree the factory executes against).
It is no longer a sibling crate inside this monorepo.

## Status

What this monorepo has done so far:

- Deleted `crates/refract/` (this PR — MIG-02).
- Updated the architecture diagrams and seam-rule listings to drop
  Refract from the factory-subsystem set.
- Retained the `refract.*` event variants (`refract.intent_submitted`,
  `refract.decomposed`, `refract.failed`) on the spine and in the
  registry manifest, attributed to `Subsystem::Portal` as the on-spine
  producer.

What is still TODO per [ADR 0014](../adr/0014-onsager-refract-boundary.md)'s
adoption checklist:

- Create the `onsager-ai/onsager-refract` repository.
- Port the deleted `crates/refract/src/` to the new repo.
- Add the `submit_spec_plan` / `update_spec` MCP tools to portal
  (sub-issue derived from #347; references ADR 0009 and ADR 0015 for
  the Spec Plan schema).
- Wire Refract as an MCP client of those tools instead of writing
  directly to the spine.

Until those steps land, the in-tree MCP registry does **not** carry
the Spec Plan submission tools; the boundary described below is the
ADR target, not the operating state.

## Boundary (target per ADR 0014)

Refract talks to Onsager **only** through the portal's public MCP
surface (ADR 0007). It is one Spec Plan producer among several —
humans authoring GitHub issues with the `issue-spec` skill are another;
the dashboard chat driving `submit_spec_plan` is another.

The Onsager monorepo retains:

- The Spec Plan type definitions in `onsager-substrate`.
- The MCP tools Refract will call (`submit_spec_plan`, `update_spec`, …
  — see Status above for landed state).
- The `refract.*` event variants on the spine — re-emitted by the
  sibling repo via the MCP tools once those land.

The Onsager monorepo no longer carries:

- `crates/refract/` (deleted in MIG-02).
- A `refract` dependency on any in-tree crate.

## Why the split

See ADR 0014 for the full rationale. In one sentence: Onsager is the
factory + substrate; Refract is a Spec Plan author. Conflating the two
under one repo (and one lockfile) blurred the seam ADR 0009
established between *what produces Spec Plans* and *what executes
them*.
