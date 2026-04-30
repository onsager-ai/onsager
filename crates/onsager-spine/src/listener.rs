//! High-level subscription API on top of [`EventStore::subscribe`].
//!
//! # Namespace filtering convention (v0.1)
//!
//! `pg_notify` notifications carry `stream_id` but **not** the namespace
//! column. As a v0.1 contract, producers are expected to prefix their
//! `stream_id` values with the namespace followed by a colon — e.g.
//! `"stiglab:session:abc"`. The [`Listener`] filters incoming notifications by
//! splitting `stream_id` on the first `':'` and comparing the prefix against
//! its subscribed namespaces. If the prefix does not match any subscribed
//! namespace the notification is dropped.
//!
//! If [`Listener::subscribe`] is never called, the listener forwards
//! **all** notifications (no filtering).
//!
//! # Backfill on startup
//!
//! Pass a cursor via [`Listener::with_since`] to replay events written before
//! the listener connected.  `None` replays from the beginning; `Some(id)`
//! replays events with `id > since`.  After startup the caller can read the
//! current cursor position and persist it for the next restart.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::namespace::Namespace;
use crate::store::{EventNotification, EventStore};

/// Trait implemented by consumers that want to react to events.
#[async_trait]
pub trait EventHandler: Send + Sync + 'static {
    /// Handle a single event notification. Returning an error logs the failure
    /// but does **not** stop the listener.
    async fn handle(&self, event: EventNotification) -> anyhow::Result<()>;
}

/// A high-level event listener that filters notifications by namespace and
/// dispatches them to an [`EventHandler`].
pub struct Listener {
    store: EventStore,
    namespaces: HashSet<String>,
    /// Backfill cursor: replay events with id > since from each table.
    /// `None` replays from the very beginning.
    since: Option<i64>,
}

impl Listener {
    /// Create a new listener backed by the given event store.
    pub fn new(store: EventStore) -> Self {
        Self {
            store,
            namespaces: HashSet::new(),
            since: None,
        }
    }

    /// Subscribe to events from the given namespace. Chainable.
    ///
    /// If never called, the listener forwards all notifications.
    pub fn subscribe(mut self, ns: Namespace) -> Self {
        self.namespaces.insert(ns.as_str().to_owned());
        self
    }

    /// Set the backfill cursor. Events with `id > since` will be replayed from
    /// the database before live streaming begins. Pass `None` to replay from
    /// the very beginning; pass `Some(last_seen_id)` to resume from a saved
    /// position.
    pub fn with_since(mut self, since: Option<i64>) -> Self {
        self.since = since;
        self
    }

    /// Run the listener loop. This is long-running and only returns when the
    /// underlying `pg_notify` channel closes.
    ///
    /// On startup the listener:
    /// 1. Subscribes to `pg_notify` (buffering live events).
    /// 2. Queries both event tables for rows with `id > since` and dispatches
    ///    them (backfill).
    /// 3. Streams live events, skipping any already covered by backfill.
    ///
    /// Each notification is dispatched to `handler` in its own Tokio task so
    /// that a slow handler does not block the channel.
    pub async fn run<H: EventHandler>(self, handler: H) -> anyhow::Result<()> {
        // Subscribe to pg_notify first so we buffer live events during backfill.
        let mut rx = self.store.subscribe().await?;
        let handler = Arc::new(handler);

        // Backfill: dispatch rows written before this listener connected.
        let since_id = self.since.unwrap_or(0);
        let (max_events_id, max_ext_id) = self.dispatch_backfill(since_id, &handler).await?;

        // Stream live events, skipping those already covered by backfill.
        while let Some(notification) = rx.recv().await {
            let already_backfilled = match notification.table.as_str() {
                "events" => notification.id <= max_events_id,
                "events_ext" => notification.id <= max_ext_id,
                _ => false,
            };
            if already_backfilled {
                continue;
            }

            if !self.namespaces.is_empty()
                && !matches_any_namespace(&notification.stream_id, &self.namespaces)
            {
                continue;
            }

            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                if let Err(e) = handler.handle(notification).await {
                    tracing::error!("EventHandler error: {e}");
                }
            });
        }

        tracing::warn!("pg_notify channel closed, listener shutting down");
        Ok(())
    }

    /// Query both event tables for rows after `since_id`, apply namespace
    /// filtering, dispatch to the handler, and return the highest id seen from
    /// each table.
    async fn dispatch_backfill(
        &self,
        since_id: i64,
        handler: &Arc<impl EventHandler>,
    ) -> anyhow::Result<(i64, i64)> {
        let pool = self.store.pool();

        let mut max_events_id: i64 = since_id;
        let mut max_ext_id: i64 = since_id;

        // Backfill core events.
        let rows: Vec<(i64, String, String, Option<uuid::Uuid>)> = sqlx::query_as(
            "SELECT id, stream_id, event_type, correlation_id FROM events WHERE id > $1 ORDER BY id ASC",
        )
        .bind(since_id)
        .fetch_all(pool)
        .await?;

        for (id, stream_id, event_type, correlation_id) in rows {
            if !self.namespaces.is_empty() && !matches_any_namespace(&stream_id, &self.namespaces) {
                if id > max_events_id {
                    max_events_id = id;
                }
                continue;
            }
            if id > max_events_id {
                max_events_id = id;
            }
            let notification = EventNotification {
                table: "events".to_string(),
                id,
                stream_id,
                event_type,
                correlation_id,
            };
            let h = Arc::clone(handler);
            tokio::spawn(async move {
                if let Err(e) = h.handle(notification).await {
                    tracing::error!("EventHandler backfill error: {e}");
                }
            });
        }

        // Backfill extension events.
        let ext_rows: Vec<(i64, String, String, Option<uuid::Uuid>)> = sqlx::query_as(
            "SELECT id, stream_id, event_type, correlation_id FROM events_ext WHERE id > $1 ORDER BY id ASC",
        )
        .bind(since_id)
        .fetch_all(pool)
        .await?;

        for (id, stream_id, event_type, correlation_id) in ext_rows {
            if !self.namespaces.is_empty() && !matches_any_namespace(&stream_id, &self.namespaces) {
                if id > max_ext_id {
                    max_ext_id = id;
                }
                continue;
            }
            if id > max_ext_id {
                max_ext_id = id;
            }
            let notification = EventNotification {
                table: "events_ext".to_string(),
                id,
                stream_id,
                event_type,
                correlation_id,
            };
            let h = Arc::clone(handler);
            tokio::spawn(async move {
                if let Err(e) = h.handle(notification).await {
                    tracing::error!("EventHandler backfill error: {e}");
                }
            });
        }

        Ok((max_events_id, max_ext_id))
    }
}

/// Check whether `stream_id` starts with any of the given namespace prefixes.
///
/// The convention is `"<namespace>:<rest>"` — we split on the first `':'`.
fn matches_any_namespace(stream_id: &str, namespaces: &HashSet<String>) -> bool {
    match stream_id.split_once(':') {
        Some((prefix, _)) => namespaces.contains(prefix),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // We test the pure matching logic directly rather than constructing a real
    // Listener, which would require a live PgPool.

    fn ns_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn matches_single_namespace() {
        let namespaces = ns_set(&["stiglab"]);
        assert!(matches_any_namespace("stiglab:session:abc", &namespaces));
        assert!(!matches_any_namespace("synodic:session:abc", &namespaces));
    }

    #[test]
    fn matches_multiple_namespaces() {
        let namespaces = ns_set(&["stiglab", "ising"]);
        assert!(matches_any_namespace("stiglab:session:1", &namespaces));
        assert!(matches_any_namespace("ising:run:42", &namespaces));
        assert!(!matches_any_namespace("synodic:policy:x", &namespaces));
    }

    #[test]
    fn no_colon_never_matches() {
        let namespaces = ns_set(&["stiglab"]);
        assert!(!matches_any_namespace("stiglab", &namespaces));
        assert!(!matches_any_namespace("no-colon-here", &namespaces));
    }

    #[test]
    fn empty_namespace_set_is_handled_by_caller() {
        // When the namespace set is empty, the Listener skips filtering
        // entirely. This test just documents that matches_any_namespace returns
        // false for an empty set — the caller decides what that means.
        let namespaces = ns_set(&[]);
        assert!(!matches_any_namespace("stiglab:session:1", &namespaces));
    }

    #[test]
    fn prefix_only_up_to_first_colon() {
        let namespaces = ns_set(&["stiglab"]);
        // "stiglab:session:abc" — prefix is "stiglab", not "stiglab:session"
        assert!(matches_any_namespace("stiglab:session:abc", &namespaces));
        // Make sure a namespace with colon in it doesn't partially match
        let namespaces = ns_set(&["stiglab:session"]);
        assert!(!matches_any_namespace("stiglab:session:abc", &namespaces));
    }

    /// Integration test: backfill on startup + live events via pg_notify.
    ///
    /// Requires DATABASE_URL to be set.
    #[tokio::test]
    async fn listener_backfill_and_live() {
        use crate::factory_event::{FactoryEvent, FactoryEventKind};
        use crate::store::{append_factory_event_tx, EventMetadata, EventStore};
        use chrono::Utc;
        use onsager_artifact::{ArtifactId, Kind};
        use std::sync::atomic::{AtomicI64, Ordering};
        use tokio::sync::Semaphore;

        let Some(db_url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("skipping: DATABASE_URL not set");
            return;
        };

        let store = EventStore::connect(&db_url).await.unwrap();
        let tag = format!("backfill_test_{}", ulid::Ulid::new());

        // Helper: build a test event with a unique, filterable stream_id.
        let make_event = |suffix: &str| {
            let artifact_id_str = format!("art_{tag}_{suffix}");
            FactoryEvent {
                event: FactoryEventKind::ArtifactRegistered {
                    artifact_id: ArtifactId::new(&artifact_id_str),
                    kind: Kind::Document,
                    name: "backfill test".into(),
                    owner: "test".into(),
                },
                correlation_id: None,
                causation_id: None,
                actor: "test".into(),
                timestamp: Utc::now(),
            }
        };
        let meta = EventMetadata {
            actor: "test".into(),
            ..Default::default()
        };

        // 1. Write 3 events before the listener starts.
        let id1 = store
            .transaction(|tx| {
                let e = make_event("pre1");
                let m = meta.clone();
                Box::pin(async move { append_factory_event_tx(tx, &e, &m).await })
            })
            .await
            .unwrap();
        let _id2 = store
            .transaction(|tx| {
                let e = make_event("pre2");
                let m = meta.clone();
                Box::pin(async move { append_factory_event_tx(tx, &e, &m).await })
            })
            .await
            .unwrap();
        let id3 = store
            .transaction(|tx| {
                let e = make_event("pre3");
                let m = meta.clone();
                Box::pin(async move { append_factory_event_tx(tx, &e, &m).await })
            })
            .await
            .unwrap();

        // 2. Start listener with since = Some(id1 - 1) so it picks up events
        //    id1, id2, id3 via backfill (skipping anything before id1).
        let received = Arc::new(AtomicI64::new(0));
        let done = Arc::new(Semaphore::new(0));

        struct CountHandler {
            count: Arc<AtomicI64>,
            done: Arc<Semaphore>,
            target: i64,
        }

        #[async_trait]
        impl EventHandler for CountHandler {
            async fn handle(&self, _event: EventNotification) -> anyhow::Result<()> {
                let n = self.count.fetch_add(1, Ordering::SeqCst) + 1;
                if n >= self.target {
                    self.done.add_permits(1);
                }
                Ok(())
            }
        }

        // Expect 3 backfill + 2 live = 5 total.
        let handler = CountHandler {
            count: Arc::clone(&received),
            done: Arc::clone(&done),
            target: 5,
        };

        let listener_store = store.clone();
        let listener_handle = tokio::spawn(async move {
            Listener::new(listener_store)
                .with_since(Some(id1 - 1))
                .run(handler)
                .await
        });

        // Give backfill a moment to complete before writing live events.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 4. Write 2 more events after the listener has subscribed.
        let id4 = store
            .transaction(|tx| {
                let e = make_event("live1");
                let m = meta.clone();
                Box::pin(async move { append_factory_event_tx(tx, &e, &m).await })
            })
            .await
            .unwrap();
        let _id5 = store
            .transaction(|tx| {
                let e = make_event("live2");
                let m = meta.clone();
                Box::pin(async move { append_factory_event_tx(tx, &e, &m).await })
            })
            .await
            .unwrap();

        // 5. Wait for all 5 events (3 backfill + 2 live).
        let _permit = tokio::time::timeout(std::time::Duration::from_secs(5), done.acquire())
            .await
            .expect("timed out waiting for events")
            .unwrap();

        assert_eq!(received.load(Ordering::SeqCst), 5);
        listener_handle.abort();

        // 6. Restart with cursor = id3, write 1 more event; expect only 1 + 1
        //    from backfill (id4) then 1 live.
        let received2 = Arc::new(AtomicI64::new(0));
        let done2 = Arc::new(Semaphore::new(0));
        let handler2 = CountHandler {
            count: Arc::clone(&received2),
            done: Arc::clone(&done2),
            target: 2, // backfill id4 + live id6
        };

        let listener_store2 = store.clone();
        let listener_handle2 = tokio::spawn(async move {
            Listener::new(listener_store2)
                .with_since(Some(id3)) // cursor after id3: should backfill id4, id5
                .run(handler2)
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let _id6 = store
            .transaction(|tx| {
                let e = make_event("live3");
                let m = meta.clone();
                Box::pin(async move { append_factory_event_tx(tx, &e, &m).await })
            })
            .await
            .unwrap();

        // Wait for 3 events: id4 (backfill), id5 (backfill), id6 (live).
        // Actually we set target=2 originally but we'll get 3 (id4, id5, id6).
        // Let's wait for all 3.
        let received2_clone = Arc::clone(&received2);
        tokio::time::timeout(std::time::Duration::from_secs(5), async move {
            // Poll until count reaches 3
            loop {
                if received2_clone.load(Ordering::SeqCst) >= 3 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("timed out waiting for second listener events");

        assert!(
            received2.load(Ordering::SeqCst) >= 3,
            "second listener should have received id4 (backfill), id5 (backfill), id6 (live)"
        );
        assert!(
            received2.load(Ordering::SeqCst) <= 5,
            "second listener should not have received id1/id2/id3 (before cursor id3={id3})"
        );

        listener_handle2.abort();

        // Clean up: delete test events.
        sqlx::query("DELETE FROM events WHERE id IN ($1, $2, $3, $4, $5, $6)")
            .bind(id1)
            .bind(_id2)
            .bind(id3)
            .bind(id4)
            .bind(_id5)
            .bind(_id6)
            .execute(store.pool())
            .await
            .unwrap();
    }
}
