//! Onsager v2 — one process, events as record (ADR 0029).
//!
//! Module map:
//! - [`config`] — env-only configuration
//! - [`db`] — pool + boot-time migration runner (`schema_migrations`)
//! - [`crypto`] — AES-256-GCM credential sealing + token generation
//!   (ported from `legacy/crates/onsager-portal/src/auth.rs`)
//! - [`auth`] — users / cookie sessions / dev-login / `AuthUser`
//!   extractor (ported from legacy portal, PAT + SSO paths dropped)
//! - [`http`] — the router and route handlers

pub mod agent_events;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod db;
pub mod domain;
pub mod events;
pub mod fleet;
pub mod github;
pub mod http;
pub mod mcp;
pub mod scheduler;
pub mod sessions;
pub mod state;

use anyhow::Context;

/// Boot the server: connect, migrate, seed dev auth (when enabled),
/// serve until shutdown.
pub async fn serve(config: config::Config) -> anyhow::Result<()> {
    let pool = db::connect(&config.database_url).await?;
    db::migrate(&pool).await.context("migrations failed")?;

    // Crash recovery (ADR 0030 D4): fail any runs a dead predecessor left
    // marked running, at-most-once.
    match scheduler::reclaim_orphaned_runs(&pool).await {
        Ok(n) if n > 0 => tracing::warn!("reclaimed {n} orphaned run(s) from a prior process"),
        Ok(_) => {}
        Err(e) => tracing::error!("orphaned-run reclaim failed: {e:#}"),
    }

    if config.dev_login_enabled() {
        auth::seed_dev_user_and_workspace(&pool)
            .await
            .context("dev-login seed failed")?;
    }

    let bind = config.bind.clone();
    let state = state::AppState::new(pool, config);
    // Lease reaper (ADR 0030 D4): expire silent machines' sessions.
    tokio::spawn(fleet::reap_stale_machines(state.clone()));
    let app = http::router(state);

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("bind {bind}"))?;
    tracing::info!(%bind, "onsager listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Boot in worker mode (ADR 0030): no database, no HTTP edge — dial the
/// control plane and run assigned agent sessions. Env-configured via
/// [`fleet::WorkerConfig`].
pub async fn serve_worker() -> anyhow::Result<()> {
    let config = fleet::WorkerConfig::from_env()?;
    fleet::run_worker(config).await
}
