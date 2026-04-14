# Event Stream Spine — Onsager v0.1

**Status**: Draft
**Owner**: Marvin
**Last updated**: 2026-04-13
**Depends on**: `artifact-model-v0.1.md`, `subsystem-map-v0.1.md`
**Related**: `onsager-vision-v0.1.md` (§5-6), `forge-v0.1.md` (§9, §12)

---

## 1. Purpose

This document formalizes the **factory event spine** — the shared PostgreSQL outbox + `pg_notify` event stream that all four Onsager subsystems communicate through.

The event spine is the factory's production log. Every state-changing action in Onsager writes an event to this spine in the same transaction as the state change itself. The spine is how subsystems stay consistent without direct coupling: Forge writes artifact lifecycle events, Stiglab writes session lifecycle upgrades, Synodic writes rule changes and escalation outcomes, Ising writes insight records — and each subsystem reads the events it needs from the shared stream.

This document defines: the event envelope schema, the two-table storage layout, the delivery mechanism, consumption patterns, ordering guarantees, the authoritative event type registry, namespacing, and the invariants that all producers and consumers must honor.

This document does **not** define:
- Session-internal events (those stay inside Stiglab; see §2)
- Subsystem-internal implementations (see each subsystem's own spec)
- The artifact data model (see `artifact-model-v0.1`)
- Inter-subsystem direct protocols (see `subsystem-map-v0.1 §4.1`)

---

## 2. The two event layers

Onsager has two fundamentally different kinds of events (see `onsager-vision-v0.1 §6`). This spec covers **only** factory events. Both layers are described here for context and to explain the boundary.

### 2.1 Session-internal events

- **Examples**: tool calls, tool results, agent thoughts, subagent spawns, context window management
- **Frequency**: high — dozens per second per active session
- **Scope**: private to a single Stiglab session
- **Consumers**: the session itself, session debugging/replay tools
- **Storage**: inside Stiglab's own runtime; never written to the factory event spine
- **Lifetime**: session-scoped; may be persisted by Stiglab for replay but are not part of the shared event contract

Session-internal events are **not** factory events. They do not appear in the `events` or `events_ext` tables. Synodic and Ising never see them directly.

### 2.2 Factory events

- **Examples**: artifact registered, state changed, shaping dispatched, gate verdict received, insight observed, rule crystallized
- **Frequency**: low — a few per minute per cluster under normal load
- **Scope**: shared across all four subsystems
- **Consumers**: any subsystem that needs to react to or observe factory activity
- **Storage**: the `events` and `events_ext` tables in PostgreSQL
- **Lifetime**: permanent (append-only; see §11)

### 2.3 The upgrade boundary

When something session-internal has governance or improvement significance, Stiglab **upgrades** it into a factory event. An ordinary file read stays internal; an attempt to delete system files upgrades into a factory event that Synodic can act on.

The ratio of internal events to upgraded factory events is a health signal. If the ratio is too low, the factory is under-governed — significant session actions are escaping observation. If too high, the factory is over-reporting noise and drowning the spine in low-value events.

This spec does not define the upgrade criteria — that is Stiglab's responsibility. This spec only requires that upgraded events conform to the factory event envelope (§4) and are written to the spine transactionally (§6).

---

## 3. Storage layout

The factory event spine uses two PostgreSQL tables. Both are append-only. The schema is defined in `migrations/001_initial.sql` and is reproduced here for reference.

### 3.1 The `events` table — core events

```sql
CREATE TABLE IF NOT EXISTS events (
    id          BIGSERIAL PRIMARY KEY,
    stream_id   TEXT NOT NULL,
    stream_type TEXT NOT NULL,
    event_type  TEXT NOT NULL,
    data        JSONB NOT NULL,
    metadata    JSONB NOT NULL DEFAULT '{}',
    sequence    BIGINT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(stream_id, sequence)
);
```

Core events are strictly enumerated, versioned lifecycle events for the primary entities of the factory. Every core event belongs to a **stream** — a sequence of events for a single entity (an artifact, a session, a node, a Forge process). Within a stream, events are totally ordered by `sequence`.

Indexes:
- `(stream_id, sequence)` — fast per-stream replay
- `(event_type)` — fast cross-stream filtering by event kind
- `(created_at)` — fast time-range queries

### 3.2 The `events_ext` table — extension events

```sql
CREATE TABLE IF NOT EXISTS events_ext (
    id           BIGSERIAL PRIMARY KEY,
    stream_id    TEXT NOT NULL,
    namespace    TEXT NOT NULL,
    event_type   TEXT NOT NULL,
    data         JSONB NOT NULL,
    metadata     JSONB NOT NULL DEFAULT '{}',
    ref_event_id BIGINT REFERENCES events(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

Extension events are **namespaced** — each component owns a namespace and writes its component-specific events here. Extension events have a wide JSON payload, allowing any component to publish signals without changing the core schema.

The `ref_event_id` column is an optional foreign key back to the `events` table, linking an extension event to the core event that caused it. This is how a Synodic policy denial can be traced back to the `forge.gate_requested` core event that triggered it.

Indexes:
- `(stream_id)` — per-stream queries
- `(namespace, event_type)` — efficient filtering by component and event kind

### 3.3 Why two tables

The two-table design separates two concerns:

**Core events** are the factory's shared vocabulary. Their types are enumerated in this spec and in subsystem specs. Adding a new core event type is a versioned change. All subsystems understand core events.

**Extension events** are component-private signals that happen to live on the shared spine for observability. Ising reads them; other components generally ignore namespaces they do not own. Adding a new extension event type does not require a spec change — only the owning component needs to know about it.

This separation means the core event contract can be kept tight and stable, while the extension layer remains flexible for component-specific needs.

---

## 4. Event envelope

### 4.1 Core event envelope

Every row in the `events` table follows this canonical envelope:

| Field | Type | Description |
|---|---|---|
| `id` | `BIGSERIAL` | Auto-assigned, globally unique, monotonically increasing row ID |
| `stream_id` | `TEXT` | The primary entity this event belongs to (e.g., `"art_8a3f"`, `"session_abc"`) |
| `stream_type` | `TEXT` | The kind of stream (`"artifact"`, `"session"`, `"node"`, `"forge"`) |
| `event_type` | `TEXT` | Dotted event type string (e.g., `"artifact.state_changed"`) |
| `data` | `JSONB` | Event-specific payload; schema varies by `event_type` |
| `metadata` | `JSONB` | Traceability metadata (see §4.3) |
| `sequence` | `BIGINT` | Per-stream sequence number, monotonically increasing, starting at 1 |
| `created_at` | `TIMESTAMPTZ` | Server-assigned insertion timestamp |

### 4.2 Extension event envelope

Every row in the `events_ext` table follows this envelope:

| Field | Type | Description |
|---|---|---|
| `id` | `BIGSERIAL` | Auto-assigned, globally unique, monotonically increasing row ID |
| `stream_id` | `TEXT` | The entity this event is associated with |
| `namespace` | `TEXT` | Component namespace (validated; see §9) |
| `event_type` | `TEXT` | Event type within the namespace |
| `data` | `JSONB` | Event-specific payload |
| `metadata` | `JSONB` | Traceability metadata (see §4.3) |
| `ref_event_id` | `BIGINT` (nullable) | Optional FK to `events.id` — the core event that caused this |
| `created_at` | `TIMESTAMPTZ` | Server-assigned insertion timestamp |

The fully qualified type of an extension event is `"{namespace}.{event_type}"` — e.g., `"synodic.policy.denied"`.

### 4.3 Metadata

The `metadata` JSONB field carries traceability information. The canonical fields are:

| Field | Type | Description |
|---|---|---|
| `correlation_id` | `string` (optional) | Groups causally related events across streams (e.g., all events triggered by a single operator action) |
| `causation_id` | `integer` (optional) | The `id` of the event that directly caused this one |
| `actor` | `string` | The entity that produced this event (e.g., `"forge"`, `"stiglab:node-3"`, `"operator:marvin"`) |

Additional metadata fields may be added by producers. Consumers must tolerate unknown fields. The `actor` field is required; `correlation_id` and `causation_id` are optional but strongly encouraged for any event that is part of a causal chain.

---

## 5. `pg_notify` delivery

### 5.1 Trigger mechanism

Both tables have `AFTER INSERT` triggers that call `pg_notify` on the `onsager_events` channel:

```sql
CREATE OR REPLACE FUNCTION notify_event() RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('onsager_events', json_build_object(
        'table', TG_TABLE_NAME,
        'id', NEW.id,
        'stream_id', NEW.stream_id,
        'event_type', NEW.event_type
    )::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
```

The notification payload is a lightweight JSON object containing the table name, event ID, stream ID, and event type. It does **not** contain the full event data — consumers that need the data must query the table by ID.

### 5.2 Delivery semantics

`pg_notify` is **at-most-once, non-durable**. This has three consequences:

1. **If a consumer is not listening when an event is inserted, it misses the notification.** There is no replay buffer in `pg_notify`.
2. **If the PostgreSQL connection drops and reconnects, notifications during the gap are lost.**
3. **Under heavy load, PostgreSQL may coalesce or drop notifications.**

These are not bugs — they are inherent properties of `pg_notify`. The tables are the source of truth; `pg_notify` is an optimization for low-latency delivery. Every consumer must be designed to work correctly even if it never receives a single notification, by falling back to table polling.

### 5.3 Channel naming

v0.1 uses a single channel: `onsager_events`. All notifications from both tables flow through this one channel. Consumers filter by the `table`, `stream_id`, and `event_type` fields in the notification payload.

A future version may introduce per-namespace channels (e.g., `onsager_events.synodic`) to reduce noise for consumers that only care about one namespace. This is an optimization, not a semantic change.

---

## 6. The outbox pattern

This is the most important architectural property of the event spine. It is inherited from `subsystem-map-v0.1 §5.7` and `forge-v0.1 §10 invariant #3`.

> **Every state-changing operation writes a factory event to the outbox in the same database transaction as the state change itself.**

Two rules follow:

1. **No state change without event.** If an artifact transitions from `in_progress` to `under_review`, the `artifact.state_changed` event must be written in the same transaction. If the event write fails, the state change is rolled back.

2. **No event without state change.** Events are not speculative, not prospective, not aspirational. An event records something that **has happened** in the same transaction. There are no "planned" events or "pending" events.

This is the classic transactional outbox pattern. It guarantees that the event spine and the system state are always consistent — there is no window where a state change is visible but its event is not, or vice versa.

### 6.1 Implementation

The current Rust implementation in `EventStore::append` and `EventStore::append_ext` inserts events into the respective tables. Callers are responsible for wrapping the state change and the event append in the same database transaction. The `EventStore` exposes its underlying `PgPool` via `EventStore::pool()` for this purpose.

A typical transactional write looks like:

```rust
let mut tx = store.pool().begin().await?;
// 1. Mutate state
sqlx::query("UPDATE artifacts SET state = $1 WHERE id = $2")
    .bind(new_state)
    .bind(artifact_id)
    .execute(&mut *tx)
    .await?;
// 2. Append event in the same transaction
sqlx::query("INSERT INTO events (...) VALUES (...)")
    .execute(&mut *tx)
    .await?;
tx.commit().await?;
```

The `pg_notify` trigger fires on commit, so consumers are notified only after both the state change and the event are durable.

---

## 7. Consumption patterns

There are three ways to consume the factory event spine. Each has different latency and completeness characteristics.

### 7.1 Polling

Query the `events` or `events_ext` table directly. The consumer maintains a cursor (last-seen `id` or `sequence`) and periodically queries for new rows.

- **Latency**: seconds to minutes, depending on poll interval
- **Completeness**: guaranteed — the table is the source of truth
- **Use case**: batch processing, periodic analysis, audit

### 7.2 Streaming (pg_notify + backfill)

Subscribe to `pg_notify` for real-time notifications, but on startup (or reconnection), backfill from the table using the last-seen cursor. This is the recommended pattern for most consumers.

- **Latency**: near-real-time (sub-second under normal load)
- **Completeness**: guaranteed, because missed notifications are caught by backfill
- **Use case**: reactive processing, live dashboards, Ising continuous observation

The current implementation is in `EventStore::subscribe()`, which returns an unbounded channel of `EventNotification` structs. The `Listener` type adds namespace filtering on top.

### 7.3 Replay

Query the `events` table for a specific stream from a given sequence number. This reconstructs the full history of an entity.

- **Latency**: N/A (on-demand)
- **Completeness**: guaranteed for a single stream
- **Use case**: event sourcing, debugging, audit trail reconstruction

The current implementation is `EventStore::query_stream(stream_id, from_sequence)`.

### 7.4 Per-subsystem primary patterns

From `subsystem-map-v0.1 §4.2`:

| Subsystem | Primary read pattern | Notes |
|---|---|---|
| **Forge** | Streaming + replay | Reads for scheduling kernel state; replays artifact streams for decision context |
| **Stiglab** | Minimal | Mostly ignores the spine; receives instructions via direct protocol from Forge |
| **Synodic** | Streaming (filtered) | Reads relevant events when evaluating gates; filters by event types it cares about |
| **Ising** | Streaming (full) | **Primary consumer** — reads the entire spine continuously for pattern detection |

Ising is unusual: it is the only subsystem whose **main input** is the event spine rather than direct protocol calls. This is because Ising's job is to observe the factory as a whole, not to respond to specific requests.

---

## 8. Event type registry

This section is the **authoritative registry** of factory event types. It consolidates the events defined across subsystem specs into a single reference.

### 8.1 Forge events (core)

From `forge-v0.1 §9`. These are the primary factory events and constitute the majority of spine traffic.

| Event type | Stream type | When emitted |
|---|---|---|
| `artifact.registered` | `artifact` | New artifact accepted |
| `artifact.state_changed` | `artifact` | Artifact transitioned between lifecycle states |
| `artifact.version_created` | `artifact` | New version committed |
| `artifact.lineage_extended` | `artifact` | New vertical or horizontal lineage entry |
| `artifact.quality_recorded` | `artifact` | New quality signal appended |
| `artifact.routed` | `artifact` | Released artifact dispatched to a consumer sink |
| `artifact.archived` | `artifact` | Artifact reached terminal state |
| `forge.shaping_dispatched` | `forge` | ShapingRequest sent to Stiglab |
| `forge.shaping_returned` | `forge` | ShapingResult received from Stiglab |
| `forge.gate_requested` | `forge` | GateRequest sent to Synodic |
| `forge.gate_verdict` | `forge` | GateVerdict received (including escalation outcomes) |
| `forge.insight_observed` | `forge` | Insight forwarded to scheduling kernel |
| `forge.decision_made` | `forge` | Scheduling kernel produced a ShapingDecision |
| `forge.idle_tick` | `forge` | Scheduling kernel returned None (emitted at reduced frequency) |
| `forge.state_changed` | `forge` | Forge process state machine transitioned |

### 8.2 Stiglab events (extension)

Session and node lifecycle events written by Stiglab to `events_ext` under the `stiglab` namespace. These are the factory-visible subset of session and node activity; session-internal events (`waiting_input`, `output`, `tool_use`) are not emitted to the spine.

| Event type | When emitted |
|---|---|
| `stiglab.session_created` | New session allocated for a shaping request |
| `stiglab.session_dispatched` | Session dispatched to a Stiglab node |
| `stiglab.session_running` | Session began active execution |
| `stiglab.session_completed` | Session finished successfully |
| `stiglab.session_failed` | Session terminated with an error |
| `stiglab.session_aborted` | Session was aborted (e.g. node lost, deadline exceeded) |
| `stiglab.event_upgraded` | A session-internal event was promoted to a factory event |
| `stiglab.node_registered` | A new Stiglab node joined the pool |
| `stiglab.node_deregistered` | A Stiglab node left the pool |
| `stiglab.node_heartbeat_missed` | A node missed its expected heartbeat |

### 8.3 Synodic events (extension)

Written to `events_ext` under the `synodic` namespace.

| Event type | When emitted |
|---|---|
| `synodic.gate_evaluated` | A gate request was evaluated and a verdict issued |
| `synodic.gate_denied` | A gate request was denied (subset of gate_evaluated, for easy filtering) |
| `synodic.gate_modified` | A gate request verdict was Modify (subset of gate_evaluated) |
| `synodic.escalation_started` | An escalation was initiated |
| `synodic.escalation_resolved` | An escalation was resolved (by human, delegate, or timeout) |
| `synodic.escalation_timed_out` | An escalation timed out and the default verdict was applied |
| `synodic.rule_proposed` | A crystallization candidate was created |
| `synodic.rule_approved` | A proposed rule was approved and entered the active set |
| `synodic.rule_disabled` | A rule was disabled |
| `synodic.rule_version_created` | A rule was modified, producing a new version |

### 8.4 Ising events (extension)

Written to `events_ext` under the `ising` namespace.

| Event type | When emitted |
|---|---|
| `insight.created` | New insight generated from factory observation |
| `insight.forwarded` | Insight forwarded to Forge's scheduling kernel |
| `insight.crystallized` | Insight promoted into a Synodic rule proposal |

### 8.5 Node events (core)

Infrastructure-level events for Stiglab node management.

| Event type | Stream type | When emitted |
|---|---|---|
| `node.registered` | `node` | New Stiglab node joined the cluster |
| `node.heartbeat` | `node` | Periodic health signal from a node |

### 8.6 Registry discipline

Adding a new **core** event type is a versioned change to this spec. All subsystems must be updated to handle (or at minimum tolerate) new core event types before they appear on the spine.

Adding a new **extension** event type requires no spec change. The owning component defines the event type and its schema. Other components must tolerate unknown extension event types in namespaces they do not own.

---

## 9. Namespacing

### 9.1 The `Namespace` type

The `Namespace` type is a validated newtype that partitions the `events_ext` table between components. Validation rules:

- 1 to 32 characters
- Must start with a lowercase ASCII letter (`[a-z]`)
- Only lowercase ASCII letters, digits, and underscores allowed (`[a-z0-9_]`)

### 9.2 Well-known namespaces

| Namespace | Owner | Purpose |
|---|---|---|
| `stiglab` | Stiglab | Session-level extension events |
| `synodic` | Synodic | Governance rules, escalations |
| `ising` | Ising | Insights, observations |
| `telegramable` | Telegramable | Telegram integration events |

These are defined as constants on the `Namespace` type. Adding a new well-known namespace means adding a new constant and is a coordinated change across the polyrepo.

### 9.3 Stream ID convention

`pg_notify` notifications carry `stream_id` but not the `namespace` column. To enable namespace-based filtering at the notification level, producers prefix their `stream_id` values with the namespace followed by a colon:

```
stiglab:session:abc
synodic:rule:policy-42
ising:insight:run-7
```

The `Listener` type splits `stream_id` on the first `':'` and compares the prefix against its subscribed namespaces. If the prefix does not match, the notification is dropped. If no namespaces are subscribed, the listener forwards all notifications.

This is a v0.1 convention. It is adequate for the current scale but couples the notification filtering to a string format. A future version may include the namespace directly in the `pg_notify` payload.

---

## 10. Ordering guarantees

### 10.1 Per-stream ordering

Within a single stream, events are **totally ordered** by `sequence`. The `UNIQUE(stream_id, sequence)` constraint enforces this at the database level. Sequence numbers are monotonically increasing, starting at 1, with no gaps under normal operation.

The current implementation assigns sequence numbers using `COALESCE((SELECT MAX(sequence) + 1 FROM events WHERE stream_id = $1), 1)` inside the insert statement. This is safe under PostgreSQL's row-level locking for the unique constraint but may become a contention point under very high write rates to a single stream.

### 10.2 No global ordering

There is **no global ordering guarantee** across streams. Two events in different streams may have `id` values that suggest an ordering (because `id` is a `BIGSERIAL`), but this ordering is not semantically meaningful. Concurrent transactions may commit in any order, and `id` assignment is not transactionally synchronized across streams.

Consumers that need cross-stream ordering must use `created_at` timestamps as a **best-effort** ordering signal, understanding that clock resolution and transaction commit timing introduce ambiguity. For causal ordering across streams, use `correlation_id` and `causation_id` in the metadata.

### 10.3 Extension events have no sequence

The `events_ext` table does not have a `sequence` column. Extension events are ordered by `created_at` within a stream. This is adequate for their purpose (component-specific signals) but means extension events do not support the same replay guarantees as core events.

---

## 11. Retention and archival

### 11.1 Append-only

Both `events` and `events_ext` are **append-only** in the hot path. Events are never updated. Events are never deleted during normal operation. This is a hard invariant — any code path that issues `UPDATE` or `DELETE` against either table is a bug.

The append-only property is what makes the event spine trustworthy as an audit log and what enables replay-based consumption patterns.

### 11.2 Archival strategy

The event tables will grow without bound. An archival strategy is needed but is **not defined in v0.1**. The expected approach is:

1. Events older than a configurable retention window are moved to cold storage (e.g., partitioned archive tables, S3-backed Parquet files)
2. The hot tables are periodically truncated to maintain query performance
3. Archival is a background operation that does not affect live reads or writes

This is deferred because the correct retention window depends on real workload characteristics that do not yet exist.

### 11.3 What archival must preserve

When archival is implemented, it must preserve:

- **Complete causal chains** — if event A caused event B, both must be archived together or neither
- **Per-stream completeness** — partial stream archives are not permitted; a stream is either fully in hot storage, fully archived, or split at a declared sequence boundary
- **Queryability** — archived events must remain queryable, even if with higher latency

---

## 12. Invariants

The following invariants must be maintained by all producers and consumers. Violation is a bug.

1. **Transactional writes** — Every event is written in the same database transaction as the state change it describes. No state change without event; no event without state change. (See §6.)

2. **Sequence monotonicity** — Within a stream, sequence numbers are strictly increasing. No duplicates, no reversals. Enforced by the `UNIQUE(stream_id, sequence)` constraint.

3. **Event immutability** — Once written, an event is never modified. No `UPDATE` on either table in the hot path. Events are facts; facts do not change.

4. **Schema backward compatibility** — The `data` JSONB payload for a given `event_type` may gain new fields over time but must not remove or change the semantics of existing fields. Consumers must tolerate unknown fields. This is how the event contract evolves without breaking existing consumers.

5. **Metadata completeness** — Every event must have an `actor` in its metadata. `correlation_id` and `causation_id` are optional but must be propagated when available — a producer that receives a correlated event and emits a downstream event must carry the `correlation_id` forward.

6. **Namespace ownership** — A component writes extension events only in its own namespace. Writing to another component's namespace is a violation. No enforcement mechanism exists at the database level in v0.1; this is a convention enforced by code review and the `Namespace` type.

7. **Notification is not acknowledgment** — A `pg_notify` notification does not mean the consumer has processed the event. Producers must not assume that emitting a notification guarantees delivery. The table is the source of truth.

8. **Consumer idempotency** — Consumers must handle receiving the same event more than once (e.g., during backfill after a reconnection). Processing an event must be idempotent or must track processed event IDs.

---

## 13. Open questions

1. **Session-internal event demotion** — The current `CoreEvent` enum includes `session.waiting_input`, `session.output`, and `session.tool_use`. These are session-internal in character and may generate high traffic. Should they be demoted to extension events under the `stiglab` namespace, or removed from the factory spine entirely? The risk of keeping them is spine noise; the risk of removing them is losing observability for Ising.

2. **Per-namespace notification channels** — Should `pg_notify` use per-namespace channels (e.g., `onsager_events.stiglab`) to reduce filtering overhead? The current single-channel approach is simple but requires every consumer to filter. As namespace count and event volume grow, per-namespace channels may be needed.

3. **Sequence assignment under concurrency** — The current `SELECT MAX(sequence) + 1` approach works but may serialize writes to the same stream. Under high write rates to a single artifact stream, this could become a bottleneck. Alternatives: advisory locks, sequence objects per stream, or accepting gaps by using the `id` column as a loose sequence proxy.

4. **Extension event ordering** — Should `events_ext` gain a per-stream sequence column for stronger replay guarantees? The current `created_at` ordering is adequate for v0.1 but may be insufficient if extension events need the same replay discipline as core events.

5. **Event schema registry** — Should the `data` JSONB schema for each event type be formally registered (e.g., JSON Schema definitions in a shared registry)? Currently the schema is implicit in the Rust types and the spec prose. A formal registry would enable automated validation and documentation generation.

6. **Archival retention window** — How long do events stay in the hot tables? This depends on query patterns, storage costs, and compliance requirements that are not yet known. Deferred to operational experience.

7. **Cross-subsystem transactions** — If a Forge state change triggers both a core event and a Synodic extension event, must they be in the same transaction? Currently yes by convention (§6), but this constrains Synodic to write against the same database. If subsystems move to separate databases, the outbox pattern needs adaptation (e.g., CDC-based relay).

8. **Notification payload enrichment** — The current `pg_notify` payload contains only `table`, `id`, `stream_id`, and `event_type`. Should it include `namespace` (for extension events) and `stream_type` (for core events) to reduce the need for follow-up queries?

---

## 14. Current implementation status

The event spine has a working implementation in the `onsager` Rust crate. This section maps spec concepts to code.

| Spec concept | Implementation | Location |
|---|---|---|
| Core event envelope | `EventRecord` struct | `src/store.rs` |
| Extension event envelope | `ExtensionEventRecord` struct | `src/extension_event.rs` |
| Event metadata | `EventMetadata` struct | `src/store.rs` |
| Core event types | `CoreEvent` enum | `src/core_event.rs` |
| Event store (read/write) | `EventStore` struct | `src/store.rs` |
| pg_notify subscription | `EventStore::subscribe()` | `src/store.rs` |
| Namespace validation | `Namespace` newtype | `src/namespace.rs` |
| Namespace-filtered listener | `Listener` struct | `src/listener.rs` |
| Notification payload | `EventNotification` struct | `src/store.rs` |
| SQL schema | Migration file | `migrations/001_initial.sql` |

### 14.1 What exists and works

- Two-table schema with triggers
- Append for core and extension events
- Per-stream query with sequence-based cursor
- Extension event query with namespace filtering
- General-purpose event query with optional filters
- `pg_notify` subscription with async channel delivery
- Namespace validation with well-known constants
- High-level listener with namespace-based notification filtering

### 14.2 What needs to evolve

- **The `CoreEvent` enum reflects a pre-Forge vocabulary.** The current enum has `TaskCreated`, session lifecycle events, and node events — inherited from the earlier Stiglab-centric design. It needs to be updated to include the Forge events from §8.1 (`artifact.*`, `forge.*`).
- **Transactional write helpers are missing.** The `EventStore` exposes `pool()` for manual transaction management, but there is no helper that enforces the "state change + event in one transaction" pattern. A `TransactionalWriter` or similar abstraction would reduce the risk of producers accidentally writing events outside a transaction.
- **Backfill-on-reconnect is not implemented.** The `Listener` subscribes to `pg_notify` but does not backfill from the table on startup or reconnection. A production-grade consumer needs the streaming pattern from §7.2.
- **No consumer cursor tracking.** Consumers have no built-in way to persist their last-seen event ID or sequence. Each consumer must implement this independently.

---

## 15. Glossary

| Term | Definition |
|---|---|
| **Factory event** | An event on the shared spine, representing a meaningful state change visible to all subsystems |
| **Session-internal event** | A high-frequency event private to a Stiglab session; never written to the spine |
| **Core event** | A factory event in the `events` table with a strictly enumerated type and per-stream sequence |
| **Extension event** | A factory event in the `events_ext` table, namespaced by component, with a wide JSON payload |
| **Stream** | A sequence of core events belonging to a single entity, ordered by `sequence` |
| **Namespace** | A validated string that partitions extension events by owning component |
| **Outbox pattern** | Writing the event in the same transaction as the state change it describes |
| **Upgrade** | Stiglab promoting a session-internal event to a factory event |
| **Backfill** | Querying the table to catch up on events missed during a `pg_notify` gap |
| **Cursor** | A consumer's record of the last event it processed, used for resumption |

---

## 16. Decision log

| Date | Decision | Rationale |
|---|---|---|
| 2026-04-13 | Two-table design (core + extension) | Keeps the shared event contract tight while allowing component-specific flexibility |
| 2026-04-13 | `pg_notify` is an optimization, not a guarantee | Matches PostgreSQL's actual delivery semantics; forces consumers to be resilient |
| 2026-04-13 | Single notification channel (`onsager_events`) in v0.1 | Simplicity; per-namespace channels are a future optimization |
| 2026-04-13 | Append-only with no archival strategy in v0.1 | Retention policy depends on operational data that does not yet exist |
| 2026-04-13 | Stream ID prefix convention for namespace filtering | Pragmatic v0.1 approach; may be replaced by richer notification payloads |
| 2026-04-13 | No global ordering guarantee | Per-stream ordering is sufficient for factory semantics; global ordering would require serialization that harms throughput |
| 2026-04-13 | Schema backward compatibility via additive-only JSONB changes | Standard event evolution strategy; avoids breaking existing consumers |
| 2026-04-13 | Namespace ownership is convention, not database-enforced | v0.1 simplicity; database-level enforcement (row-level security or check constraints) is a future option |
| 2026-04-13 | `CoreEvent` enum needs Forge-era update | Current enum predates the Forge spec; tracked as implementation evolution, not a spec change |
