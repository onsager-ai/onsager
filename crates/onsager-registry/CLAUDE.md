# Onsager registry

Factory pipeline registry — the single source of truth for what types,
adapters, gate evaluators, agent profiles, and **event types** exist in
the system. Per [ADR 0003](../../docs/adr/0003-registry-as-source-of-truth.md)
the registry crate is the authoritative catalog; runtime tables are
projections of the registry's mutation events.

## Contents

- `catalog.rs` — built-in seed catalog (artifact kinds, workflow kinds).
- `evaluators.rs` — gate evaluator trait and registry plug points.
- `events.rs` — **event-type registry manifest** (spec #131 Lever E / #150).
- `registry.rs` — `TypeDefinition`, `RegistryId`, `RegistryStatus`, etc.
- `registry_store.rs` — DB projection / CRUD.
- `seed.rs` — idempotent seed loader.

## Event manifest update process (#150, schema simplified by #272)

`events.rs` carries `pub const EVENTS: EventManifest`, a static, human-
reviewed table of every `FactoryEventKind` variant. Each row declares:

- `kind` — wire `event_type` string (matches `FactoryEventKind::event_type()`).
- `schema_version` — bumped on backwards-incompatible payload changes.
  Additive `Option<T>` fields with `#[serde(default,
  skip_serializing_if = "Option::is_none")]` do **not** bump.
- `producers` — subsystems that emit this event onto the spine.
  `Subsystem::Portal` is the catch-all for `onsager-portal` /
  webhook-driven emitters.
- `consumers` — subsystems that act on this event (dispatch off the
  `event_type` string and parse the payload). Dashboard-only reads do
  not count.
- `diagnostic_only` — `true` when no subsystem consumer is expected
  (the event is read by a non-subsystem concern: dashboard timeline,
  audit trail). Must be paired with a non-empty `reason`. Per spec
  #272 this replaces the prior `audit_only` flag.
- `reason` — `Option<&'static str>` explaining what reads this event
  today (e.g. `"rendered in dashboard event timeline"`). Required
  when `diagnostic_only` is `true`; `None` for real rows. Free-form
  by design.
- `tracking_issue` — `Option<u32>` pointing at the follow-up issue
  that tracks moving this row from diagnostic-only to a real
  consumer (or removing it). Surfaced by `xtask check-events` as a
  warn-mode signal per spec #275; ratchet to required for
  diagnostic-only rows is a follow-up.

Every row is in one of two states: **real** (non-empty `consumers`)
or **diagnostic-only** (`diagnostic_only: true` plus a non-empty
`reason`). Rows that are neither are rejected at lint time.

### When to update

| Change | Action |
| --- | --- |
| Add a `FactoryEventKind` variant | Add a row to `EVENTS` in the same PR. Producer + consumer must be wired, or the event marked `diagnostic_only: true` with a non-empty `reason`. |
| Add a new producer for an existing event | Append the subsystem to `producers`. |
| Add a new listener for an existing event | Append the subsystem to `consumers`; flip `diagnostic_only` to `false` and set `reason: None` if it was set. |
| Backwards-incompatible payload change | Bump `schema_version`. Producer + consumer support for the new version must ship together — coordinated via PRs / tests / review (`check-events` only enforces that a manifest row has ≥1 producer and is either real or diagnostic-only with a reason; it does not reason about `schema_version`). |
| Backwards-compatible payload change (new optional field) | No version bump; manifest review still required — flag the change in the PR description. |
| Remove a `FactoryEventKind` variant | Remove the manifest row in the same PR. |

### Schema-version policy

`schema_version` is a strictly increasing `u32` per event type. Skipping
versions is allowed (e.g. `1 → 3`); going backwards is not. The version
exists so consumers can branch on it during a rolling upgrade — once
all consumers have shipped past version `N-1`, version `N-1` rows can
be deleted from `events` if needed.

### Even-non-bumping payload edits need review

A `serde(default)`-guarded additive field is wire-compatible, but it
still changes the contract every consumer reads. Edits to a payload
field — even additive ones — must bump nothing on the manifest yet
**must** include a manifest review (the PR diff touches `events.rs` or
the producer/consumer fields are stale). Reviewers should ask: does the
new field land with both a producer that sets it and a consumer that
reads it? If not, hold the PR or mark the event `diagnostic_only: true`
with a `reason`.

### CI enforcement

`cargo run -p xtask -- check-events` runs in CI (see
`.github/workflows/rust.yml`) and asserts:

1. Every `FactoryEventKind` variant has a manifest row.
2. Every manifest row declares ≥1 producer and is either real (≥1
   consumer) or diagnostic-only (`diagnostic_only = true` plus a
   non-empty `reason`).
3. Every `append_ext(_, _, "<event_type>", ...)` literal under
   `crates/{forge,stiglab,synodic,ising}/src/` references an event whose
   `producers` list includes that subsystem.
4. Every `notification.event_type [!=|==] "<event_type>"` filter under
   the same source trees references an event whose `consumers` list
   includes that subsystem.

Tests (modules guarded by `#[cfg(test)]`) are excluded from checks 3
and 4. Helper-style emitters that pass `event_type` as a variable
rather than a literal (e.g. stiglab's generic
`SpineEmitter::emit(FactoryEventKind)`) are not detected by check 3 —
that's an accepted false negative; the manifest is still the source of
truth and reviewers verify the producer subsystem matches.

### Read API

The manifest is exposed at `GET /api/registry/events` (portal owns
this route as of #257) so the dashboard can render the catalog
without a hardcoded copy. Public by design — same pattern as
`/api/workflow/kinds`.

## Related

- ADR 0001 — event-bus coordination model.
- ADR 0003 — registry as source of truth.
- ADR 0004 — six-lever seam-tightening plan.
- Spec #131 — strategic plan; this manifest is **Lever E**.
- Spec #150 — this lever's implementation.
- Spec #272 — manifest schema simplification (drop `audit_only`,
  introduce `diagnostic_only` + `reason`, prune dangling wires).
