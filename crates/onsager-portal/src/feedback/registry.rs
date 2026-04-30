//! `CorrelationRegistry` — in-process map from `correlation_id` to a
//! oneshot waiter, fed by the spine `pg_notify` channel.
//!
//! The registry is the back end of the fast-write path. A handler
//! mints a `correlation_id`, registers a [`Waiter`], dispatches the
//! intent, and `await`s the waiter with a bounded timeout. A
//! background task on the registry tails [`EventStore::subscribe`]
//! and, when it sees a notification carrying a matching
//! `correlation_id`, completes the waiter.
//!
//! Notifications without a `correlation_id` (background events,
//! pre-#223 producers) are ignored. Multiple waiters per id are not
//! supported — fresh UUIDs make collisions impossible in practice.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use onsager_spine::{EventNotification, EventStore};
use tokio::sync::oneshot;
use uuid::Uuid;

/// Hard cap on the synchronous-wait budget (`2s`, per #223). Helpers
/// clamp callers to this regardless of the timeout they ask for —
/// anything that needs more is misclassified.
pub const MAX_SYNC_TIMEOUT: Duration = Duration::from_millis(2000);

/// Default bounded-channel capacity for the pg_notify pump. Generous
/// enough to absorb bursts (a typical fast-write turnover is dozens of
/// notifications/second) without giving slow consumers an OOM rope.
/// Override via [`CorrelationRegistry::start_with_capacity`].
pub const DEFAULT_NOTIFY_CAPACITY: usize = 1024;

type WaiterMap = HashMap<Uuid, oneshot::Sender<EventNotification>>;

/// Routes spine notifications back to the request handler that
/// dispatched the originating intent.
///
/// Cheap to clone — the inner state is `Arc<Mutex<...>>`. Start the
/// pg_notify pump exactly once via [`Self::start`]; subsequent calls
/// would race two listeners against the same map.
#[derive(Clone)]
pub struct CorrelationRegistry {
    waiters: Arc<Mutex<WaiterMap>>,
}

impl CorrelationRegistry {
    pub fn new() -> Self {
        Self {
            waiters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Subscribe to spine notifications and route correlation-tagged
    /// events to the matching [`Waiter`]. Spawns a background task
    /// that runs until the underlying `pg_notify` channel closes.
    /// Uses [`DEFAULT_NOTIFY_CAPACITY`]; call
    /// [`Self::start_with_capacity`] to tune.
    pub async fn start(&self, store: &EventStore) -> Result<(), sqlx::Error> {
        self.start_with_capacity(store, DEFAULT_NOTIFY_CAPACITY)
            .await
    }

    /// Like [`Self::start`] but with a caller-chosen channel capacity.
    /// Backed by [`EventStore::subscribe_bounded`] so a stalled pump
    /// applies backpressure on the sender rather than growing the
    /// channel without bound (the warning on `EventStore::subscribe`).
    pub async fn start_with_capacity(
        &self,
        store: &EventStore,
        capacity: usize,
    ) -> Result<(), sqlx::Error> {
        let mut rx = store.subscribe_bounded(capacity).await?;
        let waiters = Arc::clone(&self.waiters);
        tokio::spawn(async move {
            while let Some(notification) = rx.recv().await {
                let Some(corr) = notification.correlation_id else {
                    continue;
                };
                let sender = {
                    let mut map = waiters.lock().expect("CorrelationRegistry poisoned");
                    map.remove(&corr)
                };
                if let Some(tx) = sender {
                    // Receiver may have dropped (timeout already fired);
                    // ignore the send error in that case.
                    let _ = tx.send(notification);
                }
            }
            tracing::debug!("CorrelationRegistry pg_notify channel closed");
        });
        Ok(())
    }

    /// Reserve a slot for `correlation_id` and return a [`Waiter`]
    /// that will resolve when the matching notification arrives.
    /// Register **before** dispatching the intent — otherwise a
    /// response that lands faster than the registry insert is lost.
    pub fn register(&self, correlation_id: Uuid) -> Waiter {
        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.waiters.lock().expect("CorrelationRegistry poisoned");
            // Multiple waiters per id are not supported (UUIDs are
            // unique by construction). Catch a double-register in
            // debug builds; in release we'd cancel the previous
            // waiter implicitly otherwise — log loudly so the bug
            // surfaces in production too.
            if let Some(_dup) = map.insert(correlation_id, tx) {
                debug_assert!(
                    false,
                    "CorrelationRegistry: duplicate register for {correlation_id}"
                );
                tracing::error!(
                    %correlation_id,
                    "duplicate register; previous waiter cancelled"
                );
            }
        }
        Waiter {
            correlation_id,
            rx: Some(rx),
            waiters: Arc::clone(&self.waiters),
        }
    }

    /// Number of currently-registered waiters. Test helper.
    #[doc(hidden)]
    pub fn waiter_count(&self) -> usize {
        self.waiters
            .lock()
            .expect("CorrelationRegistry poisoned")
            .len()
    }
}

impl Default for CorrelationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors emitted by [`Waiter::timeout`] / [`await_with_timeout`].
#[derive(Debug, thiserror::Error)]
pub enum AwaitError {
    /// Timed out before a matching notification arrived.
    #[error("timed out waiting for correlation_id response")]
    Timeout,
    /// The registry was dropped before the notification arrived.
    /// Should be unreachable in normal operation — the registry is
    /// a process-lifetime singleton.
    #[error("correlation registry dropped before response")]
    Cancelled,
}

/// Reservation handle. Either resolves to a matching
/// [`EventNotification`] or, on `Drop`, releases its slot in the
/// registry so a leaked handle doesn't leak memory.
pub struct Waiter {
    correlation_id: Uuid,
    rx: Option<oneshot::Receiver<EventNotification>>,
    waiters: Arc<Mutex<WaiterMap>>,
}

impl Waiter {
    pub fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Wait up to `timeout` (clamped to [`MAX_SYNC_TIMEOUT`]) for the
    /// notification to arrive.
    pub async fn timeout(mut self, timeout: Duration) -> Result<EventNotification, AwaitError> {
        let bounded = timeout.min(MAX_SYNC_TIMEOUT);
        let rx = self.rx.take().expect("Waiter polled twice");
        let result = match tokio::time::timeout(bounded, rx).await {
            Ok(Ok(notification)) => Ok(notification),
            Ok(Err(_)) => Err(AwaitError::Cancelled),
            Err(_) => Err(AwaitError::Timeout),
        };
        if result.is_err() {
            self.release_slot();
        }
        result
    }

    fn release_slot(&self) {
        let _ = self
            .waiters
            .lock()
            .expect("CorrelationRegistry poisoned")
            .remove(&self.correlation_id);
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        // Release the slot if the caller dropped without awaiting.
        // No-op if `timeout` already removed it.
        if self.rx.is_some() {
            self.release_slot();
        }
    }
}

/// Free-function form of [`Waiter::timeout`]. Useful for callers that
/// already hold a [`Waiter`] and want a single line.
pub async fn await_with_timeout(
    waiter: Waiter,
    timeout: Duration,
) -> Result<EventNotification, AwaitError> {
    waiter.timeout(timeout).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_notification(corr: Option<Uuid>) -> EventNotification {
        EventNotification {
            table: "events".into(),
            id: 42,
            stream_id: "art_test".into(),
            event_type: "artifact.registered".into(),
            correlation_id: corr,
        }
    }

    /// `Waiter::timeout` returns the notification dispatched to the
    /// matching slot, then the slot is removed (one-shot semantics).
    #[tokio::test]
    async fn waiter_resolves_on_matching_notification() {
        let registry = CorrelationRegistry::new();
        let id = Uuid::new_v4();
        let waiter = registry.register(id);
        assert_eq!(registry.waiter_count(), 1);

        // Simulate the pg_notify pump delivering a matching notification.
        let registry_clone = registry.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let sender = {
                let mut map = registry_clone.waiters.lock().unwrap();
                map.remove(&id)
            };
            if let Some(tx) = sender {
                let _ = tx.send(fake_notification(Some(id)));
            }
        });

        let n = waiter
            .timeout(Duration::from_millis(500))
            .await
            .expect("notification");
        assert_eq!(n.correlation_id, Some(id));
        assert_eq!(registry.waiter_count(), 0);
    }

    /// Timeout fires when no notification arrives; the slot is released
    /// so a future waiter on the same id is possible (and so the map
    /// doesn't leak memory).
    #[tokio::test]
    async fn waiter_times_out_and_releases_slot() {
        let registry = CorrelationRegistry::new();
        let id = Uuid::new_v4();
        let waiter = registry.register(id);
        assert_eq!(registry.waiter_count(), 1);

        let result = waiter.timeout(Duration::from_millis(20)).await;
        assert!(matches!(result, Err(AwaitError::Timeout)));
        assert_eq!(registry.waiter_count(), 0);
    }

    /// `MAX_SYNC_TIMEOUT` clamps overlong asks down to 2s — the cap
    /// the spec contracts on (`#223`). Uses a paused clock so the
    /// assertion is on logic (clamp value), not wall-clock timing
    /// that can flake under CI load.
    #[tokio::test(start_paused = true)]
    async fn timeout_is_clamped_to_max_sync_timeout() {
        let registry = CorrelationRegistry::new();
        let id = Uuid::new_v4();
        let waiter = registry.register(id);

        // Ask for an hour. Paused clock + auto-advance means the
        // sleep inside `tokio::time::timeout` virtually advances to
        // exactly MAX_SYNC_TIMEOUT (2s) before the future resolves.
        let started = tokio::time::Instant::now();
        let result = waiter.timeout(Duration::from_secs(3600)).await;
        let elapsed = started.elapsed();

        assert!(matches!(result, Err(AwaitError::Timeout)));
        assert_eq!(
            elapsed, MAX_SYNC_TIMEOUT,
            "expected clamp to MAX_SYNC_TIMEOUT, got {elapsed:?}"
        );
    }

    /// A waiter that's dropped without polling releases its slot —
    /// otherwise leaked HTTP handler tasks would accumulate
    /// dead entries.
    #[tokio::test]
    async fn dropped_waiter_releases_slot() {
        let registry = CorrelationRegistry::new();
        let id = Uuid::new_v4();
        {
            let _waiter = registry.register(id);
            assert_eq!(registry.waiter_count(), 1);
        }
        assert_eq!(registry.waiter_count(), 0);
    }

    /// Notifications without a `correlation_id` are ignored — the
    /// pump skips them rather than scanning the map. We model this
    /// by exercising the same code path the pump uses.
    #[tokio::test]
    async fn unmatched_notification_does_not_resolve_waiter() {
        let registry = CorrelationRegistry::new();
        let id = Uuid::new_v4();
        let waiter = registry.register(id);

        // Different correlation_id — the pump's `remove` returns None,
        // so nothing happens to our waiter.
        let other = Uuid::new_v4();
        {
            let mut map = registry.waiters.lock().unwrap();
            // Simulate the pump's check + remove for the wrong id.
            assert!(map.remove(&other).is_none());
        }

        // Waiter still pending → times out cleanly.
        let result = waiter.timeout(Duration::from_millis(20)).await;
        assert!(matches!(result, Err(AwaitError::Timeout)));
    }
}
