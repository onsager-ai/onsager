//! `forge serve` — start the Forge scheduling loop with HTTP API.

use std::sync::Arc;

use async_trait::async_trait;
use axum::response::IntoResponse;
use tokio::sync::RwLock;

use onsager_spine::artifact::{ArtifactState, Kind};
use onsager_spine::factory_event::ShapingOutcome;
use onsager_spine::protocol::ShapingResult;
use onsager_spine::EventStore;

use crate::core::artifact_store::ArtifactStore;
use crate::core::kernel::BaselineKernel;
use crate::core::pipeline::{ForgePipeline, PipelineEvent, StiglabDispatcher, SynodicGate};
use crate::core::session_listener::{self, SessionCompleted, SessionCompletedHandler};

use onsager_spine::protocol::{GateRequest, GateVerdict, ShapingRequest};

/// HTTP Stiglab dispatcher — calls Stiglab's shaping endpoint.
struct HttpStiglabDispatcher {
    client: reqwest::Client,
    stiglab_url: String,
}

impl StiglabDispatcher for HttpStiglabDispatcher {
    fn dispatch(&self, request: &ShapingRequest) -> ShapingResult {
        // The pipeline tick is synchronous, so use block_in_place to allow
        // blocking on async HTTP calls without panicking inside the Tokio runtime.
        let url = format!("{}/api/shaping", self.stiglab_url);
        let body = serde_json::to_value(request).unwrap_or_default();
        let client = self.client.clone();

        match tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                client
                    .post(&url)
                    .json(&body)
                    .send()
                    .await?
                    .json::<ShapingResult>()
                    .await
            })
        }) {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("stiglab dispatch failed: {e}");
                ShapingResult {
                    request_id: request.request_id.clone(),
                    outcome: ShapingOutcome::Failed,
                    content_ref: None,
                    change_summary: String::new(),
                    quality_signals: vec![],
                    session_id: String::new(),
                    duration_ms: 0,
                    error: Some(onsager_spine::protocol::ErrorDetail {
                        code: "dispatch_error".into(),
                        message: e.to_string(),
                        retriable: Some(true),
                    }),
                }
            }
        }
    }
}

/// HTTP Synodic gate — calls Synodic's gate endpoint.
struct HttpSynodicGate {
    client: reqwest::Client,
    synodic_url: String,
}

impl SynodicGate for HttpSynodicGate {
    fn evaluate(&self, request: &GateRequest) -> GateVerdict {
        let url = format!("{}/api/gate", self.synodic_url);
        let body = serde_json::to_value(request).unwrap_or_default();
        let client = self.client.clone();

        tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Handle::current();
            match rt.block_on(async { client.post(&url).json(&body).send().await }) {
                Ok(resp) => {
                    if resp.status().is_success() {
                        match rt.block_on(resp.json::<GateVerdict>()) {
                            Ok(verdict) => verdict,
                            Err(e) => {
                                tracing::warn!(
                                    "synodic gate response parse error: {e}, defaulting to Allow"
                                );
                                GateVerdict::Allow
                            }
                        }
                    } else {
                        tracing::warn!(
                            "synodic gate returned {}, defaulting to Allow",
                            resp.status()
                        );
                        GateVerdict::Allow
                    }
                }
                Err(e) => {
                    tracing::warn!("synodic gate unavailable: {e}, defaulting to Allow");
                    GateVerdict::Allow
                }
            }
        })
    }
}

/// Shared Forge state accessible from both the HTTP API and the tick loop.
struct ForgeSharedState {
    pipeline: ForgePipeline<HttpStiglabDispatcher, HttpSynodicGate>,
    kernel: BaselineKernel,
    spine: Option<EventStore>,
}

type SharedForge = Arc<RwLock<ForgeSharedState>>;

/// Start the Forge scheduling loop with an HTTP API.
pub fn run(database_url: &str, tick_ms: u64) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async move {
        tracing_subscriber::fmt()
            .with_env_filter("forge=info")
            .init();

        tracing::info!(tick_ms, "forge: starting");

        // Connect to event spine.
        let spine = match EventStore::connect(database_url).await {
            Ok(s) => {
                tracing::info!("forge: connected to event spine");
                Some(s)
            }
            Err(e) => {
                tracing::warn!("forge: spine connection failed ({e}), running without persistence");
                None
            }
        };

        // Load existing artifacts from the spine database.
        let mut artifact_store = ArtifactStore::new();
        if let Some(ref spine) = spine {
            if let Ok(rows) = sqlx::query_as::<_, (String, String, String, String, String, i32)>(
                "SELECT artifact_id, kind, name, owner, state, current_version \
                 FROM artifacts WHERE state != 'archived'",
            )
            .fetch_all(spine.pool())
            .await
            {
                for (id, kind, name, owner, state_str, version) in &rows {
                    let kind_enum = match kind.as_str() {
                        "code" => Kind::Code,
                        "document" => Kind::Document,
                        "pull_request" => Kind::PullRequest,
                        other => Kind::Custom(other.to_string()),
                    };
                    let state = match state_str.as_str() {
                        "in_progress" => ArtifactState::InProgress,
                        "under_review" => ArtifactState::UnderReview,
                        "released" => ArtifactState::Released,
                        "archived" => ArtifactState::Archived,
                        _ => ArtifactState::Draft,
                    };
                    artifact_store.register_with_id(
                        id.clone(),
                        kind_enum,
                        name.clone(),
                        owner.clone(),
                        state,
                        *version as u32,
                    );
                }
                tracing::info!("forge: loaded {} active artifacts from spine", rows.len());
            }
        }

        let stiglab_port = std::env::var("STIGLAB_PORT").unwrap_or_else(|_| "3000".to_string());
        let stiglab_url = std::env::var("STIGLAB_URL")
            .unwrap_or_else(|_| format!("http://localhost:{stiglab_port}"));
        let synodic_port = std::env::var("SYNODIC_PORT").unwrap_or_else(|_| "3001".to_string());
        let synodic_url = std::env::var("SYNODIC_URL")
            .unwrap_or_else(|_| format!("http://localhost:{synodic_port}"));

        let client = reqwest::Client::new();

        let stiglab = HttpStiglabDispatcher {
            client: client.clone(),
            stiglab_url: stiglab_url.clone(),
        };
        let synodic = HttpSynodicGate {
            client,
            synodic_url: synodic_url.clone(),
        };

        let mut pipeline = ForgePipeline::new(stiglab, synodic);
        pipeline.store = artifact_store;

        let shared = Arc::new(RwLock::new(ForgeSharedState {
            pipeline,
            kernel: BaselineKernel::new(),
            spine,
        }));

        // Start HTTP API.
        let forge_port: u16 = std::env::var("FORGE_PORT")
            .unwrap_or_else(|_| "3002".to_string())
            .parse()
            .unwrap_or(3002);

        let app = build_api(shared.clone());
        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{forge_port}"))
            .await
            .expect("failed to bind forge port");
        tracing::info!("forge: HTTP API listening on :{forge_port}");

        // Spawn the tick loop.
        let tick_shared = shared.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(tick_ms));
            loop {
                interval.tick().await;

                // Run the pipeline tick under the write lock, then release it
                // before emitting spine events so HTTP reads aren't starved.
                let (output, spine) = {
                    let mut state = tick_shared.write().await;
                    let kernel = state.kernel.clone();
                    let output = state.pipeline.tick(&kernel);
                    let spine = state.spine.clone();
                    (output, spine)
                };

                // Emit pipeline events to the spine (lock released).
                for event in &output.events {
                    if let Some(ref spine) = spine {
                        emit_pipeline_event(spine, event).await;
                    }
                    match event {
                        PipelineEvent::IdleTick => {}
                        _ => tracing::info!("forge tick: {event:?}"),
                    }
                }
            }
        });

        // Spawn the stiglab.session_completed listener (issue #14 phase 2).
        //
        // This is the event-driven counterpart to the synchronous HTTP
        // dispatcher in the tick loop. When Stiglab emits a completion
        // event carrying an artifact_id, we record the linkage in the spine
        // so that the dashboard can render the per-run lineage without
        // rescanning the pipeline's in-memory state.
        let listener_shared = shared.clone();
        tokio::spawn(async move {
            let store = {
                let state = listener_shared.read().await;
                state.spine.clone()
            };
            let Some(store) = store else {
                tracing::info!("forge: session_completed listener disabled (no spine connection)");
                return;
            };
            let handler = SessionLinker {
                shared: listener_shared,
            };
            if let Err(e) = session_listener::run(store, handler, None).await {
                tracing::error!("forge: session_completed listener exited: {e}");
            }
        });

        // Run the HTTP server.
        axum::serve(listener, app).await.unwrap();
    });
}

/// Event-driven handler that links completed Stiglab sessions back to the
/// pipeline artifact they were shaping.
///
/// For now the work is light: log the linkage and emit a spine event so the
/// dashboard's per-run DAG has a recorded edge. The next step (not yet done)
/// is to fold the ShapingResult back into the pipeline state without the
/// synchronous HTTP roundtrip in `HttpStiglabDispatcher::dispatch`.
struct SessionLinker {
    shared: SharedForge,
}

#[async_trait]
impl SessionCompletedHandler for SessionLinker {
    async fn on_session_completed(&self, event: SessionCompleted) -> anyhow::Result<()> {
        let Some(ref artifact_id) = event.artifact_id else {
            // Non-shaping sessions (direct task POSTs) have no artifact linkage.
            return Ok(());
        };

        tracing::info!(
            session_id = %event.session_id,
            artifact_id = %artifact_id,
            duration_ms = event.duration_ms,
            "forge: session completed, recording lineage"
        );

        // Persist the linkage as a vertical_lineage row so the dashboard can
        // render it immediately — the spine event is the source of truth,
        // this is just the materialized projection. INSERT is idempotent:
        // duplicates are a no-op thanks to ON CONFLICT DO NOTHING.
        let spine = {
            let state = self.shared.read().await;
            state.spine.clone()
        };
        let Some(spine) = spine else { return Ok(()) };

        // Look up the current version for this artifact; default to 0 if
        // the artifact isn't yet tracked (e.g. dashboard-registered only).
        let version: i32 =
            sqlx::query_scalar("SELECT current_version FROM artifacts WHERE artifact_id = $1")
                .bind(artifact_id)
                .fetch_optional(spine.pool())
                .await?
                .unwrap_or(0);

        let _ = sqlx::query(
            "INSERT INTO vertical_lineage (artifact_id, version, session_id) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(artifact_id)
        .bind(version)
        .bind(&event.session_id)
        .execute(spine.pool())
        .await?;

        Ok(())
    }
}

/// Build the Forge HTTP API router.
fn build_api(shared: SharedForge) -> axum::Router {
    use axum::routing::get;

    axum::Router::new()
        .route("/api/health", get(health))
        .route("/api/status", get(status))
        .route(
            "/api/artifacts",
            get(list_artifacts).post(register_artifact),
        )
        .route("/api/artifacts/{id}", get(get_artifact))
        .with_state(shared)
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "status": "ok", "service": "forge" }))
}

async fn status(
    axum::extract::State(shared): axum::extract::State<SharedForge>,
) -> axum::Json<serde_json::Value> {
    let state = shared.read().await;
    let active = state.pipeline.store.active_artifacts();
    let process_state = state.pipeline.state.current();
    axum::Json(serde_json::json!({
        "process_state": format!("{process_state:?}"),
        "active_artifacts": active.len(),
        "artifacts": active.iter().map(|a| serde_json::json!({
            "id": a.artifact_id.as_str(),
            "kind": a.kind.to_string(),
            "name": a.name,
            "state": format!("{:?}", a.state),
            "version": a.current_version,
        })).collect::<Vec<_>>(),
    }))
}

#[derive(serde::Deserialize)]
struct RegisterRequest {
    kind: String,
    name: String,
    owner: String,
}

async fn register_artifact(
    axum::extract::State(shared): axum::extract::State<SharedForge>,
    axum::Json(req): axum::Json<RegisterRequest>,
) -> impl axum::response::IntoResponse {
    let kind = match req.kind.as_str() {
        "code" => Kind::Code,
        "document" => Kind::Document,
        "pull_request" => Kind::PullRequest,
        other => Kind::Custom(other.to_string()),
    };

    let mut state = shared.write().await;
    let id = state
        .pipeline
        .store
        .register(kind.clone(), &req.name, &req.owner);

    // Persist to spine database.
    if let Some(ref spine) = state.spine {
        let _ = sqlx::query(
            "INSERT INTO artifacts (artifact_id, kind, name, owner, created_by, state, current_version) \
             VALUES ($1, $2, $3, $4, 'forge', 'draft', 0) \
             ON CONFLICT (artifact_id) DO NOTHING",
        )
        .bind(id.as_str())
        .bind(&req.kind)
        .bind(&req.name)
        .bind(&req.owner)
        .execute(spine.pool())
        .await;
    }

    (
        axum::http::StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "artifact": {
                "id": id.as_str(),
                "kind": req.kind,
                "name": req.name,
                "owner": req.owner,
                "state": "draft",
                "current_version": 0,
            }
        })),
    )
}

async fn list_artifacts(
    axum::extract::State(shared): axum::extract::State<SharedForge>,
) -> axum::Json<serde_json::Value> {
    let state = shared.read().await;
    let artifacts: Vec<_> = state
        .pipeline
        .store
        .active_artifacts()
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.artifact_id.as_str(),
                "kind": a.kind.to_string(),
                "name": a.name,
                "state": format!("{:?}", a.state),
                "owner": a.owner,
                "version": a.current_version,
            })
        })
        .collect();
    axum::Json(serde_json::json!({ "artifacts": artifacts }))
}

async fn get_artifact(
    axum::extract::State(shared): axum::extract::State<SharedForge>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    let state = shared.read().await;
    let aid = onsager_spine::artifact::ArtifactId::new(&id);
    match state.pipeline.store.get(&aid) {
        Some(a) => axum::Json(serde_json::json!({
            "artifact": {
                "id": a.artifact_id.as_str(),
                "kind": a.kind.to_string(),
                "name": a.name,
                "state": format!("{:?}", a.state),
                "owner": a.owner,
                "version": a.current_version,
            }
        }))
        .into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "error": "artifact not found" })),
        )
            .into_response(),
    }
}

/// Emit a pipeline event to the event spine.
async fn emit_pipeline_event(spine: &EventStore, event: &PipelineEvent) {
    let metadata = onsager_spine::EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: "forge".to_string(),
    };

    let (stream_id, event_type, data) = match event {
        PipelineEvent::DecisionMade(d) => (
            format!("forge:{}", d.artifact_id),
            "forge.decision_made",
            serde_json::json!({
                "artifact_id": d.artifact_id.as_str(),
                "target_version": d.target_version,
                "priority": d.priority,
            }),
        ),
        PipelineEvent::ShapingDispatched {
            request_id,
            artifact_id,
            target_version,
        } => (
            format!("forge:{artifact_id}"),
            "forge.shaping_dispatched",
            serde_json::json!({
                "request_id": request_id,
                "artifact_id": artifact_id,
                "target_version": target_version,
            }),
        ),
        PipelineEvent::ShapingReturned {
            request_id,
            artifact_id,
            outcome,
        } => (
            format!("forge:{artifact_id}"),
            "forge.shaping_returned",
            serde_json::json!({
                "request_id": request_id,
                "artifact_id": artifact_id,
                "outcome": outcome,
            }),
        ),
        PipelineEvent::GateRequested {
            artifact_id,
            gate_point,
        } => (
            format!("forge:{artifact_id}"),
            "forge.gate_requested",
            serde_json::json!({
                "artifact_id": artifact_id,
                "gate_point": format!("{gate_point:?}"),
            }),
        ),
        PipelineEvent::GateVerdictReceived {
            artifact_id,
            gate_point,
            verdict,
        } => (
            format!("forge:{artifact_id}"),
            "forge.gate_verdict",
            serde_json::json!({
                "artifact_id": artifact_id,
                "gate_point": format!("{gate_point:?}"),
                "verdict": format!("{verdict:?}"),
            }),
        ),
        PipelineEvent::ArtifactAdvanced {
            artifact_id,
            from_state,
            to_state,
        } => (
            format!("forge:{artifact_id}"),
            "artifact.state_changed",
            serde_json::json!({
                "artifact_id": artifact_id,
                "from_state": format!("{from_state:?}"),
                "to_state": format!("{to_state:?}"),
            }),
        ),
        PipelineEvent::IdleTick => return,
        PipelineEvent::Error(msg) => (
            "forge:system".to_string(),
            "forge.error",
            serde_json::json!({ "error": msg }),
        ),
    };

    if let Err(e) = spine
        .append_ext(&stream_id, "forge", event_type, data, &metadata, None)
        .await
    {
        tracing::warn!("failed to emit forge event: {e}");
    }
}
