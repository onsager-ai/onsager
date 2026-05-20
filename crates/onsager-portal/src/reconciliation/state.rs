//! `adapter_reconciliation_state` read / write helpers.
//!
//! Mirrors spine migration 031. The table is the durable home for
//! every (adapter, workspace, resource_kind) high-water mark. The
//! poller reads its cursor in, calls `Adapter::poll_since`, and
//! writes the advance back out on success.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use onsager_github::AdapterReconciliationState;

/// The three optional cursor columns persisted on
/// `adapter_reconciliation_state`. Named so the row tuple isn't a
/// `Option<(Option<...>, Option<...>, Option<...>)>` salad inside
/// the load helper.
type CursorRow = (Option<String>, Option<DateTime<Utc>>, Option<String>);

/// Load the cursor row for the given tuple. Returns a fresh
/// default-shaped state row (no cursor, no ETag) when no row
/// exists yet — callers can pass that directly into
/// `Adapter::poll_since` and the first tick will record the
/// initial advance.
pub async fn load_state(
    pool: &PgPool,
    adapter_id: &str,
    workspace_id: &str,
    resource_kind: &str,
) -> Result<AdapterReconciliationState, sqlx::Error> {
    let row: Option<CursorRow> = sqlx::query_as(
        r#"
        SELECT last_seen_external_id, last_seen_updated_at, etag
        FROM adapter_reconciliation_state
        WHERE adapter_id = $1 AND workspace_id = $2 AND resource_kind = $3
        "#,
    )
    .bind(adapter_id)
    .bind(workspace_id)
    .bind(resource_kind)
    .fetch_optional(pool)
    .await?;

    Ok(match row {
        Some((external_id, updated_at, etag)) => AdapterReconciliationState {
            adapter_id: adapter_id.to_string(),
            workspace_id: workspace_id.to_string(),
            resource_kind: resource_kind.to_string(),
            last_seen_external_id: external_id,
            last_seen_updated_at: updated_at,
            etag,
        },
        None => AdapterReconciliationState {
            adapter_id: adapter_id.to_string(),
            workspace_id: workspace_id.to_string(),
            resource_kind: resource_kind.to_string(),
            ..Default::default()
        },
    })
}

/// Persist a cursor advance. Idempotent — repeated calls for the
/// same tuple update in place. `last_polled_at` is always stamped
/// to `NOW()` so operators can see whether a quiet project is
/// healthy (recently polled, no new resources) or stale (poller
/// hasn't run).
pub async fn upsert_state(
    pool: &PgPool,
    state: &AdapterReconciliationState,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO adapter_reconciliation_state (
            adapter_id, workspace_id, resource_kind,
            last_seen_external_id, last_seen_updated_at, etag, last_polled_at
        ) VALUES ($1, $2, $3, $4, $5, $6, NOW())
        ON CONFLICT (adapter_id, workspace_id, resource_kind) DO UPDATE
            SET last_seen_external_id = EXCLUDED.last_seen_external_id,
                last_seen_updated_at  = EXCLUDED.last_seen_updated_at,
                etag                  = EXCLUDED.etag,
                last_polled_at        = NOW()
        "#,
    )
    .bind(&state.adapter_id)
    .bind(&state.workspace_id)
    .bind(&state.resource_kind)
    .bind(state.last_seen_external_id.as_deref())
    .bind(state.last_seen_updated_at)
    .bind(state.etag.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

/// Stamp `last_polled_at` without changing the cursor — used after a
/// poll that returned no new events (or a 304 once ETag handling
/// lands). Keeps the "is this poller healthy?" signal honest even
/// on quiet projects.
pub async fn touch_polled_at(
    pool: &PgPool,
    adapter_id: &str,
    workspace_id: &str,
    resource_kind: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO adapter_reconciliation_state (
            adapter_id, workspace_id, resource_kind, last_polled_at
        ) VALUES ($1, $2, $3, NOW())
        ON CONFLICT (adapter_id, workspace_id, resource_kind) DO UPDATE
            SET last_polled_at = NOW()
        "#,
    )
    .bind(adapter_id)
    .bind(workspace_id)
    .bind(resource_kind)
    .execute(pool)
    .await?;
    Ok(())
}
