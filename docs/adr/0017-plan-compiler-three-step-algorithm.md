# ADR 0017 — Plan Compiler: three-step algorithm

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: yes
- **Tracking issues**: #347 (ADR-01), #352 (SUB-05)
- **Supersedes**: ADR 0001 (event bus is the coordination medium) —
  partial; the bus remains, but the compiler now produces the
  Execution Plan that drives every emission, replacing direct
  authoring of coordination by feature subsystems
- **Superseded by**: none

This ADR carries `Identity impact: yes` because it names *how* the
factory turns intent into work. The compiler is the deterministic
choke point — no LLM, no review-time discretion — that the rest of
the substrate trusts.

## Context

ADR 0009 named the three layers; ADR 0015 named the Spec Plan shape;
ADR 0016 named the Workflow Library. The remaining decision is the
algorithm that maps Spec Plan + Workflow Library → Execution Plan.

A good algorithm here has to be (a) deterministic — same input always
produces byte-identical output, so debugging is reproducible; (b)
LLM-free — the compiler is the substrate's bedrock, and bedrock can't
be probabilistic; (c) small enough to read in one sitting — anything
larger erodes both audits above.

## Decision

The compiler runs **three steps** plus a final validation:

```text
compile(spec_plan, workflow_library) -> Result<ExecutionPlan, CompileError>:
  plan := empty ExecutionPlan
  for each spec in spec_plan.specs:                             # ① lookup
    workflow := workflow_library.lookup(spec.kind)?
    plan.add_subgraph(workflow.instantiate(spec), spec.id)      # ② instantiate
  for each dep in spec_plan.deps:                               # ③ connect
    plan.connect(plan[dep.from].exit, plan[dep.to].entry)
  return validate_workflow(plan)                                # ④ validate (ADR 0018)
```

**Step 1 — lookup.** `WorkflowLibrary::lookup(kind)` returns the
template for this kind. Missing kind is a hard error (no fallback).

**Step 2 — instantiate.** `Workflow::instantiate(spec)` produces a
copy of the workflow's node graph with fresh node/edge UUIDs, scoped
under the spec's namespace (so two specs of the same kind do not
collide). Each node carries its executor kind and declared
provenance.

**Step 3 — connect.** For each spec-level dependency `from → to`,
wire `plan[from].exit` to `plan[to].entry`. Type-check the IO match
at connect time.

**Step 4 — validate.** Run the kernel invariants (ADR 0018) over the
fully-resolved Execution Plan. Any failure surfaces with offending
node/edge IDs.

The algorithm is **stateless** (no globals), **pure** (no I/O), and
**deterministic** (same inputs → same outputs, byte-for-byte after
canonical serialization).

## Rejected alternatives

- **LLM-assisted compile.** An LLM resolves ambiguous kind matches
  or fills in missing workflows. Rejected: makes the substrate
  bedrock probabilistic; debugging becomes "did the model agree with
  itself today."
- **Incremental compile (delta on Spec Plan edit).** Premature
  optimization; the compiler runs in milliseconds. If it ever
  matters, the algorithm is small enough to revisit.
- **More steps (separate "elaborate," "lower," "optimize" passes).**
  Three steps cover the needed work. Extra passes are infrastructure
  for problems we do not have.

## Consequences

### Positive

- **Algorithm fits on one screen.** The whole compiler is ~50 LOC of
  Rust; reviewers can audit it in one sitting.
- **Determinism is a property test.** `compile(same_input)` called
  twice produces byte-identical Execution Plans. That is one test
  case, not a wall of fixtures.
- **All non-determinism lives inside executors.** Agent output,
  human input, external API calls — those are bounded at the
  executor boundary. The pipeline shape is invariant.

### Negative

- **Hard errors on missing kinds.** A SpecRef with an unrecognized
  kind blocks compile. Pre-launch this is the right tradeoff (loud
  failure); post-launch could revisit if surface error rates demand
  it.
- **No mid-compile heuristics.** A reviewer wishing the compiler
  would "try harder" to make a Spec Plan work is wishing for a
  different system. The compiler refuses to interpret intent.

### Neutral

- **`Workflow::instantiate` is the namespacing primitive.** Fresh
  UUIDs scoped to `spec.id`. Existing UUID allocation is reused.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: the pre-push lint suite
(`xtask check-events`, `xtask lint-seams`, `xtask check-api-
contract`, `xtask check-file-budget`) is the dev-process compiler.
Same shape: lookup (rule catalog), instantiate (apply to the diff),
connect (cross-rule consistency), validate (hard-fail on any
violation). Pre-launch ratchets are the dev-process equivalent of
moving the kind taxonomy.

## Adoption checklist

- [ ] `compile` function in `crates/onsager-substrate/src/compiler.
      rs` (SUB-05, #352).
- [ ] `Workflow::instantiate(spec)` — fresh UUIDs, spec-scoped
      namespace.
- [ ] `ExecutionPlan::connect` — exit/entry wiring with type-check.
- [ ] Determinism property test (compile twice → identical output).
- [ ] Compile errors carry offending IDs.

## Out of scope

- **Compile caching across runs.** v1 recompiles on every Spec Plan
  edit. Caching is a follow-up if profiling shows it matters.
- **Cross-Spec-Plan compile** (linking two Spec Plans). A single
  Spec Plan compiles to a single Execution Plan; cross-Plan
  references are a separate concern.
