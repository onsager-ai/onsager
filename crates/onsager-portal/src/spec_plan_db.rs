//! Persistence for substrate `SpecPlan` rows authored via MCP
//! (spec #395, ADR 0015).
//!
//! The schema mirrors the on-the-wire `SpecPlan` shape: one JSONB
//! column carrying the `specs` + `deps` arrays plus audit columns.
//! See `crates/onsager-portal/migrations/010_spec_plans.sql`.
//!
//! Operations:
//!
//! - [`insert`] — first-write of a plan under
//!   `(workspace_id, spec_plan_id)`. Returns
//!   [`SpecPlanStoreError::Duplicate`] if a row already exists; the
//!   MCP `submit_spec_plan` tool surfaces that as `InvalidParams`.
//! - [`get`] — single row lookup, `Ok(None)` when missing.
//! - [`list`] — every plan for a workspace, ordered by `created_at
//!   DESC`. Used by `list_spec_plans`.
//! - [`replace_spec`] — read-modify-write helper for `update_spec`.
//!   Replaces a single [`SpecRef`] inside the JSONB plan and bumps
//!   `updated_at`.

use chrono::{DateTime, Utc};
use onsager_substrate::{SpecId, SpecPlan, SpecPlanError, SpecRef};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;

/// A persisted Spec Plan row as returned by [`get`] / [`list`].
#[derive(Debug, Clone, Serialize)]
pub struct StoredSpecPlan {
    pub workspace_id: String,
    pub spec_plan_id: String,
    pub plan: SpecPlan,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Errors returned by [`SpecPlanStore`] operations.
#[derive(Debug, Error)]
pub enum SpecPlanStoreError {
    /// A row already exists at `(workspace_id, spec_plan_id)`.
    #[error("spec plan '{0}' already exists in this workspace")]
    Duplicate(String),

    /// `update_spec` targeted a plan that does not exist.
    #[error("spec plan '{0}' not found")]
    PlanNotFound(String),

    /// `update_spec` referenced a `SpecId` that is not present in
    /// the plan's `specs` array.
    #[error("spec '{0}' not found in plan")]
    SpecNotFound(SpecId),

    /// The post-swap plan failed structural validation. Surfaced
    /// *before* the DB write — the stored row is unchanged.
    #[error("updated spec plan failed validation: {0}")]
    Validation(#[from] SpecPlanError),

    /// JSONB content failed to round-trip via serde.
    #[error("spec plan (de)serialization failed: {0}")]
    Serde(#[from] serde_json::Error),

    /// Anything else (connection, timeout, schema mismatch).
    #[error("spec plan database error: {0}")]
    Database(#[from] sqlx::Error),
}

const DUPLICATE_PK_CONSTRAINT: &str = "spec_plans_pkey";

/// `(spec_plan_id, plan_json, created_by, created_at, updated_at)`
/// row shape returned by `list`. Stays positional so sqlx pattern-
/// matches against the SELECT column order.
type ListRow = (String, Value, String, DateTime<Utc>, DateTime<Utc>);

/// First-write of a plan. Returns [`SpecPlanStoreError::Duplicate`]
/// when the row already exists; the unique pkey on
/// `(workspace_id, spec_plan_id)` is the source of truth.
pub async fn insert(
    pool: &PgPool,
    workspace_id: &str,
    spec_plan_id: &str,
    plan: &SpecPlan,
    created_by: &str,
) -> Result<StoredSpecPlan, SpecPlanStoreError> {
    let plan_json = serde_json::to_value(plan)?;
    let row: Result<(DateTime<Utc>, DateTime<Utc>), sqlx::Error> = sqlx::query_as(
        "INSERT INTO spec_plans (workspace_id, spec_plan_id, plan_json, created_by) \
         VALUES ($1, $2, $3, $4) \
         RETURNING created_at, updated_at",
    )
    .bind(workspace_id)
    .bind(spec_plan_id)
    .bind(&plan_json)
    .bind(created_by)
    .fetch_one(pool)
    .await;

    match row {
        Ok((created_at, updated_at)) => Ok(StoredSpecPlan {
            workspace_id: workspace_id.to_string(),
            spec_plan_id: spec_plan_id.to_string(),
            plan: plan.clone(),
            created_by: created_by.to_string(),
            created_at,
            updated_at,
        }),
        Err(sqlx::Error::Database(db_err))
            if db_err.constraint() == Some(DUPLICATE_PK_CONSTRAINT) =>
        {
            Err(SpecPlanStoreError::Duplicate(spec_plan_id.to_string()))
        }
        Err(e) => Err(e.into()),
    }
}

/// Read a single Spec Plan row, `Ok(None)` if not present.
pub async fn get(
    pool: &PgPool,
    workspace_id: &str,
    spec_plan_id: &str,
) -> Result<Option<StoredSpecPlan>, SpecPlanStoreError> {
    let row: Option<(Value, String, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT plan_json, created_by, created_at, updated_at FROM spec_plans \
         WHERE workspace_id = $1 AND spec_plan_id = $2",
    )
    .bind(workspace_id)
    .bind(spec_plan_id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((plan_json, created_by, created_at, updated_at)) => {
            let plan: SpecPlan = serde_json::from_value(plan_json)?;
            Ok(Some(StoredSpecPlan {
                workspace_id: workspace_id.to_string(),
                spec_plan_id: spec_plan_id.to_string(),
                plan,
                created_by,
                created_at,
                updated_at,
            }))
        }
        None => Ok(None),
    }
}

/// List every Spec Plan in a workspace ordered by recency.
pub async fn list(
    pool: &PgPool,
    workspace_id: &str,
) -> Result<Vec<StoredSpecPlan>, SpecPlanStoreError> {
    let rows: Vec<ListRow> = sqlx::query_as(
        "SELECT spec_plan_id, plan_json, created_by, created_at, updated_at \
         FROM spec_plans WHERE workspace_id = $1 \
         ORDER BY created_at DESC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for (spec_plan_id, plan_json, created_by, created_at, updated_at) in rows {
        let plan: SpecPlan = serde_json::from_value(plan_json)?;
        out.push(StoredSpecPlan {
            workspace_id: workspace_id.to_string(),
            spec_plan_id,
            plan,
            created_by,
            created_at,
            updated_at,
        });
    }
    Ok(out)
}

/// Replace a single [`SpecRef`] inside an existing plan. Identity is
/// matched on `SpecId`. Bumps `updated_at`.
///
/// `validate_after_swap` runs `SpecPlan::validate` on the mutated
/// plan **before** the DB write. Callers that need to keep the
/// invalid plan persisted (e.g. recovering from a broken row) can
/// pass `false`; the MCP `update_spec` tool always passes `true` so
/// a failed validation surfaces as `InvalidParams` *without*
/// corrupting the stored row.
pub async fn replace_spec(
    pool: &PgPool,
    workspace_id: &str,
    spec_plan_id: &str,
    new_spec: SpecRef,
    validate_after_swap: bool,
) -> Result<StoredSpecPlan, SpecPlanStoreError> {
    let mut existing = get(pool, workspace_id, spec_plan_id)
        .await?
        .ok_or_else(|| SpecPlanStoreError::PlanNotFound(spec_plan_id.to_string()))?;

    let target_id = new_spec.id.clone();
    let mut replaced = false;
    for spec in existing.plan.specs.iter_mut() {
        if spec.id == target_id {
            *spec = new_spec.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        return Err(SpecPlanStoreError::SpecNotFound(target_id));
    }

    if validate_after_swap {
        existing
            .plan
            .validate()
            .map_err(SpecPlanStoreError::Validation)?;
    }

    let plan_json = serde_json::to_value(&existing.plan)?;
    let updated_at: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE spec_plans SET plan_json = $3, updated_at = NOW() \
         WHERE workspace_id = $1 AND spec_plan_id = $2 \
         RETURNING updated_at",
    )
    .bind(workspace_id)
    .bind(spec_plan_id)
    .bind(&plan_json)
    .fetch_one(pool)
    .await?;
    existing.updated_at = updated_at;
    Ok(existing)
}
