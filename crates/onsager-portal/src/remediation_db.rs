//! Cost ledger + budget enforcement for `propose_remediation`'s
//! server-side AI calls (spec #312).
//!
//! Cost lives in `portal_remediation_calls` (migration 008). One row
//! per AI call; the budget check sums `cost_usd` across the current
//! calendar month (UTC) for the workspace and refuses new calls past
//! the configured cap. The cap defaults to a low double-digit dollar
//! amount in code; per-workspace overrides are a future dashboard
//! Settings concern and are deliberately not modeled in this slice.

use chrono::{DateTime, Datelike, TimeZone, Utc};
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::anthropic::Usage;

/// Outcome of the per-workspace budget check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BudgetStatus {
    /// At least one more call fits under the monthly cap. Carries
    /// `spent_usd` and `cap_usd` so the caller can compute remaining
    /// headroom (`cap_usd - spent_usd`) and surface a warning when
    /// the workspace is approaching the limit.
    Ok { spent_usd: f64, cap_usd: f64 },
    /// Over the monthly cap — the call must short-circuit to stub.
    OverCap { spent_usd: f64, cap_usd: f64 },
}

impl BudgetStatus {
    pub fn is_over(&self) -> bool {
        matches!(self, BudgetStatus::OverCap { .. })
    }
}

/// Sum `cost_usd` for the current calendar month (UTC) and compare
/// against `cap_usd`. The cap is the **soft pre-call gate**: a call
/// that pushes spend over the cap mid-flight still completes (we
/// charge per Anthropic's invoice either way); the next call after it
/// will be refused.
pub async fn check_budget(
    pool: &PgPool,
    workspace_id: &str,
    cap_usd: f64,
) -> anyhow::Result<BudgetStatus> {
    let start = month_start(Utc::now());
    let spent: Option<f64> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_usd), 0) FROM portal_remediation_calls \
         WHERE workspace_id = $1 AND created_at >= $2",
    )
    .bind(workspace_id)
    .bind(start)
    .fetch_one(pool)
    .await?;
    let spent = spent.unwrap_or(0.0);
    Ok(if spent >= cap_usd {
        BudgetStatus::OverCap {
            spent_usd: spent,
            cap_usd,
        }
    } else {
        BudgetStatus::Ok {
            spent_usd: spent,
            cap_usd,
        }
    })
}

/// Insert one row into `portal_remediation_calls`. Best-effort —
/// the AI call has already happened by the time we get here, so a
/// ledger failure is logged but doesn't propagate to the caller.
/// The next budget check will under-count by one row; that's
/// acceptable for the soft-cap semantics.
#[allow(clippy::too_many_arguments)]
pub async fn record_call(
    pool: &PgPool,
    workspace_id: &str,
    user_id: &str,
    artifact_id: &str,
    model: &str,
    usage: &Usage,
    cost_usd: f64,
) -> anyhow::Result<()> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO portal_remediation_calls (\
            id, workspace_id, user_id, artifact_id, model, \
            input_tokens, output_tokens, \
            cache_creation_input_tokens, cache_read_input_tokens, \
            cost_usd \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(&id)
    .bind(workspace_id)
    .bind(user_id)
    .bind(artifact_id)
    .bind(model)
    .bind(usage.input_tokens as i64)
    .bind(usage.output_tokens as i64)
    .bind(usage.cache_creation_input_tokens as i64)
    .bind(usage.cache_read_input_tokens as i64)
    .bind(cost_usd)
    .execute(pool)
    .await?;
    Ok(())
}

fn month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("month-start datetime is well-defined")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn month_start_is_first_of_month_at_midnight() {
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 23, 59, 59).unwrap();
        let start = month_start(now);
        assert_eq!(start.year(), 2026);
        assert_eq!(start.month(), 5);
        assert_eq!(start.day(), 1);
        assert_eq!(start.hour(), 0);
    }

    #[test]
    fn budget_status_is_over_predicate() {
        assert!(
            !BudgetStatus::Ok {
                spent_usd: 1.0,
                cap_usd: 10.0
            }
            .is_over()
        );
        assert!(
            BudgetStatus::OverCap {
                spent_usd: 10.5,
                cap_usd: 10.0
            }
            .is_over()
        );
    }
}
