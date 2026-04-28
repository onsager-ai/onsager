//! Dev-login mode (issue #193). Local-dev-only replacement for the
//! removed anonymous-mode branch.
//!
//! The whole module is `#[cfg(debug_assertions)]`-gated. Release builds
//! (`cargo build --release`) physically do not contain the seeder or the
//! `/api/auth/dev-login` route, so a misconfigured production deploy
//! cannot serve dev-login regardless of env-var manipulation.
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

#![cfg(debug_assertions)]

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::core::{User, Workspace, WorkspaceMember};
use crate::server::auth::{generate_session_token, SessionKind};
use crate::server::db;
use crate::server::state::AppState;

/// Fixed `github_id` for the seeded dev user. Single value (not derived
/// from `$USER`) keeps the seed idempotent across `$USER` changes — the
/// row is always upserted on the same primary key.
///
/// Negative is the type-level `SessionKind::Dev` marker (see
/// `auth::session_kind_for_github_id`).
pub const DEV_GITHUB_ID: i64 = -1;

/// Slug of the workspace the seeder creates.  Stable across boots so
/// localhost links never break between restarts.
pub const DEV_WORKSPACE_SLUG: &str = "dev";

/// Resolve the username we'll seed.  `$USER` from the boot env, falling
/// back to `dev` so the build works in CI and rootless containers where
/// `$USER` may be unset.
pub fn dev_username() -> String {
    std::env::var("USER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "dev".to_string())
}

/// `${USER}@local` — what shows up on the LoginPage button and the
/// banner.
pub fn dev_login_label(username: &str) -> String {
    format!("{username}@local")
}

/// Idempotently materialize the dev user, the dev workspace, and the
/// membership linking them.  Called once at server boot from `main.rs`.
///
/// Re-running on a warm DB does not duplicate rows: the user is upserted
/// on `github_id = DEV_GITHUB_ID`; the workspace is fetched by slug
/// before insert; the membership insert is `OR IGNORE`-style guarded.
pub async fn seed_dev_user_and_workspace(pool: &sqlx::AnyPool) -> anyhow::Result<()> {
    let username = dev_username();
    let login = dev_login_label(&username);
    let now = Utc::now();

    // Resolve-or-create the user row.  `upsert_user` keys on `github_id`
    // (UNIQUE) so the same negative ID survives across reboots.
    let user_id = match db::get_user_by_github_id(pool, DEV_GITHUB_ID).await? {
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
    db::upsert_user(pool, &user).await?;

    // Resolve-or-create the workspace.  Slug is the natural key here; a
    // collision on a fresh DB is impossible because no real user can
    // legally claim `dev` as a workspace slug before the seeder runs.
    let existing_ws = db::get_workspace_by_slug(pool, DEV_WORKSPACE_SLUG).await?;
    let workspace_id = match existing_ws {
        Some(ref ws) => ws.id.clone(),
        None => {
            let ws = Workspace {
                id: Uuid::new_v4().to_string(),
                slug: DEV_WORKSPACE_SLUG.to_string(),
                name: "Dev workspace".to_string(),
                created_by: user_id.clone(),
                created_at: now,
            };
            let member = WorkspaceMember {
                workspace_id: ws.id.clone(),
                user_id: user_id.clone(),
                joined_at: now,
            };
            db::insert_workspace_with_creator(pool, &ws, &member).await?;
            ws.id
        }
    };

    // Membership might be missing on a workspace that pre-dated the
    // seeder (or was created by an earlier `$USER`).  Backfill it.
    if !db::is_workspace_member(pool, &workspace_id, &user_id).await? {
        let member = WorkspaceMember {
            workspace_id: workspace_id.clone(),
            user_id: user_id.clone(),
            joined_at: now,
        };
        db::insert_workspace_member(pool, &member).await?;
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
/// user.  Debug-only: the route is not registered in release builds.
pub async fn dev_login(State(state): State<AppState>) -> Response {
    let user = match db::get_user_by_github_id(&state.db, DEV_GITHUB_ID).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            // Should never happen — `seed_dev_user_and_workspace` runs at
            // boot.  Surface as 500 so the failure is loud rather than
            // silently re-seeding here (which would mask boot bugs).
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
    if let Err(e) = db::create_auth_session(&state.db, &session_token, &user.id, expires_at).await {
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
        // SAFETY: tests run single-threaded by default; we restore the
        // env var below to keep cross-test ordering stable.
        let prev = std::env::var("USER").ok();
        std::env::remove_var("USER");
        assert_eq!(dev_username(), "dev");
        if let Some(v) = prev {
            std::env::set_var("USER", v);
        }
    }

    #[test]
    fn dev_login_label_appends_at_local() {
        assert_eq!(dev_login_label("alice"), "alice@local");
        assert_eq!(dev_login_label("dev"), "dev@local");
    }

    #[test]
    fn dev_github_id_is_negative_and_marks_dev() {
        // Spec: dev users are recognized by negative github_id.
        const _: () = assert!(DEV_GITHUB_ID < 0);
        assert_eq!(
            crate::server::auth::session_kind_for_github_id(DEV_GITHUB_ID),
            SessionKind::Dev
        );
    }
}
