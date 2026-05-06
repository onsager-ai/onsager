//! Integration tests for stiglab's PAT-aware `AuthUser` extractor and the
//! credential / workspace-scope routes that still consume PAT bearer auth.
//!
//! Spec #222 Slice 2b moved `/api/pats*` CRUD to portal — minting, listing,
//! and revoking PATs is exercised on the portal side. The tests here cover
//! the bits that remain stiglab-owned for the duration of Slices 2a / 3 / 4:
//!
//!   * stiglab's `verify_pat` DB primitive and `AuthUser` Bearer-vs-cookie
//!     precedence (still hot because credentials/workspaces/projects/
//!     workflows accept PAT bearer auth on stiglab),
//!   * the destructive-credential guardrail (PUT-overwrite + DELETE) maps to
//!     403 `pat_destructive_blocked`,
//!   * workspace-scoped PATs reject calls to a different workspace, and
//!   * `last_used_at` advances on a successful PAT auth.
//!
//! The auth-extractor tests pivot onto stiglab routes that survive the
//! Slice 2b move: `/api/projects` (cross-workspace project listing,
//! `AuthUser`-gated) and `/api/workspaces/{id}` (workspace get,
//! `require_workspace_access`-gated). When credentials/workspaces move in
//! later slices, these tests will follow them to portal-side coverage.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chrono::Utc;
use sqlx::pool::PoolOptions;
use sqlx::AnyPool;
use stiglab::core::{User, Workspace, WorkspaceMember};
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
        // Required for the credential set/get path used by the guardrail tests.
        credential_key: Some(stiglab::server::auth::generate_credential_key()),
        public_url: None,
        internal_dispatch_token: None,
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

async fn seed_workspace_with_member(pool: &AnyPool, user_id: &str, slug: &str) -> Workspace {
    let now = Utc::now();
    let workspace = Workspace {
        id: Uuid::new_v4().to_string(),
        slug: slug.into(),
        name: slug.into(),
        created_by: user_id.into(),
        created_at: now,
    };
    let member = WorkspaceMember {
        workspace_id: workspace.id.clone(),
        user_id: user_id.into(),
        joined_at: now,
    };
    db::insert_workspace_with_creator(pool, &workspace, &member)
        .await
        .unwrap();
    workspace
}

/// Mint a PAT directly via the DB layer, bypassing `POST /api/pats` (which
/// now lives on portal). Used to set up auth state before exercising other
/// endpoints.
///
/// PATs are workspace-scoped post-#163; when the caller doesn't already have
/// a workspace in scope, pass `None` and `mint_pat` seeds a throwaway one
/// for the user just so the row satisfies the NOT NULL constraint.
async fn mint_pat(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: Option<&str>,
    name: &str,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> (String, String) {
    let synthetic_ws;
    let workspace_id: &str = match workspace_id {
        Some(w) => w,
        None => {
            let ws = seed_workspace_with_member(
                pool,
                user_id,
                &format!("pat-{}", &Uuid::new_v4().to_string()[..8]),
            )
            .await;
            synthetic_ws = ws.id;
            synthetic_ws.as_str()
        }
    };
    let generated = generate_pat_token();
    let id = Uuid::new_v4().to_string();
    db::insert_user_pat(
        pool,
        &id,
        user_id,
        workspace_id,
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
//
// The PAT auth path lives in stiglab's `AuthUser` extractor as long as
// non-portal routes (credentials, workspaces, projects, workflows) accept
// PAT bearer auth. The tests below pivot onto `/api/projects` — a
// cross-workspace, AuthUser-gated route stiglab still owns — to exercise
// the extractor end-to-end without depending on `/api/pats` (now a portal
// proxy).

#[tokio::test]
async fn revoked_pat_returns_401_with_invalid_token_challenge() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let (pat_id, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    db::revoke_user_pat(&pool, &user.id, &pat_id).await.unwrap();
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            bearer(Request::builder().uri("/api/projects"), &token)
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
            bearer(Request::builder().uri("/api/projects"), &token)
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
    // happens to be in scope). The proof works because the PAT is pinned
    // to `pat_workspace`; sending both auths to a different workspace's
    // GET route must 403 with `pat_workspace_scope_mismatch`. If the
    // cookie principal had won, the cookie user is a member of
    // `cookie_workspace` so the same request would 200.
    let pool = test_pool().await;

    let pat_user = seed_user(&pool).await;
    let pat_workspace = seed_workspace_with_member(&pool, &pat_user.id, "pat-ws").await;
    let (_, pat_token) = mint_pat(&pool, &pat_user.id, Some(&pat_workspace.id), "ci", None).await;

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
    let cookie_workspace = seed_workspace_with_member(&pool, &cookie_user.id, "cookie-ws").await;
    let session_token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        &pool,
        &session_token,
        &cookie_user.id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();

    let state = AppState::new(pool, auth_enabled_config(), None);

    // Hit cookie_user's workspace with both auths. PAT precedence ⇒ 403
    // (PAT is pinned to pat_workspace, request hits cookie_workspace).
    // Cookie precedence would have been 200 (cookie_user is a member).
    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/api/workspaces/{}", cookie_workspace.id))
                .header(header::AUTHORIZATION, format!("Bearer {pat_token}"))
                .header(header::COOKIE, format!("stiglab_session={session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let v = read_json(resp).await;
    assert_eq!(v["error"], "pat_workspace_scope_mismatch");
}

// ── Destructive-credential guardrail ──
//
// Spec #222 Slice 2a moved `/api/workspaces/:id/credentials*` to portal,
// taking the `pat_destructive_blocked` guardrail with it. The
// route-level tests retire here (the stiglab path is now a proxy and
// portal isn't running in this harness) pending a Postgres-backed
// portal integration-test harness — same status as the Slice 2b
// `/api/pats` CRUD tests.

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
                    .uri("/api/projects")
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

// ── Workspace scope ──

#[tokio::test]
async fn workspace_scoped_pat_can_read_its_own_workspace() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let workspace = seed_workspace_with_member(&pool, &user.id, "wsa").await;
    let (_, token) = mint_pat(&pool, &user.id, Some(&workspace.id), "wsa-ci", None).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            bearer(
                Request::builder().uri(format!("/api/workspaces/{}", workspace.id)),
                &token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = read_json(resp).await;
    assert_eq!(v["workspace"]["id"], workspace.id);
}

#[tokio::test]
async fn workspace_scoped_pat_rejects_other_workspace() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let workspace_a = seed_workspace_with_member(&pool, &user.id, "wsa").await;
    let workspace_b = seed_workspace_with_member(&pool, &user.id, "wsb").await;
    let (_, token) = mint_pat(&pool, &user.id, Some(&workspace_a.id), "wsa-ci", None).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            bearer(
                Request::builder().uri(format!("/api/workspaces/{}", workspace_b.id)),
                &token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let v = read_json(resp).await;
    assert_eq!(v["error"], "pat_workspace_scope_mismatch");
}
