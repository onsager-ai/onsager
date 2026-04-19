# ADR 0001 — Event bus is the coordination medium (Option A)

- **Status**: Accepted
- **Date**: 2026-04-19
- **Tracking issues**: #27 (decision), #40 (architectural review)
- **Supersedes**: none
- **Superseded by**: none

## Context

`CLAUDE.md` and `README.md` describe Onsager as a factory event bus where
subsystems coordinate through *stigmergy* — indirect signals via the shared
`events` / `events_ext` table plus `pg_notify`, not direct calls.

The implementation as of early April 2026 does the opposite. Forge's
scheduling loop is a synchronous RPC orchestrator:

```
tick: decide → stiglab.dispatch()  [blocking HTTP]
             → synodic.evaluate()  [blocking HTTP]
             → advance
```

Call sites: `crates/forge/src/cmd/serve.rs:22–66` (`HttpStiglabDispatcher`)
and `serve.rs:68–109` (`HttpSynodicGate`), both using
`tokio::task::block_in_place(|| block_on(reqwest.post(...).send()))` inside
the tick's write-lock window.

The spine is used only as an audit log. `session_listener` at
`crates/forge/src/core/session_listener.rs` subscribes to
`stiglab.session_completed` but the `SessionLinker` (`serve.rs:285–320`) is a
no-op — its TODO explicitly admits the event path is not wired back into
pipeline state. Every shaping produces events on *two paths carrying the
same information* (sync HTTP response + async event); only the sync path
advances state.

Two P0 bugs (#28 dashboard freeze, #29 silent fail-open) and two more
(#30 lost state, #31 self-polling long poll) stem directly from this
sync-RPC coupling. #41 mitigated the acute symptoms but did not change the
architectural direction.

## Decision

**We commit to Option A: the event bus is the coordination medium.**

Concretely:

1. The `events` / `events_ext` tables plus `pg_notify` are the single
   source of truth for cross-subsystem coordination.
2. Forge's `tick` becomes a pure state machine: observe events, decide,
   emit request events. It never blocks on RPC and never holds the write
   lock across a long `await`.
3. `ShapingRequest` becomes a `shaping.requested` event. Stiglab consumes
   and emits `shaping.completed { request_id, result }`.
4. `GateRequest` becomes a `gate.requested` event. Synodic consumes and
   emits `gate.verdict { request_id, verdict }`.
5. Timeouts are event-time math (did a correlated response arrive within
   window?), not HTTP connection timeouts.
6. The sync HTTP surfaces from Forge to Stiglab/Synodic are deleted once
   all call sites have migrated.

Option B (commit to orchestration, rewrite the narrative) was rejected:
the naming (`Onsager`, `stigmergy`, `Ising` Hamiltonian / thermodynamic
analogy) and the forge-v0.1 / warehouse-and-delivery specs are all
premised on stigmergic coordination. Option A keeps the system coherent
with the vision and fixes the bugs; Option B fixes the bugs but strands
the vocabulary.

## Consequences

### Positive

- Forge tick cannot freeze the dashboard by holding a write lock during
  a 5-minute HTTP call — the long await is removed entirely.
- Event-driven timeouts give the same correctness properties with
  observability: a late `shaping.completed` event is still persisted and
  can trigger compensating logic, where a dropped HTTP response is lost.
- Subsystems are truly runtime-decoupled: any of them can restart, lag,
  or be rolled independently without stalling the others.
- The Ising feedback loop (#36) consumes one path instead of two; rule
  proposals and insight events have a stable vocabulary.
- Refract (#35) gets the same event-bus coordination "for free" and
  doesn't re-introduce sync RPC for its intent → artifact tree work.

### Negative / trade-offs

- End-to-end latency increases slightly: a sync HTTP round trip is
  replaced by `INSERT events; NOTIFY` → `LISTEN; SELECT; handle; INSERT`.
  In practice this is 10–100 ms for well-indexed reads on a local
  database, which is inside the budget for all current call sites
  (shaping is minutes, gate evaluation is seconds).
- Correlation requires request IDs on every event pair. We already do
  this via `correlation_id`, but every new request/response pair must
  enforce this consistently.
- Debugging shifts from "read an HTTP trace" to "replay the event
  stream". This is strictly more powerful but needs Dashboard support
  (an event stream viewer) to be ergonomic for operators.

### Neutral

- Schema: no new tables. `events_ext` already carries the payloads; new
  event kinds are additive rows in the `FactoryEventKind` enum.

## Migration checklist

Call sites and modules that must change to finish Option A. Each item
links to a tracking issue where the work belongs; check off as each PR
lands.

### Forge side — replace blocking RPC with event emit + listener

- [ ] Delete `HttpStiglabDispatcher`
      (`crates/forge/src/cmd/serve.rs:22–66`).
      Replace with `SpineShapingDispatcher` that `INSERT`s a
      `shaping.requested` event row; return the request ID synchronously
      and let the tick move on. Tracked in #31 follow-up + #27.
- [ ] Delete `HttpSynodicGate`
      (`crates/forge/src/cmd/serve.rs:68–109`).
      Replace with `SpineGateDispatcher` that emits `gate.requested`.
      Tracked in #29 follow-up + #27.
- [ ] Rewrite `ForgePipeline::tick`
      (`crates/forge/src/core/pipeline.rs`) as a pure state machine:
      take a snapshot of recent `shaping.completed` / `gate.verdict`
      events as tick input, apply them, emit new request events as tick
      output. No `.await` on external services inside the write lock.
- [ ] Wire `SessionLinker` (`serve.rs:285–320`) to actually fold
      `shaping.completed` / `stiglab.session_completed` events into
      pipeline state. Its own TODO already flags this.
- [ ] Remove `reqwest` from `crates/forge/Cargo.toml` once the two HTTP
      clients above are deleted.

### Stiglab side — emit events instead of returning sync payloads

- [ ] Have the shaping handler emit `shaping.completed { request_id,
      result }` on the spine in the same transaction as its DB write,
      not just as a response body to forge's polling GET.
      (`crates/stiglab/src/server/spine.rs` is where the event writer
      already lives.) Tracked in #31 follow-up.
- [ ] Delete the `GET /api/shaping/{id}?wait=Ns` endpoint once Forge no
      longer calls it. Until then, keep it for compatibility.

### Synodic side — emit verdict events instead of responding to sync gate

- [ ] Gate adapter emits `gate.verdict { request_id, verdict }` after
      evaluation. Keep the HTTP endpoint for one release while Forge
      migrates; delete after.
      (`crates/synodic/src/core/gate_adapter.rs`.)
- [ ] `InterceptEngine` reload is still per-request via HTTP; switch to
      a spine-driven cache invalidation event (`rules.updated`). Tracked
      in #32.

### Spine / protocol — drop the sync-RPC DTOs

- [ ] Move `ShapingRequest`, `GateRequest`, `GateVerdict`, `Insight` out
      of `onsager-protocol` and into the corresponding
      `FactoryEventKind` payload shapes. Delete `onsager-protocol`
      entirely once the last caller is gone. Tracked in #33 (spine
      split) and #27.
- [ ] Add new `FactoryEventKind` variants: `ShapingRequested`,
      `ShapingCompleted`, `GateRequested`, `GateVerdict` (if not already
      there), `RulesUpdated`.

### State & observability

- [ ] Forge state persistence (#30): with Option A, pipeline state
      becomes a projection of the event stream. Registering an artifact
      and advancing it emit events inside the same transaction; restart
      rebuilds in-memory state by replaying since the last snapshot.
- [ ] Dashboard: add an event stream view scoped by
      `correlation_id` so operators can reconstruct a tick from its
      emitted events. Tracked separately under the operator-surface
      theme (#38).

### Docs

- [x] This ADR exists and is linked from `CLAUDE.md`.
- [ ] Update `README.md` §Architecture to reflect the event-bus model
      once the first full cycle (request event → response event →
      tick projection) is shipped.

## Out of scope for this ADR

- Choice of event versioning strategy (additive enum variants vs. a
  separate schema registry) — revisit when the first breaking payload
  change is needed.
- Back-pressure policy when a subsystem falls behind the event stream.
  Current assumption: subsystems `SELECT` the tail up to their own
  cursor and catch up; hard back-pressure is a future ADR.
- Multi-tenant / per-workspace event-stream sharding — out of scope; see
  #39 (budget accounting) for the first concrete forcing function.
