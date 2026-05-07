use crate::core::User;
use chrono::Utc;
use sqlx::AnyPool;

// ── Users + auth sessions ──
//
// Portal owns writes to `users` / `auth_sessions` / `sso_exchange_codes`
// in production post-#222 Slice 5. Stiglab still reads these tables on
// every authenticated request via the `AuthUser` cookie extractor (and
// PAT path, which joins `users` for the principal's profile). The
// `upsert_user` and `create_auth_session` writers that follow are now
// only called from stiglab's integration tests — they seed authenticated
// fixtures directly rather than running the OAuth dance through portal.

pub async fn upsert_user(pool: &AnyPool, user: &User) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT(github_id) DO UPDATE SET
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

pub async fn create_auth_session(
    pool: &AnyPool,
    session_id: &str,
    user_id: &str,
    expires_at: chrono::DateTime<Utc>,
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

pub async fn get_user_by_github_id(pool: &AnyPool, github_id: i64) -> anyhow::Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at FROM users WHERE github_id = $1",
    )
    .bind(github_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn get_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub struct AuthSession {
    pub id: String,
    pub user_id: String,
    pub user: User,
    pub expires_at: chrono::DateTime<Utc>,
    pub created_at: chrono::DateTime<Utc>,
}

pub async fn get_auth_session(
    pool: &AnyPool,
    session_id: &str,
) -> anyhow::Result<Option<AuthSession>> {
    let row = sqlx::query_as::<_, AuthSessionRow>(
        "SELECT a.id, a.user_id, a.expires_at, a.created_at,
                u.github_id, u.github_login, u.github_name, u.github_avatar_url,
                u.created_at as user_created_at, u.updated_at as user_updated_at
         FROM auth_sessions a JOIN users u ON a.user_id = u.id
         WHERE a.id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else { return Ok(None) };

    let expires_at = chrono::DateTime::parse_from_rfc3339(&row.expires_at)?.with_timezone(&Utc);
    if expires_at < Utc::now() {
        // Expired — best-effort cleanup. Portal also expires sessions on
        // its own writes; the redundant DELETE here is cheap and avoids
        // the row sticking around when stiglab is the only reader for a
        // long-idle cookie.
        let _ = sqlx::query("DELETE FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .execute(pool)
            .await;
        return Ok(None);
    }

    let user = User {
        id: row.user_id.clone(),
        github_id: row.github_id,
        github_login: row.github_login,
        github_name: row.github_name,
        github_avatar_url: row.github_avatar_url,
        created_at: chrono::DateTime::parse_from_rfc3339(&row.user_created_at)?.with_timezone(&Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&row.user_updated_at)?.with_timezone(&Utc),
    };

    Ok(Some(AuthSession {
        id: row.id,
        user_id: row.user_id,
        user,
        expires_at,
        created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
    }))
}

// ── Row types ──

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    github_id: i64,
    github_login: String,
    github_name: Option<String>,
    github_avatar_url: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<UserRow> for User {
    type Error = anyhow::Error;

    fn try_from(row: UserRow) -> anyhow::Result<Self> {
        Ok(User {
            id: row.id,
            github_id: row.github_id,
            github_login: row.github_login,
            github_name: row.github_name,
            github_avatar_url: row.github_avatar_url,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct AuthSessionRow {
    id: String,
    user_id: String,
    expires_at: String,
    created_at: String,
    // User fields from join
    github_id: i64,
    github_login: String,
    github_name: Option<String>,
    github_avatar_url: Option<String>,
    user_created_at: String,
    user_updated_at: String,
}
