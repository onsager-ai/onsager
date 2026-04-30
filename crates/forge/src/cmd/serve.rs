//! `forge serve` — start the Forge scheduling loop with HTTP API.

use std::sync::Arc;

use async_trait::async_trait;
use axum::response::IntoResponse;
use tokio::sync::RwLock;

use onsager_artifact::Kind;
use onsager_spine::EventStore;

use crate::core::artifact_store::ArtifactStore;
use crate::core::gate_verdict_listener;
use crate::core::insight_cache::InsightCache;
use crate::core::insight_listener;
use crate::core::kernel::BaselineKernel;
use crate::core::pending::{PendingShapings, PendingVerdicts};
use crate::core::persistence;
use crate::core::pipeline::{ForgePipeline, PipelineEvent};
use crate::core::session_listener::{self, SessionCompleted, SessionCompletedHandler};
use crate::core::shaping_result_listener;
use crate::core::signal_cache::SignalCache;
use crate::core::stage_runner::{self, StageEvent};
use crate::core::trigger_subscriber::{
    self, register_artifact_from_trigger, trigger_external_ref, TriggerFired, TriggerHandler,
};
use crate::core::workflow_gates::{LiveGateEvaluator, SpineGateEmitter};
use crate::core::workflow_persistence;
use crate::core::workflow_signal_listener;

/// Shared Forge state accessible from both the HTTP API and the tick loop.
///
/// The signal cache and parking maps (`PendingVerdicts`, `PendingShapings`)
/// are deliberately *not* held here — they're cloned directly into the
/// listeners and the gate evaluator at startup. Re-introduce them only if
/// an HTTP route or tick branch actually needs to read them.
struct ForgeSharedState {
    pipeline: ForgePipeline,
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

        // Load existing artifacts from the spine database (issue #30).
        // `load_artifact_store` restores state, version, and current_version_id
        // so a mid-tick restart resumes at the last persisted transition.
        let artifact_store = match spine.as_ref() {
            Some(s) => match persistence::load_artifact_store(s.pool()).await {
                Ok(store) => {
                    tracing::info!(
                        "forge: loaded {} active artifacts from spine",
                        store.active_artifacts().len()
                    );
                    store
                }
                Err(e) => {
                    tracing::error!("forge: failed to load artifacts from spine: {e}");
                    ArtifactStore::new()
                }
            },
            None => ArtifactStore::new(),
        };

        // Phase 5 of Lever C (#148): the workflow stage runner's
        // gate evaluator emits `forge.shaping_dispatched` and
        // `forge.gate_requested` directly onto the spine. No more
        // sibling-subsystem HTTP coordination — the seam between
        // forge and stiglab/synodic is the spine exclusively.
        // STIGLAB_URL / SYNODIC_URL / SYNODIC_FAIL_POLICY /
        // STIGLAB_INTERNAL_DISPATCH_TOKEN env vars are no longer
        // consumed by forge; see CLAUDE.md / .env.example.

        // Phase-3 parking maps for the Lever C event-driven flow. The
        // listeners spawned below populate these; the pipeline tick
        // consumes them on its resume path.
        let pending_verdicts = PendingVerdicts::new();
        let pending_shapings = PendingShapings::new();

        let insight_cache = InsightCache::default();
        let mut pipeline = ForgePipeline::new(pending_verdicts.clone(), pending_shapings.clone())
            .with_insight_cache(insight_cache.clone());
        pipeline.store = artifact_store;

        // Workflows are read from the spine DB on demand — both at the top
        // of every tick (for the stage runner) and inside the trigger
        // handler (for `trigger.fired` lookups). No in-memory registry:
        // stiglab's `workflows` table is the single source of truth, which
        // means workflow edits, deactivations, and fresh creations are
        // picked up without a restart and without risking a stale-cache
        // branch that silently disagrees with the DB.
        let signals = SignalCache::new();

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

        // Spawn the tick loop. The workflow stage runner's gate
        // evaluator needs an emitter handle on the spine; if forge is
        // running without a spine (legacy dev setup) the evaluator is
        // skipped entirely — the workflow path requires the spine.
        let tick_shared = shared.clone();
        let gate_evaluator = {
            let spine_handle = {
                let state = tick_shared.read().await;
                state.spine.clone()
            };
            spine_handle.map(|store| {
                LiveGateEvaluator::new(
                    signals.clone(),
                    SpineGateEmitter::new(store),
                    pending_verdicts.clone(),
                )
            })
        };
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(tick_ms));
            loop {
                interval.tick().await;

                // Load workflows fresh from the spine DB before taking the
                // tick write lock. DB query outside the lock so HTTP reads
                // aren't blocked, and a per-tick snapshot means no stale
                // in-memory entries can mask an edit or deactivation.
                let workflows_snapshot = {
                    let spine_handle = {
                        let state = tick_shared.read().await;
                        state.spine.clone()
                    };
                    match spine_handle.as_ref() {
                        Some(s) => match workflow_persistence::load_workflows(s.pool()).await {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::error!("forge: failed to reload workflows: {e}");
                                continue;
                            }
                        },
                        None => std::collections::HashMap::new(),
                    }
                };

                // Run the pipeline tick under the write lock, then release it
                // before emitting spine events so HTTP reads aren't starved.
                let (output, spine, stage_events, stage_touched_ids) = {
                    let mut state = tick_shared.write().await;
                    let kernel = state.kernel.clone();
                    let output = state.pipeline.tick(&kernel);

                    // Workflow stage runner (issue #80) — walks every
                    // workflow-tagged artifact one step per tick. Runs
                    // under the same write lock as the pipeline so
                    // advancement decisions see a consistent artifact
                    // snapshot. Refill the per-tick dispatch budget
                    // before the pass so a burst of new artifacts can't
                    // synchronously hammer the spine under the lock.
                    //
                    // No spine connection means no event-driven gate
                    // evaluator (legacy dev setup); the stage runner
                    // is skipped for this tick.
                    let stage_events = match gate_evaluator.as_ref() {
                        Some(eval) => {
                            eval.reset_dispatch_budget();
                            stage_runner::advance_workflow_artifacts(
                                &workflows_snapshot,
                                &mut state.pipeline.store,
                                eval,
                            )
                        }
                        None => Vec::new(),
                    };
                    let stage_touched_ids: std::collections::HashSet<String> = stage_events
                        .iter()
                        .map(|e| match e {
                            StageEvent::StageEntered { artifact_id, .. }
                            | StageEvent::GatePassed { artifact_id, .. }
                            | StageEvent::GateFailed { artifact_id, .. }
                            | StageEvent::StageAdvanced { artifact_id, .. } => artifact_id.clone(),
                        })
                        .collect();

                    let spine = state.spine.clone();
                    (output, spine, stage_events, stage_touched_ids)
                };

                // Mirror state transitions to the artifacts row before the
                // audit-log event so a crash between the two leaves the
                // durable state ahead of (not behind) the event stream —
                // replay can catch up, but a state behind the event stream
                // is silent drift (issue #30). Dedupe: a single tick can
                // emit both ArtifactAdvanced and BundleSealed for the same
                // artifact; one UPDATE covers both since we snapshot
                // state + version + bundle_id together.
                if let Some(ref spine) = spine {
                    let mut seen = std::collections::HashSet::new();
                    for event in &output.events {
                        let artifact_id = match event {
                            PipelineEvent::ArtifactAdvanced { artifact_id, .. }
                            | PipelineEvent::BundleSealed { artifact_id, .. } => Some(artifact_id),
                            _ => None,
                        };
                        let Some(aid) = artifact_id else { continue };
                        if !seen.insert(aid.clone()) {
                            continue;
                        }
                        let snapshot = {
                            let state = tick_shared.read().await;
                            state
                                .pipeline
                                .store
                                .get(&onsager_artifact::ArtifactId::new(aid))
                                .cloned()
                        };
                        if let Some(artifact) = snapshot {
                            if let Err(e) =
                                persistence::persist_artifact_state(spine.pool(), &artifact).await
                            {
                                tracing::error!(
                                    artifact_id = %aid,
                                    "forge: failed to persist state transition: {e}"
                                );
                            }
                        } else {
                            tracing::warn!(
                                artifact_id = %aid,
                                "forge: state transition event references unknown artifact"
                            );
                        }
                    }
                }

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

                // Persist workflow columns for every artifact the stage
                // runner touched this tick. Same pattern as the pipeline
                // state snapshot above — DB ahead of event stream.
                if let Some(ref spine) = spine {
                    for aid in &stage_touched_ids {
                        let snapshot = {
                            let state = tick_shared.read().await;
                            state
                                .pipeline
                                .store
                                .get(&onsager_artifact::ArtifactId::new(aid))
                                .cloned()
                        };
                        if let Some(artifact) = snapshot {
                            if let Err(e) =
                                persistence::persist_artifact_state(spine.pool(), &artifact).await
                            {
                                tracing::error!(
                                    artifact_id = %aid,
                                    "forge: failed to persist stage state: {e}"
                                );
                            }
                            if let Err(e) = workflow_persistence::persist_artifact_workflow_state(
                                spine.pool(),
                                &artifact,
                            )
                            .await
                            {
                                tracing::error!(
                                    artifact_id = %aid,
                                    "forge: failed to persist workflow state: {e}"
                                );
                            }
                        }
                    }
                }

                // Emit stage lifecycle events to the spine.
                for event in &stage_events {
                    if let Some(ref spine) = spine {
                        emit_stage_event(spine, event).await;
                    }
                    tracing::info!("forge stage: {event:?}");
                }
            }
        });

        // Spawn the ising.insight_emitted listener (issue #36). It pushes
        // parsed insights into `insight_cache`, which the pipeline pulls
        // into `WorldState.insights` on every tick.
        //
        // Warm-start the listener at the current max event id so it skips
        // history-wide backfill. Replaying the entire spine on every boot
        // would delay startup and push unbounded rows through the handler;
        // insights are advisory priors, so forgoing old ones is correct —
        // new emissions arrive live via pg_notify.
        let insight_listener_shared = shared.clone();
        let insight_cache_for_listener = insight_cache.clone();
        tokio::spawn(async move {
            let store = {
                let state = insight_listener_shared.read().await;
                state.spine.clone()
            };
            let Some(store) = store else {
                tracing::info!("forge: insight listener disabled (no spine connection)");
                return;
            };
            let since = match store.max_event_id().await {
                Ok(cursor) => cursor,
                Err(e) => {
                    tracing::warn!(
                        "forge: max_event_id lookup failed ({e}); starting insight \
                         listener from the beginning"
                    );
                    None
                }
            };
            if let Err(e) = insight_listener::run(store, insight_cache_for_listener, since).await {
                tracing::error!("forge: insight listener exited: {e}");
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

        // Spawn the workflow trigger subscriber (issue #80). Handles
        // `trigger.fired` events: resolves the workflow, registers a new
        // artifact, enters stage 0.
        let trigger_shared = shared.clone();
        tokio::spawn(async move {
            let store = {
                let state = trigger_shared.read().await;
                state.spine.clone()
            };
            let Some(store) = store else {
                tracing::info!("forge: trigger subscriber disabled (no spine connection)");
                return;
            };
            let handler = WorkflowTriggerHandler {
                shared: trigger_shared,
            };
            if let Err(e) = trigger_subscriber::run(store, handler, None).await {
                tracing::error!("forge: trigger subscriber exited: {e}");
            }
        });

        // Spawn the workflow signal listener (issue #80). Translates
        // `git.ci_completed`, `git.pr_merged`, `git.pr_closed`, and
        // `stiglab.session_completed` events into SignalCache entries so
        // the stage runner's external-check / manual-approval /
        // agent-session gates resolve.
        let signals_for_listener = signals.clone();
        let signal_listener_shared = shared.clone();
        tokio::spawn(async move {
            let store = {
                let state = signal_listener_shared.read().await;
                state.spine.clone()
            };
            let Some(store) = store else {
                tracing::info!("forge: workflow signal listener disabled (no spine connection)");
                return;
            };
            if let Err(e) = workflow_signal_listener::run(store, signals_for_listener, None).await {
                tracing::error!("forge: workflow signal listener exited: {e}");
            }
        });

        // Spawn the Lever C verdict listener (spec #131 / ADR 0004 phase 3).
        // Tails `synodic.gate_verdict`, parses the typed variant, and
        // parks the embedded `GateVerdict` in `pending_verdicts` keyed by
        // its `gate_id`. Phase 4 wires the pipeline tick's resume path
        // to claim entries from this map; until then the listener
        // accumulates verdicts so the schema is exercised end-to-end.
        //
        // Warm-start at `max_event_id` so a fresh boot doesn't replay
        // every historical verdict — same rationale as the insight
        // listener above. A phase-6 follow-up persists a per-process
        // cursor so a crash doesn't drop in-flight verdicts.
        let verdict_shared = shared.clone();
        let pending_verdicts_for_listener = pending_verdicts.clone();
        tokio::spawn(async move {
            let store = {
                let state = verdict_shared.read().await;
                state.spine.clone()
            };
            let Some(store) = store else {
                tracing::info!("forge: gate_verdict listener disabled (no spine connection)");
                return;
            };
            let since = match store.max_event_id().await {
                Ok(cursor) => cursor,
                Err(e) => {
                    tracing::warn!(
                        "forge: max_event_id lookup failed ({e}); starting gate_verdict \
                         listener from the beginning"
                    );
                    None
                }
            };
            if let Err(e) =
                gate_verdict_listener::run(store, pending_verdicts_for_listener, since).await
            {
                tracing::error!("forge: gate_verdict listener exited: {e}");
            }
        });

        // Spawn the Lever C shaping-result listener. Tails
        // `stiglab.shaping_result_ready`, parses the typed variant, and
        // parks the embedded `ShapingResult` in `pending_shapings` keyed
        // by its `request_id`. Same warm-start strategy.
        let shaping_shared = shared.clone();
        let pending_shapings_for_listener = pending_shapings.clone();
        tokio::spawn(async move {
            let store = {
                let state = shaping_shared.read().await;
                state.spine.clone()
            };
            let Some(store) = store else {
                tracing::info!("forge: shaping_result listener disabled (no spine connection)");
                return;
            };
            let since = match store.max_event_id().await {
                Ok(cursor) => cursor,
                Err(e) => {
                    tracing::warn!(
                        "forge: max_event_id lookup failed ({e}); starting shaping_result \
                         listener from the beginning"
                    );
                    None
                }
            };
            if let Err(e) =
                shaping_result_listener::run(store, pending_shapings_for_listener, since).await
            {
                tracing::error!("forge: shaping_result listener exited: {e}");
            }
        });

        // Run the HTTP server.
        axum::serve(listener, app).await.unwrap();
    });
}

/// Handler for `trigger.fired` events — registers an artifact against the
/// referenced workflow and enters stage 0.
struct WorkflowTriggerHandler {
    shared: SharedForge,
}

#[async_trait]
impl TriggerHandler for WorkflowTriggerHandler {
    async fn on_trigger_fired(&self, event: TriggerFired) -> anyhow::Result<()> {
        let spine = {
            let state = self.shared.read().await;
            state.spine.clone()
        };
        // Resolve the workflow by reading the spine DB directly — no
        // in-memory cache means no stale view of active/inactive or
        // edited stages. Absent spine means forge can't advance
        // workflows at all; surface and bail.
        let Some(ref spine_ref) = spine else {
            tracing::warn!(
                workflow_id = %event.workflow_id,
                "trigger.fired dropped (no spine to resolve workflow)"
            );
            return Ok(());
        };
        let workflow =
            match workflow_persistence::load_workflow(spine_ref.pool(), &event.workflow_id).await {
                Ok(Some(w)) => w,
                Ok(None) => {
                    tracing::warn!(
                        workflow_id = %event.workflow_id,
                        "trigger.fired for unknown workflow (no row in spine)"
                    );
                    return Ok(());
                }
                Err(e) => {
                    tracing::error!(
                        workflow_id = %event.workflow_id,
                        "forge: failed to load workflow for trigger.fired: {e}"
                    );
                    return Ok(());
                }
            };

        // Dedup: a single (workflow, issue) must produce a single artifact
        // even if the trigger fires multiple times (e.g. label removed and
        // re-added, retried webhook delivery, rapid double-click). Look up
        // by external_ref before creating; on hit, drop the trigger.
        //
        // Best-effort: the lookup is a plain SELECT, not advisory-locked.
        // True concurrent ties may briefly create two rows; subsequent
        // deliveries converge via the deterministic ORDER BY in
        // `find_artifact_id_by_external_ref`. If races appear in practice,
        // upgrade to the portal's `pg_advisory_xact_lock` pattern.
        let external_ref =
            trigger_external_ref(&event.workflow_id, &event.trigger_kind, &event.payload);

        if let (Some(spine_ref), Some(ref ext_ref)) = (spine.as_ref(), external_ref.as_ref()) {
            match persistence::find_artifact_id_by_external_ref(spine_ref.pool(), ext_ref).await {
                Ok(Some(existing_id)) => {
                    tracing::info!(
                        workflow_id = %event.workflow_id,
                        external_ref = %ext_ref,
                        artifact_id = %existing_id,
                        "trigger.fired dropped (artifact already exists for this issue)"
                    );
                    return Ok(());
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        external_ref = %ext_ref,
                        "forge: dedup lookup failed, falling through to register: {e}"
                    );
                }
            }
        }

        // Perform the registration under the write lock and capture the
        // resulting StageEntered event.
        let emitted = {
            let mut state = self.shared.write().await;
            register_artifact_from_trigger(&mut state.pipeline.store, &workflow, &event)
        };

        let Some((artifact, stage_event)) = emitted else {
            tracing::info!(
                workflow_id = %event.workflow_id,
                "trigger.fired dropped (workflow inactive)"
            );
            return Ok(());
        };

        if let Some(spine) = spine {
            // Mirror both the base artifact row and the workflow tagging.
            // The external_ref column is what makes the next trigger.fired
            // for this same (workflow, issue) hit the dedup branch above.
            if let Err(e) = persistence::insert_artifact_row(
                spine.pool(),
                artifact.artifact_id.as_str(),
                &artifact.kind.to_string(),
                &artifact.name,
                &artifact.owner,
                external_ref.as_deref(),
            )
            .await
            {
                tracing::error!(
                    artifact_id = %artifact.artifact_id,
                    "forge: failed to insert trigger-registered artifact row: {e}"
                );
                return Ok(());
            }
            if let Err(e) = persistence::persist_artifact_state(spine.pool(), &artifact).await {
                tracing::error!(
                    artifact_id = %artifact.artifact_id,
                    "forge: failed to persist trigger-registered artifact state: {e}"
                );
                return Ok(());
            }
            if let Err(e) =
                workflow_persistence::persist_artifact_workflow_state(spine.pool(), &artifact).await
            {
                tracing::error!(
                    artifact_id = %artifact.artifact_id,
                    "forge: failed to persist trigger-registered artifact workflow state: {e}"
                );
                return Ok(());
            }
            emit_stage_event(&spine, &stage_event).await;
        }
        Ok(())
    }
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
) -> axum::response::Response {
    let kind = match req.kind.as_str() {
        "code" => Kind::Code,
        "document" => Kind::Document,
        "pull_request" => Kind::PullRequest,
        other => Kind::Custom(other.to_string()),
    };

    // Build the artifact up front so we know the ULID before touching any
    // store. With a spine, the DB row is written first; only on success do
    // we insert into the in-memory cache. Issue #30: the old code did the
    // two writes in the other order and ignored the DB error, producing a
    // ghost artifact on failure.
    let artifact =
        onsager_artifact::Artifact::new(kind, req.name.clone(), req.owner.clone(), "forge", vec![]);
    let id = artifact.artifact_id.clone();

    let spine = {
        let state = shared.read().await;
        state.spine.clone()
    };

    if let Some(spine) = spine.as_ref() {
        if let Err(e) = persistence::insert_artifact_row(
            spine.pool(),
            id.as_str(),
            &req.kind,
            &req.name,
            &req.owner,
            None,
        )
        .await
        {
            // Full sqlx::Error goes to the server log (which may carry
            // constraint names, column types, etc.); the HTTP client
            // only sees a stable, opaque error tag plus the artifact ID
            // it submitted for correlation.
            tracing::error!(artifact_id = %id, "forge: failed to register artifact in spine: {e}");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({
                    "error": "failed to persist artifact",
                    "artifact_id": id.as_str(),
                })),
            )
                .into_response();
        }
    }

    {
        let mut state = shared.write().await;
        state.pipeline.store.insert(artifact);
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
        .into_response()
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
    let aid = onsager_artifact::ArtifactId::new(&id);
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
///
/// Carrier events (`forge.shaping_dispatched`, `forge.gate_requested`)
/// are serialized through the typed [`FactoryEventKind`] so the full
/// `request` payload travels alongside the routing fields. Stiglab's
/// shaping listener and Synodic's gate listener consume the embedded
/// payload directly — phase 3 of spec #131 / ADR 0004 Lever C — instead
/// of falling back to a sibling-subsystem HTTP roundtrip.
async fn emit_pipeline_event(spine: &EventStore, event: &PipelineEvent) {
    let metadata = onsager_spine::EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: "forge".to_string(),
    };

    // Match-and-serialize via FactoryEventKind variants where the schema
    // exists; fall back to hand-rolled JSON for events that don't have a
    // typed variant yet (forge.error, forge.gate_verdict, forge.shaping_returned).
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
            request,
        } => {
            let kind = onsager_spine::factory_event::FactoryEventKind::ForgeShapingDispatched {
                request_id: request_id.clone(),
                artifact_id: onsager_artifact::ArtifactId::new(artifact_id),
                target_version: *target_version,
                request: Some(request.clone()),
            };
            let data = serde_json::to_value(&kind).expect("FactoryEventKind must serialize");
            (
                format!("forge:{artifact_id}"),
                "forge.shaping_dispatched",
                data,
            )
        }
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
            gate_id,
            artifact_id,
            gate_point,
            request,
        } => {
            let kind = onsager_spine::factory_event::FactoryEventKind::ForgeGateRequested {
                gate_id: gate_id.clone(),
                artifact_id: onsager_artifact::ArtifactId::new(artifact_id),
                gate_point: *gate_point,
                request: Some(request.clone()),
            };
            let data = serde_json::to_value(&kind).expect("FactoryEventKind must serialize");
            (format!("forge:{artifact_id}"), "forge.gate_requested", data)
        }
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
        PipelineEvent::BundleSealed {
            artifact_id,
            bundle_id,
            version,
        } => (
            format!("warehouse:{artifact_id}"),
            "warehouse.bundle_sealed",
            serde_json::json!({
                "artifact_id": artifact_id,
                "bundle_id": bundle_id.as_str(),
                "version": version,
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

/// Emit a workflow stage event to the spine (issue #80). Uses the
/// `workflow` namespace so the dashboard's live view can subscribe with
/// a single filter.
async fn emit_stage_event(spine: &EventStore, event: &StageEvent) {
    let metadata = onsager_spine::EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: "forge".to_string(),
    };

    let (stream_id, event_type, data) = match event {
        StageEvent::StageEntered {
            artifact_id,
            workflow_id,
            stage_index,
            stage_name,
        } => (
            format!("workflow:{artifact_id}"),
            "stage.entered",
            serde_json::json!({
                "artifact_id": artifact_id,
                "workflow_id": workflow_id,
                "stage_index": stage_index,
                "stage_name": stage_name,
            }),
        ),
        StageEvent::GatePassed {
            artifact_id,
            workflow_id,
            stage_index,
            gate_kind,
        } => (
            format!("workflow:{artifact_id}"),
            "stage.gate_passed",
            serde_json::json!({
                "artifact_id": artifact_id,
                "workflow_id": workflow_id,
                "stage_index": stage_index,
                "gate_kind": gate_kind,
            }),
        ),
        StageEvent::GateFailed {
            artifact_id,
            workflow_id,
            stage_index,
            gate_kind,
            reason,
        } => (
            format!("workflow:{artifact_id}"),
            "stage.gate_failed",
            serde_json::json!({
                "artifact_id": artifact_id,
                "workflow_id": workflow_id,
                "stage_index": stage_index,
                "gate_kind": gate_kind,
                "reason": reason,
            }),
        ),
        StageEvent::StageAdvanced {
            artifact_id,
            workflow_id,
            from_stage_index,
            to_stage_index,
        } => (
            format!("workflow:{artifact_id}"),
            "stage.advanced",
            serde_json::json!({
                "artifact_id": artifact_id,
                "workflow_id": workflow_id,
                "from_stage_index": from_stage_index,
                "to_stage_index": to_stage_index,
            }),
        ),
    };

    if let Err(e) = spine
        .append_ext(&stream_id, "workflow", event_type, data, &metadata, None)
        .await
    {
        tracing::warn!("failed to emit workflow stage event: {e}");
    }
}
