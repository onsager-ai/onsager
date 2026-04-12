use onsager_events::{CoreEvent, EventRecord, EventStore, ExtensionEventRecord};

use crate::session::{Session, SessionState};

/// An entry in a replayed timeline — either a core event or an extension event.
#[derive(Debug)]
pub enum ReplayEntry {
    Core(EventRecord),
    Extension(ExtensionEventRecord),
}

impl ReplayEntry {
    pub fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        match self {
            ReplayEntry::Core(e) => e.created_at,
            ReplayEntry::Extension(e) => e.created_at,
        }
    }
}

/// Replays the event stream for a session, reconstructing state from events.
pub struct ReplayEngine {
    store: EventStore,
}

impl ReplayEngine {
    pub fn new(store: EventStore) -> Self {
        Self { store }
    }

    /// Replay all events for a session, yielding them in chronological order.
    pub async fn replay_session(
        &self,
        session_id: &str,
        from_sequence: i64,
        include_ext: bool,
    ) -> anyhow::Result<Vec<ReplayEntry>> {
        let core_events = self.store.query_stream(session_id, from_sequence).await?;

        let mut entries: Vec<ReplayEntry> =
            core_events.into_iter().map(ReplayEntry::Core).collect();

        if include_ext {
            let ext_events = self.store.query_ext_stream(session_id).await?;
            entries.extend(ext_events.into_iter().map(ReplayEntry::Extension));
            entries.sort_by_key(|e| e.created_at());
        }

        Ok(entries)
    }

    /// Materialize a Session struct by folding the event stream.
    pub async fn materialize_session(&self, session_id: &str) -> anyhow::Result<Option<Session>> {
        let events = self.store.query_stream(session_id, 0).await?;
        if events.is_empty() {
            return Ok(None);
        }

        let mut session = Session::default();
        for record in &events {
            if let Ok(event) = serde_json::from_value::<CoreEvent>(record.data.clone()) {
                session.apply(&event);
            }
            session.updated_at = record.created_at;
            if session.created_at == session.updated_at && record.sequence == 1 {
                session.created_at = record.created_at;
            }
        }
        // Set created_at from the first event
        if let Some(first) = events.first() {
            session.created_at = first.created_at;
        }

        Ok(Some(session))
    }

    /// List all session IDs by looking for session.created events.
    pub async fn list_sessions(
        &self,
        state_filter: Option<SessionState>,
    ) -> anyhow::Result<Vec<Session>> {
        let created_events = self
            .store
            .query_events(None, Some("session.created"), None, 1000)
            .await?;

        let mut sessions = Vec::new();
        for record in created_events {
            if let Some(session) = self.materialize_session(&record.stream_id).await? {
                if let Some(filter) = &state_filter {
                    if session.state != *filter {
                        continue;
                    }
                }
                sessions.push(session);
            }
        }

        Ok(sessions)
    }
}
