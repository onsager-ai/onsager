//! Event-driven handler for stiglab session completions (issue #14 phase 2).
//!
//! The HTTP dispatcher in `cmd/serve.rs` is synchronous: it blocks the
//! pipeline tick until Stiglab returns. That works when the agent finishes
//! within the request timeout, but long-running shaping tasks need an
//! asynchronous path. This listener is the first piece of that path.
//!
//! Design:
//!
//! - subscribe to the spine with the `stiglab` namespace filter
//! - buffer notifications; for each one with `event_type ==
//!   "stiglab.session_completed"`, load the full event row from the store
//!   and parse it into a typed [`SessionCompleted`]
//! - invoke a caller-supplied [`SessionCompletedHandler`] with the parsed
//!   event, so forge-specific state mutations stay in the forge crate and
//!   spine stays a generic library
//!
//! The initial integration only handles `session_completed`; failed/aborted
//! variants are easy to add later by extending the match arm.

use std::sync::Arc;

use async_trait::async_trait;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener, Namespace};

/// A parsed stiglab.session_completed event (issue #14 phase 2).
#[derive(Debug, Clone)]
pub struct SessionCompleted {
    pub event_id: i64,
    pub session_id: String,
    pub request_id: String,
    pub duration_ms: u64,
    pub artifact_id: Option<String>,
}

/// Caller-supplied callback invoked for every session completion.
#[async_trait]
pub trait SessionCompletedHandler: Send + Sync + 'static {
    async fn on_session_completed(&self, event: SessionCompleted) -> anyhow::Result<()>;
}

/// Run a session-completed listener forever. Returns only if the underlying
/// pg_notify channel closes.
///
/// `since` is the backfill cursor — forge can persist the last seen event id
/// across restarts so a crash doesn't drop in-flight session completions.
pub async fn run<H: SessionCompletedHandler>(
    store: EventStore,
    handler: H,
    since: Option<i64>,
) -> anyhow::Result<()> {
    let dispatcher = Dispatcher {
        store: store.clone(),
        handler: Arc::new(handler),
    };
    Listener::new(store)
        .subscribe(Namespace::new("stiglab").map_err(anyhow::Error::from)?)
        .with_since(since)
        .run(dispatcher)
        .await
}

struct Dispatcher<H: SessionCompletedHandler> {
    store: EventStore,
    handler: Arc<H>,
}

#[async_trait]
impl<H: SessionCompletedHandler> EventHandler for Dispatcher<H> {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "stiglab.session_completed" {
            return Ok(());
        }
        // Skip events that live in events_ext — session_completed events are
        // always written via append_factory_event to the core events table.
        if notification.table != "events" {
            return Ok(());
        }

        let records = self
            .store
            .query_events(Some(&notification.stream_id), None, None, 100)
            .await?;
        let Some(record) = records.into_iter().find(|r| r.id == notification.id) else {
            tracing::warn!(
                id = notification.id,
                stream = %notification.stream_id,
                "session_completed notification had no matching event row"
            );
            return Ok(());
        };

        let evt: FactoryEvent = match serde_json::from_value(record.data.clone()) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse FactoryEvent");
                return Ok(());
            }
        };

        if let FactoryEventKind::StiglabSessionCompleted {
            session_id,
            request_id,
            duration_ms,
            artifact_id,
        } = evt.event
        {
            self.handler
                .on_session_completed(SessionCompleted {
                    event_id: record.id,
                    session_id,
                    request_id,
                    duration_ms,
                    artifact_id,
                })
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Capture(std::sync::Mutex<Vec<SessionCompleted>>);

    #[async_trait]
    impl SessionCompletedHandler for Capture {
        async fn on_session_completed(&self, event: SessionCompleted) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    /// The callback trait is usable with a straightforward struct; compiling
    /// this guards against accidentally breaking the Send/Sync + 'static
    /// bounds needed by the spine Listener.
    #[tokio::test]
    async fn handler_trait_is_callable() {
        let capture = Capture(Default::default());
        capture
            .on_session_completed(SessionCompleted {
                event_id: 1,
                session_id: "s".into(),
                request_id: "r".into(),
                duration_ms: 42,
                artifact_id: Some("art_test".into()),
            })
            .await
            .unwrap();
        assert_eq!(capture.0.lock().unwrap().len(), 1);
    }
}
