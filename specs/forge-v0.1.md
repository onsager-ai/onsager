# Forge — Onsager v0.1

**Status**: Draft
**Owner**: Marvin
**Last updated**: 2026-04-13
**Depends on**: `artifact-model-v0.1.md`
**Related**: `stiglab-*`, `synodic-*`, `ising-*` (forthcoming alignment)

---

## 1. Purpose

Forge is the **production line subsystem** of Onsager. It drives artifacts through their lifecycle — deciding what to shape next, dispatching shaping work, advancing artifact state, and routing finished artifacts to their consumers.

Forge is not a recorder. It is not a database. It is not a registry. It is the **active, decision-making, state-advancing core** of the Onsager factory. When Onsager is running, Forge is the thing that is *running*; the other three subsystems are services that Forge consults or delegates to.

This document defines Forge's responsibilities, its decision model, its protocols with the other three subsystems, and its invariants. It does **not** specify API endpoints, database schemas, UI surfaces, or concrete scheduling algorithms — those are implementation concerns.

---

## 2. Responsibilities

Forge does six things and only six things:

1. **Register** — accept new artifact declarations, assign IDs, enforce §7 invariants of the artifact model
2. **Decide** — determine what artifact to shape next, in what way, with what inputs, under what constraints
3. **Dispatch** — send shaping instructions to Stiglab; receive shaping results back
4. **Gate** — consult Synodic at every state transition; respect its verdict
5. **Advance** — mutate artifact state (new version, new state, new lineage entries) atomically and durably
6. **Route** — when an artifact reaches `released`, dispatch it to its declared consumers via pluggable sinks

Everything else is **out of scope**. Specifically:

- Forge does **not** run agent code. Stiglab does.
- Forge does **not** evaluate policies. Synodic does.
- Forge does **not** detect patterns or propose improvements. Ising does.
- Forge does **not** store artifact content. External systems do.
- Forge does **not** render UI. The Console does.

Forge is **the one place where artifact state can be mutated**. This single-writer constraint is inherited from `artifact-model-v0.1 §8` and is the most important architectural fact about Forge.

---

## 3. Mental model

```
         ┌──────────────────────────────────────────┐
         │                  FORGE                   │
         │                                          │
         │   ┌────────────┐      ┌──────────────┐   │
         │   │ Scheduling │ ───► │   Dispatch   │   │
         │   │   kernel   │      │   to stiglab │   │
         │   └────────────┘      └──────────────┘   │
         │         ▲                     │          │
         │         │                     ▼          │
         │   ┌────────────┐      ┌──────────────┐   │
         │   │  Ising     │      │   Synodic    │   │
         │   │  feedback  │      │     gate     │   │
         │   └────────────┘      └──────────────┘   │
         │                              │           │
         │                              ▼           │
         │                       ┌──────────────┐   │
         │                       │   Advance    │   │
         │                       │  artifact    │   │
         │                       │    state     │   │
         │                       └──────────────┘   │
         │                              │           │
         │                              ▼           │
         │                       ┌──────────────┐   │
         │                       │    Route to  │   │
         │                       │  consumers   │   │
         │                       └──────────────┘   │
         └──────────────────────────────────────────┘
```

Forge's core loop is:

```
loop:
    decision = scheduling_kernel.decide(world_state)
    shaping_result = stiglab.dispatch(decision)
    verdict = synodic.gate(shaping_result, target_state)
    if verdict.allow:
        forge.advance(artifact, shaping_result, target_state)
        if target_state == RELEASED:
            forge.route(artifact, consumers)
    emit_factory_events()
```

This is deliberately simple. Complexity lives in `scheduling_kernel.decide()` — which is pluggable — not in the loop itself.

---

## 4. The scheduling kernel

### 4.1 Role

The scheduling kernel answers one question each tick:

> **Given the current world state, what is the next artifact to shape, and how?**

Its output is a `ShapingDecision`:

- `artifact_id` — which artifact
- `target_version` — which version to produce (usually `current_version + 1`)
- `target_state` — what state transition this shaping should achieve
- `shaping_intent` — a structured description of what the shaping should accomplish (kind-specific)
- `inputs` — references to other artifacts used as horizontal lineage
- `constraints` — governance and resource constraints Stiglab must respect
- `priority` — integer for dispatch ordering
- `deadline` — optional soft deadline

### 4.2 Decision inputs

The kernel sees:

- All `in_progress` and `under_review` artifacts
- Their current `quality_signals`
- Their declared `consumers` and deadlines
- Current Stiglab node availability
- Recent Ising insights relevant to scheduling
- Synodic's recent escalation patterns (to avoid scheduling work that will be gated away)
- Factory event stream (last N events for context)

The kernel does **not** see:

- Session internal events (that's below Forge's abstraction level — see `artifact-model §3`)
- Content of artifacts (only metadata and quality signals)
- User identities beyond ownership claims

### 4.3 Pluggability

The scheduling kernel is a **replaceable module**. v0.1 ships with a baseline implementation (priority queue + deadline awareness + round-robin fairness across owners), but the contract is:

```
trait SchedulingKernel:
    fn decide(&self, world: &WorldState) -> Option<ShapingDecision>
    fn observe(&mut self, event: &FactoryEvent)
```

Any implementation that honors this contract is valid. Future implementations may incorporate learned models, multi-objective optimization, or human override channels. **The choice of algorithm is not part of this spec.** This spec only guarantees that the kernel has a well-defined interface and well-defined inputs.

### 4.4 Empty-decision semantics

If the kernel returns `None`, Forge idles (no work to dispatch this tick). This is the **normal healthy state** when no artifacts need shaping. Idle is not failure. Factory events `forge.idle_tick` are emitted at reduced frequency during idle to avoid log noise.

---

## 5. The Stiglab dispatch protocol

### 5.1 Direction

Forge → Stiglab is **one-way imperative**. Forge says "shape this artifact like this"; Stiglab executes. Stiglab does not push work back to Forge's decision queue.

### 5.2 Contract

```
ShapingRequest (Forge → Stiglab):
    request_id: ULID
    artifact_id: ArtifactId
    target_version: int
    shaping_intent: structured payload
    inputs: [ArtifactRef]
    constraints: [Constraint]
    deadline: Option<timestamp>

ShapingResult (Stiglab → Forge):
    request_id: ULID
    outcome: Completed | Failed | Partial | Aborted
    content_ref: Option<ExternalURI>
    change_summary: string
    quality_signals: [QualitySignal]
    session_id: SessionId  // for vertical lineage
    duration_ms: int
    error: Option<ErrorDetail>
```

### 5.3 Who holds session identity

**Stiglab owns session IDs**. Forge references them but does not create them. This preserves the abstraction: session is Stiglab's internal concept; Forge only cares that shaping happened and who did it (for lineage).

### 5.4 Failure modes

- `Failed` — shaping could not complete; artifact stays in previous state; Forge may re-decide or escalate
- `Partial` — shaping made progress but did not reach target state; artifact stays `in_progress`; new version may or may not be emitted based on `change_summary`
- `Aborted` — shaping was cancelled mid-flight (usually because Synodic gate denied a mid-session action); artifact stays in previous state

All failure modes emit factory events. No failure is silent.

### 5.5 What Stiglab is free to do internally

Stiglab internally manages: node allocation, agent selection, tool availability, session-internal event stream, subagent spawning, retries within a shaping request. None of this surfaces to Forge. Forge only sees the final `ShapingResult`.

This is the layering principle: **internal events stay internal, factory events rise to Forge**. (See `artifact-model §3` and prior discussion on the two event layers.)

---

## 6. The Synodic gate protocol

### 6.1 Gate points

Synodic is consulted at the following moments:

1. **Pre-dispatch** — before Forge sends a `ShapingRequest` to Stiglab. Synodic may deny the request based on the `shaping_intent` and current context.
2. **State transition** — before Forge advances an artifact to a new state. Synodic may deny the transition or require escalation.
3. **Consumer routing** — before Forge dispatches a `released` artifact to external consumers. Synodic may redact, delay, or block routing.

Within a Stiglab session (between dispatch and result), Synodic may be consulted by Stiglab itself for tool-level gating — but that is part of Stiglab's protocol with Synodic, not Forge's. Forge does not mediate intra-session gating.

### 6.2 Contract

```
GateRequest (Forge → Synodic):
    context: GateContext  // kind of gate point + relevant artifact state
    proposed_action: ProposedAction

GateVerdict (Synodic → Forge):
    verdict: Allow | Deny(reason) | Modify(new_action) | Escalate(ctx)
```

`Escalate` returns an asynchronous channel — Forge parks the decision and waits for a human (or delegated system) to respond. While parked, the artifact stays in its current state and other artifacts continue to be scheduled.

### 6.3 Escalation timeouts

Escalated decisions have a configurable timeout (per-kind and per-transition). On timeout, Synodic's default verdict applies. Default verdicts are **conservative** — a timeout on a release escalation means the release does **not** happen.

### 6.4 Verdict honoring

Forge honors Synodic verdicts **unconditionally**. Forge has no override mechanism. This is not a user-facing restriction; it is a **structural** one. If a user wants to override Synodic, they must edit a Synodic rule and re-submit — they cannot bypass Synodic through Forge.

This is the core of Onsager's governance story: **governance cannot be bypassed by the production line itself**.

---

## 7. The Ising feedback protocol

### 7.1 Direction

Ising → Forge is **advisory, not imperative**. Ising produces insights; Forge may or may not act on them. This is the opposite of the Synodic protocol (which is imperative).

### 7.2 Contract

```
Insight (Ising → Forge):
    insight_id: InsightId
    kind: Failure | Waste | Win | Anomaly
    scope: ArtifactKind | SpecificArtifact | Global
    observation: string
    evidence: [FactoryEventRef]
    suggested_action: Option<SuggestedAction>
    confidence: float
```

### 7.3 How Forge uses insights

The scheduling kernel `observe`s insights as part of its world state. How it uses them is kernel-specific:

- A baseline kernel may deprioritize artifacts matching a known failure pattern
- A learning kernel may use insights as training signal
- A human-in-the-loop kernel may surface insights to operators for manual action

**Forge itself does nothing with insights beyond forwarding them to the kernel.** This keeps Forge's core logic simple and makes insight-consumption a kernel concern.

### 7.4 Insights are not gates

Ising cannot block Forge. If Ising observes that a pattern is disastrous, its path to preventing future occurrences is through **Synodic rule crystallization** — insights become proposed rules, rules are reviewed, approved rules gate future actions. This is the full loop of continuous improvement.

The separation matters: **observation is advisory, governance is imperative**. Mixing them would make Ising a second governance layer, which violates subsystem cohesion.

---

## 8. Forge's own state machine

Forge as a running process has three states:

```
running ──pause──► paused
   ▲                  │
   └────── resume ────┘
   │
   ▼
draining ──drained──► stopped
```

- **running** — normal operation, scheduling and dispatching
- **paused** — scheduling kernel still accepts events but produces no decisions; in-flight shaping requests continue to completion; no new dispatches
- **draining** — no new decisions, no new dispatches, waiting for all in-flight shaping requests to return
- **stopped** — fully halted, no activity

Operators can pause Forge at any time — this is the emergency brake. Paused Forge still honors Synodic verdicts and processes Stiglab results; it just stops deciding new work.

`draining` is used for graceful shutdown and version upgrades.

---

## 9. Core factory events

Forge emits the following factory events. This list is **authoritative** — Forge implementations must emit exactly these event types for these situations:

| Event | When |
|---|---|
| `artifact.registered` | New artifact accepted |
| `artifact.state_changed` | Artifact transitioned between states |
| `artifact.version_created` | New version committed |
| `artifact.lineage_extended` | New vertical or horizontal lineage entry |
| `artifact.quality_recorded` | New quality signal appended |
| `artifact.routed` | Released artifact dispatched to a consumer sink |
| `artifact.archived` | Artifact terminal state reached |
| `forge.shaping_dispatched` | ShapingRequest sent to Stiglab |
| `forge.shaping_returned` | ShapingResult received |
| `forge.gate_requested` | GateRequest sent to Synodic |
| `forge.gate_verdict` | GateVerdict received (includes Escalate outcomes) |
| `forge.insight_observed` | Insight forwarded to scheduling kernel |
| `forge.decision_made` | Scheduling kernel produced a ShapingDecision |
| `forge.idle_tick` | Scheduling kernel returned None (low frequency) |
| `forge.state_changed` | Forge process state machine transitioned |

Subsystems subscribe to this stream via `pg_notify`. This list is the **contract** of Forge with the rest of Onsager. Adding new event types is a versioned change to this spec.

---

## 10. Invariants

Forge must maintain these invariants. Violation is a bug:

1. **Single writer** — only Forge mutates artifact state. No other subsystem has write access.
2. **Atomic advancement** — state transitions, version creations, and lineage updates for a single `ShapingResult` happen in one transaction.
3. **Event durability** — factory events are persisted **before** acknowledging any external action (outbox pattern).
4. **Gate honoring** — no state transition, dispatch, or routing happens against a Synodic verdict.
5. **Escalation non-blocking** — an escalated decision does not freeze the entire scheduling loop. Other artifacts continue to be scheduled.
6. **Idempotent dispatch** — if Forge dispatches the same `ShapingRequest` twice (due to retry), Stiglab must see the same `request_id` and deduplicate. Forge never mutates a `request_id`.
7. **No silent failure** — every `Failed`, `Aborted`, or `Denied` outcome produces a factory event. No shaping attempt is lost.
8. **Kernel purity** — the scheduling kernel is a pure function of world state and event stream. It has no side effects beyond emitting decisions.
9. **Pause safety** — in `paused` state, Forge continues to process incoming results and verdicts but produces no new decisions. Pausing never loses in-flight work.
10. **Consumer route at-least-once** — routing to consumers is at-least-once delivery. Consumers must be idempotent. Forge never drops a release.

---

## 11. What Forge explicitly does not do

To prevent scope creep, these are non-responsibilities:

- **Not a workflow engine** — Forge does not execute arbitrary DAGs of user-defined steps. It schedules artifact shaping. DAG-like behavior emerges from horizontal lineage (one artifact depending on another), but Forge does not expose a DAG authoring surface.
- **Not a CI/CD system** — Forge does not build, test, or deploy code artifacts. It dispatches shaping requests; what gets built is decided by the Stiglab session acting on `shaping_intent`.
- **Not an agent runtime** — Forge never runs agent code directly. Stiglab is the only subsystem that does.
- **Not a content store** — Forge holds `content_ref` pointers. Content lives in external systems.
- **Not a user management system** — Forge reads ownership and consumers from a separate identity service; it does not store user credentials or permissions.
- **Not a metrics system** — Forge emits factory events; downstream systems (Ising, external observability) aggregate them.

---

## 12. Relationship to the event stream spine

Onsager's architectural spine is a PostgreSQL outbox + `pg_notify` event stream. Forge is the **primary producer** on this stream — most factory events originate from Forge. Synodic and Ising are primary consumers; Stiglab is both producer (via shaping results) and consumer (via dispatch instructions, though dispatches are also factory events).

Forge does not own the event stream — it writes to a shared outbox table governed by a separate spec (`event-stream-v0.1` — forthcoming). Forge's only commitment is: **every state-changing operation writes a factory event to the outbox in the same transaction as the state change itself**. This is the classic outbox pattern and is how invariant #3 is enforced.

---

## 13. Open questions

1. **Parallelism** — can two `ShapingRequest`s be in-flight for the same artifact simultaneously (e.g., shaping two different dimensions of a `code` artifact in parallel)? v0.1 assumes **no** — one in-flight shaping per artifact. This simplifies invariants but may limit throughput.
2. **Priority starvation** — under sustained high-priority load, lower-priority artifacts may never be scheduled. The baseline kernel should have anti-starvation, but the spec does not mandate it. Should it?
3. **Kernel hot-swap** — can the scheduling kernel be replaced while Forge is running, or only at restart? v0.1 assumes restart-only. Hot-swap introduces consistency issues.
4. **Cross-Forge coordination** — if two Forge instances run in an HA pair, how do they avoid double-dispatching the same artifact? Requires a leader election or work-claiming mechanism, not specified in v0.1.
5. **Rollback semantics** — if a `released` artifact needs to be rolled back (bad release), is that a new shaping cycle producing v+1, or an explicit "rollback" action mutating state? v0.1 treats it as a new cycle; no rollback primitive exists.
6. **Consumer sink failure** — if routing to a consumer fails persistently, what happens? Retry forever? Dead-letter? Alert? v0.1 leaves this to the sink implementation.
7. **Synodic escalation deadlines** — who configures per-transition timeouts? The artifact kind? The Synodic rule? The operator globally? Needs alignment with Synodic spec.

---

## 14. Decision log

| Date | Decision | Rationale |
|---|---|---|
| 2026-04-13 | Forge is the single writer to artifact state | Simplifies invariant enforcement; clean separation of concerns |
| 2026-04-13 | Scheduling kernel is pluggable | Avoid locking algorithm choice at spec level |
| 2026-04-13 | Synodic verdicts are unconditionally honored | Governance cannot be bypassed structurally |
| 2026-04-13 | Ising feedback is advisory, not imperative | Observation and governance are separate concerns |
| 2026-04-13 | Session IDs owned by Stiglab, not Forge | Preserves layering: session is internal to Stiglab |
| 2026-04-13 | v0.1 assumes one in-flight shaping per artifact | Simplicity first; revisit when parallelism is needed |
| 2026-04-13 | Forge does not hold content | Inherited from artifact-model-v0.1 §6 |
| 2026-04-13 | Factory event list in §9 is authoritative | Makes Forge's contract with the rest of Onsager explicit |
