//! Integration tests for cross-environment SSO delegation.
//!
//! Covers the two concrete risk areas that aren't exercised by the unit
//! tests in `server::sso` and `server::config`:
//!   * the DB-level single-use gate on `sso_exchange_codes`, and
//!   * the mode-gating on `/api/auth/sso/redeem` and `/api/auth/sso/finish`
//!     (each route must 404 outside its own mode).
//!
//! GitHub is never called; the happy-path tests stop at redeem. The full
//! owner-callback → exchange-code → relying-finish round-trip is covered
//! by the DB tests plus the mode-gating tests — wiring a fake GitHub into
//! these integration tests would add complexity without catching new bugs.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chrono::Utc;
use sqlx::pool::PoolOptions;
use sqlx::AnyPool;
use stiglab::core::User;
use stiglab::server::config::ServerConfig;
use stiglab::server::db;
use stiglab::server::state::AppState;
use tower::ServiceExt;
use uuid::Uuid;

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
        sso_state_secret: None,
        sso_exchange_secret: None,
        sso_return_host_allowlist: Vec::new(),
        sso_auth_domain: None,
    }
}

fn owner_config() -> ServerConfig {
    ServerConfig {
        github_client_id: Some("client-id".into()),
        github_client_secret: Some("client-secret".into()),
        sso_state_secret: Some("state-secret".into()),
        sso_exchange_secret: Some("exchange-secret".into()),
        sso_return_host_allowlist: vec!["*.preview.example.com".into()],
        public_url: Some("https://app.example.com".into()),
        ..base_config()
    }
}

fn relying_config() -> ServerConfig {
    ServerConfig {
        sso_auth_domain: Some("https://app.example.com".into()),
        sso_exchange_secret: Some("exchange-secret".into()),
        public_url: Some("https://pr-1.preview.example.com".into()),
        ..base_config()
    }
}

async fn seed_user(pool: &AnyPool) -> User {
    let user = User {
        id: Uuid::new_v4().to_string(),
        github_id: 42,
        github_login: "octocat".into(),
        github_name: Some("Octo Cat".into()),
        github_avatar_url: Some("https://example.com/a.png".into()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db::upsert_user(pool, &user).await.unwrap();
    user
}

// ── DB-level exchange-code lifecycle ──

#[tokio::test]
async fn redeem_happy_path_returns_user_once() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    db::insert_sso_exchange_code(
        &pool,
        "code-xyz",
        &user.id,
        "pr-1.preview.example.com",
        Utc::now() + chrono::Duration::seconds(30),
    )
    .await
    .unwrap();

    let got = db::redeem_sso_exchange_code(&pool, "code-xyz", "pr-1.preview.example.com")
        .await
        .unwrap();
    assert!(got.is_some());
    assert_eq!(got.unwrap().github_id, user.github_id);

    // Second redemption must fail — single-use gate.
    let again = db::redeem_sso_exchange_code(&pool, "code-xyz", "pr-1.preview.example.com")
        .await
        .unwrap();
    assert!(again.is_none());
}

#[tokio::test]
async fn redeem_rejects_host_mismatch() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    db::insert_sso_exchange_code(
        &pool,
        "code-a",
        &user.id,
        "pr-1.preview.example.com",
        Utc::now() + chrono::Duration::seconds(30),
    )
    .await
    .unwrap();

    // Even with the right code, a different host must not redeem.
    let got = db::redeem_sso_exchange_code(&pool, "code-a", "pr-2.preview.example.com")
        .await
        .unwrap();
    assert!(got.is_none());

    // And the code is NOT consumed by a failed host check — the intended
    // relying party can still redeem it.
    let got = db::redeem_sso_exchange_code(&pool, "code-a", "pr-1.preview.example.com")
        .await
        .unwrap();
    assert!(got.is_some());
}

#[tokio::test]
async fn redeem_rejects_expired_code() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    db::insert_sso_exchange_code(
        &pool,
        "code-expired",
        &user.id,
        "pr-1.preview.example.com",
        // Expired one second ago.
        Utc::now() - chrono::Duration::seconds(1),
    )
    .await
    .unwrap();

    let got = db::redeem_sso_exchange_code(&pool, "code-expired", "pr-1.preview.example.com")
        .await
        .unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn redeem_rejects_unknown_code() {
    let pool = test_pool().await;
    let _ = seed_user(&pool).await;

    let got = db::redeem_sso_exchange_code(&pool, "does-not-exist", "pr-1.preview.example.com")
        .await
        .unwrap();
    assert!(got.is_none());
}

// ── Route-level mode gating ──

fn app(state: AppState) -> axum::Router {
    stiglab::server::build_router(state.clone(), &state.config)
}

fn redeem_request(bearer: &str, code: &str, host: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/auth/sso/redeem")
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "code": code, "host": host }).to_string(),
        ))
        .unwrap()
}

#[tokio::test]
async fn redeem_route_404s_outside_owner_mode() {
    let pool = test_pool().await;
    let state = AppState::new(pool, relying_config(), None);
    let resp = app(state)
        .oneshot(redeem_request("exchange-secret", "x", "h"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn redeem_route_401s_on_bad_bearer() {
    let pool = test_pool().await;
    let state = AppState::new(pool, owner_config(), None);
    let resp = app(state)
        .oneshot(redeem_request(
            "wrong-secret",
            "x",
            "pr-1.preview.example.com",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn redeem_route_happy_path() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    db::insert_sso_exchange_code(
        &pool,
        "good-code",
        &user.id,
        "pr-1.preview.example.com",
        Utc::now() + chrono::Duration::seconds(30),
    )
    .await
    .unwrap();
    let state = AppState::new(pool, owner_config(), None);

    let resp = app(state)
        .oneshot(redeem_request(
            "exchange-secret",
            "good-code",
            "pr-1.preview.example.com",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["user"]["github_id"], 42);
    assert_eq!(v["user"]["github_login"], "octocat");
}

#[tokio::test]
async fn redeem_route_rejects_used_code() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    db::insert_sso_exchange_code(
        &pool,
        "once",
        &user.id,
        "pr-1.preview.example.com",
        Utc::now() + chrono::Duration::seconds(30),
    )
    .await
    .unwrap();
    let state = AppState::new(pool, owner_config(), None);

    let app_ = app(state);
    let first = app_
        .clone()
        .oneshot(redeem_request(
            "exchange-secret",
            "once",
            "pr-1.preview.example.com",
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app_
        .oneshot(redeem_request(
            "exchange-secret",
            "once",
            "pr-1.preview.example.com",
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn finish_route_404s_outside_relying_mode() {
    let pool = test_pool().await;
    let state = AppState::new(pool, owner_config(), None);
    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/auth/sso/finish?code=whatever")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn relying_github_login_redirects_to_owner() {
    let pool = test_pool().await;
    let state = AppState::new(pool, relying_config(), None);
    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/auth/github")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with("https://app.example.com/api/auth/github?"));
    assert!(location.contains("return_to=https"));
    assert!(location.contains("preview.example.com"));
}

#[tokio::test]
async fn owner_github_login_rejects_return_to_not_on_allowlist() {
    let pool = test_pool().await;
    let state = AppState::new(pool, owner_config(), None);
    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/auth/github?return_to=https://attacker.com/cb")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn owner_github_login_accepts_allowlisted_return_to() {
    let pool = test_pool().await;
    let state = AppState::new(pool, owner_config(), None);
    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/auth/github?return_to=https://pr-1.preview.example.com/api/auth/sso/finish")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Redirects to GitHub, setting the CSRF cookie along the way.
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with("https://github.com/login/oauth/authorize"));
    // The state param must be an HMAC envelope (contains a dot), not a bare nonce.
    let url = reqwest::Url::parse(location).unwrap();
    let state_val = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    assert!(
        state_val.contains('.'),
        "expected HMAC envelope, got bare nonce: {state_val}"
    );
}
