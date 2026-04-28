//! Integration tests for dev-login (issue #193).
//!
//! Covers:
//! - Seeder is idempotent across boots — the second call must not
//!   duplicate user, workspace, or membership rows.
//! - `from_request_parts` rejects unauthenticated requests with 401
//!   (no synthetic anonymous fallback).
//! - `POST /api/auth/dev-login` mints a session cookie for the seeded
//!   user, and `GET /api/auth/me` reports `session_kind: "dev"`.
//! - Real GitHub OAuth login (mocked at the user-row level, since GitHub
//!   itself is never called) still resolves to `session_kind: "github"`.
//!
//! Compile-time absence in release builds is enforced by the
//! `#[cfg(debug_assertions)]` gates around `dev_auth::*` and the
//! `/api/auth/dev-login` route registration in `server::build_router`.
//! The whole `dev_auth` module is invisible to release `cargo build`,
//! so any new code that referenced it would fail to compile under
//! `cargo build --release`.

#![cfg(debug_assertions)]

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chrono::Utc;
use sqlx::pool::PoolOptions;
use sqlx::AnyPool;
use stiglab::core::User;
use stiglab::server::config::ServerConfig;
use stiglab::server::db;
use stiglab::server::dev_auth;
use stiglab::server::state::AppState;
use tower::ServiceExt;

async fn test_pool() -> AnyPool {
    sqlx::any::install_default_drivers();
    let pool = PoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    db::run_migrations(&pool).await.expect("migrations");
    pool
}

fn base_config() -> ServerConfig {
    ServerConfig {
        host: "0.0.0.0".into(),
        port: 3000,
        database_url: "sqlite::memory:".into(),
        static_dir: None,
        cors_origin: None,
        github_client_id: None,
        github_client_secret: None,
        credential_key: None,
        public_url: None,
        github_app_webhook_secret: None,
        sso_state_secret: None,
        sso_exchange_secret: None,
        sso_return_host_allowlist: Vec::new(),
        sso_auth_domain: None,
        internal_dispatch_token: None,
    }
}

fn app(state: AppState) -> axum::Router {
    stiglab::server::build_router(state.clone(), &state.config)
}

fn parse_session_cookie(resp_headers: &axum::http::HeaderMap) -> Option<String> {
    for v in resp_headers.get_all(header::SET_COOKIE).iter() {
        let s = v.to_str().ok()?;
        if let Some(rest) = s.strip_prefix("stiglab_session=") {
            let token = rest.split(';').next()?.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

#[tokio::test]
async fn seeder_is_idempotent_across_boots() {
    let pool = test_pool().await;

    // First boot.
    dev_auth::seed_dev_user_and_workspace(&pool).await.unwrap();
    let user_after_first = db::get_user_by_github_id(&pool, dev_auth::DEV_GITHUB_ID)
        .await
        .unwrap()
        .expect("seeded user present");
    let ws_after_first = db::get_workspace_by_slug(&pool, dev_auth::DEV_WORKSPACE_SLUG)
        .await
        .unwrap()
        .expect("seeded workspace present");
    assert!(
        db::is_workspace_member(&pool, &ws_after_first.id, &user_after_first.id)
            .await
            .unwrap(),
        "membership row landed on first boot"
    );

    // Second boot — should be a no-op apart from refreshing `updated_at`.
    dev_auth::seed_dev_user_and_workspace(&pool).await.unwrap();
    let user_after_second = db::get_user_by_github_id(&pool, dev_auth::DEV_GITHUB_ID)
        .await
        .unwrap()
        .expect("user still present");
    let ws_after_second = db::get_workspace_by_slug(&pool, dev_auth::DEV_WORKSPACE_SLUG)
        .await
        .unwrap()
        .expect("workspace still present");

    // Same primary keys after the second seed — no orphan duplicates.
    assert_eq!(user_after_first.id, user_after_second.id);
    assert_eq!(ws_after_first.id, ws_after_second.id);

    // The workspace list scoped to the dev user has exactly one entry.
    let memberships = db::list_workspaces_for_user(&pool, &user_after_second.id)
        .await
        .unwrap();
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].slug, dev_auth::DEV_WORKSPACE_SLUG);
}

#[tokio::test]
async fn me_returns_401_for_unauthenticated_request() {
    let pool = test_pool().await;
    let state = AppState::new(pool, base_config(), None);
    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // No anonymous fallback (#193): a request without a session
    // cookie or PAT must 401, not return a synthetic user.
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn dev_login_mints_session_and_me_reports_dev_kind() {
    let pool = test_pool().await;
    dev_auth::seed_dev_user_and_workspace(&pool).await.unwrap();
    let state = AppState::new(pool, base_config(), None);
    let router = app(state);

    // Mint a session cookie.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/dev-login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let token = parse_session_cookie(resp.headers()).expect("session cookie set");

    // Use the cookie to hit /me.
    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header(header::COOKIE, format!("stiglab_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["session_kind"], "dev");
    // No `auth_enabled` field — drop it from the wire shape (#193).
    assert!(json.get("auth_enabled").is_none());
    let login = json["user"]["github_login"]
        .as_str()
        .expect("github_login present");
    assert!(
        login.ends_with("@local"),
        "dev user login ends with @local: {login}"
    );
}

#[tokio::test]
async fn me_reports_github_kind_for_real_user() {
    // Stand in for the OAuth callback path — we insert a real user
    // (positive github_id) + auth_session directly, since the full
    // OAuth dance would require mocking GitHub. The branch under test
    // is `session_kind_for_github_id`, which is the only thing the
    // wire shape depends on.
    let pool = test_pool().await;
    let user = User {
        id: "user-real".into(),
        github_id: 12345,
        github_login: "octocat".into(),
        github_name: None,
        github_avatar_url: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db::upsert_user(&pool, &user).await.unwrap();
    let session_token = "test-session-token-real";
    db::create_auth_session(
        &pool,
        session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();

    let state = AppState::new(pool, base_config(), None);
    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header(header::COOKIE, format!("stiglab_session={session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["session_kind"], "github");
}
