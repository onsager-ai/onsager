// occam-allow: Observer runtime wiring lands with OBS-01 (#361). Until then the
// crate is a designed-for-future-use library with no in-tree reverse dep — the
// substrate scheduler will pull it in once the OBS-01 runtime spawns observer
// tasks. Remove this allow when the dep edge appears.
//! # onsager-observers
//!
//! The 0.2 substrate's **second citizen**: non-blocking observers that
//! audit the event stream and emit typed outputs without ever mutating
//! it. Where a [`Workflow`](onsager_substrate::Workflow) is the
//! managed-execution citizen (VSM S1/S2/S3), an [`Observer`] is the
//! audit citizen (VSM S3*). The substrate runs both — workflow nodes
//! and observers subscribe to the same spine, but observers run in
//! their own tasks off the hot path and write only to the separate
//! `observer_outputs` table.
//!
//! See [ADR 0013](../../../docs/adr/0013-observer-as-second-substrate-citizen.md).
//!
//! ## Surface
//!
//! - [`Observer`] — the trait. Implementors declare which spine event
//!   types they care about ([`subscriptions`](Observer::subscriptions))
//!   and produce typed outputs from each match ([`on_event`](Observer::on_event)).
//! - [`SpineEvent`] — what `on_event` sees: the wire `event_type`, the
//!   parsed [`FactoryEvent`](onsager_spine::FactoryEvent) payload, the
//!   spine row id (so emitted outputs can reference the triggering
//!   event), and the row's `created_at` timestamp.
//! - [`EventPattern`] — simple glob (`"artifact.*"`, `"*"`, exact
//!   match) used by [`subscriptions`](Observer::subscriptions). No
//!   full regex; v1 is wildly inclusive.
//! - [`ObserverOutput`] — the union of the three substrate-recognized
//!   output kinds: [`QualitySignal`](onsager_artifact::QualitySignal),
//!   [`Insight`], and [`Alert`].
//! - [`ObserverRuntime`] — the runtime. Holds a set of registered
//!   observers, subscribes to the spine, and fans events out: one
//!   `tokio::spawn` per (observer, event) match so a slow observer
//!   never blocks the substrate scheduler.
//! - [`ObserverOutputStore`] — Postgres-backed persistence for
//!   `ObserverOutput` rows. Schema lives at
//!   `crates/onsager-spine/migrations/028_observer_outputs.sql`.
//!
//! ## Constitutive properties
//!
//! Per ADR 0013, these are not optional:
//!
//! 1. **Non-blocking** — observers run in separate tasks; their work
//!    never delays workflow execution. The runtime fan-out spawns a
//!    fresh task per dispatch.
//! 2. **Cannot modify state** — the only writer surface this crate
//!    exposes is [`ObserverOutputStore`], which targets a *separate*
//!    table from spine business events. Observers do not have a
//!    handle to the spine `EventStore`.
//! 3. **Spine is the input** — the runtime is wired exclusively to
//!    [`EventStore::subscribe`](onsager_spine::EventStore::subscribe);
//!    there is no private coordination channel.
//! 4. **Output is typed** — `ObserverOutput` is a closed enum; the
//!    dashboard renders the three variants uniformly.

pub mod gate_deny_rate;
pub mod gate_override;
pub mod observer;
pub mod output;
pub mod pattern;
pub mod pr_churn;
pub mod runtime;
pub mod shape_retry;
pub mod store;

pub use gate_deny_rate::{GateDenyRateConfig, GateDenyRateObserver};
pub use gate_override::{GateOverrideConfig, GateOverrideObserver};
pub use observer::{Observer, SpineEvent};
pub use output::{Alert, AlertSeverity, Insight, ObserverOutput, ObserverOutputKind};
pub use pattern::EventPattern;
pub use pr_churn::{PrChurnConfig, PrChurnObserver};
pub use runtime::ObserverRuntime;
pub use shape_retry::{ShapeRetryConfig, ShapeRetryObserver};
pub use store::{ObserverOutputRecord, ObserverOutputStore, StoreError};

// Convenience re-export so callers building outputs don't have to
// reach across crates for the third variant.
pub use onsager_artifact::QualitySignal;
