//! Portal-side query layer over `onsager-spine/migrations/027_workflow_library`.
//!
//! The substrate's `WorkflowLibrary` struct
//! (`crates/onsager-substrate/src/workflow_library.rs`) exposes
//! `register` / `lookup` / `latest` — enough for the Plan Compiler to
//! resolve `kind → Workflow` at runtime. The MCP authoring surface
//! (spec #395) needs four extra read shapes the substrate intentionally
//! doesn't bake into its kernel:
//!
//! - **List every active kind** (`list_workflows_v2`).
//! - **Read a specific (kind, version) row** (`get_workflow_v2`).
//! - **Mark a row inactive** (`retire_workflow`, via the `retired_at`
//!   column added in migration 029).
//! - **Snapshot the whole active library** for `compile_dry_run` /
//!   `get_execution_plan` so the synchronous
//!   [`onsager_substrate::compile`] entry point can resolve every kind
//!   in one pass without re-entering the async runtime per spec.
//!
//! Helpers stay narrow — same `(pool, kind, …)` shape as
//! `onsager-portal::workflow_db` — and emit substrate types unchanged.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use onsager_substrate::{Workflow, WorkflowId, WorkflowLookup};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;

/// One row in `workflow_library`, hydrated for read-side responses.
///
/// `Clone` is intentionally not derived — the substrate's `Workflow`
/// carries `Box<dyn Executor>` which is not `Clone`-able through the
/// trait object. Callers that need a fresh copy of the workflow body
/// can round-trip through `serde_json::Value`; `typetag` preserves
/// the executor `kind` discriminator across the trip.
#[derive(Debug, Serialize)]
pub struct WorkflowLibraryRow {
    pub id: String,
    pub spec_kind: String,
    pub version: i32,
    pub workflow: Workflow,
    pub registered_at: DateTime<Utc>,
    pub retired_at: Option<DateTime<Utc>>,
}

/// One row's "card view" — `(spec_kind, version, retired_at)` without
/// the full workflow body. Used by `list_workflows_v2` to avoid
/// shipping every node graph in a workspace overview.
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowLibraryCard {
    pub id: String,
    pub spec_kind: String,
    pub version: i32,
    pub registered_at: DateTime<Utc>,
    pub retired_at: Option<DateTime<Utc>>,
}

/// `(id, version, workflow_json, registered_at, retired_at)` row
/// shape returned by `latest_active`. The fields stay positional so
/// sqlx can pattern-match against the column order in the SELECT.
type LatestActiveRow = (String, i32, Value, DateTime<Utc>, Option<DateTime<Utc>>);

/// `(id, workflow_json, registered_at, retired_at)` row shape returned
/// by the version-explicit branch of `get_by_kind`.
type VersionedRow = (String, Value, DateTime<Utc>, Option<DateTime<Utc>>);

/// `(id, spec_kind, version, registered_at, retired_at)` row shape
/// returned by `list_cards`.
type CardRow = (String, String, i32, DateTime<Utc>, Option<DateTime<Utc>>);

#[derive(Debug, Error)]
pub enum LibraryDbError {
    #[error("workflow library row not found")]
    NotFound,

    #[error("workflow library row is already retired")]
    AlreadyRetired,

    #[error("workflow library (de)serialization failed: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("workflow library database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Return the latest **active** (not retired) workflow for `kind`,
/// `Ok(None)` when no active row exists.
pub async fn latest_active(
    pool: &PgPool,
    spec_kind: &str,
) -> Result<Option<WorkflowLibraryRow>, LibraryDbError> {
    let row: Option<LatestActiveRow> = sqlx::query_as(
        "SELECT id, version, workflow_json, registered_at, retired_at \
         FROM workflow_library \
         WHERE spec_kind = $1 AND retired_at IS NULL \
         ORDER BY version DESC LIMIT 1",
    )
    .bind(spec_kind)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((id, version, json, registered_at, retired_at)) => Ok(Some(WorkflowLibraryRow {
            id,
            spec_kind: spec_kind.to_string(),
            version,
            workflow: serde_json::from_value(json)?,
            registered_at,
            retired_at,
        })),
        None => Ok(None),
    }
}

/// Fetch a specific row by `(spec_kind, version)`. Used by
/// `get_workflow_v2`. `version = None` → latest active row.
pub async fn get_by_kind(
    pool: &PgPool,
    spec_kind: &str,
    version: Option<i32>,
) -> Result<Option<WorkflowLibraryRow>, LibraryDbError> {
    if let Some(v) = version {
        let row: Option<VersionedRow> = sqlx::query_as(
            "SELECT id, workflow_json, registered_at, retired_at FROM workflow_library \
             WHERE spec_kind = $1 AND version = $2",
        )
        .bind(spec_kind)
        .bind(v)
        .fetch_optional(pool)
        .await?;
        match row {
            Some((id, json, registered_at, retired_at)) => Ok(Some(WorkflowLibraryRow {
                id,
                spec_kind: spec_kind.to_string(),
                version: v,
                workflow: serde_json::from_value(json)?,
                registered_at,
                retired_at,
            })),
            None => Ok(None),
        }
    } else {
        latest_active(pool, spec_kind).await
    }
}

/// List every kind in the library — one card per `(spec_kind,
/// latest-version)` pair. Retired-only kinds are still surfaced so
/// authors can see what they once had; `retired_at.is_some()` is the
/// caller's filter.
pub async fn list_cards(pool: &PgPool) -> Result<Vec<WorkflowLibraryCard>, LibraryDbError> {
    let rows: Vec<CardRow> = sqlx::query_as(
        "SELECT DISTINCT ON (spec_kind) \
            id, spec_kind, version, registered_at, retired_at \
         FROM workflow_library \
         ORDER BY spec_kind, version DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, spec_kind, version, registered_at, retired_at)| WorkflowLibraryCard {
                id,
                spec_kind,
                version,
                registered_at,
                retired_at,
            },
        )
        .collect())
}

/// Mark the **currently-active** version for `spec_kind` retired.
/// Errors with [`LibraryDbError::NotFound`] when no active row
/// exists, and [`LibraryDbError::AlreadyRetired`] when the latest
/// row is already past the retire gate (which can only happen if
/// `latest_active` raced with another caller — surfaced explicitly
/// so the MCP tool can report "already retired" without re-issuing
/// the destructive call).
pub async fn retire_latest(
    pool: &PgPool,
    spec_kind: &str,
) -> Result<WorkflowLibraryRow, LibraryDbError> {
    let active = latest_active(pool, spec_kind)
        .await?
        .ok_or(LibraryDbError::NotFound)?;
    let updated: Option<DateTime<Utc>> = sqlx::query_scalar(
        "UPDATE workflow_library SET retired_at = NOW() \
         WHERE id = $1 AND retired_at IS NULL \
         RETURNING retired_at",
    )
    .bind(&active.id)
    .fetch_optional(pool)
    .await?;

    match updated {
        Some(retired_at) => Ok(WorkflowLibraryRow {
            retired_at: Some(retired_at),
            ..active
        }),
        None => Err(LibraryDbError::AlreadyRetired),
    }
}

/// Snapshot the whole active library into an in-memory map keyed by
/// `spec_kind`. Used by `compile_dry_run` so the synchronous
/// [`onsager_substrate::compile`] entry point can resolve every
/// `SpecRef::kind` without per-spec async lookups.
///
/// `(spec_kind, version, workflow)` triples surface alongside the
/// map so the compile response can attribute each resolved workflow
/// to a concrete library row.
pub async fn snapshot_active(pool: &PgPool) -> Result<LibrarySnapshot, LibraryDbError> {
    // Window over (spec_kind, version DESC) and keep row #1 per
    // kind. DISTINCT ON gives that in one query.
    let rows: Vec<(String, String, i32, Value)> = sqlx::query_as(
        "SELECT DISTINCT ON (spec_kind) id, spec_kind, version, workflow_json \
         FROM workflow_library \
         WHERE retired_at IS NULL \
         ORDER BY spec_kind, version DESC",
    )
    .fetch_all(pool)
    .await?;

    let mut by_kind: HashMap<String, Workflow> = HashMap::new();
    let mut versions: HashMap<String, (String, i32)> = HashMap::new();
    for (id, spec_kind, version, json) in rows {
        let workflow: Workflow = serde_json::from_value(json)?;
        versions.insert(spec_kind.clone(), (id, version));
        by_kind.insert(spec_kind, workflow);
    }
    Ok(LibrarySnapshot { by_kind, versions })
}

/// An in-memory snapshot suitable for the synchronous
/// [`onsager_substrate::compile`] entry point.
///
/// Implements [`WorkflowLookup`] over `by_kind`; `WorkflowId` lookup
/// is left as `None` because the snapshot does not preserve the
/// substrate's internal `WorkflowId` identity (rows are keyed by
/// `(spec_kind, version)` in this layer). The compiler only needs
/// `by_kind` to resolve `SpecRef::kind`; `subworkflow_ref` ids that
/// reach `get` would surface as invariant-4 violations, which is the
/// correct downstream behavior.
#[derive(Debug, Default)]
pub struct LibrarySnapshot {
    by_kind: HashMap<String, Workflow>,
    /// `spec_kind → (row_id, version)` for the row that contributed
    /// the `Workflow` in `by_kind`. Surfaces in compile responses so
    /// authors can see which library row was picked.
    pub versions: HashMap<String, (String, i32)>,
}

impl LibrarySnapshot {
    /// Convenience for tests / future callers that need the kinds
    /// resolved by this snapshot.
    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.by_kind.keys().map(String::as_str)
    }
}

impl WorkflowLookup for LibrarySnapshot {
    fn get(&self, _id: WorkflowId) -> Option<&Workflow> {
        None
    }

    fn by_kind(&self, spec_kind: &str) -> Option<&Workflow> {
        self.by_kind.get(spec_kind)
    }
}
