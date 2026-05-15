# ADR 0013 — Observer as second substrate citizen

- **Status**: Accepted
- **Date**: 2026-05-15
- **Identity impact**: yes
- **Tracking issues**: #347 (ADR-01), #361 (OBS-01), #362 (OBS-02)
- **Supersedes**: ADR 0002 (process ↔ product isomorphism) — the
  observer pattern is the in-substrate realization of ADR 0002's
  outer loop; the ADR 0002 framing remains useful as a design
  principle but is no longer the operative mechanism
- **Superseded by**: none

This ADR carries `Identity impact: yes` because it introduces a
second kind of substrate citizen. Until now, the only thing the
substrate scheduled was a Workflow. Observers add a parallel,
non-blocking channel — VSM S3* (audit), structurally distinct from
S1/S2/S3 operations.

## Context

Ising (0.1) was a standalone subsystem polling the spine for signals
and emitting insights. It worked as a research vehicle but two
patterns made it the wrong long-term shape:

- **Ising owned its own loop.** A separate process / crate / event
  manifest, parallel to the main coordination channel. Maintaining
  two coordination shapes (Forge's tick + Ising's polling) was a
  drift surface.
- **Insights had no first-class type.** Ising emitted free-form rows
  into its own table; downstream consumers parsed prose. The
  dashboard could not type-check what Ising said.

The audit/observation channel is a *role*, not a subsystem. Beer's
VSM names it S3* and explicitly separates it from S3 management. The
substrate should reflect that.

## Decision

Observers are a substrate citizen, alongside Workflows. The substrate
provides:

```rust
#[async_trait]
pub trait Observer: Send + Sync {
    fn subscriptions(&self) -> Vec<EventPattern>; // glob, e.g. "artifact.*"
    async fn on_event(&mut self, event: &SpineEvent) -> Vec<ObserverOutput>;
}

pub enum ObserverOutput {
    QualitySignal(QualitySignal),
    Insight(Insight),
    Alert(Alert),
}
```

**Observer properties (constitutive, not optional):**

1. **Non-blocking.** Observers run in separate tasks; their work
   never delays workflow execution. Workflow nodes fire-and-forget
   emit to the spine; observers read asynchronously.
2. **Cannot modify state.** Observers emit `ObserverOutput` rows to
   `observer_outputs`; they do not mutate workflows, artifacts, or
   spine business events. They *audit*; they do not manage.
3. **Spine is the input.** Observers subscribe to spine events, not
   to in-memory state. No private coordination channel.
4. **Output is typed.** `QualitySignal` / `Insight` / `Alert` are
   substrate-recognized types; the dashboard can render them
   uniformly.

`crates/onsager-observers` (OBS-01) replaces `crates/ising`. Existing
Ising analyzers port to the `Observer` trait (OBS-02). MIG-03 retires
the Ising crate once the port is complete.

## Rejected alternatives

- **Keep Ising as the audit subsystem.** Retains the standalone-
  subsystem shape the 0.2 refoundation is trying to remove (MIG-03).
- **Observers as a Workflow executor.** A node that subscribes to
  events. Rejected: blurs S3 and S3*; observers would have to fit in
  a workflow's DAG, which they conceptually escape.
- **Synchronous observer gates.** Allow observers to block a
  workflow until their analysis completes. Rejected: that is what
  the Verify executor is for; mixing audit with gating recreates the
  Ising-knows-best problem at substrate scale.

## Consequences

### Positive

- **One coordination shape.** Spine events are the substrate's only
  coordination medium; observers read the same stream the scheduler
  emits to.
- **Typed audit outputs.** Dashboard renders `Alert` / `Insight` /
  `QualitySignal` with one component; humans triage with consistent
  affordances.
- **VSM mapping is honest.** S3* is structurally separate from S3 in
  the code, not just in the prose.

### Negative

- **Two citizens to teach.** Onboarding now covers Workflows and
  Observers as parallel constructs. The dashboard learns the
  distinction.
- **Observer outputs accumulate.** A separate table (`observer_
  outputs`) needs retention policy. Pre-launch we keep all; post-
  launch a follow-up.

### Neutral

- **Spine schema unchanged.** Observers consume the existing event
  stream; they add an output table but not a new coordination
  medium.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: the `onsager-pr-lifecycle`
skill is an observer — it subscribes to PR / CI webhook events,
emits triage notes, never blocks the merge. CI failures it surfaces
are `Alert`s; review-comment summaries are `Insight`s; pre-push
checks are not observers (they are substrate-Verify equivalents).

## Adoption checklist

- [ ] `crates/onsager-observers` with `Observer` trait + dispatch
      (OBS-01, #361).
- [ ] `observer_outputs` table (migration).
- [ ] Port Ising analyzers to Observer instances (OBS-02, #362).
- [ ] Retire `crates/ising` (MIG-03, #365).
- [ ] Dashboard surfaces `QualitySignal` / `Insight` / `Alert` with
      one component family.

## Out of scope

- **Observer-as-actor (observers writing back to spine business
  state).** Constitutively rejected above — observers audit, not
  manage. A future ADR could revisit if we ever need a write-side
  audit citizen, but the right shape is probably a Workflow.
- **Cross-observer composition.** Each observer is independent; if
  composition becomes necessary, it is a SubWorkflow-equivalent
  decision (the right answer is "make it a workflow").
