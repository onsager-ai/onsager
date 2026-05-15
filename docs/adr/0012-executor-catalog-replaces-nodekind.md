# ADR 0012 — Executor catalog replaces NodeKind

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: no
- **Tracking issues**: #347 (ADR-01), #353 (EXE-01), #354–#358 (EXE-02..06)
- **Supersedes**: none
- **Superseded by**: none

## Context

The 0.1 substrate models node behavior with `NodeKind`, a closed
enum (`Script`, `Agent`, `Gate`, …). Every new node type required:

- a variant on `NodeKind` in `onsager-artifact`,
- a match arm in every consumer (forge tick loop, dashboard renderer,
  serializer),
- a migration to extend the database enum.

The result was that adding a node type touched 6–10 files and shipped
across several PRs. The closed-enum constraint also leaked into the
factory's identity: feature subsystems were partly defined by which
NodeKind variants they owned.

## Decision

Remove `NodeKind`. Nodes carry an `executor: Box<dyn Executor>` where
`Executor` is a trait:

```rust
#[async_trait]
pub trait Executor: Send + Sync {
    fn executor_kind(&self) -> &'static str;
    fn declared_provenance(&self, inputs: &[Provenance]) -> Provenance;
    async fn execute(&self, ctx: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError>;
}
```

A flat `onsager-nodes` crate hosts executor implementations
side-by-side: `script.rs`, `agent.rs`, `verify.rs`, `subworkflow.rs`,
`human.rs`, etc. The `ExecutorRegistry` is the catalog — populated at
startup, looked up by string kind.

**Verify is special-cased at the kernel level** (ADR 0010): it is the
only executor allowed to upgrade provenance. The kernel checks this
via the trait's `executor_kind()` string, not via the enum-style
match the old NodeKind enforced.

**SubWorkflow is an executor** (ADR 0011), not a separate construct.

## Rejected alternatives

- **Keep NodeKind, add `Custom(String)` escape hatch.** Worst of
  both worlds: the enum still exists, every match arm still has to
  handle `Custom`, and the discoverability of variants drops.
- **Per-subsystem executor crates.** `forge-executors`, `stiglab-
  executors`, etc. Rejected: re-introduces the subsystem-as-identity
  drift the 0.2 refoundation is removing. Executors are a flat
  capability, not a subsystem boundary.
- **Plug-in style (dynamic libraries).** Premature; we have no
  external authors yet. Static registration at startup is enough.

## Consequences

### Positive

- **Adding an executor is one file.** Implement the trait, register
  it. No schema migration, no enum match arm to update.
- **Subsystems lose a defining responsibility.** With NodeKind gone,
  the residual feature-subsystem crates (forge/stiglab/synodic/ising/
  refract) can be retired (MIG-01..03) without leaving structural
  holes.
- **Type erasure is contained.** The trait object is local to the
  node; the rest of the substrate sees typed inputs/outputs.

### Negative

- **Trait-object safety constrains the signature.** No generics on
  trait methods (or `where Self: Sized` everywhere). For v1 the
  signature above is enough.
- **Registry must be populated correctly.** A workflow referencing an
  unregistered executor fails at compile time via executor-registry
  lookup (a pre-flight check separate from the five kernel invariants
  in ADR 0018); the registry is the single point of failure for "did
  you remember to register your executor."

### Neutral

- **Serialization unchanged in shape.** Nodes still serialize with
  a kind discriminator; the kind is just a string now, not an enum.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: skills are the dev-process
executor catalog. Skills carry kind + trigger + behavior; they are
discovered dynamically (the user can add one); no closed enum.
`xtask check-tools-and-skills` (ADR 0007) is the dev-process analog
of the `ExecutorRegistry` lookup check.

## Adoption checklist

- [ ] Create `crates/onsager-nodes` (EXE-01, #353).
- [ ] `Executor` trait + `ExecutorRegistry` (EXE-01).
- [ ] Implement initial executors: Script (EXE-02), Agent (EXE-03),
      Verify (EXE-04), Human (EXE-05), SubWorkflow (EXE-06).
- [ ] Remove `NodeKind` enum from `onsager-artifact` (during
      SUB-02).
- [ ] Update workflow serialization to use string `executor_kind`.

## Out of scope

- **Dynamic executor loading.** Executors register at startup. Hot-
  reload, plugin systems, sandbox isolation per executor are
  separate concerns.
- **Executor versioning.** The registry is a flat map by name; if
  we ever need `script@v2` alongside `script@v1`, the registry key
  carries the version. Not needed for v1.
