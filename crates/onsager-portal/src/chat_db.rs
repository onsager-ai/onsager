//! Postgres queries against the portal-owned `chat_threads` /
//! `chat_messages` tables (spec #470, ADR 0020).
//!
//! Chat conversations are user-private state, not factory-coordination
//! state — they don't live on the spine event bus. This module is the
//! only writer; the portal handlers in `handlers/chat.rs` call into it.
//!
//! Scope semantics:
//!
//! - `scope_type='workspace'` carries `scope_id=None`; the unique
//!   constraint collapses NULL via the `scope_id_key` generated column
//!   in migration 012 so only one rolling thread exists per
//!   `(user_id, workspace_id, 'workspace')` triple.
//! - All other scope types (`workflow`, `run`, `verdict`) carry a
//!   non-empty `scope_id`.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use sqlx::postgres::PgPool;
use ts_rs::TS;
use uuid::Uuid;

/// Wire-shape returned by `GET /api/chat/thread`.
#[derive(Debug, Clone, Serialize, FromRow, TS)]
#[ts(export, rename = "ChatThread")]
pub struct ChatThread {
    pub id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Wire-shape returned in the messages list of a thread.
#[derive(Debug, Clone, Serialize, FromRow, TS)]
#[ts(export, rename = "ChatMessage")]
pub struct ChatMessage {
    pub id: String,
    pub thread_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Wire-shape for a single search hit. Joins `chat_messages` to its
/// thread so the result can cite the source scope alongside the
/// matching message.
#[derive(Debug, Clone, Serialize, FromRow, TS)]
#[ts(export, rename = "ChatSearchHit")]
pub struct ChatSearchHit {
    pub message_id: String,
    pub thread_id: String,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub scope_type: String,
    pub scope_id: Option<String>,
}

/// Find or create the single rolling thread for
/// `(user_id, workspace_id, scope_type, scope_id)`. Uses an upsert keyed
/// on the unique `(user_id, workspace_id, scope_type, scope_id_key)`
/// constraint to make the get-or-create atomic — two concurrent first
/// reads can't fork the thread.
pub async fn get_or_create_thread(
    pool: &PgPool,
    user_id: &str,
    workspace_id: &str,
    scope_type: &str,
    scope_id: Option<&str>,
) -> anyhow::Result<ChatThread> {
    // Pre-generate a UUID; the upsert ignores it if a row already exists.
    let new_id = Uuid::new_v4().to_string();
    let row = sqlx::query_as::<_, ChatThread>(
        "INSERT INTO chat_threads (id, user_id, workspace_id, scope_type, scope_id) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (user_id, workspace_id, scope_type, scope_id_key) DO UPDATE \
             SET updated_at = chat_threads.updated_at \
         RETURNING id, user_id, workspace_id, scope_type, scope_id, created_at, updated_at",
    )
    .bind(&new_id)
    .bind(user_id)
    .bind(workspace_id)
    .bind(scope_type)
    .bind(scope_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Look up a thread by id; returns `None` if the row is absent or owned
/// by a different user / workspace. The caller must pass the
/// authenticated user_id and an authorized workspace_id — both
/// constraints are enforced at the DB layer so a buggy handler can't
/// leak threads across users or workspaces.
pub async fn get_thread_for_user(
    pool: &PgPool,
    thread_id: &str,
    user_id: &str,
    workspace_id: &str,
) -> anyhow::Result<Option<ChatThread>> {
    let row = sqlx::query_as::<_, ChatThread>(
        "SELECT id, user_id, workspace_id, scope_type, scope_id, created_at, updated_at \
         FROM chat_threads \
         WHERE id = $1 AND user_id = $2 AND workspace_id = $3",
    )
    .bind(thread_id)
    .bind(user_id)
    .bind(workspace_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Append a message to a thread. Bumps `chat_threads.updated_at` in the
/// same transaction so the thread's recency reflects its newest message.
pub async fn append_message(
    pool: &PgPool,
    thread_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
) -> anyhow::Result<ChatMessage> {
    let new_id = Uuid::new_v4().to_string();
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<_, ChatMessage>(
        "INSERT INTO chat_messages (id, thread_id, role, content, tool_call_id) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING id, thread_id, role, content, tool_call_id, created_at",
    )
    .bind(&new_id)
    .bind(thread_id)
    .bind(role)
    .bind(content)
    .bind(tool_call_id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("UPDATE chat_threads SET updated_at = NOW() WHERE id = $1")
        .bind(thread_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row)
}

/// Return the messages in a thread ordered by `created_at` ascending
/// (oldest first — natural rendering order for a chat feed).
pub async fn list_messages(pool: &PgPool, thread_id: &str) -> anyhow::Result<Vec<ChatMessage>> {
    let rows = sqlx::query_as::<_, ChatMessage>(
        "SELECT id, thread_id, role, content, tool_call_id, created_at \
         FROM chat_messages WHERE thread_id = $1 ORDER BY created_at ASC, id ASC",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Full-text search messages across all of `user_id`'s threads in
/// `workspace_id`. The workspace filter is the cross-workspace
/// isolation boundary — without it a user could read their own
/// threads in workspace W2 from a request scoped to W1.
///
/// The query uses `plainto_tsquery` so callers can pass raw user
/// strings without quoting; results are ordered by message recency
/// (the search UX is "what did I say recently about X").
pub async fn search_messages(
    pool: &PgPool,
    user_id: &str,
    workspace_id: &str,
    query: &str,
    limit: i64,
) -> anyhow::Result<Vec<ChatSearchHit>> {
    let rows = sqlx::query_as::<_, ChatSearchHit>(
        "SELECT m.id   AS message_id, \
                m.thread_id, \
                m.role, \
                m.content, \
                m.created_at, \
                t.scope_type, \
                t.scope_id \
         FROM chat_messages m \
         JOIN chat_threads t ON t.id = m.thread_id \
         WHERE t.user_id = $1 \
           AND t.workspace_id = $2 \
           AND m.content_tsv @@ plainto_tsquery('english', $3) \
         ORDER BY m.created_at DESC \
         LIMIT $4",
    )
    .bind(user_id)
    .bind(workspace_id)
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
