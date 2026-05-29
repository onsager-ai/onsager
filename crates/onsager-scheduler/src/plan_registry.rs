//! `execution_plans` registry + the stored-`SpecPlan` reader (B2,
//! #500, ADR 0023 decision 1).
//!
//! Two responsibilities:
//!
//! 1. **Read the authored plan.** `run_spec_plan` (#501) only carries
//!    `(workspace_id, spec_plan_id)`; the orchestration loads the
//!    actual `SpecPlan` from the `spec_plans` table that
//!    `submit_spec_plan` writes (`crates/onsager-portal/src/spec_plan_db.rs`).
//!    We read that table **directly via SQL** — the scheduler does not
//!    import `onsager-portal` (seam rule); the table is shared spine
//!    Postgres, read like any other spine table.
//! 2. **Record the run.** `execution_plans` maps a [`PlanId`] to the
//!    compiled-from `SpecPlan` JSON so a host restart can reload +
//!    recompile + resume any plan with non-terminal nodes
//!    ([`list_recoverable`]).

use onsager_nodes::PlanId;
use onsager_substrate::spec_plan::SpecPlan;
use sqlx::{PgPool, Row};

/// Stable namespace for [`derive_plan_id`] — frozen UUID v4.
const PLAN_ID_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    0x4f, 0x4e, 0x53, 0x47, 0x52, 0x50, 0x4c, 0x41, 0x4e, 0x52, 0x55, 0x4e, 0x49, 0x44, 0x06, 0x06,
]);

/// Derive a stable [`PlanId`] from `(workspace_id, spec_plan_id)`.
///
/// A `run_spec_plan` re-invocation for the same stored plan resolves
/// to the same `PlanId`, so the durable store sees a resume rather
/// than a forked second run (ADR 0023 decision 1).
pub fn derive_plan_id(workspace_id: &str, spec_plan_id: &str) -> PlanId {
    let seed = format!("{workspace_id}:{spec_plan_id}");
    PlanId::new(uuid::Uuid::new_v5(&PLAN_ID_NAMESPACE, seed.as_bytes()).to_string())
}

/// Load the authored `SpecPlan` for `(workspace_id, spec_plan_id)` from
/// the portal-written `spec_plans` table. `Ok(None)` when absent.
pub async fn load_spec_plan(
    pool: &PgPool,
    workspace_id: &str,
    spec_plan_id: &str,
) -> anyhow::Result<Option<SpecPlan>> {
    let row = sqlx::query(
        "SELECT plan_json FROM spec_plans WHERE workspace_id = $1 AND spec_plan_id = $2",
    )
    .bind(workspace_id)
    .bind(spec_plan_id)
    .fetch_optional(pool)
    .await?;
    match row {
        None => Ok(None),
        Some(row) => {
            let json: serde_json::Value = row.try_get("plan_json")?;
            Ok(Some(serde_json::from_value(json)?))
        }
    }
}

/// Record (or refresh) a run in `execution_plans` with status
/// `running`. Idempotent on `plan_id` so a resume re-asserts the row.
pub async fn register_plan(
    pool: &PgPool,
    plan_id: &PlanId,
    workspace_id: &str,
    spec_plan_id: Option<&str>,
    spec_plan: &SpecPlan,
) -> anyhow::Result<()> {
    let plan_json = serde_json::to_value(spec_plan)?;
    sqlx::query(
        "INSERT INTO execution_plans \
         (plan_id, workspace_id, spec_plan_id, spec_plan_json, status, updated_at) \
         VALUES ($1, $2, $3, $4, 'running', NOW()) \
         ON CONFLICT (plan_id) DO UPDATE \
         SET spec_plan_json = EXCLUDED.spec_plan_json, status = 'running', updated_at = NOW()",
    )
    .bind(plan_id.as_str())
    .bind(workspace_id)
    .bind(spec_plan_id)
    .bind(&plan_json)
    .execute(pool)
    .await?;
    Ok(())
}

/// Set a plan's terminal status (`completed` / `failed`).
pub async fn set_status(pool: &PgPool, plan_id: &PlanId, status: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE execution_plans SET status = $2, updated_at = NOW() WHERE plan_id = $1")
        .bind(plan_id.as_str())
        .bind(status)
        .execute(pool)
        .await?;
    Ok(())
}

/// A plan that was still `running` at startup — its source `SpecPlan`
/// to recompile and resume.
#[derive(Debug)]
pub struct RecoverablePlan {
    pub plan_id: PlanId,
    pub workspace_id: String,
    pub spec_plan: SpecPlan,
}

/// Every plan left in `running` status. The host recompiles each and
/// re-drives `Scheduler::run`, which resets non-terminal node states
/// to pending and re-dispatches (RUN-01 fault-tolerance note).
pub async fn list_recoverable(pool: &PgPool) -> anyhow::Result<Vec<RecoverablePlan>> {
    let rows = sqlx::query(
        "SELECT plan_id, workspace_id, spec_plan_json FROM execution_plans WHERE status = 'running'",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let plan_id: String = row.try_get("plan_id")?;
        let workspace_id: String = row.try_get("workspace_id")?;
        let json: serde_json::Value = row.try_get("spec_plan_json")?;
        let spec_plan: SpecPlan = serde_json::from_value(json)?;
        out.push(RecoverablePlan {
            plan_id: PlanId::new(plan_id),
            workspace_id,
            spec_plan,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_plan_id_is_stable_and_workspace_scoped() {
        let a = derive_plan_id("ws1", "github:42");
        let b = derive_plan_id("ws1", "github:42");
        assert_eq!(a, b, "same inputs must derive the same plan id");
        let other_ws = derive_plan_id("ws2", "github:42");
        assert_ne!(a, other_ws, "workspace must scope the derived id");
        let other_plan = derive_plan_id("ws1", "github:43");
        assert_ne!(a, other_plan, "spec plan id must scope the derived id");
    }
}
