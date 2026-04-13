# Subsystem Map — Onsager v0.1

**Status**: Draft
**Owner**: Marvin
**Last updated**: 2026-04-13
**Role**: Shared preface for all four subsystem specs
**Related**: `onsager-vision-v0.1.md`, `artifact-model-v0.1.md`, `forge-v0.1.md`

---

## 1. Purpose

This document is the **shared reference frame** for the four Onsager subsystems. It exists so that `forge-*`, `stiglab-*`, `synodic-*`, and `ising-*` specs can be written against a common vocabulary for how subsystems talk to each other, without each spec having to re-derive the interaction model.

This document does **not** define:
- What any subsystem does internally (see each subsystem's own spec)
- The data model of artifacts (see `artifact-model-v0.1`)
- Event schemas or storage layouts (see forthcoming `event-stream-spine-v0.1`)

It defines **only**: the shape and nature of inter-subsystem communication.

If anything in this document conflicts with a subsystem spec, **this document wins** for matters of inter-subsystem protocol. The subsystem spec wins for matters internal to that subsystem.

---

## 2. The four subsystems

| Subsystem | Role | Core verb |
|---|---|---|
| **Forge** | Production line — drives artifacts through their lifecycle | drives |
| **Stiglab** | Distributed shaping runtime — runs agent sessions that shape artifacts | shapes |
| **Synodic** | Governance gate — evaluates every action and state transition | gates |
| **Ising** | Continuous improvement engine — observes the factory and surfaces insights | observes |

All four interact only through two channels:

1. **Direct protocol calls** — typed request/response between a pair of subsystems
2. **Factory event spine** — a shared append-only event stream (PostgreSQL outbox + `pg_notify`)

No subsystem reads another subsystem's internal state directly. No subsystem mutates another subsystem's storage. Cross-subsystem communication is **strictly mediated** by these two channels.

---

## 3. The three protocol modes

Inter-subsystem protocols in Onsager are not uniform. They come in exactly three modes, and **every inter-subsystem interaction belongs to exactly one mode**.

### 3.1 Imperative

- **Shape**: A commands B. B executes.
- **Direction**: one-way, top-down.
- **Binding**: B must attempt the command. B may fail, but may not refuse on policy grounds (policy is Synodic's job, not the receiver's).
- **Example**: Forge → Stiglab dispatch.

### 3.2 Gated

- **Shape**: A asks B for permission before proceeding. B's verdict is binding.
- **Direction**: request from A, verdict from B. A cannot proceed against B's verdict.
- **Binding**: **Structural, not optional.** A has no override mechanism. If A wants different behavior, A must work with B's owner to change B's rules.
- **Example**: Forge → Synodic gate consultation.

### 3.3 Advisory

- **Shape**: A produces observations. B may or may not act on them.
- **Direction**: one-way, but non-binding.
- **Binding**: None. Advisory output is reference material for the receiver's own decision-making.
- **Example**: Ising → Forge insight forwarding.

---

## 4. The canonical subsystem interaction map

```
                         ┌───────────┐
                         │   FORGE   │
                         └─┬─────┬──┬┘
                 imperative│ gated│  │advisory (in)
                           ▼     ▼  ▲
                      ┌───────┐ ┌──────┐ ┌──────┐
                      │STIGLAB│ │SYNODIC│ │ ISING│
                      └───────┘ └──────┘ └──────┘
                           │        ▲       ▲
                           │ gated  │       │
                           └────────┘       │
                                            │
                      ┌─────────────────────┘
                      │  (Ising reads the factory event spine;
                      │   Forge, Stiglab, Synodic all write to it)
                      ▼
                ┌──────────────────┐
                │ FACTORY EVENT    │
                │ SPINE (pg outbox │
                │  + pg_notify)    │
                └──────────────────┘
```

### 4.1 Direct protocols (typed request/response)

| From | To | Mode | What flows |
|---|---|---|---|
| **Forge** | **Stiglab** | Imperative | Shaping requests and their results |
| **Forge** | **Synodic** | Gated | Gate consultations at dispatch / transition / routing points |
| **Stiglab** | **Synodic** | Gated | Tool-level gate consultations **inside** a session |
| **Ising** | **Forge** | Advisory | Insights forwarded to the scheduling kernel |

Four direct protocols total. That is the complete list. Any other pair of subsystems does not talk directly.

### 4.2 Event spine consumption

Every subsystem **writes** to the factory event spine for its own state-changing actions. Every subsystem **may read** from the spine for any purpose.

| Subsystem | Primary writes | Primary reads |
|---|---|---|
| **Forge** | Most factory events (see `forge-v0.1 §9`) | Reads for its own scheduling kernel state |
| **Stiglab** | Session lifecycle upgrades (start, complete, fail) | Minimal — mostly ignores spine |
| **Synodic** | Rule changes, escalation outcomes | Reads relevant events when evaluating gates |
| **Ising** | Insight records | **Primary consumer** — reads the entire spine continuously |

Ising is unusual: it is the only subsystem whose **main input** is the event spine rather than direct protocol calls. This is because Ising's job is to observe the factory as a whole, not to respond to specific requests.

---

## 5. Protocol invariants

These invariants apply across all four subsystems. Any subsystem spec that defines a protocol must honor these.

1. **No bypass** — If a protocol mode is gated, the receiver's verdict is structurally honored. There are no operator overrides, no force flags, no debug bypasses in production builds.

2. **No hidden channels** — Subsystems communicate only through §4.1 direct protocols and the event spine. Any subsystem that reads another subsystem's database, filesystem, or in-memory state is violating this invariant.

3. **Identity of session stays inside Stiglab** — The concept of "session" is Stiglab's internal abstraction. Forge sees shaping requests and results; Synodic sees tool-level gate consultations tagged with session IDs for audit; Ising sees session-scoped events upgraded into factory events. No subsystem outside Stiglab mutates session state.

4. **Artifact writes go through Forge only** — As defined in `artifact-model-v0.1 §8`. Synodic and Ising can read artifact state (via event spine or via Forge read APIs, TBD) but never write.

5. **Advisory never escalates silently to imperative** — If Ising's insight is so urgent that it needs immediate enforcement, the path is: Ising emits insight → Synodic rule crystallization pipeline → approved rule → future Forge→Synodic gates honor the new rule. There is no "emergency imperative channel" from Ising to Forge. Adding one would collapse the advisory/gated distinction and weaken governance.

6. **Protocols are versioned** — Each direct protocol in §4.1 has a version, declared at the top of the relevant subsystem spec. Breaking changes require a version bump and a migration path.

7. **Event spine writes are transactional** — Writes to the event spine happen in the same database transaction as the state change they describe (outbox pattern). No state change without a corresponding event; no event without an actual state change.

---

## 6. Anti-patterns

If you catch yourself writing any of the following in a subsystem spec, stop and reconsider:

- **"Stiglab can skip Synodic in fast paths"** — violates §5.1
- **"Ising directly pauses Forge when pattern X is detected"** — violates §5.5
- **"Synodic reads Stiglab's internal session database to check for..."** — violates §5.2 and §5.3
- **"Forge calls Ising's API to get a priority boost for artifact X"** — collapses advisory into imperative; Ising is not a service, it's an observer
- **"Stiglab writes artifact state directly when the shaping obviously succeeded"** — violates §5.4 and the single-writer invariant

Each of these might feel like a pragmatic shortcut in the moment. Each of them breaks a boundary that the architecture depends on. When a shortcut is tempting, it usually means a genuine need exists — surface the need and solve it with a new protocol, not by punching a hole through existing ones.

---

## 7. How to use this document when writing a subsystem spec

When writing `stiglab-v0.x`, `synodic-v0.x`, or `ising-v0.x`, reference this document as follows:

1. **In the §1 Related section**, list `subsystem-map-v0.1.md`.
2. **When defining any interaction with another subsystem**, cite the protocol mode from §3 by name (e.g., "Stiglab → Synodic is a gated protocol, as defined in subsystem-map §3.2").
3. **Do not redefine the interaction model.** If you feel the need to, it means either (a) this document is wrong and should be updated, or (b) your spec is drifting and should be brought back in line.
4. **Do not invent new protocol modes.** Three is the complete set for v0.1. If a new mode is genuinely needed, propose an update to this document first.

---

## 8. Decision log

| Date | Decision | Rationale |
|---|---|---|
| 2026-04-13 | Three protocol modes: imperative, gated, advisory | These are the only modes observed in the current architecture; adding more would dilute meaning |
| 2026-04-13 | Four direct protocols, no more | Anything beyond these four is indirect via the event spine |
| 2026-04-13 | Ising has no imperative or gated path to Forge | Advisory-only is how Onsager prevents observation from becoming a second governance layer |
| 2026-04-13 | Session identity stays strictly inside Stiglab | Preserves the internal/factory event layer separation |
| 2026-04-13 | Subsystem-map wins conflicts with subsystem specs on inter-subsystem matters | One place to update when the interaction model changes |
