//! [`Observer`] trait — the substrate's audit citizen.
//!
//! An observer is **stateful and serial per instance**: the runtime
//! calls [`on_event`](Observer::on_event) with `&mut self`, so an
//! observer can accumulate state across events (running counts,
//! moving averages, simple memory of "what did the last 10 events
//! look like"). The runtime serializes calls to one observer behind
//! a mutex; observers from each other run fully in parallel.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use onsager_spine::FactoryEvent;

use crate::output::ObserverOutput;
use crate::pattern::EventPattern;

/// The event shape an observer sees on each dispatch.
///
/// Carries the parsed [`FactoryEvent`] payload (so observers can
/// match on typed variants instead of re-parsing JSON) plus the
/// spine-row metadata observers reference when emitting outputs —
/// `event_id` becomes the `triggered_by_event_id` link an emitted
/// [`Insight`](crate::Insight) or [`Alert`](crate::Alert) carries
/// back to the row that caused it.
#[derive(Debug, Clone)]
pub struct SpineEvent {
    /// The `events.id` of the row on the spine.
    pub event_id: i64,
    /// Wire event type (`"artifact.state_changed"`, `"node.completed"`,
    /// ...). Same value the
    /// [`EventPattern`] matches against; passed through alongside the
    /// parsed payload so a simple observer can branch on the string
    /// without re-deriving it.
    pub event_type: String,
    /// Full parsed factory-event payload.
    pub payload: FactoryEvent,
    /// Timestamp the spine row was written.
    pub created_at: DateTime<Utc>,
}

/// An observer of substrate events.
///
/// Implementors declare a set of [`EventPattern`]s in
/// [`subscriptions`](Self::subscriptions) and produce
/// [`ObserverOutput`]s from each matching event in
/// [`on_event`](Self::on_event). The runtime guarantees:
///
/// - Each `on_event` call is delivered in its own `tokio::spawn`
///   task — slow observers do not block other observers or the
///   substrate scheduler.
/// - Per-observer calls are serialized — two events that both match
///   one observer are processed in `event_id` order, one at a time,
///   even though different observers run in parallel.
/// - The runtime persists every returned [`ObserverOutput`] before
///   moving on; observers do not have direct DB access.
#[async_trait]
pub trait Observer: Send + Sync {
    /// Event patterns this observer wants to receive. Returning an
    /// empty list disables the observer (no events match).
    fn subscriptions(&self) -> Vec<EventPattern>;

    /// Process a matching spine event.
    ///
    /// Returns the outputs to persist — usually 0 or 1, but observers
    /// may emit several (e.g. an `Alert` plus the `QualitySignal`
    /// that triggered it).
    async fn on_event(&mut self, event: &SpineEvent) -> Vec<ObserverOutput>;

    /// Lookback window the runtime should replay through this
    /// observer on startup, before the runtime signals ready and
    /// begins consuming live `pg_notify` notifications. (The
    /// subscription itself is attached *first* so notifications are
    /// buffered while hydration runs; the runtime then skips the
    /// buffered notifications that hydration already covered. See
    /// [`ObserverRuntime::run_with_ready`](crate::ObserverRuntime::run_with_ready)
    /// for the full sequence.) Returning `Some(d)` opts in: the
    /// runtime fetches events written in the last `d` and dispatches
    /// them through `on_event` to rebuild observer state (artifact-id
    /// → kind indices, sliding-window buffers, …) that would otherwise
    /// be empty after a process restart.
    ///
    /// Outputs produced during hydration are **suppressed** — the
    /// runtime drops everything `on_event` returns while replaying
    /// history, so an observer does not need to know whether it is
    /// being hydrated. Observers whose `on_event` mutates external
    /// state directly should still avoid doing so (the trait already
    /// asks observers not to — outputs are the only sanctioned write
    /// surface).
    ///
    /// Default: `None` — no hydration. Appropriate for stateless
    /// observers and for ones whose state is bounded by a window
    /// shorter than a typical restart gap (where the cold-start
    /// correctness gap is acceptable).
    ///
    /// See spec #392 for the runtime-side mechanics and the
    /// correctness rationale.
    fn hydration_window(&self) -> Option<Duration> {
        None
    }
}
