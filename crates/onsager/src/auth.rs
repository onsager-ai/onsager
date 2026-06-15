//! Users, cookie sessions, dev-login, and the `AuthUser` extractor.
//!
//! Ported from `legacy/crates/onsager-portal/src/{auth,auth_db,dev_auth}.rs`
//! with the PAT and cross-environment SSO paths dropped (no consumer in
//! v2 yet) and timestamps moved to `TIMESTAMPTZ`. GitHub OAuth login
//! returns in M2 alongside the GitHub port; until then the cookie is
//! minted by dev-login only.

use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::crypto::random_token;
use crate::state::AppState;

// ── Users + sessions ──

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    #[serde(skip)]
    pub github_id: i64,
    pub github_login: String,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
}

pub async fn upsert_user(pool: &PgPool, user: &User) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (id, github_id, github_login, github_name, github_avatar_url) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (github_id) DO UPDATE \
             SET github_login = $3, github_name = $4, github_avatar_url = $5, \
                 updated_at = NOW()",
    )
    .bind(&user.id)
    .bind(user.github_id)
    .bind(&user.github_login)
    .bind(&user.github_name)
    .bind(&user.github_avatar_url)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_user_by_github_id(pool: &PgPool, github_id: i64) -> anyhow::Result<Option<User>> {
    Ok(sqlx::query_as(
        "SELECT id, github_id, github_login, github_name, github_avatar_url \
         FROM users WHERE github_id = $1",
    )
    .bind(github_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn create_auth_session(
    pool: &PgPool,
    session_id: &str,
    user_id: &str,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO auth_sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(session_id)
        .bind(user_id)
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// Resolve a session cookie value to its (unexpired) user.
pub async fn get_session_user(pool: &PgPool, session_id: &str) -> anyhow::Result<Option<User>> {
    Ok(sqlx::query_as(
        "SELECT u.id, u.github_id, u.github_login, u.github_name, u.github_avatar_url \
         FROM auth_sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.id = $1 AND s.expires_at > NOW()",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn delete_auth_session(pool: &PgPool, session_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM auth_sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Session kind ──

/// How the user behind a request was minted. The dashboard renders a
/// persistent dev-mode banner for `Dev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    Github,
    Dev,
}

/// Negative `github_id` is the type-level signal for a dev-seeded user
/// (real GitHub user IDs are always positive).
pub fn session_kind_for_github_id(github_id: i64) -> SessionKind {
    if github_id < 0 {
        SessionKind::Dev
    } else {
        SessionKind::Github
    }
}

// ── Extractor ──

/// Authenticated user, resolved from the `onsager_session` cookie.
/// Every API request must resolve to a real `users` row.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub github_login: String,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
    pub session_kind: SessionKind,
}

pub fn parse_cookie<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(name) {
            let value = value.trim_start();
            if let Some(value) = value.strip_prefix('=') {
                return Some(value.trim());
            }
        }
    }
    None
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let session_id = parts
            .headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|h| parse_cookie(h, "onsager_session"))
            .map(str::to_string);
        let Some(session_id) = session_id else {
            return Err((StatusCode::UNAUTHORIZED, "not authenticated").into_response());
        };
        match get_session_user(&state.pool, &session_id).await {
            Ok(Some(user)) => Ok(AuthUser {
                session_kind: session_kind_for_github_id(user.github_id),
                user_id: user.id,
                github_login: user.github_login,
                github_name: user.github_name,
                github_avatar_url: user.github_avatar_url,
            }),
            Ok(None) => Err((StatusCode::UNAUTHORIZED, "session expired").into_response()),
            Err(e) => {
                tracing::error!("auth session lookup failed: {e}");
                Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
        }
    }
}

/// 404-shaped membership gate: a workspace the user can't see does not
/// exist as far as the API is concerned.
pub async fn require_workspace_access(
    pool: &PgPool,
    user: &AuthUser,
    workspace_id: &str,
) -> Result<(), Response> {
    let member: Option<(String,)> = sqlx::query_as(
        "SELECT user_id FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(&user.user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("membership lookup failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;
    if member.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "workspace not found" })),
        )
            .into_response());
    }
    Ok(())
}

// ── Dev login ──

/// Fixed `github_id` for the seeded dev user; negative marks `Dev`.
pub const DEV_GITHUB_ID: i64 = -1;
pub const DEV_WORKSPACE_SLUG: &str = "dev";

fn dev_username() -> String {
    std::env::var("USER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "dev".to_string())
}

/// Ensure an authenticated user owns at least one workspace, minting a
/// personal one on first sight. The dev seed only covers the dev user;
/// real GitHub users are provisioned here on OAuth login. Idempotent —
/// a no-op once the user has any membership.
pub async fn ensure_personal_workspace(
    pool: &PgPool,
    user_id: &str,
    login: &str,
) -> anyhow::Result<()> {
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT workspace_id FROM workspace_members WHERE user_id = $1 LIMIT 1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    if existing.is_some() {
        return Ok(());
    }

    // `github_login` is globally unique, so it makes a stable slug; on the
    // rare collision with an existing workspace (e.g. the dev seed's
    // "dev") fall back to an id-suffixed slug.
    let workspace_id = Uuid::new_v4().to_string();
    let name = format!("{login}'s workspace");
    let inserted = sqlx::query(
        "INSERT INTO workspaces (id, slug, name, created_by) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (slug) DO NOTHING",
    )
    .bind(&workspace_id)
    .bind(login.to_ascii_lowercase())
    .bind(&name)
    .bind(user_id)
    .execute(pool)
    .await?;
    let workspace_id = if inserted.rows_affected() == 0 {
        let id = Uuid::new_v4().to_string();
        let slug = format!("{}-{}", login.to_ascii_lowercase(), &id[..8]);
        sqlx::query("INSERT INTO workspaces (id, slug, name, created_by) VALUES ($1, $2, $3, $4)")
            .bind(&id)
            .bind(&slug)
            .bind(&name)
            .bind(user_id)
            .execute(pool)
            .await?;
        id
    } else {
        workspace_id
    };

    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(&workspace_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    tracing::info!(%login, workspace = %workspace_id, "provisioned personal workspace");
    Ok(())
}

/// Idempotently materialize the dev user, workspace, and membership.
/// Called at boot whenever dev-login is enabled.
pub async fn seed_dev_user_and_workspace(pool: &PgPool) -> anyhow::Result<()> {
    let login = format!("{}@local", dev_username());
    let user_id = match get_user_by_github_id(pool, DEV_GITHUB_ID).await? {
        Some(existing) => existing.id,
        None => Uuid::new_v4().to_string(),
    };
    upsert_user(
        pool,
        &User {
            id: user_id.clone(),
            github_id: DEV_GITHUB_ID,
            github_login: login.clone(),
            github_name: Some(format!("Dev ({})", dev_username())),
            github_avatar_url: None,
        },
    )
    .await?;

    let ws: Option<(String,)> = sqlx::query_as("SELECT id FROM workspaces WHERE slug = $1")
        .bind(DEV_WORKSPACE_SLUG)
        .fetch_optional(pool)
        .await?;
    let workspace_id = match ws {
        Some((id,)) => id,
        None => {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO workspaces (id, slug, name, created_by) VALUES ($1, $2, $3, $4)",
            )
            .bind(&id)
            .bind(DEV_WORKSPACE_SLUG)
            .bind("Dev workspace")
            .bind(&user_id)
            .execute(pool)
            .await?;
            id
        }
    };
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(&workspace_id)
    .bind(&user_id)
    .execute(pool)
    .await?;

    tracing::info!(%login, workspace = DEV_WORKSPACE_SLUG, "dev-login: seeded");
    Ok(())
}

/// `POST /api/auth/dev-login` — mint a session cookie for the seeded
/// dev user. Registered only when [`crate::config::Config::dev_login_enabled`].
pub async fn dev_login(State(state): State<AppState>) -> Response {
    let user = match get_user_by_github_id(&state.pool, DEV_GITHUB_ID).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::error!("dev-login: dev user missing; boot seeder didn't run?");
            return (StatusCode::INTERNAL_SERVER_ERROR, "dev user not seeded").into_response();
        }
        Err(e) => {
            tracing::error!("dev-login: failed to load dev user: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let token = random_token();
    let expires_at = Utc::now() + chrono::Duration::days(30);
    if let Err(e) = create_auth_session(&state.pool, &token, &user.id, expires_at).await {
        tracing::error!("dev-login: failed to create auth session: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    (
        StatusCode::OK,
        [(header::SET_COOKIE, session_cookie(&state, &token))],
        Json(serde_json::json!({
            "ok": true,
            "session_kind": SessionKind::Dev,
            "user": user,
        })),
    )
        .into_response()
}

/// Build the `onsager_session` Set-Cookie value; `Secure` only behind
/// an https public origin.
pub fn session_cookie(state: &AppState, token: &str) -> String {
    let secure = state
        .config
        .public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"));
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("onsager_session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000{secure_attr}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cookie_variants() {
        assert_eq!(
            parse_cookie("onsager_session=abc123; theme=dark", "onsager_session"),
            Some("abc123")
        );
        assert_eq!(
            parse_cookie("theme=dark; onsager_session=xyz", "onsager_session"),
            Some("xyz")
        );
        assert_eq!(parse_cookie("theme=dark", "onsager_session"), None);
        assert_eq!(parse_cookie("", "onsager_session"), None);
        assert_eq!(
            parse_cookie("onsager_session; theme=dark", "onsager_session"),
            None
        );
    }

    #[test]
    fn negative_github_id_is_dev() {
        assert_eq!(session_kind_for_github_id(-1), SessionKind::Dev);
        assert_eq!(session_kind_for_github_id(123), SessionKind::Github);
        const _: () = assert!(DEV_GITHUB_ID < 0);
    }

    /// DB-backed: runs only when `DATABASE_URL` is set (always in CI,
    /// which provisions a Postgres service). Proves a real OAuth user is
    /// granted exactly one personal workspace, and that re-login is a
    /// no-op rather than minting duplicates.
    #[tokio::test]
    async fn ensure_personal_workspace_provisions_once() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("skipping: DATABASE_URL unset");
            return;
        };
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool).await.unwrap();

        // Unique identity per run so the shared CI database stays clean.
        let github_id: i64 = 1_000_000 + (std::process::id() as i64 % 1_000_000);
        let login = format!("wsuser{github_id}");
        let user_id = Uuid::new_v4().to_string();
        upsert_user(
            &pool,
            &User {
                id: user_id.clone(),
                github_id,
                github_login: login.clone(),
                github_name: None,
                github_avatar_url: None,
            },
        )
        .await
        .unwrap();

        // Two calls (first login + re-login) must yield exactly one.
        ensure_personal_workspace(&pool, &user_id, &login)
            .await
            .unwrap();
        ensure_personal_workspace(&pool, &user_id, &login)
            .await
            .unwrap();

        let (members,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM workspace_members WHERE user_id = $1")
                .bind(&user_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(members, 1, "expected exactly one workspace membership");

        // Cleanup, children first (FKs).
        sqlx::query("DELETE FROM workspace_members WHERE user_id = $1")
            .bind(&user_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM workspaces WHERE created_by = $1")
            .bind(&user_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(&user_id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
