pub mod auth;
pub mod config;
pub mod db;
pub mod github_app;
pub mod handler;
pub mod routes;
pub mod session_requested_listener;
pub mod shaping_listener;
pub mod spine;
pub mod state;
pub mod workflow_db;
pub mod ws;

pub use sqlx::AnyPool;

use axum::http::{header, HeaderValue};
use axum::routing::get;
use axum::Router;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use config::ServerConfig;
use state::AppState;

/// Build the Axum router. Post-#222 Slice 6, stiglab owns only two routes:
/// `/api/health` (liveness probe) and `/agent/ws` (agent WebSocket).
/// All `/api/*` traffic is routed to portal by the edge proxy (Caddy in
/// dev, nginx in prod) so the dashboard's same-origin fetches land on
/// portal without any per-environment URL configuration.
pub fn build_router(state: AppState, config: &ServerConfig) -> Router {
    let api_routes = Router::new()
        .route("/api/health", get(routes::health::health))
        .route("/agent/ws", get(ws::agent::agent_ws_handler));

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

    let mut app = api_routes
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    // Serve static UI files if configured.
    //
    // Vite emits two classes of output into `static_dir`:
    //   * `/assets/*` — content-hashed JS/CSS/fonts; safe to cache forever.
    //   * `index.html` (plus the SPA fallback) — must revalidate so a new
    //     deploy is picked up on the next navigation without a manual
    //     refresh; an ETag keeps the wire cost minimal when unchanged.
    //
    // Both branches are wrapped in gzip+br compression. The compression
    // layer respects `Accept-Encoding` and skips already-compressed
    // content-types, so PNGs/woff2 aren't double-compressed.
    if let Some(ref static_dir) = config.static_dir {
        tracing::info!("serving static files from {static_dir}");
        let index_file = format!("{static_dir}/index.html");
        let assets_dir = format!("{static_dir}/assets");

        let compression = CompressionLayer::new().gzip(true).br(true);

        // Status-aware: only apply `immutable` to successful responses so a
        // 404 during a bad deploy or partial rollout isn't cached for a
        // year by clients and intermediaries. The closure is generic over
        // the response body type (compression wraps it); returning `None`
        // leaves the header off.
        let assets_service = ServiceBuilder::new()
            .layer(SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                |response: &axum::http::Response<_>| -> Option<HeaderValue> {
                    if response.status().is_success() {
                        Some(HeaderValue::from_static(
                            "public, max-age=31536000, immutable",
                        ))
                    } else {
                        None
                    }
                },
            ))
            .layer(compression.clone())
            .service(ServeDir::new(assets_dir));

        let shell_service = ServiceBuilder::new()
            .layer(SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache"),
            ))
            .layer(compression)
            .service(ServeDir::new(static_dir).fallback(ServeFile::new(index_file)));

        app = app
            .nest_service("/assets", assets_service)
            .fallback_service(shell_service);
    }

    app
}
