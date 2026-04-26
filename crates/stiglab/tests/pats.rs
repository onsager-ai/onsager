//! Integration tests for Personal Access Tokens (issue #143).
//!
//! Covers the contract surface most likely to break:
//!   * the PAT extractor accepts the Bearer header end-to-end and prefers
//!     it over the session cookie,
//!   * revoked / expired PATs return 401 with the documented
//!     `WWW-Authenticate` body and don't leak which arm failed,
//!   * the destructive-credential guardrail (PUT-overwrite + DELETE) maps
//!     to 403 `pat_destructive_blocked`,
//!   * `GET /api/auth/me` reports `via: "pat" | "session"`,
//!   * tenant-scoped PATs reject calls to a different workspace, and
//!   * `last_used_at` advances on a successful PAT auth.
//!
//! Reuses the in-memory SQLite + `build_router` harness established by
//! `tests/sso.rs`.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chrono::Utc;
use sqlx::pool::PoolOptions;
use sqlx::AnyPool;
use stiglab::core::{Tenant, TenantMember, User};
use stiglab::server::auth::{generate_pat_token, hash_pat_token, PAT_PREFIX_LEN};
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

fn auth_enabled_config() -> ServerConfig {
    ServerConfig {
        host: "0.0.0.0".into(),
        port: 3000,
        database_url: "sqlite::memory:".into(),
        static_dir: None,
        cors_origin: None,
        // Set both client_id + client_secret so config.auth_enabled() == true
        // — the PAT extractor only kicks in when auth is enabled.
        github_client_id: Some("client-id".into()),
        github_client_secret: Some("client-secret".into()),
        // Required for the credential set/get path used by the guardrail tests.
        credential_key: Some(stiglab::server::auth::generate_credential_key()),
        public_url: None,
        github_app_webhook_secret: None,
        sso_state_secret: None,
        sso_exchange_secret: None,
        sso_return_host_allowlist: Vec::new(),
        sso_auth_domain: None,
    }
}

async fn seed_user(pool: &AnyPool) -> User {
    let user = User {
        id: Uuid::new_v4().to_string(),
        github_id: 7,
        github_login: "patuser".into(),
        github_name: Some("PAT User".into()),
        github_avatar_url: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db::upsert_user(pool, &user).await.unwrap();
    user
}

async fn seed_tenant_with_member(pool: &AnyPool, user_id: &str, slug: &str) -> Tenant {
    let now = Utc::now();
    let tenant = Tenant {
        id: Uuid::new_v4().to_string(),
        slug: slug.into(),
        name: slug.into(),
        created_by: user_id.into(),
        created_at: now,
    };
    let member = TenantMember {
        tenant_id: tenant.id.clone(),
        user_id: user_id.into(),
        joined_at: now,
    };
    db::insert_tenant_with_creator(pool, &tenant, &member)
        .await
        .unwrap();
    tenant
}

/// Mint a PAT directly via the DB layer, bypassing `POST /api/pats`. Used
/// to set up auth state before exercising other endpoints.
async fn mint_pat(
    pool: &AnyPool,
    user_id: &str,
    tenant_id: Option<&str>,
    name: &str,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> (String, String) {
    let generated = generate_pat_token();
    let id = Uuid::new_v4().to_string();
    db::insert_user_pat(
        pool,
        &id,
        user_id,
        tenant_id,
        name,
        &generated.prefix,
        &generated.hash,
        expires_at,
    )
    .await
    .unwrap();
    (id, generated.token)
}

fn app(state: AppState) -> axum::Router {
    stiglab::server::build_router(state.clone(), &state.config)
}

fn bearer(req: axum::http::request::Builder, token: &str) -> axum::http::request::Builder {
    req.header(header::AUTHORIZATION, format!("Bearer {token}"))
}

async fn read_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ── Token format + hash invariants ──

#[test]
fn token_has_correct_prefix_and_length() {
    let pat = generate_pat_token();
    assert!(pat.token.starts_with("ons_pat_"));
    assert_eq!(pat.prefix.len(), PAT_PREFIX_LEN);
    assert_eq!(&pat.prefix, &pat.token[..PAT_PREFIX_LEN]);
    assert_eq!(pat.hash, hash_pat_token(&pat.token));
}

// ── DB-level verify_pat ──

#[tokio::test]
async fn verify_pat_rejects_unknown_prefix() {
    let pool = test_pool().await;
    let outcome = stiglab::server::auth::verify_pat(&pool, "ons_pat_doesnotexistxyz123abc")
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        stiglab::server::auth::PatVerifyOutcome::Unknown
    ));
}

#[tokio::test]
async fn verify_pat_rejects_wrong_namespace() {
    let pool = test_pool().await;
    let outcome = stiglab::server::auth::verify_pat(&pool, "ghp_aaaaaaaaaaaaaaaa")
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        stiglab::server::auth::PatVerifyOutcome::Unknown
    ));
}

#[tokio::test]
async fn verify_pat_accepts_valid_token() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let (pat_id, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    let outcome = stiglab::server::auth::verify_pat(&pool, &token)
        .await
        .unwrap();
    match outcome {
        stiglab::server::auth::PatVerifyOutcome::Ok(pat) => {
            assert_eq!(pat.id, pat_id);
            assert_eq!(pat.user_id, user.id);
        }
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_pat_reports_revoked_separately_from_unknown() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let (pat_id, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    db::revoke_user_pat(&pool, &user.id, &pat_id).await.unwrap();
    let outcome = stiglab::server::auth::verify_pat(&pool, &token)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        stiglab::server::auth::PatVerifyOutcome::Revoked
    ));
}

#[tokio::test]
async fn verify_pat_reports_expired_separately_from_unknown() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let past = Utc::now() - chrono::Duration::seconds(60);
    let (_, token) = mint_pat(&pool, &user.id, None, "ci", Some(past)).await;
    let outcome = stiglab::server::auth::verify_pat(&pool, &token)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        stiglab::server::auth::PatVerifyOutcome::Expired
    ));
}

// ── End-to-end PAT-authenticated requests ──

#[tokio::test]
async fn me_under_pat_reports_via_pat() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let (_, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            bearer(Request::builder().uri("/api/auth/me"), &token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_json(resp).await;
    assert_eq!(v["via"], "pat");
    assert_eq!(v["user"]["github_login"], "patuser");
}

#[tokio::test]
async fn me_under_session_reports_via_session_when_no_pat() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    // Mint a real session row.
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header(header::COOKIE, format!("stiglab_session={session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_json(resp).await;
    assert_eq!(v["via"], "session");
}

#[tokio::test]
async fn revoked_pat_returns_401_with_invalid_token_challenge() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let (pat_id, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    db::revoke_user_pat(&pool, &user.id, &pat_id).await.unwrap();
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            bearer(Request::builder().uri("/api/auth/me"), &token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let www = resp
        .headers()
        .get(header::WWW_AUTHENTICATE)
        .map(|v| v.to_str().unwrap().to_string());
    assert_eq!(
        www.as_deref(),
        Some("Bearer error=\"invalid_token\""),
        "WWW-Authenticate must report invalid_token"
    );
}

#[tokio::test]
async fn expired_pat_returns_401_with_invalid_token_challenge() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let past = Utc::now() - chrono::Duration::seconds(60);
    let (_, token) = mint_pat(&pool, &user.id, None, "ci", Some(past)).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            bearer(Request::builder().uri("/api/auth/me"), &token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok()),
        Some("Bearer error=\"invalid_token\"")
    );
}

#[tokio::test]
async fn pat_bearer_takes_precedence_over_cookie() {
    // A request that carries BOTH a valid PAT and a valid session cookie
    // should be authenticated as the PAT user (so that smoke-testing from
    // a CLI doesn't silently fall through to the browser session that
    // happens to be in scope).
    let pool = test_pool().await;
    let pat_user = seed_user(&pool).await;
    let cookie_user = User {
        id: Uuid::new_v4().to_string(),
        github_id: 999,
        github_login: "cookieuser".into(),
        github_name: None,
        github_avatar_url: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db::upsert_user(&pool, &cookie_user).await.unwrap();
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &cookie_user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    let (_, pat_token) = mint_pat(&pool, &pat_user.id, None, "ci", None).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header(header::AUTHORIZATION, format!("Bearer {pat_token}"))
                .header(header::COOKIE, format!("stiglab_session={session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_json(resp).await;
    assert_eq!(v["via"], "pat");
    assert_eq!(v["user"]["github_login"], "patuser");
}

// ── PAT CRUD via /api/pats ──

#[tokio::test]
async fn create_pat_returns_token_once_then_listing_hides_it() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    // Bootstrap auth via a session cookie so we can create a PAT.
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    let state = AppState::new(pool, auth_enabled_config(), None);
    let app_ = app(state);

    let exp = (Utc::now() + chrono::Duration::days(30)).to_rfc3339();
    let create = Request::builder()
        .method("POST")
        .uri("/api/pats")
        .header(header::COOKIE, format!("stiglab_session={session_token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "name": "ci", "expires_at": exp }).to_string(),
        ))
        .unwrap();
    let resp = app_.clone().oneshot(create).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    let token = body["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("ons_pat_"));
    let prefix = body["pat"]["token_prefix"].as_str().unwrap();
    assert_eq!(prefix.len(), PAT_PREFIX_LEN);

    // List — must NOT echo the secret token, only metadata + prefix.
    let list = Request::builder()
        .uri("/api/pats")
        .header(header::COOKIE, format!("stiglab_session={session_token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app_.oneshot(list).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    let pats = body["pats"].as_array().unwrap();
    assert_eq!(pats.len(), 1);
    assert_eq!(pats[0]["token_prefix"].as_str().unwrap(), prefix);
    assert!(pats[0].get("token").is_none());
    assert!(pats[0].get("token_hash").is_none());
}

#[tokio::test]
async fn create_pat_409s_on_duplicate_name() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    let state = AppState::new(pool, auth_enabled_config(), None);
    let app_ = app(state);

    let exp = (Utc::now() + chrono::Duration::days(30)).to_rfc3339();
    let make = || {
        Request::builder()
            .method("POST")
            .uri("/api/pats")
            .header(header::COOKIE, format!("stiglab_session={session_token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "name": "ci", "expires_at": exp }).to_string(),
            ))
            .unwrap()
    };
    assert_eq!(
        app_.clone().oneshot(make()).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        app_.oneshot(make()).await.unwrap().status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn create_pat_rejects_empty_name() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    let state = AppState::new(pool, auth_enabled_config(), None);

    let exp = (Utc::now() + chrono::Duration::days(7)).to_rfc3339();
    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/pats")
                .header(header::COOKIE, format!("stiglab_session={session_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "name": "  ", "expires_at": exp }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_pat_rejects_missing_expires_at() {
    // v1 contract: an explicit expiry is required. `null` is reserved for
    // a future "never expires" affordance and must not silently mint a
    // long-lived token today.
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    let state = AppState::new(pool, auth_enabled_config(), None);

    for body in [
        serde_json::json!({ "name": "ci", "expires_at": null }),
        serde_json::json!({ "name": "ci" }),
    ] {
        let resp = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/pats")
                    .header(header::COOKIE, format!("stiglab_session={session_token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn create_pat_response_carries_no_store_cache_headers() {
    // The body holds the only copy of the secret token — every
    // intermediary must be told not to cache it.
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    let state = AppState::new(pool, auth_enabled_config(), None);

    let exp = (Utc::now() + chrono::Duration::days(30)).to_rfc3339();
    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/pats")
                .header(header::COOKIE, format!("stiglab_session={session_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "name": "ci", "expires_at": exp }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        resp.headers().get("pragma").and_then(|v| v.to_str().ok()),
        Some("no-cache")
    );
}

#[tokio::test]
async fn delete_pat_revokes_so_subsequent_use_401s() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    let (pat_id, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    let state = AppState::new(pool, auth_enabled_config(), None);
    let app_ = app(state);

    // Sanity: PAT works before revoke.
    let pre = app_
        .clone()
        .oneshot(
            bearer(Request::builder().uri("/api/auth/me"), &token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pre.status(), StatusCode::OK);

    // Revoke via the API.
    let revoke = app_
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/pats/{pat_id}"))
                .header(header::COOKIE, format!("stiglab_session={session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::OK);

    // After revoke: same Bearer token now 401s with invalid_token.
    let post = app_
        .oneshot(
            bearer(Request::builder().uri("/api/auth/me"), &token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post.status(), StatusCode::UNAUTHORIZED);
}

// ── Destructive-credential guardrail ──

#[tokio::test]
async fn pat_can_create_credential_but_not_overwrite() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let (_, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    let state = AppState::new(pool, auth_enabled_config(), None);
    let app_ = app(state);

    // PUT to a brand-new credential name succeeds.
    let create = bearer(
        Request::builder()
            .method("PUT")
            .uri("/api/credentials/CI_TOKEN")
            .header(header::CONTENT_TYPE, "application/json"),
        &token,
    )
    .body(Body::from(
        serde_json::json!({ "value": "abc" }).to_string(),
    ))
    .unwrap();
    let resp = app_.clone().oneshot(create).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // PUT to the same name now must 403 with the documented body.
    let overwrite = bearer(
        Request::builder()
            .method("PUT")
            .uri("/api/credentials/CI_TOKEN")
            .header(header::CONTENT_TYPE, "application/json"),
        &token,
    )
    .body(Body::from(
        serde_json::json!({ "value": "xyz" }).to_string(),
    ))
    .unwrap();
    let resp = app_.oneshot(overwrite).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let v = read_json(resp).await;
    assert_eq!(v["error"], "pat_destructive_blocked");
}

#[tokio::test]
async fn pat_cannot_delete_credential() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    // Pre-create a credential via the DB so the DELETE actually has
    // something to refer to.
    let key = stiglab::server::auth::generate_credential_key();
    let enc = stiglab::server::auth::encrypt_credential(&key, "abc").unwrap();
    db::set_user_credential(&pool, &user.id, "CI_TOKEN", &enc)
        .await
        .unwrap();
    let (_, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            bearer(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/credentials/CI_TOKEN"),
                &token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let v = read_json(resp).await;
    assert_eq!(v["error"], "pat_destructive_blocked");
}

#[tokio::test]
async fn session_can_still_delete_credential_after_guardrail_added() {
    // Regression guard: the destructive guardrail must only fire for PATs.
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let key = stiglab::server::auth::generate_credential_key();
    let enc = stiglab::server::auth::encrypt_credential(&key, "abc").unwrap();
    db::set_user_credential(&pool, &user.id, "CI_TOKEN", &enc)
        .await
        .unwrap();
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/credentials/CI_TOKEN")
                .header(header::COOKIE, format!("stiglab_session={session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ── last_used_at ──

#[tokio::test]
async fn last_used_at_advances_after_pat_auth() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let (pat_id, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    // Sanity: brand-new PAT has no last_used_at.
    {
        let pats = db::list_user_pats(&pool, &user.id).await.unwrap();
        assert!(pats
            .iter()
            .any(|p| p.id == pat_id && p.last_used_at.is_none()));
    }

    let state = AppState::new(pool.clone(), auth_enabled_config(), None);
    let resp = app(state)
        .oneshot(
            bearer(
                Request::builder()
                    .uri("/api/auth/me")
                    .header(header::USER_AGENT, "test-agent/1.0"),
                &token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The touch is best-effort + spawned, so poll briefly for the update.
    let mut populated = false;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        let pats = db::list_user_pats(&pool, &user.id).await.unwrap();
        if let Some(p) = pats.iter().find(|p| p.id == pat_id) {
            if p.last_used_at.is_some() {
                populated = true;
                assert_eq!(p.last_used_user_agent.as_deref(), Some("test-agent/1.0"));
                break;
            }
        }
    }
    assert!(populated, "last_used_at should be set after a PAT auth");
}

// ── Tenant scope ──

#[tokio::test]
async fn tenant_scoped_pat_can_read_its_own_workspace() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let tenant = seed_tenant_with_member(&pool, &user.id, "wsa").await;
    let (_, token) = mint_pat(&pool, &user.id, Some(&tenant.id), "wsa-ci", None).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            bearer(
                Request::builder().uri(format!("/api/tenants/{}", tenant.id)),
                &token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_json(resp).await;
    assert_eq!(v["tenant"]["id"], tenant.id);
}

#[tokio::test]
async fn tenant_scoped_pat_rejects_other_workspace() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let tenant_a = seed_tenant_with_member(&pool, &user.id, "wsa").await;
    let tenant_b = seed_tenant_with_member(&pool, &user.id, "wsb").await;
    let (_, token) = mint_pat(&pool, &user.id, Some(&tenant_a.id), "wsa-ci", None).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            bearer(
                Request::builder().uri(format!("/api/tenants/{}", tenant_b.id)),
                &token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let v = read_json(resp).await;
    assert_eq!(v["error"], "pat_tenant_scope_mismatch");
}
