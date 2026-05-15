//! [`SpineClient`] — the thin async port over the spine event store
//! that executors use at runtime.
//!
//! Executors are pure logic; they don't open database connections or
//! mount HTTP routes. They take a `&dyn SpineClient` and call into it
//! when they need to emit a runtime event or read the current state of
//! an artifact. The real implementation (a sqlx-backed adapter over the
//! `events` / `events_ext` tables) lives in `onsager-spine`; tests
//! pass a mock.
//!
//! This trait is intentionally minimal — only the surface
//! the first wave of executors (EXE-02..06) actually needs. New
//! capabilities (transactional reads across multiple artifacts, batched
//! emits, listening for replies) extend this trait rather than
//! introducing parallel ports.

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId};
use thiserror::Error;

/// Error returned by a [`SpineClient`] call.
///
/// Carries a free-text message rather than a closed enum: adapter
/// crates (`onsager-spine`, mocks) report their own underlying errors
/// without forcing every executor crate to depend on sqlx / reqwest /
/// etc. Executors typically convert this into [`crate::ExecutorError`]
/// via the `?` operator.
#[derive(Debug, Error)]
#[error("spine error: {0}")]
pub struct SpineError(pub String);

impl SpineError {
    /// Build a spine error from any displayable value.
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Async port over the spine event store, scoped to the surface an
/// executor needs while it runs.
///
/// Object-safe by design: no generic methods, no associated types.
/// Held inside [`crate::ExecutorContext`] as `Arc<dyn SpineClient>`.
#[async_trait]
pub trait SpineClient: Send + Sync + std::fmt::Debug {
    /// Append a runtime event to the spine, tagged with a kind string
    /// (the wire-format event name) and a JSON payload. The adapter
    /// owns timestamping and durability.
    async fn emit(&self, kind: &str, payload: serde_json::Value) -> Result<(), SpineError>;

    /// Read the current state of an artifact by id, if it exists.
    /// `Ok(None)` for a missing id is not an error — executors decide
    /// whether absence is a failure condition.
    async fn read_artifact(&self, id: &ArtifactId) -> Result<Option<Artifact>, SpineError>;
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use onsager_artifact::Kind;
    use std::sync::Mutex;

    /// In-memory mock used by this crate's tests. Captures every
    /// `emit` call and looks up artifacts in a fixed table.
    #[derive(Debug, Default)]
    pub struct MockSpine {
        pub emitted: Mutex<Vec<(String, serde_json::Value)>>,
    }

    #[async_trait]
    impl SpineClient for MockSpine {
        async fn emit(&self, kind: &str, payload: serde_json::Value) -> Result<(), SpineError> {
            self.emitted
                .lock()
                .unwrap()
                .push((kind.to_string(), payload));
            Ok(())
        }

        async fn read_artifact(&self, _id: &ArtifactId) -> Result<Option<Artifact>, SpineError> {
            Ok(None)
        }
    }

    /// A fresh, empty `Artifact` for cases where a test only needs the
    /// shape — never reads any specific field.
    pub fn dummy_artifact() -> Artifact {
        Artifact::new(Kind::Document, "fixture", "marvin", "test", vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn mock_spine_captures_emits() {
        let mock = Arc::new(MockSpine::default());
        // Exercise the trait surface — that's what executors see.
        let spine: Arc<dyn SpineClient> = Arc::clone(&mock) as _;
        spine
            .emit("test.kind", serde_json::json!({"hello": "world"}))
            .await
            .unwrap();
        spine
            .emit("test.other", serde_json::json!({"n": 7}))
            .await
            .unwrap();

        let emitted = mock.emitted.lock().unwrap();
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0].0, "test.kind");
        assert_eq!(emitted[1].0, "test.other");
    }

    #[tokio::test]
    async fn mock_spine_read_artifact_returns_none_by_default() {
        let spine: Arc<dyn SpineClient> = Arc::new(MockSpine::default());
        let id = dummy_artifact().artifact_id.clone();
        assert!(spine.read_artifact(&id).await.unwrap().is_none());
    }

    #[test]
    fn spine_error_displays_message() {
        let err = SpineError::new("connection refused");
        assert_eq!(err.to_string(), "spine error: connection refused");
    }
}
