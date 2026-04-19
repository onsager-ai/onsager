//! Event-driven handler for stiglab session completions (issue #14 phase 2).
//!
//! The HTTP dispatcher in `cmd/serve.rs` is synchronous: it blocks the
//! pipeline tick until Stiglab returns. Long-running shaping tasks need an
//! asynchronous path, and this listener is the first piece of it.
//!
//! Why no namespace filter: the [`Listener`] namespace filter keys on a
//! `stream_id` prefix convention (`"<ns>:..."`). Stiglab currently writes
//! session events with the raw session UUID as `stream_id`, so subscribing
//! to the `stiglab` namespace would drop every notification. We filter on
//! `event_type` here instead, which is unambiguous.
//!
//! Data layout: `stiglab::server::spine::SpineEmitter::emit` writes events
//! into `events_ext` with the [`FactoryEventKind`] variant directly as the
//! `data` column; the `FactoryEvent` envelope is only used in the core
//! `events` table. Both code paths are supported below so a future migration
//! either way doesn't break this listener.

use std::sync::Arc;

use async_trait::async_trait;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};

/// A parsed stiglab.session_completed event (issue #14 phase 2, #39, #60).
#[derive(Debug, Clone)]
pub struct SessionCompleted {
    pub event_id: i64,
    pub session_id: String,
    pub request_id: String,
    pub duration_ms: u64,
    pub artifact_id: Option<String>,
    /// LLM token usage, if the producing runtime reported it (issue #39).
    pub token_usage: Option<onsager_spine::factory_event::TokenUsage>,
    /// Working-tree branch the agent pushed (issue #60), used by the portal
    /// for PR↔session vertical lineage. `None` when the agent didn't write
    /// to a git working dir.
    pub branch: Option<String>,
    /// PR number known at session completion (issue #60). Optional for the
    /// same reason as `branch`.
    pub pr_number: Option<u64>,
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
    Listener::new(store).with_since(since).run(dispatcher).await
}

struct Dispatcher<H: SessionCompletedHandler> {
    store: EventStore,
    handler: Arc<H>,
}

impl<H: SessionCompletedHandler> Dispatcher<H> {
    /// Load the event payload and parse it into the typed variant.
    async fn load_session_completed(
        &self,
        notification: &EventNotification,
    ) -> anyhow::Result<Option<SessionCompleted>> {
        let (id, kind) = match notification.table.as_str() {
            "events" => {
                let Some(row) = self.store.get_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                let envelope: FactoryEvent = serde_json::from_value(row.data)?;
                (row.id, envelope.event)
            }
            "events_ext" => {
                let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                let kind: FactoryEventKind = serde_json::from_value(row.data)?;
                (row.id, kind)
            }
            _ => return Ok(None),
        };

        let FactoryEventKind::StiglabSessionCompleted {
            session_id,
            request_id,
            duration_ms,
            artifact_id,
            token_usage,
            branch,
            pr_number,
        } = kind
        else {
            return Ok(None);
        };

        Ok(Some(SessionCompleted {
            event_id: id,
            session_id,
            request_id,
            duration_ms,
            artifact_id,
            token_usage,
            branch,
            pr_number,
        }))
    }
}

#[async_trait]
impl<H: SessionCompletedHandler> EventHandler for Dispatcher<H> {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "stiglab.session_completed" {
            return Ok(());
        }

        match self.load_session_completed(&notification).await {
            Ok(Some(evt)) => self.handler.on_session_completed(evt).await?,
            Ok(None) => {
                tracing::debug!(
                    id = notification.id,
                    table = %notification.table,
                    "session_completed notification had no matching row or mismatched variant"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load session_completed event");
            }
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
                token_usage: None,
                branch: None,
                pr_number: None,
            })
            .await
            .unwrap();
        assert_eq!(capture.0.lock().unwrap().len(), 1);
    }
}
