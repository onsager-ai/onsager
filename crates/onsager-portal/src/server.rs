//! HTTP server wiring.

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::config::Config;
use crate::gate::GateClient;
use crate::handlers::{
    auth as auth_handlers, credentials as credential_handlers, github_app as github_app_handlers,
    installations as installation_handlers, pats as pat_handlers, projects as project_handlers,
    webhook, workflow_kinds as workflow_kind_handlers, workflows as workflow_handlers,
    workspaces as workspace_handlers,
};
use crate::state::AppState;

/// Boot the webhook server. Blocks until the listener exits.
pub async fn run(config: Config) -> anyhow::Result<()> {
    config.assert_sso_consistent();

    let pool = crate::db::connect(&config.database_url).await?;
    crate::migrate::run(&pool).await?;
    let spine = onsager_spine::EventStore::connect(&config.database_url).await?;
    // Register the GitHub adapter into the spine catalog. Best-effort:
    // a missing `artifact_adapters` table (older migration set) shouldn't
    // block portal boot. See `onsager_github::adapter::register`.
    if let Err(e) = onsager_github::adapter::register(&pool, "default").await {
        tracing::warn!(error = %e, "github adapter registration skipped");
    }

    // Seed the dev user / workspace in debug builds so the dev-login
    // button works on a fresh DB. Stiglab's `workspaces` /
    // `workspace_members` tables must already exist; in `just dev` the
    // boot order is portal→stiglab so portal doesn't see them on a cold
    // start. Best-effort: if the tables aren't there yet, log and
    // continue — stiglab's first migration run will create them, and
    // the next portal start will seed.
    #[cfg(debug_assertions)]
    if let Err(e) = crate::dev_auth::seed_dev_user_and_workspace(&pool).await {
        tracing::warn!(error = %e, "portal dev-login seeder skipped (workspaces table missing?)");
    }

    let gate = Arc::new(GateClient::new(config.synodic_url.clone()));

    let state = AppState {
        pool,
        spine,
        config: Arc::new(config.clone()),
        gate,
    };

    // The stiglab reverse proxy preserves the request path, so portal
    // accepts every URL the GitHub App might post to. `/webhooks/github`
    // is canonical; `/api/webhooks/github` and `/api/github-app/webhook`
    // are backward-compat aliases for older App configurations
    // (#222 Slice 1).
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/webhooks/github", post(webhook::handle))
        .route("/api/webhooks/github", post(webhook::handle))
        .route("/api/github-app/webhook", post(webhook::handle))
        // Auth routes (#222 Slice 5). Stiglab proxies `/api/auth/*`
        // here so dashboard fetches keep working pre–API_BASE cutover.
        .route("/api/auth/github", get(auth_handlers::github_login))
        .route(
            "/api/auth/github/callback",
            get(auth_handlers::github_callback),
        )
        .route("/api/auth/me", get(auth_handlers::me))
        .route("/api/auth/logout", post(auth_handlers::logout))
        .route("/api/auth/sso/redeem", post(auth_handlers::sso_redeem))
        .route("/api/auth/sso/finish", get(auth_handlers::sso_finish))
        // Personal Access Tokens (#222 Slice 2b). Stiglab proxies
        // `/api/pats*` here so dashboard fetches keep working pre–API_BASE
        // cutover. Auth (cookie or PAT bearer) is enforced by the
        // `AuthUser` extractor in each handler.
        .route(
            "/api/pats",
            get(pat_handlers::list_pats).post(pat_handlers::create_pat),
        )
        .route("/api/pats/{id}", delete(pat_handlers::delete_pat))
        // Per-workspace credential CRUD (#222 Slice 2a). Stiglab proxies
        // `/api/workspaces/{id}/credentials*` here so dashboard fetches
        // keep working pre–API_BASE cutover. Auth (cookie or PAT bearer
        // — PATs are gated by the `pat_destructive_blocked` guardrail
        // for overwrites/deletes) is enforced by the `AuthUser`
        // extractor in each handler.
        .route(
            "/api/workspaces/{workspace_id}/credentials",
            get(credential_handlers::list_credentials),
        )
        .route(
            "/api/workspaces/{workspace_id}/credentials/{name}",
            put(credential_handlers::set_credential).delete(credential_handlers::delete_credential),
        )
        // Workspace + member + project CRUD (#222 Slice 3a). Stiglab
        // proxies these URLs through `routes::portal::proxy` so the
        // dashboard's API_BASE cutover (Slice 6) can land independently.
        // Auth (cookie or PAT bearer) is enforced by the `AuthUser`
        // extractor; PAT-pinned principals are 403'd against
        // mismatched workspace IDs by `require_workspace_access`.
        .route(
            "/api/workspaces",
            get(workspace_handlers::list_workspaces).post(workspace_handlers::create_workspace),
        )
        .route(
            "/api/workspaces/{workspace_id}",
            get(workspace_handlers::get_workspace),
        )
        .route(
            "/api/workspaces/{workspace_id}/members",
            get(workspace_handlers::list_members),
        )
        .route(
            "/api/workspaces/{workspace_id}/projects",
            get(project_handlers::list_projects).post(project_handlers::add_project),
        )
        .route(
            "/api/projects",
            get(project_handlers::list_all_projects_for_user),
        )
        .route(
            "/api/projects/{project_id}",
            get(project_handlers::get_project).delete(project_handlers::delete_project),
        )
        // GitHub App installation routes (#222 Slice 3b). Same proxy
        // shape as Slice 3a above; the `github_app_installations`
        // schema lives in `crates/onsager-portal/migrations/007_github_app_installations.sql`.
        .route(
            "/api/workspaces/{workspace_id}/github-installations",
            get(installation_handlers::list_installations)
                .post(installation_handlers::register_installation),
        )
        .route(
            "/api/workspaces/{workspace_id}/github-installations/{install_row_id}",
            delete(installation_handlers::delete_installation),
        )
        .route(
            "/api/workspaces/{workspace_id}/github-installations/{install_row_id}/accessible-repos",
            get(installation_handlers::list_accessible_repos),
        )
        .route(
            "/api/workspaces/{workspace_id}/github-installations/{install_row_id}/repos/{owner}/{repo}/labels",
            get(installation_handlers::list_repo_labels),
        )
        // GitHub App install-flow + discovery (#222 Slice 3b).
        .route("/api/github-app/config", get(github_app_handlers::config))
        .route(
            "/api/github-app/install-start",
            get(github_app_handlers::install_start),
        )
        .route(
            "/api/github-app/callback",
            get(github_app_handlers::install_callback),
        )
        // Workflow CRUD + GitHub side-effects (#222 Slice 4). Stiglab
        // proxies these URLs through `routes::portal::proxy` so the
        // dashboard's API_BASE cutover (Slice 6) can land independently.
        // Auth (cookie or PAT bearer) is enforced by the `AuthUser`
        // extractor; PAT-pinned principals are 403'd against
        // mismatched workspace IDs by `require_workspace_access`.
        // Workflow rows live on the spine `workflows` /
        // `workflow_stages` tables (Lever D #149); portal is now the
        // only writer.
        .route(
            "/api/workflows",
            get(workflow_handlers::list_workflows).post(workflow_handlers::create_workflow),
        )
        .route(
            "/api/workflows/{id}",
            get(workflow_handlers::get_workflow)
                .patch(workflow_handlers::patch_workflow)
                .delete(workflow_handlers::delete_workflow),
        )
        .route(
            "/api/workflows/{id}/runs",
            get(workflow_handlers::list_workflow_runs),
        )
        // Workflow artifact-kind catalog (issue #102 / #222 Slice 4) —
        // public registry pass-through; the dashboard fetches this
        // without a session to render the workflow-builder's kind picker.
        .route(
            "/api/workflow/kinds",
            get(workflow_kind_handlers::list_workflow_kinds),
        );

    // Dev-login is debug-only — `cargo build --release` strips both the
    // route handler symbol and this registration so production deploys
    // physically cannot serve it regardless of env-var manipulation.
    #[cfg(debug_assertions)]
    let app = app.route("/api/auth/dev-login", post(crate::dev_auth::dev_login));

    let app = app.with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(bind = %config.bind, "onsager-portal listening");
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}
