//! The audit record (ADR 0029: events are the record, not the
//! protocol). One insert helper, one read shape; nothing dispatches
//! off this table.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

/// Append one audit event. Failures are logged, never propagated — the
/// record must not be able to fail the work it records.
pub async fn record(
    pool: &PgPool,
    workspace_id: &str,
    run_id: Option<&str>,
    kind: &str,
    payload: serde_json::Value,
) {
    let result = sqlx::query(
        "INSERT INTO events (workspace_id, run_id, kind, payload) VALUES ($1, $2, $3, $4)",
    )
    .bind(workspace_id)
    .bind(run_id)
    .bind(kind)
    .bind(payload)
    .execute(pool)
    .await;
    if let Err(e) = result {
        tracing::error!(kind, "failed to record event: {e}");
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Event {
    pub id: i64,
    pub workspace_id: String,
    pub run_id: Option<String>,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A run's timeline, oldest first.
pub async fn for_run(pool: &PgPool, run_id: &str) -> anyhow::Result<Vec<Event>> {
    Ok(sqlx::query_as(
        "SELECT id, workspace_id, run_id, kind, payload, created_at \
         FROM events WHERE run_id = $1 ORDER BY id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?)
}
