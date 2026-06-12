//! The router. One process, one external surface: every route the
//! product has lives here.

use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};

use crate::auth;
use crate::state::AppState;

mod credentials;
mod me;
mod workflows;
mod workspaces;

pub fn router(state: AppState) -> Router {
    let mut app = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/providers", get(me::auth_providers))
        .route("/api/auth/me", get(me::get_me))
        .route("/api/auth/logout", post(me::logout))
        .route("/api/workspaces", get(workspaces::list_workspaces))
        .route(
            "/api/workspaces/{workspace_id}/credentials",
            get(credentials::list_credentials),
        )
        .route(
            "/api/workspaces/{workspace_id}/credentials/{name}",
            put(credentials::set_credential).delete(credentials::delete_credential),
        )
        .route(
            "/api/workflows",
            get(workflows::list_workflows).post(workflows::create_workflow),
        )
        .route(
            "/api/workflows/{id}",
            get(workflows::get_workflow).patch(workflows::patch_workflow),
        )
        .route("/api/workflows/{id}/fire", post(workflows::fire_workflow))
        .route("/api/workflows/{id}/runs", get(workflows::list_runs))
        .route("/api/runs/{id}", get(workflows::get_run))
        .route("/api/artifacts", get(workflows::list_artifacts))
        .route("/api/artifacts/{id}", get(workflows::get_artifact))
        // Agent-facing MCP output channel (session-token auth).
        .route("/mcp/messages", post(crate::mcp::messages));

    if state.config.dev_login_enabled() {
        app = app.route("/api/auth/dev-login", post(auth::dev_login));
    }

    // Production serves the built dashboard from the same process —
    // one container, no edge dispatcher (ADR 0029). `ONSAGER_STATIC_DIR`
    // points at the Vite dist; unknown paths fall back to index.html
    // (SPA routing). Dev leaves it unset and uses Vite directly.
    if let Ok(dir) = std::env::var("ONSAGER_STATIC_DIR")
        && !dir.is_empty()
    {
        let index = std::path::Path::new(&dir).join("index.html");
        app = app.fallback_service(
            tower_http::services::ServeDir::new(&dir)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
    }

    app.with_state(state)
        .layer(tower_http::trace::TraceLayer::new_for_http())
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}
