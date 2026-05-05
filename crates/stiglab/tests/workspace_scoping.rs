//! Integration tests for issue #164: per-workspace API surface.
//!
//! Mirrors the contract test items in the spec:
//!   * `?workspace=` is required on every workspace-scoped list endpoint
//!     (400 on miss).
//!   * Sessions filtered by workspace don't leak across workspaces.
//!   * Detail endpoints return 404 (not 403) when the caller isn't a
//!     member of the owning workspace — matches the `require_workspace_access`
//!     contract documented in `routes/mod.rs`.
//!   * PATs pinned to W1 cannot list `?workspace=W2` (the principal-level
//!     guardrail returns 403 with `pat_workspace_scope_mismatch`).
//!   * Per-workspace credentials store and retrieve under the new path.
//!
//! Reuses the SQLite-in-memory + `build_router` pattern from
//! `tests/pats.rs` and `tests/workflow_webhook.rs`.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chrono::Utc;
use sqlx::pool::PoolOptions;
use sqlx::AnyPool;
use stiglab::core::{Session, SessionState, User, Workspace, WorkspaceMember};
use stiglab::server::auth::{generate_credential_key, generate_pat_token};
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
        credential_key: Some(generate_credential_key()),
        public_url: None,
        internal_dispatch_token: None,
    }
}

async fn seed_user(pool: &AnyPool, login: &str, github_id: i64) -> User {
    let user = User {
        id: Uuid::new_v4().to_string(),
        github_id,
        github_login: login.into(),
        github_name: None,
        github_avatar_url: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db::upsert_user(pool, &user).await.unwrap();
    user
}

async fn seed_workspace(pool: &AnyPool, user_id: &str, slug: &str) -> Workspace {
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

async fn seed_session_in_workspace(pool: &AnyPool, user_id: &str, workspace_id: &str) -> Session {
    let session = Session {
        id: Uuid::new_v4().to_string(),
        task_id: Uuid::new_v4().to_string(),
        node_id: "n1".into(),
        state: SessionState::Pending,
        prompt: "p".into(),
        output: None,
        working_dir: None,
        artifact_id: None,
        artifact_version: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db::insert_session_with_user_project_workspace(
        pool,
        &session,
        Some(user_id),
        None,
        Some(workspace_id),
    )
    .await
    .unwrap();
    session
}

async fn mint_pat(pool: &AnyPool, user_id: &str, workspace_id: &str) -> String {
    let generated = generate_pat_token();
    let id = Uuid::new_v4().to_string();
    db::insert_user_pat(
        pool,
        &id,
        user_id,
        workspace_id,
        "ci",
        &generated.prefix,
        &generated.hash,
        None,
    )
    .await
    .unwrap();
    generated.token
}

fn app(state: AppState) -> axum::Router {
    stiglab::server::build_router(state.clone(), &state.config)
}

fn cookie(token: &str) -> String {
    format!("stiglab_session={token}")
}

async fn seed_session_cookie(pool: &AnyPool, user_id: &str) -> String {
    let token = stiglab::server::auth::generate_session_token();
    db::create_auth_session(
        pool,
        &token,
        user_id,
        Utc::now() + chrono::Duration::days(1),
    )
    .await
    .unwrap();
    token
}

async fn read_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ── ?workspace= is required on list endpoints ──

#[tokio::test]
async fn list_sessions_without_workspace_returns_400() {
    let pool = test_pool().await;
    let user = seed_user(&pool, "u", 1).await;
    let _ws = seed_workspace(&pool, &user.id, "w1").await;
    let session_token = seed_session_cookie(&pool, &user.id).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header(header::COOKIE, cookie(&session_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_nodes_without_workspace_returns_400() {
    let pool = test_pool().await;
    let user = seed_user(&pool, "u", 1).await;
    let _ws = seed_workspace(&pool, &user.id, "w1").await;
    let session_token = seed_session_cookie(&pool, &user.id).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/nodes")
                .header(header::COOKIE, cookie(&session_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── parent contract test: cross-workspace session listing must not leak ──

#[tokio::test]
async fn list_sessions_filters_by_workspace_no_cross_leak() {
    let pool = test_pool().await;
    let user = seed_user(&pool, "u", 1).await;
    let w1 = seed_workspace(&pool, &user.id, "w1").await;
    let w2 = seed_workspace(&pool, &user.id, "w2").await;
    let s_w1 = seed_session_in_workspace(&pool, &user.id, &w1.id).await;
    let _s_w2 = seed_session_in_workspace(&pool, &user.id, &w2.id).await;

    let session_token = seed_session_cookie(&pool, &user.id).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions?workspace={}", w1.id))
                .header(header::COOKIE, cookie(&session_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    let sessions = body["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "only the W1 session should appear");
    assert_eq!(sessions[0]["id"], s_w1.id);
}

// ── 404 (not 403) on cross-workspace detail access ──

#[tokio::test]
async fn get_session_in_other_workspace_returns_404_not_403() {
    let pool = test_pool().await;
    // owner_w1 only belongs to W1. owner_w2 owns the session in W2.
    let owner_w1 = seed_user(&pool, "wonly1", 1).await;
    let owner_w2 = seed_user(&pool, "wonly2", 2).await;
    let _w1 = seed_workspace(&pool, &owner_w1.id, "w1").await;
    let w2 = seed_workspace(&pool, &owner_w2.id, "w2").await;
    let s_w2 = seed_session_in_workspace(&pool, &owner_w2.id, &w2.id).await;

    // owner_w1 logs in and asks for s_w2 — must 404.
    let session_token = seed_session_cookie(&pool, &owner_w1.id).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{}", s_w2.id))
                .header(header::COOKIE, cookie(&session_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "cross-workspace detail must 404, not 403"
    );
}

// ── PAT pinned to W1 cannot list ?workspace=W2 ──

#[tokio::test]
async fn pat_pinned_to_w1_cannot_list_w2_sessions() {
    let pool = test_pool().await;
    let user = seed_user(&pool, "u", 1).await;
    let w1 = seed_workspace(&pool, &user.id, "w1").await;
    let w2 = seed_workspace(&pool, &user.id, "w2").await;
    let _s_w2 = seed_session_in_workspace(&pool, &user.id, &w2.id).await;

    let token = mint_pat(&pool, &user.id, &w1.id).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions?workspace={}", w2.id))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = read_json(resp).await;
    assert_eq!(body["error"], "pat_workspace_scope_mismatch");
}

// ── per-workspace credentials live under /api/workspaces/:workspace/credentials ──

#[tokio::test]
async fn credentials_are_scoped_per_workspace() {
    let pool = test_pool().await;
    let user = seed_user(&pool, "u", 1).await;
    let w1 = seed_workspace(&pool, &user.id, "w1").await;
    let w2 = seed_workspace(&pool, &user.id, "w2").await;
    let session_token = seed_session_cookie(&pool, &user.id).await;
    let state = AppState::new(pool, auth_enabled_config(), None);
    let app_ = app(state);

    // PUT a credential in W1.
    let put = Request::builder()
        .method("PUT")
        .uri(format!(
            "/api/workspaces/{}/credentials/CLAUDE_CODE_OAUTH_TOKEN",
            w1.id
        ))
        .header(header::COOKIE, cookie(&session_token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "value": "w1-secret" }).to_string(),
        ))
        .unwrap();
    let resp = app_.clone().oneshot(put).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET in W1 sees it.
    let list_w1 = Request::builder()
        .uri(format!("/api/workspaces/{}/credentials", w1.id))
        .header(header::COOKIE, cookie(&session_token))
        .body(Body::empty())
        .unwrap();
    let resp = app_.clone().oneshot(list_w1).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    let names: Vec<&str> = body["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["CLAUDE_CODE_OAUTH_TOKEN"]);

    // GET in W2 — same user, different workspace — sees nothing.
    let list_w2 = Request::builder()
        .uri(format!("/api/workspaces/{}/credentials", w2.id))
        .header(header::COOKIE, cookie(&session_token))
        .body(Body::empty())
        .unwrap();
    let resp = app_.oneshot(list_w2).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert!(body["credentials"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn credentials_in_other_workspace_404_for_non_member() {
    let pool = test_pool().await;
    let owner = seed_user(&pool, "owner", 1).await;
    let outsider = seed_user(&pool, "outsider", 2).await;
    let w1 = seed_workspace(&pool, &owner.id, "w1").await;
    // outsider has no membership in w1.
    let outsider_token = seed_session_cookie(&pool, &outsider.id).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/api/workspaces/{}/credentials", w1.id))
                .header(header::COOKIE, cookie(&outsider_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Outsider must see workspace-not-found, not the credential surface.
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── credential lookup at session-launch time ──

#[tokio::test]
async fn workspace_scoped_credential_lookup_returns_only_the_workspace_value() {
    // Direct DB-level check that the per-workspace lookup honours the
    // partition: a user with two workspaces holding the same credential
    // name in each gets the W1 value when asked about W1, never the W2
    // value (and vice versa). This is the contract the session-launch
    // path relies on to send the right token to the agent runner.
    let pool = test_pool().await;
    let user = seed_user(&pool, "u", 1).await;
    let w1 = seed_workspace(&pool, &user.id, "w1").await;
    let w2 = seed_workspace(&pool, &user.id, "w2").await;
    let key = stiglab::server::auth::generate_credential_key();

    let enc_w1 = stiglab::server::auth::encrypt_credential(&key, "w1-token").unwrap();
    let enc_w2 = stiglab::server::auth::encrypt_credential(&key, "w2-token").unwrap();
    db::set_user_credential(&pool, &w1.id, &user.id, "CLAUDE_CODE_OAUTH_TOKEN", &enc_w1)
        .await
        .unwrap();
    db::set_user_credential(&pool, &w2.id, &user.id, "CLAUDE_CODE_OAUTH_TOKEN", &enc_w2)
        .await
        .unwrap();

    let creds_w1 = db::get_all_user_credential_values(&pool, &w1.id, &user.id)
        .await
        .unwrap();
    assert_eq!(creds_w1.len(), 1);
    let plaintext_w1 = stiglab::server::auth::decrypt_credential(&key, &creds_w1[0].1).unwrap();
    assert_eq!(plaintext_w1, "w1-token");

    let creds_w2 = db::get_all_user_credential_values(&pool, &w2.id, &user.id)
        .await
        .unwrap();
    assert_eq!(creds_w2.len(), 1);
    let plaintext_w2 = stiglab::server::auth::decrypt_credential(&key, &creds_w2[0].1).unwrap();
    assert_eq!(plaintext_w2, "w2-token");
}

// ── github_app_installations.install_id is UNIQUE ──

#[tokio::test]
async fn github_app_install_id_is_unique() {
    let pool = test_pool().await;
    let user = seed_user(&pool, "u", 1).await;
    let w1 = seed_workspace(&pool, &user.id, "w1").await;
    let w2 = seed_workspace(&pool, &user.id, "w2").await;

    let install_a = stiglab::core::GitHubAppInstallation {
        id: Uuid::new_v4().to_string(),
        workspace_id: w1.id.clone(),
        install_id: 42,
        account_login: "acme".into(),
        account_type: stiglab::core::GitHubAccountType::Organization,
        created_at: Utc::now(),
    };
    db::insert_github_app_installation(&pool, &install_a, None)
        .await
        .unwrap();

    // Re-using the same numeric install_id under a different workspace
    // must fail — the unique index documented in
    // `migrations/002_workspace_scoped_credentials.sql` enforces the
    // 1:1 install_id ↔ workspace_id invariant the webhook handler
    // depends on.
    let install_b = stiglab::core::GitHubAppInstallation {
        id: Uuid::new_v4().to_string(),
        workspace_id: w2.id.clone(),
        install_id: 42,
        account_login: "globex".into(),
        account_type: stiglab::core::GitHubAccountType::Organization,
        created_at: Utc::now(),
    };
    let result = db::insert_github_app_installation(&pool, &install_b, None).await;
    assert!(
        result.is_err(),
        "duplicate install_id must violate the UNIQUE index"
    );
}

// ── session_logs forwards the auth helper's response verbatim ──

#[tokio::test]
async fn session_logs_for_other_workspace_returns_404_not_generic_body() {
    // Regression for the Copilot review on PR #189: the previous
    // implementation replaced the auth helper's response with a
    // generic `{ "error": "see body" }` body, hiding both the
    // `pat_workspace_scope_mismatch` 403 and the rewritten "session
    // not found" 404. The forwarded response should pass through.
    let pool = test_pool().await;
    let owner_w1 = seed_user(&pool, "wonly1", 1).await;
    let owner_w2 = seed_user(&pool, "wonly2", 2).await;
    let _w1 = seed_workspace(&pool, &owner_w1.id, "w1").await;
    let w2 = seed_workspace(&pool, &owner_w2.id, "w2").await;
    let s_w2 = seed_session_in_workspace(&pool, &owner_w2.id, &w2.id).await;

    let session_token = seed_session_cookie(&pool, &owner_w1.id).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{}/logs", s_w2.id))
                .header(header::COOKIE, cookie(&session_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = read_json(resp).await;
    assert_eq!(body["error"], "session not found");
}

// ── POST /api/tasks honours an explicit workspace_id ──

#[tokio::test]
async fn create_task_with_explicit_workspace_id_files_under_that_workspace() {
    // Regression for the Copilot review on PR #189: post-#164 a session
    // created with no project_id used to land with `workspace_id =
    // NULL`, making it invisible to every `?workspace=` listing. The
    // new path lets callers pass `workspace_id` directly so the
    // session is discoverable + correctly credentialed.
    let pool = test_pool().await;
    let user = seed_user(&pool, "u", 1).await;
    let w = seed_workspace(&pool, &user.id, "w-tasks").await;

    // Seed a node so the task dispatch can pick a target.
    sqlx::query(
        "INSERT INTO nodes (id, name, hostname, status, max_sessions, \
                            active_sessions, last_heartbeat, registered_at) \
         VALUES ($1, $2, $3, 'online', 4, 0, $4, $4)",
    )
    .bind("n1")
    .bind("n1")
    .bind("local")
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .unwrap();

    let session_token = seed_session_cookie(&pool, &user.id).await;
    let state = AppState::new(pool.clone(), auth_enabled_config(), None);

    let body = serde_json::json!({
        "prompt": "hi",
        "workspace_id": w.id,
    });
    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/tasks")
                .header(header::COOKIE, cookie(&session_token))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // The session row carries the workspace, so it shows up under
    // the W-scoped listing.
    let listed = db::list_sessions_for_user_in_workspace(&pool, &user.id, &w.id)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1, "session should appear under W's listing");
}

#[tokio::test]
async fn create_task_rejects_conflicting_project_and_workspace_ids() {
    // When both project_id and workspace_id are set and disagree, the
    // server 400s rather than silently preferring one — the dashboard
    // should pick which scope is authoritative.
    let pool = test_pool().await;
    let user = seed_user(&pool, "u", 1).await;
    let w_a = seed_workspace(&pool, &user.id, "w-a").await;
    let w_b = seed_workspace(&pool, &user.id, "w-b").await;

    // The node lookup runs before the workspace validation in
    // `create_task`; without a node it 503s and we never reach the
    // conflict check.
    sqlx::query(
        "INSERT INTO nodes (id, name, hostname, status, max_sessions, \
                            active_sessions, last_heartbeat, registered_at) \
         VALUES ($1, $2, $3, 'online', 4, 0, $4, $4)",
    )
    .bind("n1")
    .bind("n1")
    .bind("local")
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .unwrap();

    // Seed an installation + project owned by w_a.
    let install = stiglab::core::GitHubAppInstallation {
        id: Uuid::new_v4().to_string(),
        workspace_id: w_a.id.clone(),
        install_id: 1234,
        account_login: "acme".into(),
        account_type: stiglab::core::GitHubAccountType::Organization,
        created_at: Utc::now(),
    };
    db::insert_github_app_installation(&pool, &install, None)
        .await
        .unwrap();
    let project = stiglab::core::Project {
        id: Uuid::new_v4().to_string(),
        workspace_id: w_a.id.clone(),
        github_app_installation_id: install.id.clone(),
        repo_owner: "acme".into(),
        repo_name: "widgets".into(),
        default_branch: "main".into(),
        created_at: Utc::now(),
    };
    db::insert_project(&pool, &project).await.unwrap();

    let session_token = seed_session_cookie(&pool, &user.id).await;
    let state = AppState::new(pool, auth_enabled_config(), None);

    let body = serde_json::json!({
        "prompt": "hi",
        "project_id": project.id,
        // Disagrees with project.workspace_id (= w_a.id).
        "workspace_id": w_b.id,
    });
    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/tasks")
                .header(header::COOKIE, cookie(&session_token))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
