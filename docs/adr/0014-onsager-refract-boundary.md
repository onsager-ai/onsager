# ADR 0014 — Onsager–Refract boundary: Refract leaves the monorepo

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: no
- **Tracking issues**: #347 (ADR-01), MIG follow-ups
- **Supersedes**: none
- **Superseded by**: none

## Context

Refract (0.1) was the intent decomposer — given a high-level intent
("migrate all legacy auth callers"), it produced an artifact tree
(one artifact per call site). It lived alongside the factory
subsystems in `crates/refract/`.

The 0.2 substrate makes the boundary between *what the factory
executes* and *what produces the things-to-execute* explicit. The
three-layer pipeline (ADR 0009) terminates at the Spec Plan: the
factory's input is a Spec Plan, however it was produced. Refract is
one producer of Spec Plans. Humans authoring GitHub issues are
another. Dashboards driving the MCP `create_spec` tool are another.

Keeping Refract inside the monorepo:

- conflates "factory that executes Spec Plans" with "tool that
  produces Spec Plans," blurring the seam ADR 0009 established;
- forces Refract's LLM-heavy dependencies into the same lockfile as
  the deterministic substrate;
- makes the substrate's identity (deterministic, kernel-invariant-
  checked) harder to read because a non-deterministic producer
  ships under the same `onsager-ai/onsager` umbrella.

## Decision

**Refract moves to its own repository** (`onsager-ai/onsager-refract`,
sibling to `onsager-ai/onsager-skills`). Its public interface to
Onsager is the Spec Plan format (ADR 0015) and the MCP tools (ADR
0007) it calls to submit them.

The Onsager monorepo retains:

- The Spec Plan type definitions in `onsager-substrate`.
- The MCP tools Refract calls (`submit_spec_plan`, `update_spec`,
  etc.).
- ADR 0014 and a `docs/related-work/refract.md` pointer.

The Onsager monorepo loses:

- `crates/refract/` (MIG-02 deletes it).
- Refract-specific event variants (reassigned, deprecated, or
  re-emitted by the sibling repo via the MCP tools).
- Refract-specific dashboard surfaces (replaced by the generic Spec
  Plan editor surfaced through the MCP tools).

## Rejected alternatives

- **Keep Refract in-tree, behind a feature flag.** Pre-launch we do
  not need flags (root CLAUDE.md operating posture); and the flag
  would not address the identity conflation.
- **Move Refract to a sub-crate of an external workspace.** Same
  lockfile, same shared CI. Defeats the purpose.
- **Delete Refract entirely.** Refract is research-valuable and has
  working code; the right move is repo separation, not deletion.

## Consequences

### Positive

- **Onsager identity sharpens.** Onsager is the factory + the
  substrate. Refract is a Spec Plan author, one among possibly many.
- **Refract evolves on its own clock.** LLM-heavy dependencies and
  model upgrades land without rebuilding the deterministic
  substrate.
- **Public MCP surface gets a real external client.** Refract is
  the first thing that submits Spec Plans via MCP rather than
  in-process; that pressure-tests the boundary.

### Negative

- **Two repos to coordinate.** A schema change to Spec Plan requires
  rolling the MCP tool first, then the Refract client. Versioning
  the tool schemas (ADR 0007 schemars-derived) is the mitigation.
- **Existing Refract code moves.** A one-time port. Pre-launch the
  cost is bounded (no users, no deployments to migrate).

### Neutral

- **Spec Plan format unchanged in concept** (ADR 0015 defines it).
  Refract's output shape is the same; only its location changes.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: the `issue-spec` skill is the
human-side Refract. Humans (with the skill's help) decompose a vague
goal into a GitHub issue — that issue is the Spec Plan node. Moving
Refract out is structurally identical to keeping the `issue-spec`
skill in `onsager-skills` rather than in `crates/`.

## Adoption checklist

- [ ] Create `onsager-ai/onsager-refract` repository.
- [ ] Port `crates/refract/src/` to the new repo.
- [ ] Add `submit_spec_plan` MCP tool to portal (#347-derived
      follow-up, references ADR 0009 and ADR 0015).
- [ ] Refract becomes an MCP client of `submit_spec_plan` instead of
      writing directly to the spine.
- [ ] MIG-02 deletes `crates/refract/` from this monorepo.
- [ ] `docs/related-work/refract.md` points to the new repo.

## Out of scope

- **Other potential repo splits** (e.g. moving the dashboard out).
  Refract is uniquely positioned because of its LLM dependencies +
  Spec-Plan-producer role; the dashboard is the factory's primary
  surface and stays.
- **Cross-repo CI orchestration.** The Spec Plan schema is the
  contract; CI in each repo enforces its own side.
