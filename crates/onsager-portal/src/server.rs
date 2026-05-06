//! HTTP server wiring.

use std::sync::Arc;

use axum::routing::{delete, get, post};
use axum::Router;

use crate::config::Config;
use crate::gate::GateClient;
use crate::handlers::{auth as auth_handlers, pats as pat_handlers, webhook};
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
        .route("/api/pats/{id}", delete(pat_handlers::delete_pat));

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
