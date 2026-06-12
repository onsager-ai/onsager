# Warehouse & Delivery — Onsager v0.1

**Status**: Draft
**Owner**: Marvin
**Last updated**: 2026-04-17
**Depends on**: `artifact-model-v0.1`, `forge-v0.1`
**Related**: `synodic-v0.1`, `stiglab-v0.1`

---

## 1. Purpose

Today, when an artifact reaches `Released`, Forge marks a database row and stops. No content leaves the factory. This spec defines the two missing pieces that turn a release into an actual delivery:

1. **Warehouse** — an internal, content-addressed store where Forge seals artifact outputs into immutable **bundles** before they ship. Bundles are what the factory physically produces; they live inside Onsager and are retained for lineage, recall, and audit.
2. **Delivery** — a portable, pluggable interface (`Consumer`) that ships bundles to external systems (GitHub, webhook, S3, deploy targets) and tracks per-consumer receipts with independent retry.

Together they complete the factory metaphor: production fills the warehouse; delivery leaves the warehouse.

---

## 2. Non-goals

- Does **not** replace the artifact model. `Artifact` keeps its mutable identity, owner, consumers, and state.
- Does **not** define which external systems Onsager integrates with beyond an initial set. Consumer implementations are additive.
- Does **not** specify a particular object-store backend. The warehouse abstracts over filesystem / Postgres large objects / S3-compatible stores.
- Does **not** expand Forge's responsibilities — Forge writes to the warehouse and enqueues deliveries; it does not operate either subsystem.

---

## 3. Mental model

```
      ┌─ production ─────────────┐   ┌─ storage ─┐   ┌─ delivery ────────────┐
      │                          │   │           │   │                       │
      │  Forge → Stiglab         │   │ Warehouse │   │  Consumer trait       │
      │   (shape artifact)       │──►│  (bundles)│──►│   GitHub              │
      │                          │   │ immutable │   │   Webhook             │
      │                          │   │ content-  │   │   S3                  │
      │                          │   │ addressed │   │   Filesystem          │
      └──────────────────────────┘   └───────────┘   └───────────────────────┘
          mutable artifact              sealed             per-consumer
            identity                    bundles            deliveries
            (v1, v2, v3...)             (immutable)        (with retries)
```

The **artifact** is the logical product — it persists across reworks and carries the identity users talk about ("my-api-service"). The **bundle** is a sealed snapshot of what the factory produced at one release. Many bundles per artifact over its lifetime; one bundle is never rewritten.

---

## 4. Data model

### 4.1 Artifact (existing, augmented)

```
current_bundle_id: Option<BundleId>   // pointer; advances on each release
bundle_history: [BundleId]            // ordered, append-only
```

Artifact identity, owner, kind, state, and consumer list remain mutable and keep their existing semantics.

### 4.2 Bundle (new, immutable)

```
BundleId                       // content hash of the manifest
artifact_id                    // which logical product this is a version of
version: u32                   // monotonic per artifact (1, 2, 3...)
supersedes: Option<BundleId>   // prior bundle, if any
manifest: Manifest             // file list, sizes, per-entry content hashes
content_ref: URI               // pointer into the warehouse backend
sealed_at: Timestamp
sealed_by: SessionId           // the stiglab session that produced it
metadata: Map<String, Value>   // kind, build provenance, etc.
```

A bundle is **immutable once sealed**. Sealing is atomic: either the manifest and all content blobs are written and the bundle row is inserted, or the operation fails and nothing is persisted.

### 4.3 Delivery (new)

```
DeliveryId
bundle_id
consumer_id
kind: Initial | Rework         // see §6
prior_receipt: Option<Receipt> // for Rework: what the consumer got last time
status: Pending | InFlight | Succeeded | Failed | Abandoned
attempts: u32
last_error: Option<String>
receipt: Option<Receipt>       // returned by the consumer on success
created_at, updated_at
```

Deliveries are independent of the pipeline. A delivery can retry, fail, or be abandoned without changing the artifact's state.

### 4.4 Consumer (new)

```
ConsumerId
artifact_id                    // or tenant-scoped, see §11
kind: GitHub | Webhook | S3 | Filesystem | ...
config: ConsumerConfig         // kind-specific
retry_policy: RetryPolicy
enabled: bool
```

---

## 5. Lifecycle

### 5.1 Production → seal

1. Forge dispatches shaping via Stiglab (unchanged).
2. On `ShapingResult::Complete`, Forge calls `warehouse.seal(artifact_id, outputs)`.
3. Warehouse computes content hashes, writes blobs to the backend, writes the manifest, inserts the `Bundle` row atomically, and returns `BundleId`.
4. Forge advances artifact state to `Released` and sets `current_bundle_id`.
5. Forge emits `FactoryEvent::BundleSealed { artifact_id, bundle_id }`.

Sealing is a Synodic gate point: Synodic may inspect the bundle contents before it is promoted to `current`. (Adds a fourth gate point to `forge-v0.1 §6.1`.)

### 5.2 Seal → enqueue delivery

For each enabled consumer on the artifact, Forge inserts a `Delivery` row with `status: Pending`. Forge does not ship. A separate delivery worker consumes `BundleSealed` events and drives the delivery state machine.

### 5.3 Delivery worker

- Picks up `Pending` deliveries, marks `InFlight`, calls `consumer.deliver(bundle, kind, prior_receipt)`.
- On success: writes `Receipt`, sets `Succeeded`, emits `FactoryEvent::DeliverySucceeded`.
- On failure: applies `retry_policy`, reschedules or marks `Abandoned`, emits `FactoryEvent::DeliveryFailed`.

Deliveries are **at-least-once**. Consumers must be idempotent keyed on `(bundle_id, consumer_id)`.

---

## 6. Rework

Rework is the mechanism that preserves mutable *product* identity on top of immutable *bundle* content.

### 6.1 Trigger

- A user action ("rework this artifact") in the console
- An Ising insight with `suggested_action: Rework`
- A Synodic escalation outcome

### 6.2 Transition

```
Released ──rework──► InProgress
```

The artifact re-enters the pipeline. `current_bundle_id` is not cleared — it still points at the last released bundle. The rework request carries a `reason` recorded in factory events.

### 6.3 Sealing a rework

When the rework completes, a new bundle `v(n+1)` is sealed with `supersedes: v(n)`. On success:

- `current_bundle_id` advances to `v(n+1)`
- `bundle_history` appends `v(n+1)`
- Deliveries are enqueued with `kind: Rework` and `prior_receipt` set to the consumer's most recent successful receipt for this artifact (if any)

### 6.4 What consumers do with rework

- **GitHub**: open a follow-up PR referencing `prior_receipt.pr_url`, or push a commit to the same branch if the prior PR is still open.
- **Webhook**: POST with `event: rework`, `prior_receipt` in the payload.
- **S3**: write the new bundle under its content hash; update the `latest/` alias for the artifact; do not delete prior objects.
- **Deploy**: issue a new deployment; leave prior deployment revision retrievable for rollback.

Consumers MAY refuse a rework (e.g., if the prior delivery hasn't been accepted yet) and return `RejectRework` in the receipt — the delivery is marked `Abandoned` with a reason, and the bundle stays in the warehouse available for a later retry or a different consumer.

### 6.5 No mutation

Rework never mutates a prior bundle. Recall is achieved by advancing the current pointer and issuing a superseding bundle, not by rewriting history. This is the single most important invariant of this spec (§9.1).

---

## 7. Consumer contract

```rust
#[async_trait]
trait Consumer {
    async fn deliver(
        &self,
        bundle: &Bundle,
        kind: DeliveryKind,
        prior_receipt: Option<&Receipt>,
    ) -> Result<Receipt, DeliveryError>;

    fn kind(&self) -> ConsumerKind;
    fn validate_config(config: &ConsumerConfig) -> Result<(), ConfigError>;
}
```

`Receipt` is a typed enum per consumer kind, serialized for storage:

```rust
enum Receipt {
    GitHub { pr_url: String, commit_sha: String, branch: String },
    Webhook { status: u16, response_id: Option<String> },
    S3 { key: String, version_id: Option<String>, etag: String },
    Filesystem { path: PathBuf },
    // ...
}
```

`DeliveryError` distinguishes **retryable** (network, rate limit) from **terminal** (bad config, permission denied) so the retry policy can behave correctly.

---

## 8. Warehouse backend

The warehouse is backend-agnostic. v0.1 ships one backend:

- **`filesystem`** — content blobs under `$ONSAGER_WAREHOUSE_ROOT/blobs/`, manifests in Postgres. Default for local dev.

Planned but out of scope for v0.1:

- **`postgres-lo`** — content in Postgres large objects. Zero external infra; suitable for small deployments.
- **S3/R2** — for larger deployments.

Backend is configured via env var; Forge and the delivery worker only see the `Warehouse` trait:

```rust
#[async_trait]
trait Warehouse {
    async fn seal(&self, request: SealRequest) -> Result<Bundle, SealError>;
    async fn fetch(&self, bundle_id: &BundleId) -> Result<Bundle, FetchError>;
    async fn exists(&self, bundle_id: &BundleId) -> Result<bool, FetchError>;
}
```

where `SealRequest` carries `artifact_id`, `sealed_by`, operator metadata, and
the `Outputs` list. The trait returns a full [`Bundle`] — id, version, manifest,
and timestamps — so callers do not need a follow-up fetch.

---

## 9. Invariants

1. **Bundle immutability**: once `sealed_at` is set, no field of a bundle or any of its content blobs may change.
2. **Monotonic versions**: for a given artifact, `version` is strictly increasing; no gaps, no reuse.
3. **Supersession chain**: `supersedes` forms a linear chain per artifact, not a DAG. Every bundle after v1 supersedes exactly one prior bundle.
4. **Current pointer advances only**: `current_bundle_id` only moves to bundles newer (by `version`) than the one it currently points to.
5. **Delivery independence**: delivery state never affects artifact state. A permanently failing consumer does not block future reworks or other consumers.
6. **At-least-once**: every sealed bundle produces at least one delivery attempt per enabled consumer. Consumers idempotent on `(bundle_id, consumer_id)`.
7. **No silent failures**: every seal, delivery attempt, success, and failure emits a factory event.

---

## 10. Governance hooks

- **Pre-seal gate** (new): Synodic may inspect bundle contents before sealing — e.g., reject a bundle containing secrets.
- **Pre-deliver gate** (replaces `forge-v0.1 §6.1` "Consumer routing"): Synodic may redact, delay, or block delivery per-consumer. Redaction produces a new bundle (never a mutation of the original) with `metadata.redacted_from = original_bundle_id`.
- **Rework gate**: Synodic may require escalation before a rework may be initiated on a released artifact with live downstream receipts.

---

## 11. Open questions

1. **Consumer scope** — are consumers per-artifact, per-artifact-kind, or tenant-wide with selectors? v0.1 proposes per-artifact for clarity, but fleet-wide consumers (e.g., "mirror every released bundle to S3") are a likely v0.2 requirement.
2. **Retention** — how long are old bundles kept in the warehouse? Forever (immutability) or with a configurable retention policy that archives cold bundles? v0.1 proposes forever; revisit when storage pressure is measured.
3. **Partial bundles** — can a `ShapingResult::Partial` seal a bundle, or must sealing only happen on `Complete`? v0.1 proposes Complete-only; Partials do not reach the warehouse.
4. **Cross-artifact composition** — can a bundle reference blobs from another bundle (dedup / composition)? v0.1 says no; each bundle is self-contained. Content-addressing already dedups blobs at the backend level.
5. **Delivery ordering** — must deliveries respect bundle version order per consumer (v2 never arrives before v1's success)? v0.1 proposes yes; makes rework semantics coherent. Requires per-`(artifact, consumer)` serialization in the delivery worker.
6. **Recall** — is there a first-class "recall" operation that marks a prior bundle as defective without producing a rework? Likely yes in v0.2; v0.1 treats recall as "rework with reason=Recall".

---

## 12. Implementation order

Suggested v0.1 slicing, each piece independently shippable:

1. Warehouse trait + filesystem backend + `Bundle` data model + `seal` call in Forge's release path.
2. `current_bundle_id` pointer + `bundle_history` on Artifact; factory event `BundleSealed`.
3. Rework transition `Released → InProgress` + re-seal path.
4. `Consumer` trait + `Delivery` table + delivery worker (no real consumers yet; dummy sink for tests).
5. First real consumer: **GitHub** (PR-per-bundle, follow-up PR on rework).
6. Second consumer: **Webhook** (generic, unblocks operator-built integrations).
7. Synodic pre-seal and pre-deliver gates.
8. Postgres-lo backend; S3 backend.
