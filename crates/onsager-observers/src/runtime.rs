//! [`ObserverRuntime`] — drives the observer fan-out from the spine.
//!
//! Lifecycle:
//!
//! 1. Caller builds an `ObserverRuntime` with an `EventStore` and an
//!    `ObserverOutputStore`.
//! 2. Caller [`register`](ObserverRuntime::register)s zero or more
//!    observers, each with a stable string id.
//! 3. Caller spawns [`run`](ObserverRuntime::run) on a tokio task. The
//!    loop subscribes to `pg_notify`, fetches each row's full
//!    `FactoryEvent` payload, and fans out to matching observers —
//!    one `tokio::spawn` per (observer, event), so a slow observer
//!    never blocks another observer or the substrate scheduler.
//!
//! ## Per-observer concurrency
//!
//! Each observer instance lives behind a `tokio::sync::Mutex`. At
//! most one `on_event` call to a given observer runs at a time —
//! observers can safely keep `&mut self` state without external
//! locking. Across different observers, dispatches run fully in
//! parallel.
//!
//! **Ordering note.** Per-observer calls are serialized but not
//! guaranteed FIFO in `event_id` order: `tokio::sync::Mutex` does
//! not promise FIFO wakeup, so two events arriving close in time
//! may be processed by one observer in either order. Observers that
//! genuinely need monotonic ordering must inspect `event_id` /
//! `created_at` themselves; for the v1 audit use case the
//! at-most-one-at-a-time guarantee is what callers depend on.
//!
//! ## Backpressure
//!
//! By default the runtime subscribes via [`EventStore::subscribe`],
//! which is unbounded — fine for the audit workload most observers
//! produce, but a slow observer behind dozens of registered
//! observers can grow memory unboundedly. Use
//! [`ObserverRuntime::with_capacity`] to switch to
//! [`EventStore::subscribe_bounded`] with a fixed-capacity channel;
//! the substrate writer will block once the buffer fills, applying
//! backpressure upstream rather than buffering forever.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Duration;
use onsager_spine::{EventNotification, EventStore, FactoryEvent};
use tokio::sync::{Mutex, oneshot};

use crate::observer::{Observer, SpineEvent};
use crate::pattern::EventPattern;
use crate::store::ObserverOutputStore;

/// One observer registered with the runtime.
///
/// `id` is the stable identifier persisted on every emitted
/// [`ObserverOutput`](crate::ObserverOutput)'s row. Callers should
/// pick a short, dotted slug — `"ising.flaky_test"`,
/// `"obs.workflow_latency"` — that survives across restarts.
struct Registered {
    id: String,
    patterns: Vec<EventPattern>,
    /// Cached at registration so the hydration phase doesn't have to
    /// lock the observer mutex (and so each observer's window is read
    /// exactly once — the spec is "declare alongside subscriptions",
    /// not "ask repeatedly").
    hydration_window: Option<Duration>,
    observer: Arc<Mutex<Box<dyn Observer>>>,
}

/// The observer runtime.
///
/// Built with [`new`](Self::new), populated with
/// [`register`](Self::register), and driven by [`run`](Self::run).
/// Cheap to construct; the cost is the live spine subscription
/// `run` opens.
pub struct ObserverRuntime {
    event_store: EventStore,
    output_store: ObserverOutputStore,
    observers: Vec<Registered>,
    /// `Some(n)` switches the spine subscription to the
    /// bounded variant with capacity `n`; `None` keeps the unbounded
    /// default. See [`ObserverRuntime::with_capacity`].
    capacity: Option<usize>,
}

impl ObserverRuntime {
    /// Wire the runtime to an existing spine and output store.
    pub fn new(event_store: EventStore, output_store: ObserverOutputStore) -> Self {
        Self {
            event_store,
            output_store,
            observers: Vec::new(),
            capacity: None,
        }
    }

    /// Switch the spine subscription to the bounded variant with the
    /// given channel capacity. The substrate writer blocks once the
    /// buffer fills, applying backpressure upstream — appropriate
    /// when observers are expected to lag (large fan-outs, slow DB
    /// writes). Without this, the runtime uses
    /// [`EventStore::subscribe`] (unbounded).
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }

    /// Register one observer under a stable id. Chainable.
    ///
    /// `id` is what the runtime writes into `observer_outputs.observer_id`
    /// — pick something stable across restarts. The observer's
    /// declared [`subscriptions`](Observer::subscriptions) and
    /// [`hydration_window`](Observer::hydration_window) are read
    /// once and cached; observers cannot change either at runtime.
    pub fn register<O: Observer + 'static>(mut self, id: impl Into<String>, observer: O) -> Self {
        let patterns = observer.subscriptions();
        let hydration_window = observer.hydration_window();
        self.observers.push(Registered {
            id: id.into(),
            patterns,
            hydration_window,
            observer: Arc::new(Mutex::new(Box::new(observer))),
        });
        self
    }

    /// Register an observer that is already boxed. Useful when the
    /// caller stores observer instances by name in a registry of
    /// their own.
    pub fn register_boxed(mut self, id: impl Into<String>, observer: Box<dyn Observer>) -> Self {
        let patterns = observer.subscriptions();
        let hydration_window = observer.hydration_window();
        self.observers.push(Registered {
            id: id.into(),
            patterns,
            hydration_window,
            observer: Arc::new(Mutex::new(observer)),
        });
        self
    }

    /// Register every observer that ships with the crate. Picks
    /// stable ids (`obs.<analyzer>`) that the runtime persists onto
    /// every emitted output's row; deployments that want a different
    /// shape should call [`register`](Self::register) directly.
    ///
    /// The set mirrors the analyzers Ising shipped in 0.1 (`#362` /
    /// OBS-02): gate-override rate, gate-deny rate, shape-retry
    /// spike, PR churn. Each uses its `Default` configuration; pass
    /// a custom `Config` via [`register`](Self::register) for
    /// non-default tuning.
    pub fn default_observers(self) -> Self {
        use crate::gate_deny_rate::GateDenyRateObserver;
        use crate::gate_override::GateOverrideObserver;
        use crate::pr_churn::PrChurnObserver;
        use crate::shape_retry::ShapeRetryObserver;

        self.register("obs.gate_override", GateOverrideObserver::default())
            .register("obs.gate_deny_rate", GateDenyRateObserver::default())
            .register("obs.shape_retry_spike", ShapeRetryObserver::default())
            .register("obs.pr_churn", PrChurnObserver::default())
    }

    /// Number of registered observers. Mostly useful for tests.
    pub fn observer_count(&self) -> usize {
        self.observers.len()
    }

    /// Run the subscription loop. Only returns when the underlying
    /// `pg_notify` channel closes or the spine fails.
    pub async fn run(self) -> Result<()> {
        self.run_with_ready(None).await
    }

    /// Like [`run`](Self::run), but signals on `ready` once the spine
    /// subscription is attached, history has been replayed, and the
    /// fan-out loop is about to consume its first live notification.
    /// Tests use this to wait for readiness deterministically instead
    /// of sleeping a fixed duration. Dropping the receiver is fine —
    /// the runtime simply proceeds.
    ///
    /// Startup sequence (spec #392):
    ///
    /// 1. Subscribe to `pg_notify`. The live channel starts
    ///    buffering immediately so events written during hydration
    ///    are not lost.
    /// 2. Capture
    ///    [`max_event_id`](onsager_spine::EventStore::max_event_id)
    ///    as a cutoff.
    /// 3. For every registered observer that opted into hydration
    ///    via [`hydration_window`](Observer::hydration_window),
    ///    replay events in `[now - window, cutoff]` through
    ///    `on_event` — but suppress the outputs. Observer state
    ///    (artifact-id → kind indices, sliding-window buffers, …) is
    ///    rebuilt to the shape it would have had if the observer had
    ///    been online over the window.
    /// 4. Signal `ready`.
    /// 5. Drive the live loop, skipping events with `id <= cutoff`
    ///    (those were already hydrated).
    pub async fn run_with_ready(self, ready: Option<oneshot::Sender<()>>) -> Result<()> {
        let mut rx_unbounded;
        let mut rx_bounded;
        let recv: &mut dyn AnyReceiver = match self.capacity {
            None => {
                rx_unbounded = self
                    .event_store
                    .subscribe()
                    .await
                    .context("subscribe to spine pg_notify")?;
                &mut rx_unbounded
            }
            Some(cap) => {
                rx_bounded = self
                    .event_store
                    .subscribe_bounded(cap)
                    .await
                    .context("subscribe_bounded to spine pg_notify")?;
                &mut rx_bounded
            }
        };

        // Cutoff captured AFTER subscribe so any event with id >
        // cutoff is guaranteed to also arrive via the live channel
        // (subscribe started first, channel is buffering). Events
        // with id <= cutoff are hydrated below and skipped when the
        // live loop sees them.
        let cutoff_id = self
            .event_store
            .max_event_id()
            .await
            .context("read max_event_id for hydration cutoff")?
            .unwrap_or(0);

        // Replay history through hydrating observers. Outputs are
        // suppressed — see `hydrate_observers`.
        hydrate_observers(&self.observers, &self.event_store, cutoff_id).await?;

        // Subscription attached AND hydration complete — only now
        // is the runtime really "ready". Test waiters rely on this
        // ordering.
        if let Some(tx) = ready {
            let _ = tx.send(());
        }

        let registered: Arc<Vec<Registered>> = Arc::new(self.observers);
        let event_store = self.event_store;
        let output_store = self.output_store;

        while let Some(notification) = recv.recv().await {
            // Cheap pre-filter: observers see core spine events only
            // (`events_ext` carries subsystem-private payloads that are
            // not `FactoryEvent`-shaped). Doing this once here saves
            // one `tokio::spawn` + one `SELECT FROM events WHERE id` per
            // observer per ignored notification.
            if notification.table != "events" {
                continue;
            }
            // Skip events already replayed during hydration. A
            // notification with `id <= cutoff_id` is one of:
            //  - a row written before subscribe attached, then
            //    captured by the cutoff and replayed by
            //    `hydrate_observers`;
            //  - a row written between subscribe and the cutoff read
            //    (also replayed — the cutoff is post-subscribe).
            // Either way the observer state already accounts for it.
            if notification.id <= cutoff_id {
                continue;
            }
            // Dispatch each matching observer in its own task so a
            // slow observer cannot back up the channel.
            for obs in registered.iter() {
                if !any_pattern_matches(&obs.patterns, &notification.event_type) {
                    continue;
                }
                let observer = Arc::clone(&obs.observer);
                let observer_id = obs.id.clone();
                let event_store = event_store.clone();
                let output_store = output_store.clone();
                let notification = notification.clone();
                tokio::spawn(async move {
                    if let Err(e) = dispatch_one(
                        &observer_id,
                        observer,
                        notification,
                        event_store,
                        output_store,
                    )
                    .await
                    {
                        tracing::error!(observer = %observer_id, error = %e, "observer dispatch failed");
                    }
                });
            }
        }

        tracing::warn!("observer runtime: pg_notify channel closed, shutting down");
        Ok(())
    }

    /// Dispatch a single event directly, bypassing the spine
    /// subscription. Exposed for tests and for use cases that
    /// already have a parsed `SpineEvent` in hand (e.g. replaying
    /// from history). Uses the same per-observer mutex as the live
    /// loop.
    pub async fn dispatch_for_test(&self, event: SpineEvent) -> Vec<i64> {
        let mut output_ids = Vec::new();
        for obs in &self.observers {
            if !any_pattern_matches(&obs.patterns, &event.event_type) {
                continue;
            }
            let mut guard = obs.observer.lock().await;
            let outputs = guard.on_event(&event).await;
            drop(guard);
            for out in outputs {
                match self
                    .output_store
                    .record(&obs.id, Some(event.event_id), &out)
                    .await
                {
                    Ok(id) => output_ids.push(id),
                    Err(e) => {
                        tracing::error!(observer = %obs.id, error = %e, "persist observer output failed");
                    }
                }
            }
        }
        output_ids
    }

    /// Replay a pre-built list of historical [`SpineEvent`]s through
    /// the runtime's observers, with **outputs suppressed**.
    ///
    /// Exposed primarily for tests that want to drive the hydration
    /// path without standing up a live spine subscription. The
    /// `run_with_ready` startup path uses the same dispatch shape
    /// after fetching events via the spine.
    ///
    /// Events should be supplied in `event_id` ASC order; the runtime
    /// dispatches them as-is and observers that depend on monotonic
    /// ordering rely on the caller's ordering.
    pub async fn hydrate_from_events(&self, history: Vec<SpineEvent>) {
        for event in history {
            dispatch_hydration(&self.observers, &event).await;
        }
    }
}

fn any_pattern_matches(patterns: &[EventPattern], event_type: &str) -> bool {
    patterns.iter().any(|p| p.matches(event_type))
}

/// Compute the spine-side `event_type = ANY(...)` filter for the
/// hydration query.
///
/// Returns `None` (no server-side filter; scan the window) when any
/// hydrating observer subscribes via a wildcard pattern. Otherwise
/// returns the sorted, deduplicated union of exact event types — the
/// query only fetches rows whose `event_type` is in this set, which
/// keeps the replay cost proportional to what the observers actually
/// care about.
fn hydration_event_type_filter(observers: &[&Registered]) -> Option<Vec<String>> {
    let mut exacts: BTreeSet<String> = BTreeSet::new();
    for obs in observers {
        for pat in &obs.patterns {
            if pat.is_wildcard() {
                return None;
            }
            exacts.insert(pat.as_str().to_owned());
        }
    }
    Some(exacts.into_iter().collect())
}

/// Dispatch one historical event through every matching observer
/// with the observer's hydration window respected and outputs
/// suppressed.
///
/// "Output suppression" is the spec's key correctness rule
/// (#392): hydration must not write to `observer_outputs`,
/// otherwise restarts would re-emit insights for last week's
/// patterns. Discarding the returned `Vec<ObserverOutput>` here is
/// where that rule is enforced — observers don't need to know
/// whether they're being hydrated, and the same `on_event`
/// implementation handles both modes.
async fn dispatch_hydration(observers: &[Registered], event: &SpineEvent) {
    let now = chrono::Utc::now();
    for obs in observers {
        let Some(window) = obs.hydration_window else {
            // Observer didn't opt into hydration. Live-only — its
            // state will warm up from incoming events after ready.
            continue;
        };
        // Per-observer window: the spec is explicit that hydration
        // bounds are per-observer, so an observer with a tight
        // window does not get re-fed events older than it cares
        // about even if a sibling observer requested a longer
        // window.
        if event.created_at < now - window {
            continue;
        }
        if !any_pattern_matches(&obs.patterns, &event.event_type) {
            continue;
        }
        let mut guard = obs.observer.lock().await;
        // Outputs are intentionally discarded — see function
        // doc-comment. The observer's state mutations
        // (artifact-id → kind index updates, sliding-window
        // pushes, …) persist; the returned insights/alerts do not.
        let _ = guard.on_event(event).await;
    }
}

/// Replay historical events through the runtime's observers before
/// the live loop starts.
///
/// One bulk query against `events` — the union of declared hydration
/// windows — and one dispatch pass per event. Observers without a
/// declared hydration window are skipped. Failures to parse a row's
/// payload are logged and skipped (matching the live loop's
/// best-effort behavior); they do not abort hydration.
async fn hydrate_observers(
    observers: &[Registered],
    event_store: &EventStore,
    cutoff_id: i64,
) -> Result<()> {
    let hydrating: Vec<&Registered> = observers
        .iter()
        .filter(|o| o.hydration_window.is_some())
        .collect();
    if hydrating.is_empty() {
        return Ok(());
    }

    // Union of declared windows: the spec says "the runtime fetches
    // the union of declared windows". Per-observer windows are still
    // honored at dispatch time (`dispatch_hydration` re-checks each
    // event's age against each observer's own window).
    let max_window = hydrating
        .iter()
        .filter_map(|o| o.hydration_window)
        .max()
        .expect("at least one hydrating observer");
    let since = chrono::Utc::now() - max_window;
    let event_types = hydration_event_type_filter(&hydrating);

    let rows = event_store
        .query_events_for_replay(since, cutoff_id, event_types.as_deref())
        .await
        .context("query events for observer hydration")?;

    let total = rows.len();
    let mut dispatched = 0usize;
    for record in rows {
        let payload: FactoryEvent = match serde_json::from_value(record.data) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    event_id = record.id,
                    event_type = %record.event_type,
                    error = %e,
                    "skip hydration row: parse FactoryEvent failed"
                );
                continue;
            }
        };
        let spine_event = SpineEvent {
            event_id: record.id,
            event_type: record.event_type,
            payload,
            created_at: record.created_at,
        };
        dispatch_hydration(observers, &spine_event).await;
        dispatched += 1;
    }
    tracing::info!(
        observers = hydrating.len(),
        rows = total,
        dispatched,
        cutoff_id,
        window_seconds = max_window.num_seconds(),
        "observer runtime: hydration complete"
    );
    Ok(())
}

/// Trait abstracting over `mpsc::UnboundedReceiver` and `mpsc::Receiver`
/// so [`ObserverRuntime::run_with_ready`] can drive either kind of
/// channel through one loop.
#[async_trait::async_trait]
trait AnyReceiver: Send {
    async fn recv(&mut self) -> Option<EventNotification>;
}

#[async_trait::async_trait]
impl AnyReceiver for tokio::sync::mpsc::UnboundedReceiver<EventNotification> {
    async fn recv(&mut self) -> Option<EventNotification> {
        tokio::sync::mpsc::UnboundedReceiver::recv(self).await
    }
}

#[async_trait::async_trait]
impl AnyReceiver for tokio::sync::mpsc::Receiver<EventNotification> {
    async fn recv(&mut self) -> Option<EventNotification> {
        tokio::sync::mpsc::Receiver::recv(self).await
    }
}

/// Per-task work: fetch the spine row, lock the observer, dispatch,
/// persist outputs.
///
/// **Error visibility.** Failures here (`get_event_by_id` returning
/// `None`, JSON parse failures, etc.) are surfaced only via
/// `tracing::error!` today; the event drops out of the observer's
/// view with no `observer_outputs` row. Promoting these to a typed
/// dead-letter (synthetic `Alert` keyed to `triggered_by_event_id`
/// or a `tokio_metrics`-style counter) is tracked as a follow-up to
/// #361 — the right shape is a substrate-wide decision (it applies
/// equally to scheduler-side parse failures) and is out of scope
/// for OBS-01.
async fn dispatch_one(
    observer_id: &str,
    observer: Arc<Mutex<Box<dyn Observer>>>,
    notification: EventNotification,
    event_store: EventStore,
    output_store: ObserverOutputStore,
) -> Result<()> {
    debug_assert_eq!(
        notification.table, "events",
        "dispatch_one should only see core spine events; \
         run_with_ready filters events_ext before fan-out"
    );

    let record = event_store
        .get_event_by_id(notification.id)
        .await?
        .context("spine row vanished between pg_notify and fetch")?;

    let payload: FactoryEvent =
        serde_json::from_value(record.data).context("parse FactoryEvent from spine row")?;

    let spine_event = SpineEvent {
        event_id: record.id,
        event_type: record.event_type,
        payload,
        created_at: record.created_at,
    };

    let mut guard = observer.lock().await;
    let outputs = guard.on_event(&spine_event).await;
    drop(guard);

    for out in outputs {
        if let Err(e) = output_store
            .record(observer_id, Some(spine_event.event_id), &out)
            .await
        {
            tracing::error!(observer = %observer_id, error = %e, "persist observer output failed");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{Insight, ObserverOutput};
    use crate::pattern::EventPattern;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts events it sees and emits an insight for each one.
    struct CountingObserver {
        seen: Arc<AtomicUsize>,
        patterns: Vec<EventPattern>,
    }

    #[async_trait]
    impl Observer for CountingObserver {
        fn subscriptions(&self) -> Vec<EventPattern> {
            self.patterns.clone()
        }
        async fn on_event(&mut self, ev: &SpineEvent) -> Vec<ObserverOutput> {
            let _ = ev; // referenced for shape; we don't branch on it here
            let n = self.seen.fetch_add(1, Ordering::SeqCst) + 1;
            vec![ObserverOutput::Insight(Insight::new(
                format!("seen #{n}"),
                0.5,
            ))]
        }
    }

    #[test]
    fn default_observers_declare_expected_subscriptions() {
        // Each ported analyzer should subscribe to its inputs without
        // panicking on construction; this also pins the subscription
        // shape so a future rename doesn't silently break the event
        // fan-out without test coverage.
        use crate::gate_deny_rate::GateDenyRateObserver;
        use crate::gate_override::GateOverrideObserver;
        use crate::pr_churn::PrChurnObserver;
        use crate::shape_retry::ShapeRetryObserver;

        let gate_override = GateOverrideObserver::default()
            .subscriptions()
            .into_iter()
            .map(|p| p.as_str().to_owned())
            .collect::<Vec<_>>();
        assert!(gate_override.contains(&"forge.gate_verdict".to_string()));
        assert!(gate_override.contains(&"artifact.registered".to_string()));

        let gate_deny = GateDenyRateObserver::default()
            .subscriptions()
            .into_iter()
            .map(|p| p.as_str().to_owned())
            .collect::<Vec<_>>();
        assert!(gate_deny.contains(&"forge.gate_verdict".to_string()));

        let retry = ShapeRetryObserver::default()
            .subscriptions()
            .into_iter()
            .map(|p| p.as_str().to_owned())
            .collect::<Vec<_>>();
        assert!(retry.contains(&"forge.shaping_returned".to_string()));

        let churn = PrChurnObserver::default()
            .subscriptions()
            .into_iter()
            .map(|p| p.as_str().to_owned())
            .collect::<Vec<_>>();
        assert!(churn.contains(&"git.pr_opened".to_string()));
        assert!(churn.contains(&"git.pr_merged".to_string()));
    }

    #[test]
    fn any_pattern_matches_correctly() {
        let patterns = vec![
            EventPattern::new("artifact.*"),
            EventPattern::new("node.completed"),
        ];
        assert!(any_pattern_matches(&patterns, "artifact.state_changed"));
        assert!(any_pattern_matches(&patterns, "node.completed"));
        assert!(!any_pattern_matches(&patterns, "node.started"));
    }

    /// In-process dispatch path: no spine, no DB. We invoke the
    /// per-observer fan-out through `dispatch_for_test`-equivalent
    /// glue: lock the mutex, call `on_event`, count.
    #[tokio::test]
    async fn observer_matches_pattern_and_receives_event_in_memory() {
        let seen = Arc::new(AtomicUsize::new(0));
        let observer: Box<dyn Observer> = Box::new(CountingObserver {
            seen: Arc::clone(&seen),
            patterns: vec![EventPattern::new("artifact.*")],
        });

        // Simulate the runtime's per-observer logic without a real
        // EventStore.
        let observer = Arc::new(Mutex::new(observer));
        let event_type = "artifact.state_changed".to_string();
        let patterns = observer.lock().await.subscriptions();
        assert!(any_pattern_matches(&patterns, &event_type));

        let spine_event = SpineEvent {
            event_id: 1,
            event_type,
            payload: dummy_factory_event(),
            created_at: chrono::Utc::now(),
        };

        let outputs = {
            let mut guard = observer.lock().await;
            guard.on_event(&spine_event).await
        };

        assert_eq!(seen.load(Ordering::SeqCst), 1);
        assert_eq!(outputs.len(), 1);
        assert!(matches!(outputs[0], ObserverOutput::Insight(_)));
    }

    #[tokio::test]
    async fn unmatched_event_does_not_invoke_observer() {
        let seen = Arc::new(AtomicUsize::new(0));
        let observer: Box<dyn Observer> = Box::new(CountingObserver {
            seen: Arc::clone(&seen),
            patterns: vec![EventPattern::new("artifact.*")],
        });
        let observer = Arc::new(Mutex::new(observer));

        let patterns = observer.lock().await.subscriptions();
        let event_type = "node.started";
        if any_pattern_matches(&patterns, event_type) {
            let spine_event = SpineEvent {
                event_id: 1,
                event_type: event_type.into(),
                payload: dummy_factory_event(),
                created_at: chrono::Utc::now(),
            };
            observer.lock().await.on_event(&spine_event).await;
        }

        assert_eq!(seen.load(Ordering::SeqCst), 0);
    }

    fn dummy_factory_event() -> FactoryEvent {
        use chrono::Utc;
        use onsager_artifact::{ArtifactId, ArtifactState};
        use onsager_spine::FactoryEventKind;
        FactoryEvent {
            event: FactoryEventKind::ArtifactStateChanged {
                artifact_id: ArtifactId::new("art_test"),
                from_state: ArtifactState::Draft,
                to_state: ArtifactState::InProgress,
            },
            correlation_id: None,
            causation_id: None,
            actor: "test".into(),
            timestamp: Utc::now(),
        }
    }

    // -----------------------------------------------------------------
    // Hydration (#392) — unit-level coverage
    //
    // Exercises the runtime-side replay logic without standing up a
    // spine. The end-to-end DB-backed scenario (subscribe → cutoff
    // → hydrate → live) is covered in tests/runtime_hydration_e2e.rs
    // behind the DATABASE_URL gate.
    // -----------------------------------------------------------------

    /// Test observer that records every event it sees and would
    /// gladly emit on each one — useful for asserting that hydration
    /// *does* drive `on_event` while *not* persisting outputs.
    struct RecordingObserver {
        seen_ids: Arc<std::sync::Mutex<Vec<i64>>>,
        patterns: Vec<EventPattern>,
        hydration: Option<Duration>,
    }

    #[async_trait]
    impl Observer for RecordingObserver {
        fn subscriptions(&self) -> Vec<EventPattern> {
            self.patterns.clone()
        }
        fn hydration_window(&self) -> Option<Duration> {
            self.hydration
        }
        async fn on_event(&mut self, ev: &SpineEvent) -> Vec<ObserverOutput> {
            self.seen_ids.lock().unwrap().push(ev.event_id);
            vec![ObserverOutput::Insight(Insight::new(
                format!("would-emit for {}", ev.event_id),
                0.5,
            ))]
        }
    }

    fn registered(
        id: &str,
        patterns: Vec<EventPattern>,
        hydration: Option<Duration>,
        obs: Box<dyn Observer>,
    ) -> Registered {
        Registered {
            id: id.into(),
            patterns,
            hydration_window: hydration,
            observer: Arc::new(Mutex::new(obs)),
        }
    }

    fn ev(event_id: i64, event_type: &str, age: Duration) -> SpineEvent {
        SpineEvent {
            event_id,
            event_type: event_type.into(),
            payload: dummy_factory_event(),
            created_at: chrono::Utc::now() - age,
        }
    }

    #[tokio::test]
    async fn hydration_dispatches_to_observers_that_opted_in() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
        let obs = Box::new(RecordingObserver {
            seen_ids: Arc::clone(&seen),
            patterns: vec![EventPattern::new("artifact.*")],
            hydration: Some(Duration::days(7)),
        });
        let observers = vec![registered(
            "obs.recording",
            vec![EventPattern::new("artifact.*")],
            Some(Duration::days(7)),
            obs,
        )];

        // Three events inside the window — all should be dispatched.
        let history = vec![
            ev(10, "artifact.registered", Duration::hours(1)),
            ev(11, "artifact.state_changed", Duration::minutes(30)),
            ev(12, "artifact.archived", Duration::seconds(5)),
        ];
        for e in history {
            dispatch_hydration(&observers, &e).await;
        }

        let ids = seen.lock().unwrap().clone();
        assert_eq!(ids, vec![10, 11, 12]);
    }

    #[tokio::test]
    async fn hydration_skips_observers_without_window() {
        // Same patterns, but no opt-in — observer must not be invoked
        // during hydration. Live mode would still drive it; hydration
        // is opt-in only.
        let seen = Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
        let obs = Box::new(RecordingObserver {
            seen_ids: Arc::clone(&seen),
            patterns: vec![EventPattern::new("artifact.*")],
            hydration: None,
        });
        let observers = vec![registered(
            "obs.no_hydrate",
            vec![EventPattern::new("artifact.*")],
            None,
            obs,
        )];

        dispatch_hydration(
            &observers,
            &ev(10, "artifact.registered", Duration::minutes(5)),
        )
        .await;
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hydration_skips_events_outside_observer_window() {
        // Observer wants 1 hour of history; the event is 2 hours
        // old. Bounded window — older events do not get replayed
        // even if they would have matched.
        let seen = Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
        let obs = Box::new(RecordingObserver {
            seen_ids: Arc::clone(&seen),
            patterns: vec![EventPattern::new("artifact.registered")],
            hydration: Some(Duration::hours(1)),
        });
        let observers = vec![registered(
            "obs.bounded",
            vec![EventPattern::new("artifact.registered")],
            Some(Duration::hours(1)),
            obs,
        )];

        dispatch_hydration(
            &observers,
            &ev(10, "artifact.registered", Duration::hours(2)),
        )
        .await;
        assert!(seen.lock().unwrap().is_empty());

        // An in-window event still drives the observer.
        dispatch_hydration(
            &observers,
            &ev(11, "artifact.registered", Duration::minutes(10)),
        )
        .await;
        assert_eq!(seen.lock().unwrap().clone(), vec![11]);
    }

    #[tokio::test]
    async fn hydration_skips_events_that_do_not_match_pattern() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
        let obs = Box::new(RecordingObserver {
            seen_ids: Arc::clone(&seen),
            patterns: vec![EventPattern::new("forge.gate_verdict")],
            hydration: Some(Duration::days(1)),
        });
        let observers = vec![registered(
            "obs.specific",
            vec![EventPattern::new("forge.gate_verdict")],
            Some(Duration::days(1)),
            obs,
        )];

        // Pattern mismatch — even though in window.
        dispatch_hydration(
            &observers,
            &ev(10, "artifact.registered", Duration::minutes(5)),
        )
        .await;
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hydration_outputs_are_suppressed() {
        // The recording observer would happily emit one Insight per
        // event; the contract is that hydration discards them. We
        // verify by reaching directly into the observer state — the
        // events were seen (state mutated) but no path here would
        // have persisted to `observer_outputs` (there is no
        // `output_store.record(...)` call inside `dispatch_hydration`,
        // so the suppression is structural, not behavioral).
        let seen = Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
        let obs = Box::new(RecordingObserver {
            seen_ids: Arc::clone(&seen),
            patterns: vec![EventPattern::new("artifact.*")],
            hydration: Some(Duration::days(7)),
        });
        let observers = vec![registered(
            "obs.suppress",
            vec![EventPattern::new("artifact.*")],
            Some(Duration::days(7)),
            obs,
        )];

        for i in 0..5 {
            dispatch_hydration(
                &observers,
                &ev(20 + i, "artifact.registered", Duration::minutes(1)),
            )
            .await;
        }
        // Events drove `on_event`...
        assert_eq!(seen.lock().unwrap().len(), 5);
        // ...but we have no path through `dispatch_hydration` that
        // would have written to `observer_outputs`. The E2E DB
        // test confirms the storage is empty after a restart.
    }

    #[tokio::test]
    async fn restart_rebuilds_gate_override_kind_index_via_hydration() {
        // The motivating scenario from spec #392: a code artifact
        // registered before restart, with no `artifact.registered`
        // replay, would mean post-restart verdicts can't be grouped
        // by kind. With hydration, the registration is replayed and
        // the verdicts trip the override-rate insight as before.
        use crate::gate_override::{GateOverrideObserver, TAG};
        use chrono::Utc;
        use onsager_artifact::{ArtifactId, Kind};
        use onsager_spine::factory_event::{FactoryEventKind, GatePoint, VerdictSummary};

        let id = ArtifactId::new("art_restart");
        let mut obs = GateOverrideObserver::default();
        // Sanity: the observer opts into hydration.
        assert_eq!(obs.hydration_window(), Some(Duration::days(7)));

        // History: the registration is the only event before "restart".
        let history = vec![SpineEvent {
            event_id: 1,
            event_type: "artifact.registered".into(),
            payload: FactoryEvent {
                event: FactoryEventKind::ArtifactRegistered {
                    artifact_id: id.clone(),
                    kind: Kind::Code,
                    name: "t".into(),
                    owner: "marvin".into(),
                },
                correlation_id: None,
                causation_id: None,
                actor: "test".into(),
                timestamp: Utc::now(),
            },
            created_at: Utc::now() - Duration::hours(1),
        }];

        // Replay the registration into the observer's state. Drive
        // the observer directly (not through `dispatch_hydration`,
        // since we want to inspect emitted outputs from the post-
        // restart verdicts below — those go through live `on_event`).
        for ev in &history {
            // Hydration call: outputs discarded.
            let _ = obs.on_event(ev).await;
        }

        // Post-restart: a burst of denies arrives over the live
        // stream. Without hydration, the observer's
        // `artifacts: HashMap<String, Kind>` is empty and the
        // verdicts drop out of grouping (`evaluate`'s
        // `self.artifacts.get(...)` returns `None`). With
        // hydration it returns `Some(Kind::Code)` and the rate trips.
        let mut emitted = Vec::new();
        for i in 0..5 {
            let ev = SpineEvent {
                event_id: 10 + i,
                event_type: "forge.gate_verdict".into(),
                payload: FactoryEvent {
                    event: FactoryEventKind::ForgeGateVerdict {
                        artifact_id: id.clone(),
                        gate_point: GatePoint::PreDispatch,
                        verdict: VerdictSummary::Deny,
                    },
                    correlation_id: None,
                    causation_id: None,
                    actor: "test".into(),
                    timestamp: Utc::now(),
                },
                created_at: Utc::now(),
            };
            emitted.extend(obs.on_event(&ev).await);
        }
        assert_eq!(
            emitted.len(),
            1,
            "post-restart verdicts must group by kind via hydrated index, got {emitted:?}"
        );
        match &emitted[0] {
            ObserverOutput::Insight(i) => {
                assert_eq!(i.tag.as_deref(), Some(TAG));
            }
            _ => panic!("expected Insight"),
        }
    }

    #[test]
    fn hydration_event_type_filter_unions_exact_patterns() {
        let o1 = registered(
            "o1",
            vec![
                EventPattern::new("artifact.registered"),
                EventPattern::new("forge.gate_verdict"),
            ],
            Some(Duration::days(1)),
            Box::new(RecordingObserver {
                seen_ids: Arc::new(std::sync::Mutex::new(Vec::new())),
                patterns: vec![],
                hydration: None,
            }),
        );
        let o2 = registered(
            "o2",
            vec![EventPattern::new("forge.shaping_returned")],
            Some(Duration::days(1)),
            Box::new(RecordingObserver {
                seen_ids: Arc::new(std::sync::Mutex::new(Vec::new())),
                patterns: vec![],
                hydration: None,
            }),
        );
        let filter = hydration_event_type_filter(&[&o1, &o2]).expect("all exact -> Some(...)");
        // BTreeSet ordering keeps this stable.
        assert_eq!(
            filter,
            vec![
                "artifact.registered".to_string(),
                "forge.gate_verdict".to_string(),
                "forge.shaping_returned".to_string(),
            ]
        );
    }

    #[test]
    fn hydration_event_type_filter_returns_none_for_wildcard() {
        // Any wildcard in any hydrating observer disables the
        // server-side filter — we can't enumerate the matching
        // event types up front.
        let o1 = registered(
            "o1",
            vec![EventPattern::new("artifact.*")],
            Some(Duration::days(1)),
            Box::new(RecordingObserver {
                seen_ids: Arc::new(std::sync::Mutex::new(Vec::new())),
                patterns: vec![],
                hydration: None,
            }),
        );
        assert!(hydration_event_type_filter(&[&o1]).is_none());

        let o2 = registered(
            "o2",
            vec![EventPattern::new("*")],
            Some(Duration::days(1)),
            Box::new(RecordingObserver {
                seen_ids: Arc::new(std::sync::Mutex::new(Vec::new())),
                patterns: vec![],
                hydration: None,
            }),
        );
        assert!(hydration_event_type_filter(&[&o2]).is_none());
    }
}
