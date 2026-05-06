//! Postgres queries against the portal-owned `user_pats` table.
//!
//! Portal owns the table per spec #222 Slice 2b
//! (`crates/onsager-portal/migrations/005_user_pats.sql`). Stiglab still
//! reads it through its own `AnyPool` helpers (PAT verification in its
//! `AuthUser` extractor) — same database, separate connection pool.
//! Writes go through this module only.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPool;

#[derive(Debug, Clone)]
pub struct UserPat {
    pub id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub name: String,
    pub token_prefix: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_used_ip: Option<String>,
    pub last_used_user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Column projection used by every SELECT below. Shared so the read path
/// and the prefix-lookup path can't drift in column order — and the
/// projection lines up with the field order on [`UserPatRow`] /
/// [`UserPatWithHashRow`] so `query_as` populates by name, not by index.
const PAT_FIELDS: &str = "id, user_id, workspace_id, name, token_prefix, expires_at, \
                          last_used_at, last_used_ip, last_used_user_agent, created_at, revoked_at";

#[derive(sqlx::FromRow)]
struct UserPatRow {
    id: String,
    user_id: String,
    workspace_id: String,
    name: String,
    token_prefix: String,
    expires_at: Option<String>,
    last_used_at: Option<String>,
    last_used_ip: Option<String>,
    last_used_user_agent: Option<String>,
    created_at: String,
    revoked_at: Option<String>,
}

/// Same shape as [`UserPatRow`] with `token_hash` appended — the
/// prefix-lookup path returns the hash alongside the row so the caller
/// can constant-time compare without re-querying.
#[derive(sqlx::FromRow)]
struct UserPatWithHashRow {
    id: String,
    user_id: String,
    workspace_id: String,
    name: String,
    token_prefix: String,
    expires_at: Option<String>,
    last_used_at: Option<String>,
    last_used_ip: Option<String>,
    last_used_user_agent: Option<String>,
    created_at: String,
    revoked_at: Option<String>,
    token_hash: String,
}

fn parse_optional_ts(v: Option<String>) -> anyhow::Result<Option<DateTime<Utc>>> {
    match v {
        Some(s) => Ok(Some(DateTime::parse_from_rfc3339(&s)?.with_timezone(&Utc))),
        None => Ok(None),
    }
}

impl TryFrom<UserPatRow> for UserPat {
    type Error = anyhow::Error;

    fn try_from(row: UserPatRow) -> anyhow::Result<Self> {
        Ok(UserPat {
            id: row.id,
            user_id: row.user_id,
            workspace_id: row.workspace_id,
            name: row.name,
            token_prefix: row.token_prefix,
            expires_at: parse_optional_ts(row.expires_at)?,
            last_used_at: parse_optional_ts(row.last_used_at)?,
            last_used_ip: row.last_used_ip,
            last_used_user_agent: row.last_used_user_agent,
            created_at: DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
            revoked_at: parse_optional_ts(row.revoked_at)?,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_user_pat(
    pool: &PgPool,
    id: &str,
    user_id: &str,
    workspace_id: &str,
    name: &str,
    token_prefix: &str,
    token_hash: &str,
    expires_at: Option<DateTime<Utc>>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_pats (id, user_id, workspace_id, name, token_prefix, token_hash, \
                                scopes, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(user_id)
    .bind(workspace_id)
    .bind(name)
    .bind(token_prefix)
    .bind(token_hash)
    .bind("[\"*\"]")
    .bind(expires_at.map(|d| d.to_rfc3339()))
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_user_pats(pool: &PgPool, user_id: &str) -> anyhow::Result<Vec<UserPat>> {
    let q =
        format!("SELECT {PAT_FIELDS} FROM user_pats WHERE user_id = $1 ORDER BY created_at DESC");
    let rows = sqlx::query_as::<_, UserPatRow>(&q)
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// Look up candidate PATs by token prefix. The caller must verify the
/// `token_hash` against the presented token in constant time before
/// accepting any row.
pub async fn find_pats_by_prefix(
    pool: &PgPool,
    token_prefix: &str,
) -> anyhow::Result<Vec<(UserPat, String)>> {
    let q = format!("SELECT {PAT_FIELDS}, token_hash FROM user_pats WHERE token_prefix = $1");
    let rows = sqlx::query_as::<_, UserPatWithHashRow>(&q)
        .bind(token_prefix)
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|r| {
            let hash = r.token_hash.clone();
            let pat: UserPat = UserPatRow {
                id: r.id,
                user_id: r.user_id,
                workspace_id: r.workspace_id,
                name: r.name,
                token_prefix: r.token_prefix,
                expires_at: r.expires_at,
                last_used_at: r.last_used_at,
                last_used_ip: r.last_used_ip,
                last_used_user_agent: r.last_used_user_agent,
                created_at: r.created_at,
                revoked_at: r.revoked_at,
            }
            .try_into()?;
            Ok((pat, hash))
        })
        .collect()
}

pub async fn revoke_user_pat(pool: &PgPool, user_id: &str, pat_id: &str) -> anyhow::Result<bool> {
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        "UPDATE user_pats SET revoked_at = $1 \
         WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(&now)
    .bind(pat_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn touch_user_pat(
    pool: &PgPool,
    pat_id: &str,
    ip: Option<&str>,
    user_agent: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE user_pats SET last_used_at = $1, last_used_ip = $2, last_used_user_agent = $3 \
         WHERE id = $4",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(ip)
    .bind(user_agent)
    .bind(pat_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Membership check used by the create-PAT handler. Workspaces still live
/// in stiglab's runtime migrations until Slice 3 of spec #222 moves them
/// into the spine; portal reads `workspace_members` via raw SQL against
/// the same DB. Once workspaces move, this becomes a typed join.
pub async fn is_workspace_member(
    pool: &PgPool,
    workspace_id: &str,
    user_id: &str,
) -> anyhow::Result<bool> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT user_id FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}
