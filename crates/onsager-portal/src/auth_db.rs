//! Postgres queries against portal-owned auth tables.
//!
//! Portal owns `users`, `auth_sessions`, and `sso_exchange_codes` per
//! spec #222 Slice 5. Stiglab still reads these tables through its
//! own `AnyPool` helpers (cookie validation in its `AuthUser`
//! extractor) — same database, separate connection pool. Writes go
//! through this module only.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPool;

#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub github_id: i64,
    pub github_login: String,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub id: String,
    pub user_id: String,
    pub user: User,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// One row from the `users` table — the column order matches the
/// `SELECT` projections below.
type UserRow = (
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
);

/// Joined row from `auth_sessions` + `users` — the column order matches
/// the `SELECT` projection in [`get_auth_session`].
type AuthSessionUserRow = (
    String,
    String,
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
);

// ── Users ──

pub async fn upsert_user(pool: &PgPool, user: &User) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT(github_id) DO UPDATE SET \
            github_login = $3, github_name = $4, github_avatar_url = $5, updated_at = $7",
    )
    .bind(&user.id)
    .bind(user.github_id)
    .bind(&user.github_login)
    .bind(&user.github_name)
    .bind(&user.github_avatar_url)
    .bind(user.created_at.to_rfc3339())
    .bind(user.updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_user_by_github_id(pool: &PgPool, github_id: i64) -> anyhow::Result<Option<User>> {
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at \
         FROM users WHERE github_id = $1",
    )
    .bind(github_id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_user).transpose()
}

pub async fn get_user(pool: &PgPool, user_id: &str) -> anyhow::Result<Option<User>> {
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_user).transpose()
}

fn row_to_user(row: UserRow) -> anyhow::Result<User> {
    let (id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at) = row;
    Ok(User {
        id,
        github_id,
        github_login,
        github_name,
        github_avatar_url,
        created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&Utc),
    })
}

// ── Auth sessions ──

pub async fn create_auth_session(
    pool: &PgPool,
    session_id: &str,
    user_id: &str,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO auth_sessions (id, user_id, expires_at, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(expires_at.to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_auth_session(
    pool: &PgPool,
    session_id: &str,
) -> anyhow::Result<Option<AuthSession>> {
    let row: Option<AuthSessionUserRow> = sqlx::query_as(
        "SELECT a.id, a.user_id, a.expires_at, a.created_at, \
                u.github_id, u.github_login, u.github_name, u.github_avatar_url, \
                u.created_at as user_created_at, u.updated_at as user_updated_at \
         FROM auth_sessions a JOIN users u ON a.user_id = u.id \
         WHERE a.id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else { return Ok(None) };
    let (
        id,
        user_id,
        expires_at_s,
        created_at_s,
        github_id,
        github_login,
        github_name,
        github_avatar_url,
        user_created_at_s,
        user_updated_at_s,
    ) = row;

    let expires_at = DateTime::parse_from_rfc3339(&expires_at_s)?.with_timezone(&Utc);
    if expires_at < Utc::now() {
        let _ = delete_auth_session(pool, session_id).await;
        return Ok(None);
    }

    let user = User {
        id: user_id.clone(),
        github_id,
        github_login,
        github_name,
        github_avatar_url,
        created_at: DateTime::parse_from_rfc3339(&user_created_at_s)?.with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&user_updated_at_s)?.with_timezone(&Utc),
    };

    Ok(Some(AuthSession {
        id,
        user_id,
        user,
        expires_at,
        created_at: DateTime::parse_from_rfc3339(&created_at_s)?.with_timezone(&Utc),
    }))
}

pub async fn delete_auth_session(pool: &PgPool, session_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM auth_sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── SSO exchange codes ──

pub async fn insert_sso_exchange_code(
    pool: &PgPool,
    code: &str,
    user_id: &str,
    return_to_host: &str,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sso_exchange_codes (code, user_id, return_to_host, expires_at, redeemed_at, created_at) \
         VALUES ($1, $2, $3, $4, NULL, $5)",
    )
    .bind(code)
    .bind(user_id)
    .bind(return_to_host)
    .bind(expires_at.to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Atomically consume an exchange code. The UPDATE is the single-use gate —
/// it succeeds for exactly one caller (the one who sees `rows_affected == 1`);
/// concurrent or repeat calls get `None`. The return-to-host check runs in the
/// UPDATE predicate so a code issued for host A can't be redeemed by host B
/// even if both are in the owner's allowlist.
pub async fn redeem_sso_exchange_code(
    pool: &PgPool,
    code: &str,
    return_to_host: &str,
) -> anyhow::Result<Option<User>> {
    let now = Utc::now().to_rfc3339();
    let rows = sqlx::query(
        "UPDATE sso_exchange_codes \
         SET redeemed_at = $1 \
         WHERE code = $2 \
           AND redeemed_at IS NULL \
           AND expires_at > $1 \
           AND return_to_host = $3",
    )
    .bind(&now)
    .bind(code)
    .bind(return_to_host)
    .execute(pool)
    .await?;

    if rows.rows_affected() == 0 {
        return Ok(None);
    }

    let user_id: String =
        sqlx::query_scalar("SELECT user_id FROM sso_exchange_codes WHERE code = $1")
            .bind(code)
            .fetch_one(pool)
            .await?;
    get_user(pool, &user_id).await
}
