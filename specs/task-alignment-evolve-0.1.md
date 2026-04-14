# Task: Onsager repo v0.1 alignment and evolution

You are working on the `onsager-ai/onsager` Rust crate — the shared event spine
client library for the Onsager polyrepo. Your job is to execute a prioritized
alignment and evolution pass on the existing v0.1 draft. This work was planned
after a thorough architecture review; your job is to execute the plan faithfully,
not to re-design.

## Ground rules

1. **Specs in `specs/` are the source of truth.** When code and spec disagree,
   the code must be changed to match the spec — not the other way around.
   If a spec change is genuinely needed, surface it as a question before touching
   the spec. Never silently edit a spec to make failing code pass.

2. **Before touching any file, read the relevant spec.** The specs are:
   - `specs/onsager-vision-v0.1.md`
   - `specs/subsystem-map-v0.1.md`
   - `specs/artifact-model-v0.1.md`
   - `specs/forge-v0.1.md`
   - `specs/stiglab-v0.1.md`
   - `specs/synodic-v0.1.md`
   - `specs/ising-v0.1.md`
   - `specs/event-stream-spine-v0.1.md`

3. **Conventions** (from `CLAUDE.md`):
   - Rust edition 2021, `rustfmt`, `clippy -D warnings`
   - `thiserror` for library errors, `anyhow` for application errors
   - Unit tests co-located in `#[cfg(test)]` modules
   - Commit messages: imperative mood, under 72 chars for the subject line
   - Small, focused commits

4. **The crate has not yet been published to crates.io.** This means breaking
   changes are free — there are no external consumers to keep compatible with.
   Prefer aggressive cleanup over deprecation paths. When in doubt, delete.

5. **Before committing, always run**:
   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```
   All three must pass. No exceptions. Tests include integration tests that
   require a live PostgreSQL instance — see §6 below.

6. **PostgreSQL for tests.** Several tasks require a running PostgreSQL instance
   with the schema from `migrations/`. Use either:

   - A dev Docker container:
     ```bash
     docker run -d --name onsager-test-pg \
       -e POSTGRES_PASSWORD=onsager \
       -e POSTGRES_DB=onsager \
       -p 5433:5432 \
       postgres:16
     ```
     Then apply migrations from `migrations/*.sql` in order.

   - Or a `DATABASE_URL` environment variable pointing to any PostgreSQL that
     has the schema applied.

   Tests that require a database must **not** be marked `#[ignore]`. They must
   run as part of `cargo test` by default, reading `DATABASE_URL` from the
   environment. If `DATABASE_URL` is unset, the test should fail with a clear
   message telling the developer to set it up — not silently skip.

   Add a short `README.md` section or a `CONTRIBUTING.md` snippet documenting
   the test database setup.

7. **Group work into logical commits** (see the commit plan in §4 below). Do not
   lump everything into one giant commit.

## Phase 0: Produce a plan first

Before editing any file, read all eight specs plus every file under `src/` and
`migrations/`. Then produce a written plan that:

- Confirms your understanding of each task in §3
- Proposes the exact commit sequence (titles + one-line descriptions)
- Flags any place where you think a task is ambiguous, where a spec looks wrong,
  or where you want a human decision before proceeding

Stop and wait for human confirmation before executing the plan. Do not start
editing until the plan is approved.

## Phase 1: Tasks (in order)

### 1. Fix stale `Related` headers in specs

The following specs have `Related` headers that still say "(forthcoming)" or
"(pending revision)" for files that now exist:

- `specs/forge-v0.1.md` — `Related: stiglab-*, synodic-*, ising-* (forthcoming alignment)`
- `specs/stiglab-v0.1.md` — `Related: forge-v0.1.md, synodic-* (forthcoming)`
- `specs/artifact-model-v0.1.md` — `Related: onsager-vision-v0.1.md (pending revision), forge-v0.1.md (forthcoming)`

Update each `Related` header to reference the concrete files that now exist in
`specs/`. Do not touch any other part of these specs.

### 2. Update README to demote Telegramable from subsystem to adapter

`README.md` currently reads:

> sibling repos (`stiglab`, `synodic`, `ising`, `telegramable`) depend on it

The `specs/` directory contains subsystem specs only for Forge/Stiglab/Synodic/
Ising. Telegramable has no spec; it only holds a reserved namespace in
`events_ext`. Reflect this distinction in the README.

Change to:

> Subsystem repos (`stiglab`, `synodic`, `ising`) and adapter repos
> (`telegramable`, and future consumers) depend on it

Apply the same distinction wherever else the README or `CLAUDE.md` mentions
Telegramable alongside the subsystems.

### 3. Reconcile Synodic event naming across three sources

Three sources currently disagree on Synodic event type names:

- **`specs/synodic-v0.1.md` §9.1** lists 10 events like `synodic.gate_evaluated`,
  `synodic.escalation_started`, `synodic.rule_proposed`, etc.
- **`specs/event-stream-spine-v0.1.md` §8.3** lists 5 different names
  (`rule.created`, `rule.updated`, `rule.archived`, `escalation.opened`,
  `escalation.resolved`).
- **`src/factory_event.rs`** uses yet a third set (`synodic.rule_changed`,
  `synodic.escalation_resolved`) and only has two Synodic variants at all.

The source of truth is `synodic-v0.1.md §9.1`. Update the other two sources to
match:

- Update `event-stream-spine-v0.1.md §8.3` to list exactly the 10 events from
  synodic-v0.1 §9.1, with the same names.
- Update `src/factory_event.rs` to include all 10 Synodic events as
  `FactoryEventKind` variants, each with the correct `event_type()` string
  matching `synodic.*`.

### 4. Reconcile Stiglab event naming

`specs/stiglab-v0.1.md §5.3` declares 10 session/node events all prefixed
`stiglab.*`:

- `stiglab.session_created`, `stiglab.session_dispatched`, `stiglab.session_running`,
  `stiglab.session_completed`, `stiglab.session_failed`, `stiglab.session_aborted`,
  `stiglab.event_upgraded`, `stiglab.node_registered`, `stiglab.node_deregistered`,
  `stiglab.node_heartbeat_missed`

`src/factory_event.rs` currently has 4 session variants using the string prefix
`session.*` (without the `stiglab.` namespace). Align by:

- Update `src/factory_event.rs` variants to use the `stiglab.*` event_type strings.
- Add the 6 missing events as new variants.
- Update `event-stream-spine-v0.1.md §8.2` if it disagrees with stiglab-v0.1 §5.3.

The subsystem spec is the source of truth for its own events.

### 5. Reconcile Ising insight lifecycle events

`specs/ising-v0.1.md §7` describes an insight lifecycle
`detected → validated → forwarded → [crystallized]`, but
`specs/event-stream-spine-v0.1.md §8.4` declares event types
`insight.created`, `insight.forwarded`, `insight.crystallized`
(no `detected` or `validated`).

Align the lifecycle and event types. Recommended resolution:

- Treat `detected` and `validated` as internal Ising states that do not
  produce factory events (they are pre-spine). Only forwarding and
  crystallization (and any final insight record) become factory events.
- Update `ising-v0.1.md §7` to clarify that `detected` and `validated` are
  internal stages not emitted to the spine.
- Keep `event-stream-spine-v0.1.md §8.4` as it is, or expand to match whatever
  final list you settle on.
- Ensure `src/factory_event.rs` Ising variants match the final event list.

If this resolution feels wrong after reading the specs, surface the question in
Phase 0 instead of guessing.

### 6. Fix the `RuleProposed` namespace collision

`src/factory_event.rs` has a `RuleProposed` variant under Ising, but per
`synodic-v0.1 §9.1` there is also a `synodic.rule_proposed` event (the
crystallization candidate). These are two different events with similar names.

Disambiguate by renaming the Ising variant and adjusting its `event_type()`
string to make the producer obvious. Recommended names:

- Ising's: `IsingRuleProposed` → `ising.rule_proposed`
  (insight packaged as a rule proposal, sent to Synodic's crystallization pipeline)
- Synodic's: `SynodicRuleProposed` → `synodic.rule_proposed`
  (crystallization candidate that Synodic has accepted into review)

If the two are semantically the same event with different emitters, merge into
one and clarify in the spec. Otherwise keep them distinct but namespace-clear.

### 7. Delete `CoreEvent` entirely; unify on `FactoryEventKind`

Current state:
- `src/core_event.rs` defines an old `CoreEvent` enum (pre-Forge vocabulary:
  `TaskCreated`, `SessionCreated`, `SessionToolUse`, etc.).
- `src/factory_event.rs` defines the new `FactoryEventKind` enum (Forge-era,
  aligned with `forge-v0.1 §9`).
- `src/store.rs` `EventStore::append()` only accepts `&CoreEvent`, which means
  `FactoryEventKind` is defined but not wired to the store.
- Both enums are re-exported from `src/lib.rs`.

**The crate has not been published. There are no external consumers. Delete
the old event model outright.**

Required changes:

a. **Delete `src/core_event.rs` entirely.** Remove the module declaration from
   `src/lib.rs`. Remove the `pub use core_event::CoreEvent;` re-export.

b. **Delete the old `EventStore::append(&CoreEvent, ...)` method.** Replace it
   with `EventStore::append_factory_event(&self, event: &FactoryEvent, metadata: &EventMetadata) -> Result<i64, sqlx::Error>`.

   The new method serializes the `FactoryEvent` envelope into the `data` JSONB
   column and uses helper methods on `FactoryEventKind` to populate the columns:
   - `FactoryEventKind::event_type() -> &'static str` (already exists)
   - `FactoryEventKind::stream_type() -> &'static str` (already exists or add)
   - `FactoryEventKind::stream_id() -> &str` (add this — derive from the event
     payload, e.g., `ArtifactRegistered { artifact_id, .. }` returns
     `artifact_id.as_str()`; `ForgeDecisionMade` uses the artifact id; session
     events use the session id; etc.)

   There is no backward-compat `append` method. Callers must use
   `append_factory_event`.

c. **Do not carry over the three session-internal variants.** `CoreEvent` had
   `SessionWaitingInput`, `SessionOutput`, `SessionToolUse` — these are explicitly
   flagged as session-internal in `event-stream-spine-v0.1 §8.2` and should
   never have been on the factory spine. They do not exist in `FactoryEventKind`
   and must not be added during this migration.

d. **Update `examples/producer_consumer.rs`** to use
   `EventStore::append_factory_event` with a `FactoryEvent`. Pick any reasonable
   example event (e.g., `FactoryEventKind::ArtifactRegistered { ... }`).

e. **Update the README and `CLAUDE.md` code examples** to use
   `append_factory_event` and `FactoryEvent`. Do not leave any example showing
   the deleted `CoreEvent` API.

f. **Update or delete any tests that reference `CoreEvent`.** Replace them with
   equivalent tests using `FactoryEvent` / `FactoryEventKind`.

g. **Update `event-stream-spine-v0.1.md §14`** to reflect that `CoreEvent` has
   been removed. The section currently says the `CoreEvent` enum "needs to
   evolve" — update it to say it has been replaced by `FactoryEventKind`, and
   remove any references to `CoreEvent` from the implementation status table.

After this task completes, `grep -r CoreEvent` across the entire repo should
return zero results.

### 8. Add transactional writer helper

Invariant #3 in `forge-v0.1.md §10` (event durability via outbox pattern)
currently depends on developer discipline, not on the API. Fix this.

Add a method to `EventStore` that takes a closure and guarantees state change
and event append happen in the same transaction:

```rust
impl EventStore {
    pub async fn transaction<F, R>(&self, f: F) -> Result<R, sqlx::Error>
    where
        F: for<'c> FnOnce(&'c mut sqlx::Transaction<'_, sqlx::Postgres>)
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<R, sqlx::Error>> + Send + 'c>>;
}
```

Or a simpler shape if you prefer — the goal is: the caller writes state mutations
and event appends inside a single closure, the `EventStore` guarantees they are
either both committed or both rolled back.

Also add a helper that takes a `&mut Transaction` and appends a factory event
within it, so that within the closure the caller can append events without
reimplementing the insert SQL. Suggested shape:

```rust
pub async fn append_factory_event_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &FactoryEvent,
    metadata: &EventMetadata,
) -> Result<i64, sqlx::Error>
```

Add **integration tests** (not `#[ignore]`d) that demonstrate:

1. A commit case: a state-change SQL + an event append inside the closure both
   land in the database.
2. A rollback case: the closure returns `Err`, and neither the state change nor
   the event persists.
3. A panic case: the closure panics, and the transaction rolls back cleanly.

These tests require `DATABASE_URL` per §6 of the ground rules. They must fail
with a clear message if `DATABASE_URL` is unset.

### 9. Replace `ArtifactId::generate()` with ULID

`src/artifact.rs` `ArtifactId::generate()` currently uses
`&uuid::Uuid::new_v4().to_string()[..8]` — only 32 bits of entropy. Birthday
collision becomes likely around ~65K artifacts.

Replace with ULID:

1. Add `ulid` to `Cargo.toml` dependencies:
   ```toml
   ulid = { version = "1", features = ["serde"] }
   ```

2. Update `ArtifactId::generate()` to produce `art_<26-char-ulid>`, e.g.
   `art_01HXYZABC123DEFGHJKMNPQRS`. Use `ulid::Ulid::new().to_string()`.

3. Update the doc comment on `ArtifactId` to specify the new format exactly.

4. Add a unit test showing that 10,000 generated IDs are all unique.

5. Add a unit test showing that generated IDs preserve lexicographic time
   ordering (this is a ULID property and a reason to prefer it over UUIDv4).

ULID has two advantages over UUID here: (a) lexicographic sort matches time
order, which makes event spine browsing natural; (b) 128 bits of entropy, no
collision risk at any realistic scale.

### 10. Implement backfill in `Listener`

`specs/event-stream-spine-v0.1.md §7.2` requires the "streaming + backfill" pattern
as the recommended consumption model. `src/listener.rs` currently implements only
the streaming half — if a consumer starts after events have been written, those
events are lost.

Add a `since: Option<i64>` parameter (or a `BackfillCursor` type) to `Listener`
that, on startup:

1. Queries the `events` and `events_ext` tables for rows with `id > since`
2. Dispatches them through the same `EventHandler` that handles streaming events
3. Only then subscribes to `pg_notify`
4. On each handled event, tracks the latest `id` so a consumer can persist its
   own cursor for the next restart

The consumer is still responsible for *persisting* the cursor — `Listener` does
not manage storage. But it must accept a cursor on startup and expose the current
position.

Add an integration test (requiring `DATABASE_URL`) that:

1. Writes several events to the database.
2. Starts a `Listener` with `since = None` (fresh start).
3. Confirms all pre-existing events are dispatched via backfill.
4. Writes more events after the listener subscribes.
5. Confirms the new events are dispatched via pg_notify.
6. Restarts the listener with a cursor and confirms it only backfills events
   after the cursor.

Update `examples/producer_consumer.rs` to demonstrate the backfill pattern.

### 11. Misc minor fixes

These are small fixes from the review that don't deserve their own commit but
should be folded in where they naturally belong:

- `src/store.rs` line 67: `serde_json::to_value(event).unwrap_or_default()` —
  change to `.expect("event serialization must not fail")` or properly propagate
  the error. Silently falling back to `Value::Null` is wrong.
- `FactoryEventKind` does not derive `PartialEq` / `Eq`. Add them — they are
  useful for testing and impose no runtime cost. If any field type blocks this
  (e.g., `f64` for confidence), use `PartialEq` only and document why.
- `mpsc::unbounded_channel()` in `EventStore::subscribe()` — at minimum add a
  doc comment warning that slow consumers can cause memory growth. Ideally,
  add a `subscribe_bounded(capacity)` alternative that uses `mpsc::channel`
  with backpressure.

## Commit plan (suggested grouping)

Split the work into commits like this. Adjust grouping in your Phase 0 plan if
you think something else makes more sense.

1. `docs: fix stale Related headers in v0.1 specs` (task 1)
2. `docs: demote telegramable from subsystem to adapter in README` (task 2)
3. `specs: reconcile synodic event naming across specs and code` (task 3)
4. `specs: reconcile stiglab event naming with stiglab-v0.1 §5.3` (task 4)
5. `specs: clarify ising insight lifecycle vs factory events` (task 5)
6. `fix: disambiguate RuleProposed namespace collision` (task 6)
7. `refactor: delete CoreEvent and unify on FactoryEventKind` (task 7)
8. `feat: add transactional writer helper for outbox pattern` (task 8)
9. `fix: replace ArtifactId generation with ULID` (task 9)
10. `feat: implement backfill in Listener for missed pg_notify events` (task 10)
11. `fix: misc cleanup from v0.1 review` (task 11)

Each commit must independently pass `fmt`, `clippy`, and `test`. Do not rely on
"the next commit will fix it."

## Out of scope for this task

Do not attempt any of the following. They are either human decisions or v0.2
work:

- Publishing to crates.io
- Starting a sibling repo (stiglab, synodic, ising implementations)
- HA / multi-instance Forge coordination
- Branching artifact versions
- Event archival strategy
- Synodic rule DSL design

## When you are done

Produce a summary with:

- List of commits created with their titles and hashes
- Any spec questions you chose to surface instead of answering autonomously
- Any tasks you did not complete and why
- Any follow-up issues you think belong in a v0.1.1 milestone
- Confirmation that `grep -r CoreEvent` returns zero results across the repo
