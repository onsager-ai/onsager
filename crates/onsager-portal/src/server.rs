//! HTTP server wiring.

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::config::Config;
use crate::gate::GateClient;
use crate::handlers::webhook;
use crate::state::AppState;

/// Boot the webhook server. Blocks until the listener exits.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let pool = crate::db::connect(&config.database_url).await?;
    crate::migrate::run(&pool).await?;
    let spine = onsager_spine::EventStore::connect(&config.database_url).await?;
    // Register the GitHub adapter into the spine catalog. Best-effort:
    // a missing `artifact_adapters` table (older migration set) shouldn't
    // block portal boot. See `onsager_github::adapter::register`.
    if let Err(e) = onsager_github::adapter::register(&pool, "default").await {
        tracing::warn!(error = %e, "github adapter registration skipped");
    }
    let gate = Arc::new(GateClient::new(config.synodic_url.clone()));

    let state = AppState {
        pool,
        spine,
        config: Arc::new(config.clone()),
        gate,
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/webhooks/github", post(webhook::handle))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(bind = %config.bind, "onsager-portal listening");
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}
