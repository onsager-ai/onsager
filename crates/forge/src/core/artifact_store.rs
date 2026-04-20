//! In-memory artifact store — the single-writer for artifact state (forge-v0.1 §10 invariant #1).
//!
//! Only Forge mutates artifact state. This store enforces:
//! - Valid state transitions (via `ArtifactState::can_transition_to`)
//! - Atomic version creation with lineage
//! - Quality signal append-only semantics

use std::collections::HashMap;

use chrono::Utc;

use onsager_artifact::{
    Artifact, ArtifactId, ArtifactState, ArtifactVersion, BundleId, ContentRef, Kind,
    VerticalLineage,
};
use onsager_protocol::ShapingResult;
use onsager_spine::factory_event::ShapingOutcome;

/// In-memory artifact store. Production implementations would back this
/// with PostgreSQL; this provides the domain logic and invariant enforcement.
#[derive(Debug, Default)]
pub struct ArtifactStore {
    artifacts: HashMap<String, Artifact>,
}

impl ArtifactStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new artifact in `Draft` state.
    pub fn register(
        &mut self,
        kind: Kind,
        name: impl Into<String>,
        owner: impl Into<String>,
    ) -> ArtifactId {
        let artifact = Artifact::new(kind, name, owner, "forge", vec![]);
        let id = artifact.artifact_id.clone();
        self.artifacts.insert(id.as_str().to_owned(), artifact);
        id
    }

    /// Insert a pre-built artifact. Used both for DB-first registration
    /// (issue #30) and for restoring state during startup reload — the
    /// caller builds the `Artifact` with the right state/version/bundle
    /// fields set and this just drops it into the map.
    pub fn insert(&mut self, artifact: Artifact) {
        self.artifacts
            .insert(artifact.artifact_id.as_str().to_owned(), artifact);
    }

    /// Get an artifact by ID.
    pub fn get(&self, id: &ArtifactId) -> Option<&Artifact> {
        self.artifacts.get(id.as_str())
    }

    /// Mutable access — used by the workflow stage runner (issue #80) to
    /// update `workflow_id`, `current_stage_index`,
    /// `workflow_parked_reason`, and the artifact state when a stage
    /// declares a `target_state`. Version/bundle/lineage mutations still
    /// go through `advance` and `record_bundle` so the state-machine and
    /// invariants stay enforced.
    pub fn get_mut(&mut self, id: &ArtifactId) -> Option<&mut Artifact> {
        self.artifacts.get_mut(id.as_str())
    }

    /// List all artifacts in non-terminal states.
    pub fn active_artifacts(&self) -> Vec<&Artifact> {
        self.artifacts
            .values()
            .filter(|a| !matches!(a.state, ArtifactState::Archived))
            .collect()
    }

    /// Advance an artifact's state after a successful shaping + gate approval.
    ///
    /// This enforces the state machine and creates a new version atomically
    /// (forge invariant #2).
    pub fn advance(
        &mut self,
        id: &ArtifactId,
        target_state: ArtifactState,
        result: &ShapingResult,
    ) -> Result<(), ArtifactStoreError> {
        let artifact = self
            .artifacts
            .get_mut(id.as_str())
            .ok_or_else(|| ArtifactStoreError::NotFound(id.clone()))?;

        if !artifact.state.can_transition_to(target_state) {
            return Err(ArtifactStoreError::InvalidTransition {
                artifact_id: id.clone(),
                from: artifact.state,
                to: target_state,
            });
        }

        // Only create a new version for successful outcomes.
        if result.outcome == ShapingOutcome::Completed {
            let version = ArtifactVersion {
                version: artifact.current_version + 1,
                created_at: Utc::now(),
                created_by_session: result.session_id.clone(),
                content_ref: result.content_ref.clone().unwrap_or(ContentRef {
                    uri: format!("pending://{}", result.request_id),
                    checksum: None,
                }),
                change_summary: result.change_summary.clone(),
                parent_version: if artifact.current_version > 0 {
                    Some(artifact.current_version)
                } else {
                    None
                },
            };

            artifact.current_version = version.version;
            artifact.vertical_lineage.push(VerticalLineage {
                session_id: result.session_id.clone(),
                version: version.version,
                recorded_at: Utc::now(),
            });
            artifact.versions.push(version);
        }

        // Append quality signals from the result.
        artifact
            .quality_signals
            .extend(result.quality_signals.clone());

        // Transition state.
        artifact.state = target_state;

        Ok(())
    }

    /// Record a newly sealed bundle on the artifact
    /// (warehouse-and-delivery-v0.1 §6.3).
    ///
    /// No-op if the artifact is not found (the caller already emits an error
    /// event for the missing artifact; a missing bundle recording is strictly
    /// subordinate to that).
    pub fn record_bundle(&mut self, id: &ArtifactId, bundle_id: BundleId) {
        if let Some(artifact) = self.artifacts.get_mut(id.as_str()) {
            artifact.record_bundle(bundle_id);
        }
    }

    /// Archive an artifact (any state -> Archived).
    pub fn archive(&mut self, id: &ArtifactId, _reason: &str) -> Result<(), ArtifactStoreError> {
        let artifact = self
            .artifacts
            .get_mut(id.as_str())
            .ok_or_else(|| ArtifactStoreError::NotFound(id.clone()))?;

        if artifact.state == ArtifactState::Archived {
            return Err(ArtifactStoreError::AlreadyArchived(id.clone()));
        }

        artifact.state = ArtifactState::Archived;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactStoreError {
    #[error("artifact not found: {0}")]
    NotFound(ArtifactId),
    #[error("invalid transition for {artifact_id}: {from} -> {to}")]
    InvalidTransition {
        artifact_id: ArtifactId,
        from: ArtifactState,
        to: ArtifactState,
    },
    #[error("artifact already archived: {0}")]
    AlreadyArchived(ArtifactId),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::Kind;

    fn make_result(outcome: ShapingOutcome) -> ShapingResult {
        ShapingResult {
            request_id: "req_001".into(),
            outcome,
            content_ref: Some(ContentRef {
                uri: "git://repo@abc123".into(),
                checksum: None,
            }),
            change_summary: "implemented feature X".into(),
            quality_signals: vec![],
            session_id: "sess_001".into(),
            duration_ms: 5000,
            error: None,
        }
    }

    #[test]
    fn register_and_get() {
        let mut store = ArtifactStore::new();
        let id = store.register(Kind::Code, "my-service", "marvin");
        let art = store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::Draft);
        assert_eq!(art.current_version, 0);
    }

    #[test]
    fn advance_through_lifecycle() {
        let mut store = ArtifactStore::new();
        let id = store.register(Kind::Code, "my-service", "marvin");

        let result = make_result(ShapingOutcome::Completed);

        // Draft -> InProgress
        store
            .advance(&id, ArtifactState::InProgress, &result)
            .unwrap();
        let art = store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::InProgress);
        assert_eq!(art.current_version, 1);
        assert_eq!(art.versions.len(), 1);

        // InProgress -> UnderReview
        store
            .advance(&id, ArtifactState::UnderReview, &result)
            .unwrap();
        let art = store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::UnderReview);
        assert_eq!(art.current_version, 2);

        // UnderReview -> Released
        store
            .advance(&id, ArtifactState::Released, &result)
            .unwrap();
        assert_eq!(store.get(&id).unwrap().state, ArtifactState::Released);
    }

    #[test]
    fn advance_invalid_transition() {
        let mut store = ArtifactStore::new();
        let id = store.register(Kind::Code, "my-service", "marvin");
        let result = make_result(ShapingOutcome::Completed);

        // Draft -> Released is invalid
        let err = store.advance(&id, ArtifactState::Released, &result);
        assert!(err.is_err());
    }

    #[test]
    fn advance_partial_does_not_create_version() {
        let mut store = ArtifactStore::new();
        let id = store.register(Kind::Code, "my-service", "marvin");

        let result = make_result(ShapingOutcome::Partial);
        store
            .advance(&id, ArtifactState::InProgress, &result)
            .unwrap();

        let art = store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::InProgress);
        assert_eq!(art.current_version, 0); // No version bump
        assert!(art.versions.is_empty());
    }

    #[test]
    fn archive_from_any_state() {
        let mut store = ArtifactStore::new();
        let id = store.register(Kind::Code, "my-service", "marvin");
        store.archive(&id, "cancelled").unwrap();
        assert_eq!(store.get(&id).unwrap().state, ArtifactState::Archived);
    }

    #[test]
    fn archive_already_archived() {
        let mut store = ArtifactStore::new();
        let id = store.register(Kind::Code, "my-service", "marvin");
        store.archive(&id, "cancelled").unwrap();
        assert!(store.archive(&id, "again").is_err());
    }

    #[test]
    fn active_artifacts_excludes_archived() {
        let mut store = ArtifactStore::new();
        let id1 = store.register(Kind::Code, "active", "marvin");
        let id2 = store.register(Kind::Code, "archived", "marvin");
        store.archive(&id2, "done").unwrap();

        let active = store.active_artifacts();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].artifact_id, id1);
    }

    #[test]
    fn advance_records_vertical_lineage() {
        let mut store = ArtifactStore::new();
        let id = store.register(Kind::Code, "my-service", "marvin");

        let result = make_result(ShapingOutcome::Completed);
        store
            .advance(&id, ArtifactState::InProgress, &result)
            .unwrap();

        let art = store.get(&id).unwrap();
        assert_eq!(art.vertical_lineage.len(), 1);
        assert_eq!(art.vertical_lineage[0].session_id, "sess_001");
        assert_eq!(art.vertical_lineage[0].version, 1);
    }
}
