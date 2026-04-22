//! `forge serve` — start the Forge scheduling loop with HTTP API.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::response::IntoResponse;
use chrono::Utc;
use tokio::sync::RwLock;

use onsager_artifact::Kind;
use onsager_protocol::{
    EscalationContext, GateRequest, GateVerdict, ShapingRequest, ShapingResult,
};
use onsager_spine::factory_event::ShapingOutcome;
use onsager_spine::EventStore;

use crate::core::artifact_store::ArtifactStore;
use crate::core::insight_cache::InsightCache;
use crate::core::insight_listener;
use crate::core::kernel::BaselineKernel;
use crate::core::persistence;
use crate::core::pipeline::{ForgePipeline, PipelineEvent, StiglabDispatcher, SynodicGate};
use crate::core::session_listener::{self, SessionCompleted, SessionCompletedHandler};
use crate::core::signal_cache::SignalCache;
use crate::core::stage_runner::{self, StageEvent};
use crate::core::trigger_subscriber::{
    self, register_artifact_from_trigger, TriggerFired, TriggerHandler,
};
use crate::core::workflow::Workflow;
use crate::core::workflow_gates::LiveGateEvaluator;
use crate::core::workflow_persistence;
use crate::core::workflow_signal_listener;

/// Default Forge → upstream HTTP timeout. Bounds the worst case for both the
/// Stiglab dispatcher and the Synodic gate so a single hung upstream cannot
/// freeze the pipeline tick (and, transitively, every read on the shared
/// pipeline state — see issue #28).
const FORGE_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Per-request wait window applied to the Stiglab status long-poll. The full
/// shaping deadline can still be longer; the dispatcher loops until then.
const STIGLAB_WAIT_WINDOW: Duration = Duration::from_secs(20);

/// HTTP Stiglab dispatcher — kicks off shaping via `POST /api/shaping`
/// (which now returns 202 immediately — issue #31) and then polls
/// `GET /api/shaping/{session_id}?wait=Ns` with a bounded window until the
/// session reaches a terminal state or the per-request deadline elapses.
///
/// `request.request_id` is sent as the `Idempotency-Key` header so that a
/// retry from a dropped connection collapses onto the original session
/// instead of dispatching a second agent.
struct HttpStiglabDispatcher {
    client: reqwest::Client,
    stiglab_url: String,
}

#[derive(serde::Deserialize)]
struct ShapingAccepted {
    session_id: String,
    #[allow(dead_code)]
    request_id: Option<String>,
}

impl HttpStiglabDispatcher {
    async fn run(&self, request: &ShapingRequest) -> Result<ShapingResult, anyhow::Error> {
        let create_url = format!("{}/api/shaping", self.stiglab_url);
        let body = serde_json::to_value(request)?;

        let resp = self
            .client
            .post(&create_url)
            .header("Idempotency-Key", &request.request_id)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "stiglab POST /api/shaping returned {status}: {text}"
            ));
        }

        // Either 202 (new endpoint) or legacy 200 with a full ShapingResult.
        let body_text = resp.text().await?;
        if status == reqwest::StatusCode::OK {
            // Legacy path — body already contains the terminal ShapingResult.
            let result: ShapingResult = serde_json::from_str(&body_text)?;
            return Ok(result);
        }

        let accepted: ShapingAccepted = serde_json::from_str(&body_text)?;

        // Poll the status endpoint until the session is terminal or until the
        // request's soft deadline expires. The wait window per request is
        // capped so individual HTTP roundtrips stay HTTP-intermediary friendly.
        let overall_deadline = request
            .deadline
            .and_then(|d| {
                let remaining = d.signed_duration_since(Utc::now()).num_milliseconds();
                if remaining <= 0 {
                    None
                } else {
                    Some(tokio::time::Instant::now() + Duration::from_millis(remaining as u64))
                }
            })
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(300));

        let status_url = format!("{}/api/shaping/{}", self.stiglab_url, accepted.session_id);

        loop {
            let now = tokio::time::Instant::now();
            if now >= overall_deadline {
                return Err(anyhow::anyhow!("stiglab shaping timed out"));
            }
            let remaining = overall_deadline.saturating_duration_since(now);
            let wait = remaining.min(STIGLAB_WAIT_WINDOW);

            let resp = self
                .client
                .get(&status_url)
                .query(&[("wait", format!("{}s", wait.as_secs().max(1)))])
                .send()
                .await?;
            let resp_status = resp.status();
            if resp_status == reqwest::StatusCode::OK {
                return Ok(resp.json::<ShapingResult>().await?);
            }
            if resp_status == reqwest::StatusCode::ACCEPTED {
                // Still running — drain body and loop.
                let _ = resp.text().await;
                continue;
            }
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "stiglab GET status returned {resp_status}: {text}"
            ));
        }
    }
}

impl StiglabDispatcher for HttpStiglabDispatcher {
    fn dispatch(&self, request: &ShapingRequest) -> ShapingResult {
        // The pipeline tick is synchronous, so block_in_place lets us await
        // async HTTP calls without panicking inside the Tokio runtime.
        let outcome = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.run(request))
        });

        match outcome {
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
                    error: Some(onsager_protocol::ErrorDetail {
                        code: "dispatch_error".into(),
                        message: e.to_string(),
                        retriable: Some(true),
                    }),
                }
            }
        }
    }
}

/// What to do when the Synodic gate is unreachable or unparsable
/// (issue #29).
///
/// `Escalate` is the default because the pipeline already handles
/// `GateVerdict::Escalate` non-blockingly (forge invariant #5): the artifact
/// stays put while the escalation is parked, instead of either silently
/// advancing (`Allow`) or hard-stopping (`Deny`) on every transient blip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SynodicFailPolicy {
    Allow,
    Deny,
    Escalate,
}

impl SynodicFailPolicy {
    fn from_env() -> Self {
        match std::env::var("SYNODIC_FAIL_POLICY")
            .ok()
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("allow") => SynodicFailPolicy::Allow,
            Some("deny") => SynodicFailPolicy::Deny,
            Some("escalate") | None => SynodicFailPolicy::Escalate,
            Some(other) => {
                tracing::warn!("SYNODIC_FAIL_POLICY={other} is not recognized; using \"escalate\"");
                SynodicFailPolicy::Escalate
            }
        }
    }

    fn verdict(self, reason: &str) -> GateVerdict {
        match self {
            SynodicFailPolicy::Allow => GateVerdict::Allow,
            SynodicFailPolicy::Deny => GateVerdict::Deny {
                reason: format!("synodic-fail-deny: {reason}"),
            },
            SynodicFailPolicy::Escalate => GateVerdict::Escalate {
                context: EscalationContext {
                    escalation_id: format!("synodic-fail-{}", ulid::Ulid::new()),
                    reason: format!("synodic gate failure: {reason}"),
                    // Issue #37: the default escalation target is the
                    // supervisor agent, but operators can redirect to a
                    // specific human (e.g. `"human:marvin"`) via env var
                    // when the Stiglab supervisor profile isn't yet wired.
                    target: escalation_default_target(),
                    timeout_at: Utc::now() + chrono::Duration::minutes(15),
                },
            },
        }
    }
}

/// Who receives escalations when Synodic says so but no specific target is
/// set on the verdict (issue #37). Reads `ESCALATION_DEFAULT_TARGET`;
/// defaults to `"supervisor"` so the supervisor Stiglab agent is the
/// natural first responder.
fn escalation_default_target() -> String {
    std::env::var("ESCALATION_DEFAULT_TARGET").unwrap_or_else(|_| "supervisor".to_string())
}

/// Distinguishes failure modes for the Synodic gate so that transient
/// network problems escalate (recoverable) while protocol-level errors
/// (4xx, parse failures) deny outright (loud failures, not silent ones).
#[derive(Debug, Clone, PartialEq, Eq)]
enum SynodicGateFailure {
    /// Network / timeout / connection error.
    Transient(String),
    /// 4xx response — request shape is wrong, fix the caller.
    BadRequest(String),
    /// 5xx response — synodic itself is sick.
    ServerError(String),
    /// 2xx response that didn't deserialize as `GateVerdict`.
    Parse(String),
}

impl SynodicGateFailure {
    fn into_verdict(self, policy: SynodicFailPolicy) -> GateVerdict {
        match self {
            SynodicGateFailure::Transient(reason) | SynodicGateFailure::ServerError(reason) => {
                tracing::warn!(
                    "synodic gate transient failure ({reason}); applying policy {policy:?}"
                );
                policy.verdict(&reason)
            }
            SynodicGateFailure::BadRequest(reason) => {
                // Schema drift / bad caller — refuse to advance regardless of
                // policy. Allowing this would let bugs ride through governance.
                tracing::error!("synodic gate rejected request ({reason}); denying");
                GateVerdict::Deny {
                    reason: format!("synodic-bad-request: {reason}"),
                }
            }
            SynodicGateFailure::Parse(reason) => {
                tracing::error!("synodic gate response parse error ({reason}); denying");
                GateVerdict::Deny {
                    reason: format!("synodic-parse-error: {reason}"),
                }
            }
        }
    }
}

/// HTTP Synodic gate — calls Synodic's gate endpoint.
struct HttpSynodicGate {
    client: reqwest::Client,
    synodic_url: String,
    fail_policy: SynodicFailPolicy,
}

impl HttpSynodicGate {
    async fn evaluate_async(
        &self,
        request: &GateRequest,
    ) -> Result<GateVerdict, SynodicGateFailure> {
        let url = format!("{}/api/gate", self.synodic_url);
        let body = serde_json::to_value(request)
            .map_err(|e| SynodicGateFailure::Parse(format!("request serialization: {e}")))?;

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SynodicGateFailure::Transient(e.to_string()))?;

        let status = resp.status();
        if status.is_success() {
            return resp
                .json::<GateVerdict>()
                .await
                .map_err(|e| SynodicGateFailure::Parse(e.to_string()));
        }
        let text = resp.text().await.unwrap_or_default();
        if status.is_client_error() {
            Err(SynodicGateFailure::BadRequest(format!("{status}: {text}")))
        } else {
            Err(SynodicGateFailure::ServerError(format!("{status}: {text}")))
        }
    }
}

impl SynodicGate for HttpSynodicGate {
    fn evaluate(&self, request: &GateRequest) -> GateVerdict {
        let policy = self.fail_policy;
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match self.evaluate_async(request).await {
                    Ok(verdict) => verdict,
                    Err(failure) => failure.into_verdict(policy),
                }
            })
        })
    }
}

/// Shared Forge state accessible from both the HTTP API and the tick loop.
struct ForgeSharedState {
    pipeline: ForgePipeline<HttpStiglabDispatcher, HttpSynodicGate>,
    kernel: BaselineKernel,
    spine: Option<EventStore>,
    /// Active + in-flight workflows (issue #80). Keyed by workflow_id.
    workflows: std::collections::HashMap<String, Workflow>,
    /// Shared signal cache populated by the workflow signal listener and
    /// consumed by the gate evaluator on each stage-runner tick. Held
    /// here so the HTTP API can introspect pending gates if a future
    /// endpoint exposes them.
    #[allow(dead_code)]
    signals: SignalCache,
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

        let stiglab_port = std::env::var("STIGLAB_PORT").unwrap_or_else(|_| "3000".to_string());
        let stiglab_url = std::env::var("STIGLAB_URL")
            .unwrap_or_else(|_| format!("http://localhost:{stiglab_port}"));
        let synodic_port = std::env::var("SYNODIC_PORT").unwrap_or_else(|_| "3001".to_string());
        let synodic_url = std::env::var("SYNODIC_URL")
            .unwrap_or_else(|_| format!("http://localhost:{synodic_port}"));

        let client = reqwest::Client::builder()
            .timeout(FORGE_HTTP_TIMEOUT)
            .build()
            .expect("failed to build reqwest client");

        let fail_policy = SynodicFailPolicy::from_env();
        tracing::info!(?fail_policy, "forge: synodic gate fail-policy");

        let stiglab = HttpStiglabDispatcher {
            client: client.clone(),
            stiglab_url: stiglab_url.clone(),
        };
        let synodic = HttpSynodicGate {
            client: client.clone(),
            synodic_url: synodic_url.clone(),
            fail_policy,
        };
        // A second pair for the workflow gate evaluator (issue #80). It
        // lives inside the spawned tick task and uses its own reqwest
        // handles so the pipeline and the stage runner can dispatch in
        // parallel without sharing mutable state.
        let evaluator_stiglab = HttpStiglabDispatcher {
            client: client.clone(),
            stiglab_url: stiglab_url.clone(),
        };
        let evaluator_synodic = HttpSynodicGate {
            client,
            synodic_url: synodic_url.clone(),
            fail_policy,
        };

        let insight_cache = InsightCache::default();
        let mut pipeline =
            ForgePipeline::new(stiglab, synodic).with_insight_cache(insight_cache.clone());
        pipeline.store = artifact_store;

        // Load workflows (issue #80). Absent spine → empty registry.
        let workflows = match spine.as_ref() {
            Some(s) => match workflow_persistence::load_workflows(s.pool()).await {
                Ok(w) => {
                    tracing::info!("forge: loaded {} workflows from spine", w.len());
                    w
                }
                Err(e) => {
                    tracing::error!("forge: failed to load workflows: {e}");
                    std::collections::HashMap::new()
                }
            },
            None => std::collections::HashMap::new(),
        };

        let signals = SignalCache::new();

        let shared = Arc::new(RwLock::new(ForgeSharedState {
            pipeline,
            kernel: BaselineKernel::new(),
            spine,
            workflows,
            signals: signals.clone(),
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
        let gate_evaluator =
            LiveGateEvaluator::new(signals.clone(), evaluator_stiglab, evaluator_synodic);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(tick_ms));
            loop {
                interval.tick().await;

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
                    // synchronously hammer Stiglab under the lock.
                    gate_evaluator.reset_dispatch_budget();
                    let workflows_snapshot = state.workflows.clone();
                    let stage_events = stage_runner::advance_workflow_artifacts(
                        &workflows_snapshot,
                        &mut state.pipeline.store,
                        &gate_evaluator,
                    );
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
        let (spine, workflow_opt) = {
            let state = self.shared.read().await;
            (
                state.spine.clone(),
                state.workflows.get(&event.workflow_id).cloned(),
            )
        };
        let Some(workflow) = workflow_opt else {
            tracing::warn!(
                workflow_id = %event.workflow_id,
                "trigger.fired for unknown workflow"
            );
            return Ok(());
        };

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
            if let Err(e) = persistence::insert_artifact_row(
                spine.pool(),
                artifact.artifact_id.as_str(),
                &artifact.kind.to_string(),
                &artifact.name,
                &artifact.owner,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fail_policy_default_is_escalate_when_unset() {
        // Use a subprocess-isolated approach: clear the var locally for this thread.
        // SAFETY: tests in this module are run sequentially via SYNODIC_FAIL_POLICY-
        // sensitive guards; we serialize on the env var via the policy_env_lock below.
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("SYNODIC_FAIL_POLICY");
        assert_eq!(SynodicFailPolicy::from_env(), SynodicFailPolicy::Escalate);
    }

    #[test]
    fn fail_policy_parses_known_values() {
        let _g = ENV_LOCK.lock().unwrap();
        for (raw, expected) in [
            ("allow", SynodicFailPolicy::Allow),
            ("Allow", SynodicFailPolicy::Allow),
            ("deny", SynodicFailPolicy::Deny),
            ("escalate", SynodicFailPolicy::Escalate),
        ] {
            std::env::set_var("SYNODIC_FAIL_POLICY", raw);
            assert_eq!(SynodicFailPolicy::from_env(), expected, "for {raw}");
        }
        std::env::remove_var("SYNODIC_FAIL_POLICY");
    }

    #[test]
    fn fail_policy_unknown_value_falls_back_to_escalate() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("SYNODIC_FAIL_POLICY", "garbage");
        assert_eq!(SynodicFailPolicy::from_env(), SynodicFailPolicy::Escalate);
        std::env::remove_var("SYNODIC_FAIL_POLICY");
    }

    #[test]
    fn transient_failure_escalates_under_default_policy() {
        let v = SynodicGateFailure::Transient("connection refused".into())
            .into_verdict(SynodicFailPolicy::Escalate);
        assert!(matches!(v, GateVerdict::Escalate { .. }));
    }

    #[test]
    fn transient_failure_denies_under_deny_policy() {
        let v = SynodicGateFailure::Transient("connection refused".into())
            .into_verdict(SynodicFailPolicy::Deny);
        assert!(matches!(v, GateVerdict::Deny { .. }));
    }

    #[test]
    fn server_error_treated_like_transient() {
        let v = SynodicGateFailure::ServerError("503 down".into())
            .into_verdict(SynodicFailPolicy::Escalate);
        assert!(matches!(v, GateVerdict::Escalate { .. }));
    }

    #[test]
    fn bad_request_always_denies_regardless_of_policy() {
        for policy in [
            SynodicFailPolicy::Allow,
            SynodicFailPolicy::Deny,
            SynodicFailPolicy::Escalate,
        ] {
            let v = SynodicGateFailure::BadRequest("400 bad shape".into()).into_verdict(policy);
            assert!(
                matches!(v, GateVerdict::Deny { .. }),
                "policy={policy:?} should still deny"
            );
        }
    }

    #[test]
    fn parse_error_always_denies_regardless_of_policy() {
        for policy in [
            SynodicFailPolicy::Allow,
            SynodicFailPolicy::Deny,
            SynodicFailPolicy::Escalate,
        ] {
            let v = SynodicGateFailure::Parse("missing field".into()).into_verdict(policy);
            assert!(
                matches!(v, GateVerdict::Deny { .. }),
                "policy={policy:?} should still deny"
            );
        }
    }

    #[test]
    fn allow_policy_returns_allow_for_transient() {
        let v =
            SynodicGateFailure::Transient("timeout".into()).into_verdict(SynodicFailPolicy::Allow);
        assert!(matches!(v, GateVerdict::Allow));
    }

    // The from_env tests mutate the process-wide environment, which is shared
    // across threads when cargo test runs them in parallel. Serialize them.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
