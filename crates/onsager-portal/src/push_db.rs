//! Web Push subscription storage (ADR 0020 § Invocation channels,
//! spec #472).
//!
//! Owns the `push_subscriptions` table (portal migration 013). One row
//! per (user, browser) pairing — endpoint is the unique key. Re-
//! subscribing the same browser upserts the keys rather than dupes the
//! row. Dispatch reads all rows for the target user and fans the push
//! out; rows that the push service marks `gone` (HTTP 410) are pruned
//! by the dispatcher (see `push::dispatch`).

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::postgres::PgPool;
use ts_rs::TS;
use uuid::Uuid;

/// One persisted Web Push subscription row.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, rename = "PushSubscriptionRow")]
pub struct PushSubscriptionRow {
    pub id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub endpoint: String,
    pub p256dh_key: String,
    pub auth_secret: String,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Upsert a subscription. Re-subscribing the same browser (same
/// endpoint) updates the keys + workspace + user_agent instead of
/// duplicating the row, keeping `id` stable across re-subscriptions.
pub async fn upsert(
    pool: &PgPool,
    user_id: &str,
    workspace_id: &str,
    endpoint: &str,
    p256dh_key: &str,
    auth_secret: &str,
    user_agent: Option<&str>,
) -> sqlx::Result<PushSubscriptionRow> {
    let id = Uuid::new_v4().to_string();
    let row = sqlx::query_as::<_, (String, String, String, String, String, String, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)>(
        r#"
        INSERT INTO push_subscriptions
            (id, user_id, workspace_id, endpoint, p256dh_key, auth_secret, user_agent)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (user_id, endpoint) DO UPDATE
        SET workspace_id = EXCLUDED.workspace_id,
            p256dh_key   = EXCLUDED.p256dh_key,
            auth_secret  = EXCLUDED.auth_secret,
            user_agent   = EXCLUDED.user_agent
        RETURNING id, user_id, workspace_id, endpoint, p256dh_key, auth_secret, user_agent, created_at, last_used_at
        "#,
    )
    .bind(&id)
    .bind(user_id)
    .bind(workspace_id)
    .bind(endpoint)
    .bind(p256dh_key)
    .bind(auth_secret)
    .bind(user_agent)
    .fetch_one(pool)
    .await?;
    Ok(PushSubscriptionRow {
        id: row.0,
        user_id: row.1,
        workspace_id: row.2,
        endpoint: row.3,
        p256dh_key: row.4,
        auth_secret: row.5,
        user_agent: row.6,
        created_at: row.7,
        last_used_at: row.8,
    })
}

/// Delete a subscription by `(user_id, endpoint)`. The dashboard calls
/// this when the user turns push off (and the browser unsubscribes); the
/// dispatcher calls it when the push service returns 410 Gone.
pub async fn delete_by_endpoint(pool: &PgPool, user_id: &str, endpoint: &str) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM push_subscriptions WHERE user_id = $1 AND endpoint = $2")
        .bind(user_id)
        .bind(endpoint)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// List all subscriptions for a user — dispatch reads this to fan a
/// push out to every authorised browser.
pub async fn list_for_user(pool: &PgPool, user_id: &str) -> sqlx::Result<Vec<PushSubscriptionRow>> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String, String, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)>(
        "SELECT id, user_id, workspace_id, endpoint, p256dh_key, auth_secret, user_agent, created_at, last_used_at
         FROM push_subscriptions WHERE user_id = $1
         ORDER BY created_at DESC"
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PushSubscriptionRow {
            id: r.0,
            user_id: r.1,
            workspace_id: r.2,
            endpoint: r.3,
            p256dh_key: r.4,
            auth_secret: r.5,
            user_agent: r.6,
            created_at: r.7,
            last_used_at: r.8,
        })
        .collect())
}

/// Mark `last_used_at = NOW()` for a subscription. Dispatch calls this
/// after a successful push so an operator can spot dead subscriptions.
pub async fn mark_used(pool: &PgPool, id: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE push_subscriptions SET last_used_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
