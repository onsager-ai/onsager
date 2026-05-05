//! Integration tests for the `/api/projects/:id/issues/:number/replay-trigger`
//! route (spec #203). The handler reaches out to GitHub for live label data
//! when access is granted, so these tests focus on the paths that short-
//! circuit before that fetch — `require_project_for_user`'s 404 contract
//! and the cross-workspace boundary.
//!
//! End-to-end coverage of the dry-run match resolution + spine emission
//! requires a GitHub stub and is left to manual testing on the preview
//! deploy. The unit tests in `onsager_spine::webhook_routing` already pin
//! the payload shape produced by `build_trigger_fired_events`.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chrono::Utc;
use sqlx::pool::PoolOptions;
use sqlx::AnyPool;
use stiglab::core::{
    GitHubAccountType, GitHubAppInstallation, Project, User, Workspace, WorkspaceMember,
};
use stiglab::server::auth::generate_credential_key;
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

async fn seed_project(pool: &AnyPool, workspace_id: &str) -> Project {
    let install = GitHubAppInstallation {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.into(),
        install_id: 1234,
        account_login: "acme".into(),
        account_type: GitHubAccountType::Organization,
        created_at: Utc::now(),
    };
    db::insert_github_app_installation(pool, &install, None)
        .await
        .unwrap();
    let project = Project {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.into(),
        github_app_installation_id: install.id.clone(),
        repo_owner: "acme".into(),
        repo_name: "widgets".into(),
        default_branch: "main".into(),
        created_at: Utc::now(),
    };
    db::insert_project(pool, &project).await.unwrap();
    project
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

fn cookie(token: &str) -> String {
    format!("stiglab_session={token}")
}

fn app(state: AppState) -> axum::Router {
    stiglab::server::build_router(state.clone(), &state.config)
}

fn replay_request(project_id: &str, issue_number: u64, session_token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!(
            "/api/projects/{project_id}/issues/{issue_number}/replay-trigger"
        ))
        .header(header::COOKIE, cookie(session_token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"dry_run":true}"#))
        .unwrap()
}

#[tokio::test]
async fn replay_trigger_unknown_project_is_404() {
    let pool = test_pool().await;
    let user = seed_user(&pool, "alice", 1).await;
    let _workspace = seed_workspace(&pool, &user.id, "w-a").await;
    let session_token = seed_session_cookie(&pool, &user.id).await;

    let state = AppState::new(pool, auth_enabled_config(), None);
    let resp = app(state)
        .oneshot(replay_request("does-not-exist", 1, &session_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn replay_trigger_cross_workspace_is_404() {
    // alice owns w_a, bob owns w_b. bob's project is invisible to alice;
    // a hit against it must 404 (not 403) to avoid leaking workspace
    // membership — same contract as the rest of the project-scoped routes.
    let pool = test_pool().await;
    let alice = seed_user(&pool, "alice", 1).await;
    let bob = seed_user(&pool, "bob", 2).await;
    let _w_a = seed_workspace(&pool, &alice.id, "w-a").await;
    let w_b = seed_workspace(&pool, &bob.id, "w-b").await;
    let bob_project = seed_project(&pool, &w_b.id).await;

    let alice_token = seed_session_cookie(&pool, &alice.id).await;

    let state = AppState::new(pool, auth_enabled_config(), None);
    let resp = app(state)
        .oneshot(replay_request(&bob_project.id, 1, &alice_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn replay_trigger_unauthenticated_is_401() {
    let pool = test_pool().await;
    let user = seed_user(&pool, "alice", 1).await;
    let workspace = seed_workspace(&pool, &user.id, "w-a").await;
    let project = seed_project(&pool, &workspace.id).await;

    let state = AppState::new(pool, auth_enabled_config(), None);
    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/projects/{}/issues/1/replay-trigger",
                    project.id
                ))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"dry_run":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
