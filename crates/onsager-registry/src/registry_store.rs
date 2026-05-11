//! DB-facing registry operations — query, propose, approve, deprecate.
//!
//! This is the projection layer that makes the registry usable from code.
//! The registry tables (`artifact_types`, `artifact_adapters`,
//! `gate_evaluators`, `agent_profiles`) are the source of truth.
//!
//! Per spec #285 these mutations no longer publish a `registry.*` spine
//! event — the previous events had no in-tree consumer or dashboard
//! reader and were instrumented pre-emptively for a registry timeline
//! UI that never landed. Add an event back when there is a real
//! consumer.
//!
//! See `registry.rs` for the trait-based plug points and value objects.

use chrono::{DateTime, Utc};
use onsager_spine::EventStore;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};

use crate::registry::{DEFAULT_WORKSPACE, RegistryStatus, TypeDefinition};

/// Row-like view of an entry in one of the four registry tables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryRecord {
    pub id: String,
    pub workspace_id: String,
    pub revision: i32,
    pub status: RegistryStatus,
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Which registry table to target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryKind {
    Type,
    Adapter,
    Gate,
    Profile,
}

impl RegistryKind {
    fn table(&self) -> &'static str {
        match self {
            Self::Type => "artifact_types",
            Self::Adapter => "artifact_adapters",
            Self::Gate => "gate_evaluators",
            Self::Profile => "agent_profiles",
        }
    }

    fn id_column(&self) -> &'static str {
        match self {
            Self::Type => "type_id",
            Self::Adapter => "adapter_id",
            Self::Gate => "evaluator_id",
            Self::Profile => "profile_id",
        }
    }

    fn payload_column(&self) -> &'static str {
        // `artifact_types` uses `definition`; the other three use `config`.
        match self {
            Self::Type => "definition",
            _ => "config",
        }
    }
}

/// High-level API for reading and mutating the registry.
#[derive(Clone)]
pub struct RegistryStore {
    store: EventStore,
    workspace_id: String,
}

impl RegistryStore {
    pub fn new(store: EventStore) -> Self {
        Self {
            store,
            workspace_id: DEFAULT_WORKSPACE.to_owned(),
        }
    }

    pub fn with_workspace(mut self, workspace_id: impl Into<String>) -> Self {
        self.workspace_id = workspace_id.into();
        self
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    /// List all entries of a kind in the current workspace, ordered by id.
    pub async fn list(&self, kind: RegistryKind) -> sqlx::Result<Vec<RegistryRecord>> {
        let sql = format!(
            "SELECT {id} AS id, workspace_id, revision, status, {payload} AS config, \
                    created_at, updated_at \
             FROM {table} \
             WHERE workspace_id = $1 \
             ORDER BY {id}",
            id = kind.id_column(),
            payload = kind.payload_column(),
            table = kind.table(),
        );
        let rows = sqlx::query(&sql)
            .bind(&self.workspace_id)
            .fetch_all(self.store.pool())
            .await?;
        rows.into_iter().map(row_to_record).collect()
    }

    /// Fetch a single entry by id. Returns `None` if not present.
    pub async fn get(&self, kind: RegistryKind, id: &str) -> sqlx::Result<Option<RegistryRecord>> {
        let sql = format!(
            "SELECT {id} AS id, workspace_id, revision, status, {payload} AS config, \
                    created_at, updated_at \
             FROM {table} \
             WHERE workspace_id = $1 AND {id} = $2",
            id = kind.id_column(),
            payload = kind.payload_column(),
            table = kind.table(),
        );
        let row = sqlx::query(&sql)
            .bind(&self.workspace_id)
            .bind(id)
            .fetch_optional(self.store.pool())
            .await?;
        row.map(row_to_record).transpose()
    }

    /// Propose a new artifact type. Inserts at revision 1 with status
    /// `proposed`. Idempotent: rerunning with the same id is a no-op.
    /// Per spec #285 no spine event is emitted.
    pub async fn propose_type(&self, def: &TypeDefinition, actor: &str) -> anyhow::Result<bool> {
        let workspace = self.workspace_id.clone();
        let actor = actor.to_owned();
        let def = def.clone();

        self.store
            .transaction(move |tx| {
                Box::pin(async move { propose_type_in_tx(tx, &workspace, &def, &actor).await })
            })
            .await
            .map_err(anyhow::Error::from)
    }

    /// Approve a previously-proposed type (or no-op if already approved).
    pub async fn approve_type(&self, type_id: &str, actor: &str) -> anyhow::Result<bool> {
        let workspace = self.workspace_id.clone();
        let actor = actor.to_owned();
        let type_id = type_id.to_owned();

        self.store
            .transaction(move |tx| {
                Box::pin(async move { approve_type_in_tx(tx, &workspace, &type_id, &actor).await })
            })
            .await
            .map_err(anyhow::Error::from)
    }

    /// Deprecate a type. Returns `true` if the row was actually
    /// flipped to `deprecated`, `false` if it was already deprecated.
    /// Per spec #285 no spine event is emitted and no `reason` column
    /// exists on `artifact_types`; if a deprecation rationale needs to
    /// be persisted, that's a schema change to the registry tables
    /// rather than an event emission.
    pub async fn deprecate_type(&self, type_id: &str, actor: &str) -> anyhow::Result<bool> {
        let workspace = self.workspace_id.clone();
        let actor = actor.to_owned();
        let type_id = type_id.to_owned();

        self.store
            .transaction(move |tx| {
                Box::pin(
                    async move { deprecate_type_in_tx(tx, &workspace, &type_id, &actor).await },
                )
            })
            .await
            .map_err(anyhow::Error::from)
    }
}

// ---------------------------------------------------------------------------
// Transactional helpers (return sqlx::Error so they compose with transaction())
// ---------------------------------------------------------------------------

async fn propose_type_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: &str,
    def: &TypeDefinition,
    _actor: &str,
) -> sqlx::Result<bool> {
    let definition = serde_json::to_value(def)
        .map_err(|e| sqlx::Error::Protocol(format!("serialize TypeDefinition: {e}")))?;

    let inserted: Option<(i32,)> = sqlx::query_as(
        r#"
        INSERT INTO artifact_types (type_id, workspace_id, revision, status, definition)
        VALUES ($1, $2, 1, 'proposed', $3)
        ON CONFLICT (workspace_id, type_id) DO NOTHING
        RETURNING revision
        "#,
    )
    .bind(def.type_id.as_str())
    .bind(workspace_id)
    .bind(&definition)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(inserted.is_some())
}

async fn approve_type_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: &str,
    type_id: &str,
    _actor: &str,
) -> sqlx::Result<bool> {
    let updated: Option<(i32,)> = sqlx::query_as(
        r#"
        UPDATE artifact_types SET status = 'approved'
        WHERE workspace_id = $1 AND type_id = $2 AND status <> 'approved'
        RETURNING revision
        "#,
    )
    .bind(workspace_id)
    .bind(type_id)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(updated.is_some())
}

async fn deprecate_type_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: &str,
    type_id: &str,
    _actor: &str,
) -> sqlx::Result<bool> {
    let updated: Option<(i32,)> = sqlx::query_as(
        r#"
        UPDATE artifact_types SET status = 'deprecated'
        WHERE workspace_id = $1 AND type_id = $2 AND status <> 'deprecated'
        RETURNING revision
        "#,
    )
    .bind(workspace_id)
    .bind(type_id)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(updated.is_some())
}

fn row_to_record(row: sqlx::postgres::PgRow) -> sqlx::Result<RegistryRecord> {
    let status: String = row.try_get("status")?;
    let status = match status.as_str() {
        "proposed" => RegistryStatus::Proposed,
        "approved" => RegistryStatus::Approved,
        "deprecated" => RegistryStatus::Deprecated,
        other => {
            return Err(sqlx::Error::Protocol(format!(
                "unknown registry status: {other}"
            )));
        }
    };
    Ok(RegistryRecord {
        id: row.try_get("id")?,
        workspace_id: row.try_get("workspace_id")?,
        revision: row.try_get("revision")?,
        status,
        config: row.try_get("config")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_kind_table_mapping() {
        assert_eq!(RegistryKind::Type.table(), "artifact_types");
        assert_eq!(RegistryKind::Adapter.table(), "artifact_adapters");
        assert_eq!(RegistryKind::Gate.table(), "gate_evaluators");
        assert_eq!(RegistryKind::Profile.table(), "agent_profiles");
    }

    #[test]
    fn registry_kind_columns() {
        assert_eq!(RegistryKind::Type.id_column(), "type_id");
        assert_eq!(RegistryKind::Type.payload_column(), "definition");
        assert_eq!(RegistryKind::Adapter.payload_column(), "config");
        assert_eq!(RegistryKind::Gate.payload_column(), "config");
    }
}
