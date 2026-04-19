//! HTTP server for the Synodic governance dashboard.
//!
//! Serves the React dashboard as static files and exposes a JSON API
//! for governance events, rules, and health checks.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{FromRef, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::core::engine_cache::EngineCache;
use crate::core::gate_adapter;
use crate::core::storage::pool::{create_storage, resolve_database_url};
use crate::core::storage::{
    CreateGovernanceEvent, GovernanceEvent, GovernanceEventFilters, RuleProposal, Storage,
};

/// Run the Synodic web server (dashboard + API)
#[derive(Parser)]
pub struct ServeCmd {
    /// Port to listen on (defaults to $PORT or 3000)
    #[arg(long, env = "PORT", default_value = "3000")]
    port: u16,

    /// Directory containing the built dashboard files
    #[arg(long, env = "SYNODIC_DASHBOARD_DIR")]
    dashboard_dir: Option<String>,
}

/// HTTP application state.
///
/// Bundles the durable [`Storage`] handle with an in-process
/// [`EngineCache`] so the `/gate` handler avoids rebuilding the full
/// `InterceptEngine` on every call (issue #32). Cloning `AppState` is
/// cheap — it's two `Arc` clones.
///
/// `FromRef` impls let handlers extract just the slice they need —
/// existing handlers continue to take `State<Arc<dyn Storage>>` while
/// `gate_handler` takes both.
#[derive(Clone)]
struct AppState {
    storage: Arc<dyn Storage>,
    engine_cache: Arc<EngineCache>,
}

impl FromRef<AppState> for Arc<dyn Storage> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.storage)
    }
}

impl FromRef<AppState> for Arc<EngineCache> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.engine_cache)
    }
}

impl ServeCmd {
    pub async fn run(self) -> Result<()> {
        let db_url = resolve_database_url();
        eprintln!("Connecting to database...");
        let storage = create_storage(&db_url).await?;
        let state = AppState {
            storage: Arc::from(storage),
            engine_cache: Arc::new(EngineCache::new()),
        };

        let api = Router::new()
            .route("/health", get(health))
            .route("/events", get(list_events).post(create_event))
            .route("/events/{id}", get(get_event))
            .route("/events/{id}/resolve", patch(resolve_event))
            .route("/stats", get(get_stats))
            .route("/rules", get(list_rules))
            .route("/gate", post(gate_handler))
            // Rule proposal queue (issue #36 Step 2)
            .route("/rule-proposals", get(list_rule_proposals))
            .route("/rule-proposals/{id}/resolve", patch(resolve_rule_proposal))
            // Escalation resolution proposals (issue #37)
            .route(
                "/escalations/{id}/propose-resolution",
                post(propose_escalation_resolution),
            );

        // Spawn the ising.rule_proposed listener so Synodic consumes
        // proposals off the spine in real time (issue #36 Step 2). A spine
        // connection failure here is non-fatal — the HTTP API stays up and
        // operators can retry via backfill on the next restart.
        if let Ok(spine_url) = std::env::var("DATABASE_URL") {
            let listener_storage = Arc::clone(&state.storage);
            tokio::spawn(async move {
                match onsager_spine::EventStore::connect(&spine_url).await {
                    Ok(spine) => {
                        if let Err(e) =
                            crate::core::proposal_listener::run(spine, listener_storage, None).await
                        {
                            tracing::error!("synodic: rule_proposed listener exited: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "synodic: rule_proposed listener disabled (spine connect failed: {e})"
                        );
                    }
                }
            });
        } else {
            tracing::info!("synodic: DATABASE_URL not set; rule_proposed listener disabled");
        }

        let mut app = Router::new().nest("/api", api).with_state(state);

        // Serve dashboard static files if directory is configured and exists.
        // Use index.html as the SPA fallback so client-side routing works.
        if let Some(ref dir) = self.dashboard_dir {
            let path = std::path::Path::new(dir);
            if path.is_dir() {
                eprintln!("Serving dashboard from {dir}");
                let index = path.join("index.html");
                let spa = ServeDir::new(dir).not_found_service(ServeFile::new(index));
                app = app.fallback_service(spa);
            } else {
                eprintln!("Dashboard directory {dir} not found, skipping static files");
            }
        }

        let addr = SocketAddr::from(([0, 0, 0, 0], self.port));
        eprintln!("Synodic server listening on http://{addr}");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

const VALID_SEVERITIES: &[&str] = &["critical", "high", "medium", "low"];

// ---------------------------------------------------------------------------
// API handlers
// ---------------------------------------------------------------------------

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn list_events(
    State(store): State<Arc<dyn Storage>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<GovernanceEvent>>, AppError> {
    let filters = GovernanceEventFilters {
        event_type: params.get("type").cloned(),
    };
    let events = store.get_governance_events(filters).await?;
    Ok(Json(events))
}

async fn get_event(
    State(store): State<Arc<dyn Storage>>,
    Path(id): Path<String>,
) -> Result<Json<GovernanceEvent>, AppError> {
    let event = store
        .get_governance_event(&id)
        .await?
        .ok_or_else(|| AppError::NotFound("event not found".into()))?;
    Ok(Json(event))
}

async fn create_event(
    State(store): State<Arc<dyn Storage>>,
    Json(body): Json<CreateGovernanceEvent>,
) -> Result<(StatusCode, Json<GovernanceEvent>), AppError> {
    if let Some(ref sev) = body.severity {
        if !VALID_SEVERITIES.contains(&sev.as_str()) {
            return Err(AppError::BadRequest(format!(
                "invalid severity '{sev}', must be one of: {}",
                VALID_SEVERITIES.join(", ")
            )));
        }
    }
    let event = store.create_governance_event(body).await?;
    Ok((StatusCode::CREATED, Json(event)))
}

#[derive(Deserialize)]
struct ResolveBody {
    notes: Option<String>,
}

async fn resolve_event(
    State(store): State<Arc<dyn Storage>>,
    Path(id): Path<String>,
    Json(body): Json<ResolveBody>,
) -> Result<StatusCode, AppError> {
    store
        .get_governance_event(&id)
        .await?
        .ok_or_else(|| AppError::NotFound("event not found".into()))?;
    store.resolve_governance_event(&id, body.notes).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct Stats {
    total: usize,
    unresolved: usize,
    by_type: HashMap<String, usize>,
    by_severity: HashMap<String, usize>,
}

async fn get_stats(State(store): State<Arc<dyn Storage>>) -> Result<Json<Stats>, AppError> {
    let events = store
        .get_governance_events(GovernanceEventFilters::default())
        .await?;

    let total = events.len();
    let unresolved = events.iter().filter(|e| !e.resolved).count();

    let mut by_type: HashMap<String, usize> = HashMap::new();
    let mut by_severity: HashMap<String, usize> = HashMap::new();

    for e in &events {
        *by_type.entry(e.event_type.clone()).or_default() += 1;
        *by_severity.entry(e.severity.clone()).or_default() += 1;
    }

    Ok(Json(Stats {
        total,
        unresolved,
        by_type,
        by_severity,
    }))
}

#[derive(Serialize)]
struct ApiRule {
    name: String,
    description: String,
    pattern: String,
    event_type: String,
    severity: String,
    category_id: String,
    enabled: bool,
}

async fn list_rules(State(store): State<Arc<dyn Storage>>) -> Result<Json<Vec<ApiRule>>, AppError> {
    let rules = store.get_rules(false).await?;
    let categories = store.get_threat_categories().await?;

    // Build a lookup from category_id → severity
    let severity_map: HashMap<String, String> =
        categories.into_iter().map(|c| (c.id, c.severity)).collect();

    let api_rules: Vec<ApiRule> = rules
        .into_iter()
        .map(|r| {
            let severity = severity_map
                .get(&r.category_id)
                .cloned()
                .unwrap_or_else(|| "medium".to_string());
            ApiRule {
                name: r.id,
                description: r.description,
                pattern: r.condition_value,
                event_type: r.condition_type,
                severity,
                category_id: r.category_id,
                enabled: r.enabled,
            }
        })
        .collect();

    Ok(Json(api_rules))
}

// ---------------------------------------------------------------------------
// Rule proposal queue (issue #36 Step 2)
// ---------------------------------------------------------------------------

async fn list_rule_proposals(
    State(store): State<Arc<dyn Storage>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<RuleProposal>>, AppError> {
    let status = params.get("status").map(String::as_str);
    let proposals = store.list_rule_proposals(status).await?;
    Ok(Json(proposals))
}

#[derive(Deserialize)]
struct ResolveProposalBody {
    status: String,
    notes: Option<String>,
}

async fn resolve_rule_proposal(
    State(store): State<Arc<dyn Storage>>,
    Path(id): Path<String>,
    Json(body): Json<ResolveProposalBody>,
) -> Result<StatusCode, AppError> {
    if body.status != "approved" && body.status != "rejected" {
        return Err(AppError::BadRequest(format!(
            "status must be approved or rejected, got {}",
            body.status
        )));
    }
    store
        .resolve_rule_proposal(&id, &body.status, body.notes)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Escalation resolution proposals (issue #37)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ProposeResolutionBody {
    artifact_id: String,
    /// Identity of the proposer: `"supervisor"`, `"human:<id>"`, or any
    /// free-form string representing the delegate.
    proposer: String,
    /// One of `"allow" | "deny" | "modify" | "escalate"`.
    proposed_verdict: String,
    rationale: String,
}

/// POST `/api/escalations/{id}/propose-resolution` — record a delegate's
/// proposed resolution on the spine as `synodic.gate_resolution_proposed`.
/// The proposal is not applied; Forge/Synodic wiring converts accepted
/// proposals into a final verdict on a separate path.
async fn propose_escalation_resolution(
    Path(escalation_id): Path<String>,
    Json(body): Json<ProposeResolutionBody>,
) -> Result<StatusCode, AppError> {
    let verdict = match body.proposed_verdict.to_ascii_lowercase().as_str() {
        "allow" => onsager_spine::factory_event::VerdictSummary::Allow,
        "deny" => onsager_spine::factory_event::VerdictSummary::Deny,
        "modify" => onsager_spine::factory_event::VerdictSummary::Modify,
        "escalate" => onsager_spine::factory_event::VerdictSummary::Escalate,
        other => {
            return Err(AppError::BadRequest(format!(
                "proposed_verdict must be allow|deny|modify|escalate, got {other}"
            )));
        }
    };

    let Ok(spine_url) = std::env::var("DATABASE_URL") else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "DATABASE_URL not set; can't emit spine event"
        )));
    };
    let spine = onsager_spine::EventStore::connect(&spine_url)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("spine connect: {e}")))?;

    let event = onsager_spine::factory_event::FactoryEventKind::SynodicGateResolutionProposed {
        escalation_id: escalation_id.clone(),
        artifact_id: onsager_artifact::ArtifactId::new(&body.artifact_id),
        proposer: body.proposer,
        proposed_verdict: verdict,
        rationale: body.rationale,
    };
    let data = serde_json::to_value(&event).expect("FactoryEventKind must serialize");
    let metadata = onsager_spine::EventMetadata {
        actor: "synodic".into(),
        ..Default::default()
    };
    spine
        .append_ext(
            &escalation_id,
            "synodic",
            event.event_type(),
            data,
            &metadata,
            None,
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("append_ext: {e}")))?;

    Ok(StatusCode::ACCEPTED)
}

// ---------------------------------------------------------------------------
// Gate handler (Onsager protocol)
// ---------------------------------------------------------------------------

async fn gate_handler(
    State(store): State<Arc<dyn Storage>>,
    State(engine_cache): State<Arc<EngineCache>>,
    Json(req): Json<onsager_protocol::GateRequest>,
) -> Result<Json<onsager_protocol::GateVerdict>, AppError> {
    // Cached: if no rule has been added/updated/deleted since the last call,
    // we reuse the compiled `InterceptEngine` (issue #32). The only DB cost
    // on a hit is one cheap `(COUNT, MAX(updated_at))` aggregate.
    let engine = engine_cache.get_or_refresh(&*store).await?;

    let intercept_req = gate_adapter::gate_request_to_intercept(&req);
    let resp = engine.evaluate(&intercept_req);
    let verdict = gate_adapter::intercept_to_gate_verdict(&resp);

    Ok(Json(verdict))
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

enum AppError {
    Internal(anyhow::Error),
    NotFound(String),
    BadRequest(String),
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Internal(err) => {
                eprintln!("Internal error: {err:#}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": "internal server error" })),
                )
                    .into_response()
            }
            Self::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
            Self::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
        }
    }
}
