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
use onsager_artifact::{Artifact, ArtifactId, ContentRef};
use thiserror::Error;

/// Error returned by a [`SpineClient`] call.
///
/// Carries a free-text message rather than a closed enum: adapter
/// crates (`onsager-spine`, mocks) report their own underlying errors
/// without forcing every executor crate to depend on sqlx / reqwest /
/// etc. Executors typically convert this into [`crate::nodes::ExecutorError`]
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

/// One output an agent session emitted, read back from its
/// `events_ext` manifest (spec #520 §4a). Carries identity + content
/// pointer + agent-supplied metadata — **never** provenance. The
/// `AgentExecutor` stamps `Provenance::Uncertain { source: Agent }`
/// when it packages these onto the typed DAG (spec #520 §4c); the agent
/// must not be able to self-declare a stronger provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct EmittedArtifact {
    /// Stable id minted at emit time (`art_<ulid>`).
    pub artifact_id: String,
    /// Pointer to the addressable content the session produced.
    pub content_ref: ContentRef,
    /// Artifact kind tag (`code` / `document` / `pull_request` / …).
    pub kind: String,
    /// Human-readable summary of what was emitted.
    pub summary: String,
}

/// An agent session's output manifest: the outputs it emitted plus
/// whether it explicitly declared it produced nothing.
///
/// `emitted.is_empty() && !declared_empty` is the liveness-gate trip
/// condition (spec #520 §4c) — the session ended having neither
/// delivered nor declared empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionManifest {
    pub emitted: Vec<EmittedArtifact>,
    pub declared_empty: bool,
}

/// Async port over the spine event store, scoped to the surface an
/// executor needs while it runs.
///
/// Object-safe by design: no generic methods, no associated types.
/// Held inside [`crate::nodes::ExecutorContext`] as `Arc<dyn SpineClient>`.
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

    /// Read an agent session's output manifest (spec #520 §4a / §4c).
    ///
    /// The default returns an empty manifest so adapters that never
    /// serve agent sessions (most test stubs) don't have to override
    /// it. The deployed substrate scheduler's adapter overrides this to
    /// query the `events_ext` `session:<id>` stream — that read is what
    /// the `AgentExecutor`'s authoritative liveness gate keys off.
    async fn read_session_manifest(
        &self,
        _session_id: &str,
    ) -> Result<SessionManifest, SpineError> {
        Ok(SessionManifest::default())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use onsager_artifact::Kind;
    use std::sync::Mutex;

    /// In-memory mock used by this crate's tests. Captures every
    /// `emit` call, looks up artifacts in a fixed table, and serves a
    /// canned [`SessionManifest`] so the `AgentExecutor` liveness gate
    /// (spec #520 §4c) is unit-testable without a database.
    #[derive(Debug, Default)]
    pub struct MockSpine {
        pub emitted: Mutex<Vec<(String, serde_json::Value)>>,
        pub manifest: Mutex<SessionManifest>,
    }

    impl MockSpine {
        /// Build a mock that serves `manifest` from
        /// `read_session_manifest`.
        pub fn with_manifest(manifest: SessionManifest) -> Self {
            Self {
                emitted: Mutex::new(Vec::new()),
                manifest: Mutex::new(manifest),
            }
        }
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

        async fn read_session_manifest(
            &self,
            _session_id: &str,
        ) -> Result<SessionManifest, SpineError> {
            Ok(self.manifest.lock().unwrap().clone())
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
