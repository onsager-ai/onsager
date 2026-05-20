//! Dev-login mode (issue #193). Replaces the removed anonymous-mode branch.
//!
//! Enabled in two ways:
//! - **Debug builds** (`cargo build`): always available.
//! - **Release builds** with `DEV_LOGIN_ENABLED=true`: opt-in for Railway
//!   preview environments that need a login path without GitHub OAuth.
//!
//! Two pieces:
//! - [`seed_dev_user_and_workspace`] is called once at server boot. It
//!   idempotently materializes a per-developer `${USER}@local` user, a
//!   default workspace, and the membership linking them. Re-running on a
//!   warm DB produces no duplicate rows.
//! - [`dev_login`] handles `POST /api/auth/dev-login` — clicking the
//!   "Dev Login as ${USER}@local" button on `LoginPage` mints a real
//!   session cookie for the seeded user.
//!
//! Why a negative `github_id`? Real GitHub user IDs are always positive,
//! so the negative range is a free namespace for synthetic users. The
//! auth extractor reads `user.github_id < 0` to decide
//! `session_kind: dev`, which is what drives the dashboard's persistent
//! dev-mode banner. No extra column on `auth_sessions` required.
//!
//! `workspaces` and `workspace_members` are still owned by stiglab's
//! runtime migrations until Slice 3 of spec #222 moves them into the
//! spine. Portal writes to those tables via raw SQL — same DB, same
//! shape, no shared crate needed.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::auth::{SessionKind, generate_session_token};
use crate::auth_db::{self, User};
use crate::state::AppState;

/// Fixed `github_id` for the seeded dev user. Single value (not derived
/// from `$USER`) keeps the seed idempotent across `$USER` changes — the
/// row is always upserted on the same primary key.
///
/// Negative is the type-level `SessionKind::Dev` marker (see
/// `auth::session_kind_for_github_id`).
pub const DEV_GITHUB_ID: i64 = -1;

/// Slug of the workspace the seeder creates. Stable across boots so
/// localhost links never break between restarts.
pub const DEV_WORKSPACE_SLUG: &str = "dev";

/// Resolve the username we'll seed. `$USER` from the boot env, falling
/// back to `dev` so the build works in CI and rootless containers where
/// `$USER` may be unset.
pub fn dev_username() -> String {
    dev_username_from(std::env::var("USER").ok())
}

/// Pure variant of [`dev_username`] used for testing without env mutation.
fn dev_username_from(user: Option<String>) -> String {
    user.filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "dev".to_string())
}

/// `${USER}@local` — what shows up on the LoginPage button and the banner.
pub fn dev_login_label(username: &str) -> String {
    format!("{username}@local")
}

/// Idempotently materialize the dev user, the dev workspace, and the
/// membership linking them. Called once at server boot from `server::run`.
pub async fn seed_dev_user_and_workspace(pool: &PgPool) -> anyhow::Result<()> {
    let username = dev_username();
    let login = dev_login_label(&username);
    let now = Utc::now();

    // Resolve-or-create the user row. `upsert_user` keys on `github_id`
    // (UNIQUE) so the same negative ID survives across reboots.
    let user_id = match auth_db::get_user_by_github_id(pool, DEV_GITHUB_ID).await? {
        Some(existing) => existing.id,
        None => Uuid::new_v4().to_string(),
    };
    let user = User {
        id: user_id.clone(),
        github_id: DEV_GITHUB_ID,
        github_login: login.clone(),
        github_name: Some(format!("Dev ({username})")),
        github_avatar_url: None,
        created_at: now,
        updated_at: now,
    };
    auth_db::upsert_user(pool, &user).await?;

    // Resolve-or-create the workspace. Stiglab owns the `workspaces` /
    // `workspace_members` schema until Slice 3, so this writes raw SQL
    // matching stiglab's CREATE TABLE shape.
    let existing_ws_id: Option<(String,)> =
        sqlx::query_as("SELECT id FROM workspaces WHERE slug = $1")
            .bind(DEV_WORKSPACE_SLUG)
            .fetch_optional(pool)
            .await?;
    let workspace_id = match existing_ws_id {
        Some((id,)) => id,
        None => {
            let id = Uuid::new_v4().to_string();
            let mut tx = pool.begin().await?;
            sqlx::query(
                "INSERT INTO workspaces (id, slug, name, created_by, created_at) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&id)
            .bind(DEV_WORKSPACE_SLUG)
            .bind("Dev workspace")
            .bind(&user_id)
            .bind(now.to_rfc3339())
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO workspace_members (workspace_id, user_id, joined_at) \
                 VALUES ($1, $2, $3)",
            )
            .bind(&id)
            .bind(&user_id)
            .bind(now.to_rfc3339())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            id
        }
    };

    // Membership might be missing on a workspace that pre-dated the seeder
    // (or was created by an earlier `$USER`). Backfill it.
    let has_member: Option<(String,)> = sqlx::query_as(
        "SELECT user_id FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(&workspace_id)
    .bind(&user_id)
    .fetch_optional(pool)
    .await?;
    if has_member.is_none() {
        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, joined_at) VALUES ($1, $2, $3)",
        )
        .bind(&workspace_id)
        .bind(&user_id)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
    }

    tracing::info!(
        user_id = %user_id,
        login = %login,
        workspace = %DEV_WORKSPACE_SLUG,
        "dev-login: seeded user + workspace (debug build)"
    );
    Ok(())
}

/// `POST /api/auth/dev-login` — mint a session cookie for the seeded dev
/// user. Debug-only: the route is not registered in release builds.
pub async fn dev_login(State(state): State<AppState>) -> Response {
    let user = match auth_db::get_user_by_github_id(&state.pool, DEV_GITHUB_ID).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::error!("dev-login: dev user missing; boot seeder didn't run?");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "dev user not seeded" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("dev-login: failed to load dev user: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let session_token = generate_session_token();
    let expires_at = Utc::now() + chrono::Duration::days(30);
    if let Err(e) =
        auth_db::create_auth_session(&state.pool, &session_token, &user.id, expires_at).await
    {
        tracing::error!("dev-login: failed to create auth session: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    tracing::info!(
        user_id = %user.id,
        login = %user.github_login,
        "dev-login: minted session"
    );

    let secure = state
        .config
        .public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"));
    let secure_attr = if secure { "; Secure" } else { "" };
    let cookie = format!(
        "stiglab_session={session_token}; Path=/; HttpOnly; SameSite=Lax; \
         Max-Age=2592000{secure_attr}"
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::SET_COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(
            serde_json::to_vec(&serde_json::json!({
                "ok": true,
                "session_kind": SessionKind::Dev,
                "user": {
                    "id": user.id,
                    "github_login": user.github_login,
                    "github_name": user.github_name,
                    "github_avatar_url": user.github_avatar_url,
                },
            }))
            .expect("dev-login response serializes"),
        ))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_username_falls_back_to_dev() {
        // Test the pure variant so we don't have to mutate the
        // process-wide env (which races with parallel tests under
        // edition 2024's unsafe env API).
        assert_eq!(dev_username_from(None), "dev");
        assert_eq!(dev_username_from(Some(String::new())), "dev");
        assert_eq!(dev_username_from(Some("   ".into())), "dev");
        assert_eq!(dev_username_from(Some("alice".into())), "alice");
    }

    #[test]
    fn dev_login_label_appends_at_local() {
        assert_eq!(dev_login_label("alice"), "alice@local");
        assert_eq!(dev_login_label("dev"), "dev@local");
    }

    #[test]
    fn dev_github_id_is_negative_and_marks_dev() {
        const _: () = assert!(DEV_GITHUB_ID < 0);
        assert_eq!(
            crate::auth::session_kind_for_github_id(DEV_GITHUB_ID),
            SessionKind::Dev
        );
    }
}
