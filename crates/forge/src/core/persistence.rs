//! Forge state persistence (issue #30).
//!
//! Forge tick transitions are applied to an in-memory [`ArtifactStore`] while
//! the spine database holds the durable projection. Without a write path
//! from the tick to the DB, a restart rolls every active artifact back to
//! whatever state was in `artifacts` at registration time — all advances,
//! version bumps, and sealed bundles survive only as append-only rows in
//! `events_ext`, which nothing reads back.
//!
//! This module provides two halves of the projection:
//!
//! - [`load_artifact_store`]: on startup, read the `artifacts` table and
//!   rebuild the in-memory store. Includes `current_bundle_id` so sealed
//!   releases survive restart (forge invariant: the warehouse pointer on
//!   each artifact is the tip of its bundle chain).
//! - [`persist_artifact_state`]: after a tick produces an
//!   `ArtifactAdvanced` or `BundleSealed` event and releases the write
//!   lock, mirror the resulting in-memory state to the `artifacts` row.
//!
//! The write is best-effort at the call site: failures are returned so
//! the caller can log loudly, but the tick itself does not rollback. If
//! the DB write fails, the next successful transition or a deliberate
//! reconciliation pass will catch the drift.

use onsager_artifact::{Artifact, ArtifactId, ArtifactState, BundleId, Kind};
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::artifact_store::ArtifactStore;

/// Map an [`ArtifactState`] to the `state` TEXT value used by the
/// `artifacts` CHECK constraint (see `002_artifacts.sql`).
pub fn state_to_db(state: ArtifactState) -> &'static str {
    match state {
        ArtifactState::Draft => "draft",
        ArtifactState::InProgress => "in_progress",
        ArtifactState::UnderReview => "under_review",
        ArtifactState::Released => "released",
        ArtifactState::Archived => "archived",
    }
}

/// Inverse of [`state_to_db`]. Unknown values fall back to `Draft` — the
/// CHECK constraint guarantees no unknown value can reach this function
/// from the DB, so this branch only protects against migration drift.
pub fn state_from_db(s: &str) -> ArtifactState {
    match s {
        "in_progress" => ArtifactState::InProgress,
        "under_review" => ArtifactState::UnderReview,
        "released" => ArtifactState::Released,
        "archived" => ArtifactState::Archived,
        _ => ArtifactState::Draft,
    }
}

fn kind_from_db(s: &str) -> Kind {
    match s {
        "code" => Kind::Code,
        "document" => Kind::Document,
        "pull_request" => Kind::PullRequest,
        other => Kind::Custom(other.to_string()),
    }
}

/// Rebuild an [`ArtifactStore`] from the `artifacts` table.
///
/// Skips rows in `archived` state — the in-memory store is for active
/// artifacts only (forge-v0.1 §10).
pub async fn load_artifact_store(pool: &PgPool) -> Result<ArtifactStore, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT artifact_id, kind, name, owner, state, current_version, current_bundle_id \
         FROM artifacts \
         WHERE state != 'archived'",
    )
    .fetch_all(pool)
    .await?;

    let mut store = ArtifactStore::new();
    for row in &rows {
        let id: String = row.get("artifact_id");
        let kind: String = row.get("kind");
        let name: String = row.get("name");
        let owner: String = row.get("owner");
        let state_str: String = row.get("state");
        let version: i32 = row.get("current_version");
        let bundle_id: Option<String> = row.get("current_bundle_id");

        let mut artifact = Artifact::new(kind_from_db(&kind), name, owner, "forge", vec![]);
        artifact.artifact_id = ArtifactId::new(&id);
        artifact.state = state_from_db(&state_str);
        artifact.current_version = version as u32;
        artifact.current_bundle_id = bundle_id.map(BundleId::new);
        store.insert(artifact);
    }

    Ok(store)
}

/// Register a new artifact in the spine database.
///
/// Runs the INSERT inside an explicit transaction so the row and any
/// caller-supplied extensions (e.g. a factory event emit) either commit
/// together or not at all. The in-memory store is updated only after the
/// transaction commits — no partial state is visible to subsequent
/// requests.
pub async fn insert_artifact_row(
    pool: &PgPool,
    artifact_id: &str,
    kind: &str,
    name: &str,
    owner: &str,
) -> Result<(), sqlx::Error> {
    let mut tx: Transaction<'_, Postgres> = pool.begin().await?;

    sqlx::query(
        "INSERT INTO artifacts \
             (artifact_id, kind, name, owner, created_by, state, current_version) \
         VALUES ($1, $2, $3, $4, 'forge', 'draft', 0) \
         ON CONFLICT (artifact_id) DO NOTHING",
    )
    .bind(artifact_id)
    .bind(kind)
    .bind(name)
    .bind(owner)
    .execute(&mut *tx)
    .await?;

    tx.commit().await
}

/// Mirror the post-tick state of `artifact` to the `artifacts` row.
///
/// Writes `state`, `current_version`, and `current_bundle_id` from the
/// in-memory snapshot. The trigger in `002_artifacts.sql` refreshes
/// `updated_at` automatically.
pub async fn persist_artifact_state(pool: &PgPool, artifact: &Artifact) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE artifacts \
            SET state = $1, current_version = $2, current_bundle_id = $3 \
          WHERE artifact_id = $4",
    )
    .bind(state_to_db(artifact.state))
    .bind(artifact.current_version as i32)
    .bind(artifact.current_bundle_id.as_ref().map(|b| b.as_str()))
    .bind(artifact.artifact_id.as_str())
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrip() {
        for s in [
            ArtifactState::Draft,
            ArtifactState::InProgress,
            ArtifactState::UnderReview,
            ArtifactState::Released,
            ArtifactState::Archived,
        ] {
            assert_eq!(state_from_db(state_to_db(s)), s);
        }
    }

    #[test]
    fn state_from_db_unknown_defaults_draft() {
        assert_eq!(state_from_db("not_a_state"), ArtifactState::Draft);
    }

    #[test]
    fn kind_from_db_preserves_custom() {
        assert_eq!(kind_from_db("code"), Kind::Code);
        assert_eq!(kind_from_db("document"), Kind::Document);
        assert_eq!(kind_from_db("pull_request"), Kind::PullRequest);
        assert_eq!(kind_from_db("my_kind"), Kind::Custom("my_kind".to_string()));
    }
}
