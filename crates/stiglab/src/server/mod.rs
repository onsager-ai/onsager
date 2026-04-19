pub mod auth;
pub mod config;
pub mod db;
pub mod github_app;
pub mod handler;
pub mod routes;
pub mod spine;
pub mod state;
pub mod ws;

pub use sqlx::AnyPool;

use axum::routing::{any, get, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use config::ServerConfig;
use state::AppState;

/// Build the Axum router with all API routes, CORS, and optional static file serving.
pub fn build_router(state: AppState, config: &ServerConfig) -> Router {
    let api_routes = Router::new()
        .route("/api/health", get(routes::health::health))
        .route("/api/nodes", get(routes::nodes::list_nodes))
        .route("/api/tasks", post(routes::tasks::create_task))
        .route("/api/shaping", post(routes::shaping::create_shaping))
        .route(
            "/api/shaping/{session_id}",
            get(routes::shaping::get_shaping_status),
        )
        .route("/api/sessions", get(routes::sessions::list_sessions))
        .route("/api/sessions/{id}", get(routes::sessions::get_session))
        .route(
            "/api/sessions/{id}/logs",
            get(routes::sessions::session_logs),
        )
        .route("/agent/ws", get(ws::agent::agent_ws_handler))
        // Auth routes
        .route("/api/auth/github", get(routes::auth::github_login))
        .route(
            "/api/auth/github/callback",
            get(routes::auth::github_callback),
        )
        .route("/api/auth/me", get(routes::auth::me))
        .route("/api/auth/logout", post(routes::auth::logout))
        // Credential routes
        .route(
            "/api/credentials",
            get(routes::credentials::list_credentials),
        )
        .route(
            "/api/credentials/{name}",
            put(routes::credentials::set_credential).delete(routes::credentials::delete_credential),
        )
        // Tenant / workspace routes (issue #59 — Phase 0)
        .route(
            "/api/tenants",
            get(routes::tenants::list_tenants).post(routes::tenants::create_tenant),
        )
        .route("/api/tenants/{id}", get(routes::tenants::get_tenant))
        .route(
            "/api/tenants/{id}/members",
            get(routes::tenants::list_members),
        )
        .route(
            "/api/tenants/{id}/github-installations",
            get(routes::tenants::list_installations).post(routes::tenants::register_installation),
        )
        .route(
            "/api/tenants/{id}/github-installations/{install_id}",
            axum::routing::delete(routes::tenants::delete_installation),
        )
        .route(
            "/api/tenants/{id}/github-installations/{install_id}/accessible-repos",
            get(routes::tenants::list_accessible_repos),
        )
        .route(
            "/api/github-app/config",
            get(routes::tenants::github_app_config),
        )
        .route(
            "/api/github-app/install-start",
            get(routes::tenants::github_app_install_start),
        )
        .route(
            "/api/github-app/install-callback",
            get(routes::tenants::github_app_install_callback),
        )
        .route(
            "/api/tenants/{id}/projects",
            get(routes::tenants::list_projects).post(routes::tenants::add_project),
        )
        .route(
            "/api/projects",
            get(routes::tenants::list_all_projects_for_user),
        )
        .route(
            "/api/projects/{id}",
            get(routes::tenants::get_project).delete(routes::tenants::delete_project),
        )
        // Governance proxy — forwards to synodic on internal port
        .route("/api/governance/{*path}", any(routes::governance::proxy))
        // Spine API — exposes shared event spine data to the dashboard
        .route("/api/spine/events", get(routes::spine::list_events))
        .route(
            "/api/spine/artifacts",
            get(routes::spine::list_artifacts).post(routes::spine::register_artifact),
        )
        .route(
            "/api/spine/artifacts/{id}",
            get(routes::spine::get_artifact),
        )
        .route(
            "/api/spine/artifacts/{id}/retry",
            post(routes::spine::retry_artifact),
        )
        .route(
            "/api/spine/artifacts/{id}/abort",
            post(routes::spine::abort_artifact),
        )
        .route(
            "/api/spine/artifacts/{id}/override-gate",
            post(routes::spine::override_gate),
        );

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

    // Serve static UI files if configured
    if let Some(ref static_dir) = config.static_dir {
        tracing::info!("serving static files from {static_dir}");
        let index_file = format!("{static_dir}/index.html");
        app = app.fallback_service(ServeDir::new(static_dir).fallback(ServeFile::new(index_file)));
    }

    app
}
