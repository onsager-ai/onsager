//! [`SqlxPlanStore`] — a durable, spine-backed [`PlanStore`] (B1, #499).
//!
//! Replaces the ephemeral `InMemoryPlanStore` on the deployed run path
//! so node states and materialized artifacts survive a host restart
//! (RUN-01 #359 specced the scheduler as "spine-persisted,
//! restart-safe"). Persists all four [`PlanStore`] methods against the
//! `execution_plan_nodes` / `execution_plan_artifacts` tables
//! (`crates/onsager-spine/migrations/006_execution_plans.sql`).
//!
//! Lives in the scheduler host (not in `onsager-nodes`) for the same
//! reason [`crate::spine_client::SpineEventStoreClient`] does: the
//! executor-catalog crate stays database-free and test-friendly, and
//! only the deployed host pays the sqlx cost. `InMemoryPlanStore`
//! remains the test seam; no production path constructs it.

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, NodeId};
use onsager_nodes::{NodeState, PlanId, PlanStore, PlanStoreError};
use sqlx::PgPool;
use sqlx::Row;

/// Map a [`NodeState`] to its persisted text form. Matches the
/// `CHECK (state IN (...))` constraint and the serde `snake_case`
/// rename, kept explicit so a constraint mismatch is a compile-time
/// match-exhaustiveness signal rather than a runtime surprise.
fn node_state_to_str(state: NodeState) -> &'static str {
    match state {
        NodeState::Pending => "pending",
        NodeState::Ready => "ready",
        NodeState::Running => "running",
        NodeState::Completed => "completed",
        NodeState::Failed => "failed",
    }
}

/// Inverse of [`node_state_to_str`]. A row carrying an unknown state
/// is a schema-vs-code drift; surface it as a store error rather than
/// guessing.
fn node_state_from_str(s: &str) -> Result<NodeState, PlanStoreError> {
    match s {
        "pending" => Ok(NodeState::Pending),
        "ready" => Ok(NodeState::Ready),
        "running" => Ok(NodeState::Running),
        "completed" => Ok(NodeState::Completed),
        "failed" => Ok(NodeState::Failed),
        other => Err(PlanStoreError::new(format!(
            "unknown node state '{other}' in execution_plan_nodes"
        ))),
    }
}

/// Durable [`PlanStore`] over the spine Postgres. Cheap to clone
/// (wraps a reference-counted `PgPool`).
#[derive(Clone)]
pub struct SqlxPlanStore {
    pool: PgPool,
}

impl std::fmt::Debug for SqlxPlanStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlxPlanStore").finish_non_exhaustive()
    }
}

impl SqlxPlanStore {
    /// Wrap a pool. The pool must point at a database with the
    /// execution-plan tables applied (see
    /// `crates/onsager-spine/migrations/006_execution_plans.sql`; the
    /// scheduler host ensures them at startup via [`crate::migrate`]).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PlanStore for SqlxPlanStore {
    async fn set_node_state(
        &self,
        plan_id: &PlanId,
        node_id: NodeId,
        state: NodeState,
    ) -> Result<(), PlanStoreError> {
        sqlx::query(
            "INSERT INTO execution_plan_nodes (plan_id, node_id, state, updated_at) \
             VALUES ($1, $2, $3, NOW()) \
             ON CONFLICT (plan_id, node_id) \
             DO UPDATE SET state = EXCLUDED.state, updated_at = NOW()",
        )
        .bind(plan_id.as_str())
        .bind(node_id.to_string())
        .bind(node_state_to_str(state))
        .execute(&self.pool)
        .await
        .map_err(|e| PlanStoreError::new(format!("set_node_state failed: {e}")))?;
        Ok(())
    }

    async fn node_states(
        &self,
        plan_id: &PlanId,
    ) -> Result<std::collections::HashMap<NodeId, NodeState>, PlanStoreError> {
        let rows =
            sqlx::query("SELECT node_id, state FROM execution_plan_nodes WHERE plan_id = $1")
                .bind(plan_id.as_str())
                .fetch_all(&self.pool)
                .await
                .map_err(|e| PlanStoreError::new(format!("node_states failed: {e}")))?;

        let mut out = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let node_id_str: String = row
                .try_get("node_id")
                .map_err(|e| PlanStoreError::new(format!("node_id decode failed: {e}")))?;
            let state_str: String = row
                .try_get("state")
                .map_err(|e| PlanStoreError::new(format!("state decode failed: {e}")))?;
            let uuid = uuid::Uuid::parse_str(&node_id_str)
                .map_err(|e| PlanStoreError::new(format!("node_id not a uuid: {e}")))?;
            out.insert(NodeId::new(uuid), node_state_from_str(&state_str)?);
        }
        Ok(out)
    }

    async fn put_artifact(
        &self,
        plan_id: &PlanId,
        artifact_id: &ArtifactId,
        artifact: Artifact,
    ) -> Result<(), PlanStoreError> {
        let json = serde_json::to_value(&artifact)
            .map_err(|e| PlanStoreError::new(format!("artifact serialize failed: {e}")))?;
        sqlx::query(
            "INSERT INTO execution_plan_artifacts (plan_id, artifact_id, artifact_json, updated_at) \
             VALUES ($1, $2, $3, NOW()) \
             ON CONFLICT (plan_id, artifact_id) \
             DO UPDATE SET artifact_json = EXCLUDED.artifact_json, updated_at = NOW()",
        )
        .bind(plan_id.as_str())
        .bind(artifact_id.as_str())
        .bind(&json)
        .execute(&self.pool)
        .await
        .map_err(|e| PlanStoreError::new(format!("put_artifact failed: {e}")))?;
        Ok(())
    }

    async fn get_artifact(
        &self,
        plan_id: &PlanId,
        artifact_id: &ArtifactId,
    ) -> Result<Option<Artifact>, PlanStoreError> {
        let row = sqlx::query(
            "SELECT artifact_json FROM execution_plan_artifacts \
             WHERE plan_id = $1 AND artifact_id = $2",
        )
        .bind(plan_id.as_str())
        .bind(artifact_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| PlanStoreError::new(format!("get_artifact failed: {e}")))?;

        match row {
            None => Ok(None),
            Some(row) => {
                let json: serde_json::Value = row.try_get("artifact_json").map_err(|e| {
                    PlanStoreError::new(format!("artifact_json decode failed: {e}"))
                })?;
                let artifact = serde_json::from_value(json).map_err(|e| {
                    PlanStoreError::new(format!("artifact deserialize failed: {e}"))
                })?;
                Ok(Some(artifact))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_state_round_trips_through_text() {
        for state in [
            NodeState::Pending,
            NodeState::Ready,
            NodeState::Running,
            NodeState::Completed,
            NodeState::Failed,
        ] {
            let s = node_state_to_str(state);
            assert_eq!(node_state_from_str(s).unwrap(), state);
        }
    }

    #[test]
    fn unknown_node_state_is_an_error() {
        assert!(node_state_from_str("bogus").is_err());
    }
}
