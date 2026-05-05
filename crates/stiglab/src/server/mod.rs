pub mod auth;
pub mod config;
pub mod db;
#[cfg(debug_assertions)]
pub mod dev_auth;
pub mod github_app;
pub mod handler;
pub mod proxy_cache;
pub mod routes;
pub mod shaping_listener;
pub mod spine;
pub mod sso;
pub mod state;
pub mod webhook_router;
pub mod workflow_activation;
pub mod workflow_db;
pub mod ws;

pub use sqlx::AnyPool;

use axum::http::{header, HeaderValue};
use axum::routing::{any, delete, get, post, put};
use axum::Router;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use config::ServerConfig;
use state::AppState;

/// Build the Axum router with all API routes, CORS, and optional static file serving.
pub fn build_router(state: AppState, config: &ServerConfig) -> Router {
    let api_routes = Router::new()
        .route("/api/health", get(routes::health::health))
        .route("/api/nodes", get(routes::nodes::list_nodes))
        .route("/api/tasks", post(routes::tasks::create_task))
        // The legacy `POST /api/shaping` (forge → stiglab dispatch)
        // and the `GET /api/shaping/{session_id}` long-poll status
        // endpoint are both gone. Forge dispatch flows through the
        // spine via `forge.shaping_dispatched` (consumed by
        // `shaping_listener`); the dashboard reads session state via
        // `GET /api/sessions/{id}` and the spine event feed.
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
        // Cross-environment SSO delegation.
        //
        // * `/api/auth/sso/redeem` (POST, owner only): server-to-server
        //   redemption of an opaque exchange code. Requires
        //   `Authorization: Bearer $SSO_EXCHANGE_SECRET`. 404s when this
        //   process is not an owner with delegation enabled.
        // * `/api/auth/sso/finish` (GET, relying only): browser lands here
        //   after the owner completes the OAuth dance; we redeem the code
        //   and mint a local session. 404s when this process owns the
        //   OAuth app directly.
        .route("/api/auth/sso/redeem", post(routes::auth::sso_redeem))
        .route("/api/auth/sso/finish", get(routes::auth::sso_finish))
        // Credential routes — per-workspace post-#164.  Each workspace
        // carries its own secret store; sessions launched in W1 will
        // never reach for a token registered in W2.
        .route(
            "/api/workspaces/{workspace_id}/credentials",
            get(routes::credentials::list_credentials),
        )
        .route(
            "/api/workspaces/{workspace_id}/credentials/{name}",
            put(routes::credentials::set_credential).delete(routes::credentials::delete_credential),
        )
        // Personal Access Tokens (issue #143)
        .route(
            "/api/pats",
            get(routes::pats::list_pats).post(routes::pats::create_pat),
        )
        .route("/api/pats/{id}", delete(routes::pats::delete_pat))
        // Workspace routes (issue #59 — Phase 0; renamed from "tenant" →
        // "workspace" in issue #163).
        .route(
            "/api/workspaces",
            get(routes::workspaces::list_workspaces).post(routes::workspaces::create_workspace),
        )
        .route(
            "/api/workspaces/{id}",
            get(routes::workspaces::get_workspace),
        )
        .route(
            "/api/workspaces/{id}/members",
            get(routes::workspaces::list_members),
        )
        .route(
            "/api/workspaces/{id}/github-installations",
            get(routes::workspaces::list_installations)
                .post(routes::workspaces::register_installation),
        )
        .route(
            "/api/workspaces/{id}/github-installations/{install_id}",
            axum::routing::delete(routes::workspaces::delete_installation),
        )
        .route(
            "/api/workspaces/{id}/github-installations/{install_id}/accessible-repos",
            get(routes::workspaces::list_accessible_repos),
        )
        .route(
            "/api/workspaces/{id}/github-installations/{install_id}/repos/{owner}/{repo}/labels",
            get(routes::workspaces::list_repo_labels),
        )
        .route(
            "/api/github-app/config",
            get(routes::workspaces::github_app_config),
        )
        .route(
            "/api/github-app/install-start",
            get(routes::workspaces::github_app_install_start),
        )
        .route(
            "/api/github-app/callback",
            get(routes::workspaces::github_app_install_callback),
        )
        .route(
            "/api/workspaces/{id}/projects",
            get(routes::workspaces::list_projects).post(routes::workspaces::add_project),
        )
        .route(
            "/api/projects",
            get(routes::workspaces::list_all_projects_for_user),
        )
        .route(
            "/api/projects/{id}",
            get(routes::workspaces::get_project).delete(routes::workspaces::delete_project),
        )
        // Live-data hydration for reference-only artifacts (#170 / #167 / #171).
        // The dashboard joins skeleton rows from `/api/spine/artifacts?kind=...`
        // with the hydrated payloads here on `external_ref`.
        .route(
            "/api/projects/{id}/issues",
            get(routes::projects::list_project_issues),
        )
        .route(
            "/api/projects/{id}/issues/{number}",
            get(routes::projects::get_project_issue),
        )
        .route(
            "/api/projects/{id}/pulls",
            get(routes::projects::list_project_pulls),
        )
        .route(
            "/api/projects/{id}/backfill",
            post(routes::projects::backfill_project),
        )
        // Manual replay of `workflow.trigger_fired` for a single issue —
        // active counterpart to the passive `issues.labeled` webhook
        // path (#203). Useful when debugging an end-to-end workflow run
        // that didn't fire.
        .route(
            "/api/projects/{id}/issues/{number}/replay-trigger",
            post(routes::projects::replay_issue_trigger),
        )
        // Governance proxy — forwards to synodic on internal port
        .route("/api/governance/{*path}", any(routes::governance::proxy))
        // Portal webhook proxy — forwards `/webhooks/github` to the
        // onsager-portal binary running on an internal port so the
        // Railway service exposes a single external origin.
        .route("/webhooks/github", any(routes::portal::proxy))
        // Workflow runtime webhook receiver (issue #81). Distinct from the
        // legacy `/webhooks/github` portal proxy — this endpoint feeds the
        // workflow runtime on stiglab directly. GitHub caps webhook payloads
        // at 25 MiB; a 1 MiB cap here is generous for the events we handle
        // (`issues`, `pull_request`, `check_*`, `status`) while blunting
        // DoS-via-giant-body.
        .route(
            "/api/webhooks/github",
            post(routes::webhooks::handle).layer(axum::extract::DefaultBodyLimit::max(1024 * 1024)),
        )
        // Forgiving alias for the webhook receiver. `/api/github-app/*`
        // hosts the GET-only OAuth/install flow and had no POST handler
        // here, so a GitHub App configured to POST its webhook to
        // `/api/github-app/webhook` (a plausible-looking but wrong URL)
        // had every delivery silently dropped. Accept it so a
        // misconfigured App heals itself.
        .route(
            "/api/github-app/webhook",
            post(routes::webhooks::handle).layer(axum::extract::DefaultBodyLimit::max(1024 * 1024)),
        )
        // Workflow CRUD (issue #81).
        .route(
            "/api/workflows",
            get(routes::workflows::list_workflows).post(routes::workflows::create_workflow),
        )
        .route(
            "/api/workflows/{id}",
            get(routes::workflows::get_workflow)
                .patch(routes::workflows::patch_workflow)
                .delete(routes::workflows::delete_workflow),
        )
        // Live runs (artifacts flowing through this workflow).
        .route(
            "/api/workflows/{id}/runs",
            get(routes::workflows::list_workflow_runs),
        )
        // Workflow artifact-kind catalog (issue #102). Runtime surface of
        // the registry's `workflow_builtin_types()` — lets the dashboard
        // render the kind picker without hardcoding the union.
        .route(
            "/api/workflow/kinds",
            get(routes::workflow_kinds::list_workflow_kinds),
        )
        // Event-type registry manifest (spec #131 Lever E / #150).
        // Static, human-reviewed manifest of every FactoryEventKind
        // variant — which subsystems produce it and which consume it.
        .route(
            "/api/registry/events",
            get(routes::registry_events::list_events),
        )
        // Trigger-kind registry manifest (spec #237 / parent #236).
        // Static, human-reviewed manifest of every TriggerKind variant —
        // its producer subsystem, category, and UI form shape. Read by
        // the dashboard's `<TriggerKindPicker>`.
        .route(
            "/api/registry/triggers",
            get(routes::registry_triggers::list_triggers),
        )
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

    // Dev-login (issue #193). The route is only registered in debug
    // builds — `cargo build --release` strips the symbol, so a release
    // deploy physically cannot serve `/api/auth/dev-login` regardless of
    // env-var configuration.
    #[cfg(debug_assertions)]
    let api_routes = api_routes.route(
        "/api/auth/dev-login",
        post(crate::server::dev_auth::dev_login),
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
