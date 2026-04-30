//! Integration tests for the stiglab workflow webhook receiver (issue #81).
//!
//! These tests exercise the `POST /api/webhooks/github` handler end-to-end
//! against an in-memory SQLite database and no spine. They verify:
//! - signature validation (valid / invalid / missing)
//! - `issues.labeled` routing through `workflow_db::find_active_github_workflows_for_label`
//! - pre-shared secret encryption round-trip
//! - 401/400 posture for unknown installations and malformed payloads
//!
//! The full E2E "event appears on the spine" path requires Postgres and is
//! covered by the onsager-spine integration harness. Here we go as far as the
//! routing decision — the unit tests in `server::webhook_router` cover the
//! exact event payload shapes.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use ring::hmac;
use sqlx::pool::PoolOptions;
use sqlx::{AnyPool, PgPool};
use stiglab::core::workflow::{TriggerKind, Workflow, WorkflowStage};
use stiglab::core::{
    GateKind, GitHubAccountType, GitHubAppInstallation, Workspace, WorkspaceMember,
};
use stiglab::server::auth::encrypt_credential;
use stiglab::server::config::ServerConfig;
use stiglab::server::db;
use stiglab::server::spine::SpineEmitter;
use stiglab::server::state::AppState;
use stiglab::server::workflow_db;
use tower::ServiceExt;
use uuid::Uuid;

/// 32-byte hex key used by `encrypt_credential` and `decrypt_credential`.
const TEST_KEY_HEX: &str = "0011223344556677889900112233445566778899001122334455667788990011";

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

/// Spine emitter wired to `DATABASE_URL` for tests that seed
/// `workflows` rows. Returns `None` when the env var is unset so the
/// test can skip cleanly in a sqlite-only run; matches the convention
/// in `crates/onsager-warehouse/tests/warehouse_flow.rs`. CI provides
/// the URL via the rust.yml `postgres:16` service container; locally
/// `just dev` exposes one on `:5432`.
async fn try_spine() -> Option<SpineEmitter> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(SpineEmitter::connect(&url).await.expect("spine connect"))
}

/// Best-effort cleanup so a fresh test run on an existing DB doesn't
/// trip over leftover rows from a prior failed run. Targeted at
/// `(workspace_id, install_id)` so we don't touch other tests' rows.
async fn reset_spine_workflows(spine: &PgPool, workspace_id: &str) {
    let _ = sqlx::query("DELETE FROM workflows WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(spine)
        .await;
}

async fn seed_workspace_and_installation(pool: &AnyPool, install_numeric_id: i64) -> String {
    let workspace_id = Uuid::new_v4().to_string();
    // Seed a user row for the workspace creator.
    sqlx::query(
        "INSERT INTO users (id, github_id, github_login, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind("u1")
    .bind(1i64)
    .bind("u1")
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();

    let w = Workspace {
        id: workspace_id.clone(),
        slug: format!("w-{}", &workspace_id[..8]),
        name: "workspace".into(),
        created_by: "u1".into(),
        created_at: Utc::now(),
    };
    db::insert_workspace(pool, &w).await.unwrap();
    db::insert_workspace_member(
        pool,
        &WorkspaceMember {
            workspace_id: workspace_id.clone(),
            user_id: "u1".into(),
            joined_at: Utc::now(),
        },
    )
    .await
    .unwrap();

    let cipher = encrypt_credential(TEST_KEY_HEX, "webhook-shared-secret").unwrap();
    let install = GitHubAppInstallation {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.clone(),
        install_id: install_numeric_id,
        account_login: "acme".into(),
        account_type: GitHubAccountType::Organization,
        created_at: Utc::now(),
    };
    db::insert_github_app_installation(pool, &install, Some(&cipher))
        .await
        .unwrap();

    workspace_id
}

/// Seed an installation as the OAuth-callback flow does — row exists, but
/// no per-install webhook-secret cipher is stored. The global App secret
/// from `GITHUB_APP_WEBHOOK_SECRET` is expected to cover it.
async fn seed_workspace_and_installation_without_cipher(
    pool: &AnyPool,
    install_numeric_id: i64,
) -> String {
    let workspace_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO users (id, github_id, github_login, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind("u1")
    .bind(1i64)
    .bind("u1")
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();

    let w = Workspace {
        id: workspace_id.clone(),
        slug: format!("w-{}", &workspace_id[..8]),
        name: "workspace".into(),
        created_by: "u1".into(),
        created_at: Utc::now(),
    };
    db::insert_workspace(pool, &w).await.unwrap();
    db::insert_workspace_member(
        pool,
        &WorkspaceMember {
            workspace_id: workspace_id.clone(),
            user_id: "u1".into(),
            joined_at: Utc::now(),
        },
    )
    .await
    .unwrap();

    let install = GitHubAppInstallation {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.clone(),
        install_id: install_numeric_id,
        account_login: "acme".into(),
        account_type: GitHubAccountType::Organization,
        created_at: Utc::now(),
    };
    db::insert_github_app_installation(pool, &install, None)
        .await
        .unwrap();

    workspace_id
}

async fn seed_active_workflow(spine: &PgPool, workspace_id: &str, install_id: i64, label: &str) {
    let now = Utc::now();
    let wf = Workflow {
        id: format!("wf_{}", Uuid::new_v4()),
        workspace_id: workspace_id.to_string(),
        name: "sdd".into(),
        trigger_kind: TriggerKind::GithubIssueWebhook,
        repo_owner: "acme".into(),
        repo_name: "widgets".into(),
        trigger_label: label.to_string(),
        install_id,
        preset_id: Some("github-issue-to-pr".into()),
        active: true,
        created_by: "u1".into(),
        created_at: now,
        updated_at: now,
    };
    let stage = WorkflowStage {
        id: Uuid::new_v4().to_string(),
        workflow_id: wf.id.clone(),
        seq: 0,
        gate_kind: GateKind::AgentSession,
        params: serde_json::json!({}),
    };
    workflow_db::insert_workflow_with_stages(spine, &wf, &[stage])
        .await
        .unwrap();
}

fn app_state_with_spine(pool: AnyPool, spine: Option<SpineEmitter>) -> AppState {
    let mut s = app_state(pool);
    s.spine = spine;
    s
}

fn app_state(pool: AnyPool) -> AppState {
    let mut config = ServerConfig {
        host: "127.0.0.1".into(),
        port: 0,
        database_url: "sqlite::memory:".into(),
        static_dir: None,
        cors_origin: None,
        github_client_id: None,
        github_client_secret: None,
        credential_key: Some(TEST_KEY_HEX.to_string()),
        public_url: None,
        github_app_webhook_secret: None,
        sso_state_secret: None,
        sso_exchange_secret: None,
        sso_return_host_allowlist: Vec::new(),
        sso_auth_domain: None,
        internal_dispatch_token: None,
    };
    // Auth disabled — the webhook receiver doesn't need AuthUser; the CRUD
    // routes aren't exercised here.
    config.github_client_id = None;
    config.github_client_secret = None;
    AppState::new(pool, config, None)
}

fn hmac_header(body: &[u8], secret: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let tag = hmac::sign(&key, body);
    format!("sha256={}", hex::encode(tag.as_ref()))
}

fn issues_labeled_payload(install_id: i64, label: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "action": "labeled",
        "issue": {"number": 123, "title": "fix the bug"},
        "label": {"name": label},
        "repository": {"name": "widgets", "owner": {"login": "acme"}},
        "installation": {"id": install_id},
    }))
    .unwrap()
}

#[tokio::test]
async fn webhook_with_valid_signature_returns_202() {
    let Some(spine) = try_spine().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = test_pool().await;
    let install_id = 987;
    let workspace_id = seed_workspace_and_installation(&pool, install_id).await;
    reset_spine_workflows(spine.pool(), &workspace_id).await;
    seed_active_workflow(spine.pool(), &workspace_id, install_id, "spec").await;

    let state = app_state_with_spine(pool, Some(spine));
    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = issues_labeled_payload(install_id, "spec");
    let sig = hmac_header(&body, b"webhook-shared-secret");

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/github")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", sig)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn webhook_alias_under_github_app_prefix_returns_202() {
    // `/api/github-app/*` hosts the GET-only OAuth/install flow and had
    // no handler at `/api/github-app/webhook`, so an App configured to
    // POST its webhook there never reached the handler. Verify the alias
    // catches it.
    let Some(spine) = try_spine().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = test_pool().await;
    let install_id = 987;
    let workspace_id = seed_workspace_and_installation(&pool, install_id).await;
    reset_spine_workflows(spine.pool(), &workspace_id).await;
    seed_active_workflow(spine.pool(), &workspace_id, install_id, "spec").await;

    let state = app_state_with_spine(pool, Some(spine));
    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = issues_labeled_payload(install_id, "spec");
    let sig = hmac_header(&body, b"webhook-shared-secret");

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/github-app/webhook")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", sig)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn webhook_with_bad_signature_returns_401() {
    let pool = test_pool().await;
    let install_id = 987;
    seed_workspace_and_installation(&pool, install_id).await;

    let state = app_state(pool);
    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = issues_labeled_payload(install_id, "spec");
    let sig = hmac_header(&body, b"wrong-secret");
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/github")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", sig)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_without_signature_returns_401() {
    let pool = test_pool().await;
    let install_id = 987;
    seed_workspace_and_installation(&pool, install_id).await;

    let state = app_state(pool);
    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = issues_labeled_payload(install_id, "spec");
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/github")
                .header("x-github-event", "issues")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_with_unknown_installation_returns_401() {
    let pool = test_pool().await;
    // No installation seeded — installation.id=9999 won't match.
    let state = app_state(pool);
    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = issues_labeled_payload(9999, "spec");
    let sig = hmac_header(&body, b"webhook-shared-secret");
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/github")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", sig)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_with_missing_installation_id_returns_400() {
    let pool = test_pool().await;
    let state = app_state(pool);
    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = serde_json::to_vec(&serde_json::json!({
        "action": "labeled",
        "issue": {"number": 1},
    }))
    .unwrap();
    let sig = hmac_header(&body, b"anything");
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/github")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", sig)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn webhook_with_malformed_body_returns_400() {
    let pool = test_pool().await;
    let state = app_state(pool);
    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = b"not json".to_vec();
    let sig = hmac_header(&body, b"anything");
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/github")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", sig)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn webhook_with_global_app_secret_is_verified_and_accepted() {
    // Install row exists (via OAuth callback) but has no per-install
    // cipher — the `GITHUB_APP_WEBHOOK_SECRET` env var is the fallback.
    let pool = test_pool().await;
    let install_id = 126368686;
    seed_workspace_and_installation_without_cipher(&pool, install_id).await;
    let mut state = app_state(pool);
    let mut cfg = state.config.clone();
    cfg.github_app_webhook_secret = Some("railway-app-secret".into());
    state.config = cfg;

    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = issues_labeled_payload(install_id, "spec");
    let sig = hmac_header(&body, b"railway-app-secret");

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/github")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", sig)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn webhook_with_global_app_secret_rejects_bad_signature() {
    let pool = test_pool().await;
    let install_id = 126368686;
    seed_workspace_and_installation_without_cipher(&pool, install_id).await;
    let mut state = app_state(pool);
    let mut cfg = state.config.clone();
    cfg.github_app_webhook_secret = Some("railway-app-secret".into());
    state.config = cfg;

    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = issues_labeled_payload(install_id, "spec");
    let sig = hmac_header(&body, b"wrong-secret");

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/github")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", sig)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_with_global_app_secret_but_unknown_install_returns_401() {
    // Regression guard: the global-secret fallback must NOT accept
    // webhooks for installations stiglab has never seen.
    let pool = test_pool().await;
    let mut state = app_state(pool);
    let mut cfg = state.config.clone();
    cfg.github_app_webhook_secret = Some("railway-app-secret".into());
    state.config = cfg;

    let config = state.config.clone();
    let app = stiglab::server::build_router(state, &config);

    let body = issues_labeled_payload(9999, "spec");
    let sig = hmac_header(&body, b"railway-app-secret");

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/github")
                .header("x-github-event", "issues")
                .header("x-hub-signature-256", sig)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
