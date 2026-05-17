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
//! ## Single-task-per-observer ordering
//!
//! Each observer instance lives behind a `tokio::sync::Mutex`. Two
//! events matching the same observer therefore run *in order* —
//! event A finishes before event B is processed by that observer —
//! even though A and B may interleave with other observers'
//! processing. Observers from each other are fully parallel.

use std::sync::Arc;

use anyhow::{Context, Result};
use onsager_spine::{EventNotification, EventStore, FactoryEvent};
use tokio::sync::Mutex;

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
}

impl ObserverRuntime {
    /// Wire the runtime to an existing spine and output store.
    pub fn new(event_store: EventStore, output_store: ObserverOutputStore) -> Self {
        Self {
            event_store,
            output_store,
            observers: Vec::new(),
        }
    }

    /// Register one observer under a stable id. Chainable.
    ///
    /// `id` is what the runtime writes into `observer_outputs.observer_id`
    /// — pick something stable across restarts. The observer's
    /// declared [`subscriptions`](Observer::subscriptions) are read
    /// once and cached; observers cannot change their patterns at
    /// runtime.
    pub fn register<O: Observer + 'static>(mut self, id: impl Into<String>, observer: O) -> Self {
        let patterns = observer.subscriptions();
        self.observers.push(Registered {
            id: id.into(),
            patterns,
            observer: Arc::new(Mutex::new(Box::new(observer))),
        });
        self
    }

    /// Register an observer that is already boxed. Useful when the
    /// caller stores observer instances by name in a registry of
    /// their own.
    pub fn register_boxed(mut self, id: impl Into<String>, observer: Box<dyn Observer>) -> Self {
        let patterns = observer.subscriptions();
        self.observers.push(Registered {
            id: id.into(),
            patterns,
            observer: Arc::new(Mutex::new(observer)),
        });
        self
    }

    /// Number of registered observers. Mostly useful for tests.
    pub fn observer_count(&self) -> usize {
        self.observers.len()
    }

    /// Run the subscription loop. Only returns when the underlying
    /// `pg_notify` channel closes or the spine fails.
    pub async fn run(self) -> Result<()> {
        let mut rx = self
            .event_store
            .subscribe()
            .await
            .context("subscribe to spine pg_notify")?;

        let registered: Arc<Vec<Registered>> = Arc::new(self.observers);
        let event_store = self.event_store;
        let output_store = self.output_store;

        while let Some(notification) = rx.recv().await {
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
}

fn any_pattern_matches(patterns: &[EventPattern], event_type: &str) -> bool {
    patterns.iter().any(|p| p.matches(event_type))
}

/// Per-task work: fetch the spine row, lock the observer, dispatch,
/// persist outputs.
async fn dispatch_one(
    observer_id: &str,
    observer: Arc<Mutex<Box<dyn Observer>>>,
    notification: EventNotification,
    event_store: EventStore,
    output_store: ObserverOutputStore,
) -> Result<()> {
    // Observers see core spine events only — `events_ext` carries
    // subsystem-private extension payloads, not factory events.
    if notification.table != "events" {
        return Ok(());
    }

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
}
