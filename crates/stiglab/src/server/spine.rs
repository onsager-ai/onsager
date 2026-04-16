//! Optional event spine integration for emitting factory events to the
//! Onsager event store.

use onsager_spine::factory_event::FactoryEventKind;
use onsager_spine::{EventMetadata, EventStore};

/// Emits factory events to the Onsager event spine under the "stiglab" namespace.
#[derive(Clone)]
pub struct SpineEmitter {
    store: EventStore,
}

impl SpineEmitter {
    /// Connect to the Onsager event store at the given database URL.
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let store = EventStore::connect(database_url).await?;
        Ok(Self { store })
    }

    /// Emit a factory event to the extension event table under the "stiglab"
    /// namespace. Returns the assigned event ID.
    pub async fn emit(&self, event: FactoryEventKind) -> Result<i64, sqlx::Error> {
        let metadata = EventMetadata {
            correlation_id: None,
            causation_id: None,
            actor: "stiglab".to_string(),
        };
        let data = serde_json::to_value(&event).unwrap_or_default();
        let stream_id = event.stream_id();
        let event_type = event.event_type();

        self.store
            .append_ext(&stream_id, "stiglab", event_type, data, &metadata, None)
            .await
    }

    /// Get a reference to the underlying PostgreSQL pool for direct queries.
    pub fn pool(&self) -> &sqlx::PgPool {
        self.store.pool()
    }

    /// Emit a raw event to the extension event table under a given namespace.
    /// Used for events that don't map to a `FactoryEventKind` variant (e.g.,
    /// artifact registration from the dashboard).
    pub async fn emit_raw(
        &self,
        stream_id: &str,
        namespace: &str,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<i64, sqlx::Error> {
        let metadata = EventMetadata {
            correlation_id: None,
            causation_id: None,
            actor: namespace.to_string(),
        };
        self.store
            .append_ext(
                stream_id,
                namespace,
                event_type,
                data.clone(),
                &metadata,
                None,
            )
            .await
    }

    /// Emit a session-started event.
    pub async fn emit_session_started(
        &self,
        session_id: &str,
        request_id: &str,
        node_id: &str,
    ) -> Result<i64, sqlx::Error> {
        self.emit(FactoryEventKind::StiglabSessionCreated {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            node_id: node_id.to_string(),
        })
        .await
    }

    /// Emit a session-completed event.
    pub async fn emit_session_completed(
        &self,
        session_id: &str,
        request_id: &str,
        duration_ms: u64,
    ) -> Result<i64, sqlx::Error> {
        self.emit(FactoryEventKind::StiglabSessionCompleted {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            duration_ms,
        })
        .await
    }

    /// Emit a session-failed event.
    pub async fn emit_session_failed(
        &self,
        session_id: &str,
        request_id: &str,
        error: &str,
    ) -> Result<i64, sqlx::Error> {
        self.emit(FactoryEventKind::StiglabSessionFailed {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            error: error.to_string(),
        })
        .await
    }
}
