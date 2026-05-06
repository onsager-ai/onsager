pub mod auth;
pub mod config;
pub mod db;
pub mod github_app;
pub mod handler;
pub mod proxy_cache;
pub mod routes;
pub mod shaping_listener;
pub mod spine;
pub mod state;
pub mod workflow_activation;
pub mod workflow_db;
pub mod ws;

pub use sqlx::AnyPool;

use axum::http::{header, HeaderValue};
use axum::routing::{any, get, post};
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
        // Auth routes (#222 Slice 5). Stiglab keeps the public `/api/auth/*`
        // URLs but reverse-proxies them to portal so the OAuth dance, SSO
        // delegation, `/api/auth/me`, and `/api/auth/logout` all run in
        // portal. The proxy preserves Set-Cookie and Location so the
        // dashboard / browser round-trip is byte-identical to direct calls.
        // The dashboard's API_BASE cutover lands in Slice 6; until then,
        // these legacy URLs stay live.
        .route("/api/auth/github", any(routes::portal::proxy))
        .route("/api/auth/github/callback", any(routes::portal::proxy))
        .route("/api/auth/me", any(routes::portal::proxy))
        .route("/api/auth/logout", any(routes::portal::proxy))
        .route("/api/auth/sso/redeem", any(routes::portal::proxy))
        .route("/api/auth/sso/finish", any(routes::portal::proxy))
        // Credential routes (#222 Slice 2a). Portal owns
        // `/api/workspaces/:id/credentials*`; stiglab proxies them so
        // dashboard fetches keep working pre–API_BASE cutover. PAT
        // bearer auth + the destructive-credential guardrail
        // (`pat_destructive_blocked`) live on portal now. Stiglab
        // still decrypts these rows in-process at session-launch time
        // (`session_credentials.rs`) — same database, separate
        // connection pool, portal is the only writer.
        .route(
            "/api/workspaces/{workspace_id}/credentials",
            any(routes::portal::proxy),
        )
        .route(
            "/api/workspaces/{workspace_id}/credentials/{name}",
            any(routes::portal::proxy),
        )
        // Personal Access Tokens (issue #143). Spec #222 Slice 2b moved
        // `/api/pats*` to portal; stiglab keeps the URLs as reverse
        // proxies so the dashboard's API_BASE cutover (Slice 6) can land
        // independently. Portal's `AuthUser` extractor honors both cookie
        // and PAT bearer auth, so the proxy preserves full behavior.
        .route("/api/pats", any(routes::portal::proxy))
        .route("/api/pats/{id}", any(routes::portal::proxy))
        // Workspace + member + project CRUD (#222 Slice 3a). Portal owns
        // these routes; stiglab proxies them so dashboard fetches keep
        // working pre–API_BASE cutover (Slice 6). Schema for
        // `workspaces` / `workspace_members` / `projects` lives in the
        // spine migration (`020_workspaces_to_spine.sql`); stiglab
        // still reads the same Postgres tables for the in-process
        // session/task lookups (`db::is_workspace_member`,
        // `db::get_project`, etc.) — same database, separate
        // connection pool, portal is the only writer.
        .route("/api/workspaces", any(routes::portal::proxy))
        .route("/api/workspaces/{id}", any(routes::portal::proxy))
        .route("/api/workspaces/{id}/members", any(routes::portal::proxy))
        .route("/api/workspaces/{id}/projects", any(routes::portal::proxy))
        .route("/api/projects", any(routes::portal::proxy))
        .route("/api/projects/{id}", any(routes::portal::proxy))
        // GitHub App installation + install-flow routes (Slice 3b
        // pending). Stiglab still owns these — they live alongside the
        // workspace routes above and read/write `github_app_installations`
        // which moves to portal in 3b.
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
        // Portal webhook proxy — every GitHub delivery (PR lineage,
        // workflow trigger fire, gate signals) is handled by the
        // onsager-portal binary. The proxy preserves raw body bytes
        // and forwards `X-Hub-Signature-256` / `X-GitHub-Event` so
        // the signature check on the portal side still verifies.
        // `/api/webhooks/github` and `/api/github-app/webhook` are
        // backward-compat aliases — older GitHub Apps configured
        // against the workflow-runtime path or the install-flow path
        // continue to work without touching their webhook URL.
        .route("/webhooks/github", any(routes::portal::proxy))
        .route("/api/webhooks/github", any(routes::portal::proxy))
        .route("/api/github-app/webhook", any(routes::portal::proxy))
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

    // Dev-login (issue #193) lives on portal post-#222 Slice 5; the
    // stiglab-side URL stays a reverse-proxy entry so the dashboard's
    // `LoginPage` button keeps working pre–API_BASE cutover. Debug-only
    // on both ends — release builds of portal don't register the route.
    #[cfg(debug_assertions)]
    let api_routes = api_routes.route("/api/auth/dev-login", any(routes::portal::proxy));

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
