//! Per-workspace, per-user credential CRUD against the
//! `user_credentials` table (portal migration 006).
//!
//! Spec #222 Slice 2a moved the routes from stiglab → portal; the
//! supporting DB functions move with them. Stiglab still reads
//! encrypted values from the same Postgres table at session-launch
//! time (decrypted in-process and handed to the agent as env vars),
//! but portal is the only writer.

use chrono::Utc;
use sqlx::postgres::PgPool;

#[derive(Debug, Clone)]
pub struct UserCredential {
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(sqlx::FromRow)]
struct UserCredentialRow {
    name: String,
    created_at: String,
    updated_at: String,
}

pub async fn set_user_credential(
    pool: &PgPool,
    workspace_id: &str,
    user_id: &str,
    name: &str,
    encrypted_value: &str,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO user_credentials (id, user_id, workspace_id, name, encrypted_value, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $6) \
         ON CONFLICT(workspace_id, user_id, name) DO UPDATE SET encrypted_value = $5, updated_at = $6",
    )
    .bind(&id)
    .bind(user_id)
    .bind(workspace_id)
    .bind(name)
    .bind(encrypted_value)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Insert a fresh credential row but never overwrite an existing one.
/// Used by the PAT path to make the destructive-credential guardrail
/// race-safe — a pre-check + upsert lets two concurrent PAT `PUT`s for
/// the same new name both pass the existence check and silently
/// overwrite. With this helper the second PUT becomes a no-op at the
/// DB and the handler returns `pat_destructive_blocked`.
///
/// Returns `true` when a new row was inserted, `false` when a row
/// with the same `(workspace_id, user_id, name)` already existed.
pub async fn insert_user_credential_if_absent(
    pool: &PgPool,
    workspace_id: &str,
    user_id: &str,
    name: &str,
    encrypted_value: &str,
) -> anyhow::Result<bool> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        "INSERT INTO user_credentials (id, user_id, workspace_id, name, encrypted_value, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $6) \
         ON CONFLICT(workspace_id, user_id, name) DO NOTHING",
    )
    .bind(&id)
    .bind(user_id)
    .bind(workspace_id)
    .bind(name)
    .bind(encrypted_value)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn get_user_credentials(
    pool: &PgPool,
    workspace_id: &str,
    user_id: &str,
) -> anyhow::Result<Vec<UserCredential>> {
    let rows = sqlx::query_as::<_, UserCredentialRow>(
        "SELECT name, created_at, updated_at FROM user_credentials \
         WHERE workspace_id = $1 AND user_id = $2 ORDER BY name",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| UserCredential {
            name: r.name,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

pub async fn delete_user_credential(
    pool: &PgPool,
    workspace_id: &str,
    user_id: &str,
    name: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM user_credentials \
         WHERE workspace_id = $1 AND user_id = $2 AND name = $3",
    )
    .bind(workspace_id)
    .bind(user_id)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

/// Membership check — used by the credentials route's
/// `require_workspace_access` helper. Reads from the spine-shared
/// `workspace_members` table (still owned by stiglab's runtime
/// migrations until #222 Slice 3).
pub async fn is_workspace_member(
    pool: &PgPool,
    workspace_id: &str,
    user_id: &str,
) -> anyhow::Result<bool> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// True if the user has at least one credential row in `workspace_id`
/// matching one of `names`. Used by the workflow-activate gate (issue
/// #156) to refuse activation when the owner has no Claude auth
/// credential — without this check, the workflow would be active but
/// every session would fail with "stdout closed without result event".
///
/// Checks by exact name match because the Claude CLI keys on specific
/// env var names (`CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`).
/// A user with only custom-named credentials would silently activate
/// into a doomed workflow without the name filter.
pub async fn user_has_credential_in(
    pool: &PgPool,
    workspace_id: &str,
    user_id: &str,
    names: &[&str],
) -> anyhow::Result<bool> {
    if names.is_empty() {
        return Ok(false);
    }
    // Build `name IN ($3, $4, ...)` with placeholders matched to the
    // sqlx binding count.
    let placeholders: Vec<String> = (3..=names.len() + 2).map(|i| format!("${i}")).collect();
    let sql = format!(
        "SELECT name FROM user_credentials \
         WHERE workspace_id = $1 AND user_id = $2 AND name IN ({}) LIMIT 1",
        placeholders.join(", ")
    );
    let mut q = sqlx::query_scalar::<_, String>(&sql)
        .bind(workspace_id)
        .bind(user_id);
    for n in names {
        q = q.bind(*n);
    }
    let row = q.fetch_optional(pool).await?;
    Ok(row.is_some())
}
