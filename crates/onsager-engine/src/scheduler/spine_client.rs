//! [`SpineEventStoreClient`] — a real [`SpineClient`] backed by
//! [`onsager_spine::EventStore`].
//!
//! Bridges the executor-facing port (in `onsager-nodes`) to the
//! sqlx-backed spine implementation. Every `emit` call writes one
//! `events_ext` row in the `substrate` namespace.
//!
//! Lives here (in the scheduler binary) rather than in `onsager-nodes`
//! to keep the executor-catalog crate database-free — the library
//! stays test-friendly, only the deployed host pays the sqlx cost.
//!
//! ## `read_artifact` returns `None` in v1
//!
//! The spine `artifacts` schema (`crates/onsager-spine/migrations/
//! 002_artifacts.sql`) splits an `Artifact` across several tables
//! (`artifacts`, `artifact_versions`, `vertical_lineage`,
//! `horizontal_lineage`, `quality_signals`); reconstructing a single
//! `Artifact` value here would duplicate read logic that doesn't yet
//! exist in `onsager-artifact`. The deployed scheduler's executors
//! reach upstream artifacts through the [`PlanStore`], not through
//! `read_artifact` — every executor in the catalog today gathers its
//! inputs from `ExecutorContext::inputs`. Returning `Ok(None)` here
//! is the same contract `StubSpine` ships with in the catalog's own
//! tests. Wire a real reader when an executor needs it.

use crate::nodes::{EmittedArtifact, SessionManifest, SpineClient, SpineError};
use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId};
use onsager_spine::{EventMetadata, EventStore};

/// Substrate-scheduler [`SpineClient`] backed by the real spine event
/// store. Every emit writes one `events_ext` row in the `substrate`
/// namespace.
///
/// `workspace_id` scopes the indexed `events_ext.workspace_id` column
/// so dashboard queries can isolate runs per tenant. The bare
/// constructor pins it to `"default"`; production wiring threads each
/// fire's workflow-row workspace through [`Self::with_workspace`] so
/// node lifecycle events stay queryable per workspace alongside the
/// triggering fire (Copilot review feedback on PR #390).
#[derive(Clone)]
pub struct SpineEventStoreClient {
    store: EventStore,
    actor: String,
    workspace_id: String,
}

impl std::fmt::Debug for SpineEventStoreClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpineEventStoreClient")
            .field("actor", &self.actor)
            .field("workspace_id", &self.workspace_id)
            .finish_non_exhaustive()
    }
}

impl SpineEventStoreClient {
    pub fn new(store: EventStore, actor: impl Into<String>) -> Self {
        Self {
            store,
            actor: actor.into(),
            workspace_id: "default".to_string(),
        }
    }

    /// Build a workspace-scoped clone. The new client emits every
    /// event under `workspace_id`; the original is left untouched.
    pub fn with_workspace(&self, workspace_id: impl Into<String>) -> Self {
        Self {
            store: self.store.clone(),
            actor: self.actor.clone(),
            workspace_id: workspace_id.into(),
        }
    }
}

#[async_trait]
impl SpineClient for SpineEventStoreClient {
    async fn emit(&self, kind: &str, payload: serde_json::Value) -> Result<(), SpineError> {
        let plan_id = payload
            .get("plan_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let node_id = payload
            .get("node_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let stream_id = format!("substrate:{plan_id}:{node_id}");
        let metadata = EventMetadata {
            correlation_id: None,
            causation_id: None,
            actor: self.actor.clone(),
        };
        self.store
            .append_ext(
                &self.workspace_id,
                &stream_id,
                "substrate",
                kind,
                payload,
                &metadata,
                None,
            )
            .await
            .map_err(|e| SpineError::new(format!("append_ext failed: {e}")))?;
        Ok(())
    }

    async fn read_artifact(&self, _id: &ArtifactId) -> Result<Option<Artifact>, SpineError> {
        // See module docs — v1 returns None; executors gather inputs
        // through PlanStore, not through cross-plan spine reads.
        Ok(None)
    }

    /// Read an agent session's output manifest for the
    /// `AgentExecutor`'s authoritative liveness gate (spec #520 §4c).
    ///
    /// Delegates to the shared manifest contract in
    /// [`onsager_spine::session_manifest`] — the same code path portal's
    /// MCP tools (§4a) write through — and scopes the read to this
    /// client's `workspace_id` so a session-id collision across tenants
    /// can't leak outputs. Maps the spine-side records onto the
    /// executor-port `SessionManifest` (the two structs live on opposite
    /// sides of the seam; this is the adapter that bridges them).
    async fn read_session_manifest(&self, session_id: &str) -> Result<SessionManifest, SpineError> {
        let manifest = onsager_spine::session_manifest::read_manifest(
            &self.store,
            &self.workspace_id,
            session_id,
        )
        .await
        .map_err(|e| SpineError::new(format!("read_session_manifest failed: {e}")))?;
        Ok(SessionManifest {
            emitted: manifest
                .emitted
                .into_iter()
                .map(|r| EmittedArtifact {
                    artifact_id: r.artifact_id,
                    content_ref: r.content_ref,
                    kind: r.kind,
                    summary: r.summary,
                })
                .collect(),
            declared_empty: manifest.declared_empty,
        })
    }
}
