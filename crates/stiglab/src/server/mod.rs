pub mod auth;
pub mod config;
pub mod db;
pub mod github_app;
pub mod handler;
pub mod routes;
pub mod session_cancel_requested_listener;
pub mod session_requested_listener;
pub mod shaping_listener;
pub mod spine;
pub mod state;
pub mod workflow_db;
pub mod ws;

pub use sqlx::AnyPool;

use axum::Router;
use axum::routing::get;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use config::ServerConfig;
use state::AppState;

/// Build the Axum router. Stiglab binds to loopback only (ADR 0006);
/// portal terminates `/agent/ws` from outside and proxies bytes to
/// the loopback `/agent/ws-internal` route below (ADR 0008).
pub fn build_router(state: AppState, config: &ServerConfig) -> Router {
    let api_routes = Router::new().route("/agent/ws-internal", get(ws::agent::agent_ws_handler));

    // Configure CORS
    let cors = if let Some(ref origin) = config.cors_origin {
        tracing::info!("CORS restricted to origin: {origin}");
        CorsLayer::new()
            .allow_origin(
                origin
                    .parse::<axum::http::HeaderValue>()
                    .expect("invalid CORS origin"),
            )
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    } else {
        tracing::warn!("CORS is permissive (set STIGLAB_CORS_ORIGIN to restrict)");
        CorsLayer::permissive()
    };

    api_routes
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
