//! [`SchedulerService`] — long-running task that subscribes to the
//! spine and drives [`TriggerBridge`] on every `trigger.fired` event.
//!
//! The service is the binary's main loop. It:
//!
//! 1. Connects to the spine (sqlx via [`EventStore`]) and to the
//!    persisted [`onsager_substrate::workflow_library::WorkflowLibrary`].
//! 2. Subscribes an unfiltered [`Listener`] with a `with_since`
//!    cursor that starts from the current `events_ext` tail (live
//!    only on a normal restart). The handler filters by `event_type
//!    == "trigger.fired"` after the notification arrives — the
//!    stream-id-prefix filter the listener offers can't be trusted
//!    here because trigger.fired producers don't all share a
//!    stream-id prefix (see `crates/onsager-portal/src/mcp/tools/
//!    workflows.rs`).
//! 3. Dispatches each `trigger.fired` notification through a per-
//!    fire [`TriggerBridge`] scoped to the workflow's
//!    `workspace_id`. Failures log and continue (one bad trigger
//!    does not stop the loop).

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use onsager_nodes::{ExecutorRegistry, NoOpExecutor, PlanStore, SpineClient};
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};
use onsager_substrate::workflow_library::WorkflowLibrary as PersistedWorkflowLibrary;
use sqlx::Row;

use crate::bridge::{PreloadedWorkflow, TriggerBridge, WorkflowMeta};
use crate::plan_registry::{self, derive_plan_id};
use crate::plan_runner::{PlanRunner, SchedulerLibrarySnapshot};
use crate::plan_store::SqlxPlanStore;
use crate::spine_client::SpineEventStoreClient;

/// Configuration the service reads from env / CLI.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// Postgres connection URL — `$DATABASE_URL` in env-driven
    /// deployments.
    pub database_url: String,
    /// Actor stamped on every spine emit. Defaults to
    /// `"substrate-scheduler"`.
    pub actor: String,
    /// When `true`, replay every historical `trigger.fired` event on
    /// startup. Defaults to `false` — the typical container restart
    /// resumes "live only", and replays use `onsager trigger replay`
    /// for surgical re-fires.
    pub replay_history: bool,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL").unwrap_or_default(),
            actor: "substrate-scheduler".to_string(),
            replay_history: false,
        }
    }
}

/// The deployed scheduler service.
pub struct SchedulerService {
    config: ServiceConfig,
    /// Optional executor registry override — production builds use
    /// the default (NoOp + the catalog the binary registers); tests
    /// inject a custom registry via `with_registry`.
    registry: Option<Arc<ExecutorRegistry>>,
}

impl SchedulerService {
    pub fn new(config: ServiceConfig) -> Self {
        Self {
            config,
            registry: None,
        }
    }

    /// Inject a custom [`ExecutorRegistry`]. Used by tests; binary
    /// builds rely on the default registry that
    /// [`default_executor_registry`] returns.
    pub fn with_registry(mut self, registry: Arc<ExecutorRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Connect to the spine, build the bridge, and run the listener
    /// loop. Returns only when the listener loop terminates (the
    /// `pg_notify` channel closes).
    pub async fn run(self) -> anyhow::Result<()> {
        let store = EventStore::connect(&self.config.database_url)
            .await
            .with_context(|| format!("connecting to {}", self.config.database_url))?;
        // Ensure the durable execution-plan tables exist (idempotent;
        // the canonical schema is the spine migration, but the
        // scheduler connects via EventStore which does not migrate).
        crate::migrate::run(store.pool())
            .await
            .context("ensuring execution-plan tables")?;
        let library = PersistedWorkflowLibrary::new(store.pool().clone());
        let base_spine = SpineEventStoreClient::new(store.clone(), &self.config.actor);
        let registry = self
            .registry
            .unwrap_or_else(|| Arc::new(default_executor_registry()));
        let plan_store: Arc<dyn PlanStore> = Arc::new(SqlxPlanStore::new(store.pool().clone()));

        // Recovery: a prior host may have died mid-run. Re-drive every
        // plan still marked `running` — Scheduler::run resets
        // non-terminal node states to pending and re-dispatches
        // (RUN-01 fault-tolerance, B1 #499).
        if let Err(e) =
            recover_running_plans(&store, &registry, &base_spine, &library, &plan_store).await
        {
            tracing::warn!("plan recovery pass failed (continuing to live listen): {e}");
        }

        let handler = TriggerHandler {
            registry,
            base_spine,
            library,
            store: store.clone(),
            plan_store,
        };

        // Default: live-only. `replay_history = true` rewinds to id=0
        // so the listener replays every prior trigger.fired event.
        //
        // The listener seeds `max_events_id` and `max_ext_id` both
        // from `since_id`. The global `EventStore::max_event_id()`
        // returns the GREATEST of the two tables, so if the core
        // `events` table is ahead of `events_ext`, the live loop
        // would silently drop fresh `events_ext` rows with ids less
        // than that global max. trigger.fired lives in `events_ext`,
        // so we read its max directly. (Copilot review on PR #390.)
        let since = if self.config.replay_history {
            Some(0)
        } else {
            events_ext_max_id(&store)
                .await
                .context("reading events_ext cursor")?
        };

        // Production trigger.fired emitters are inconsistent on the
        // stream-id shape — `crates/onsager-portal/src/mcp/tools/
        // workflows.rs` writes `stream_id = workflow_id` (no
        // `workflow:` prefix) while other paths use `workflow:<id>`.
        // The listener's namespace filter matches by `stream_id`
        // prefix only, so a `.subscribe(Namespace::workflow())` call
        // here would drop the MCP-fired ones. Take all notifications
        // and filter by `event_type` inside the handler instead. The
        // body-load + decode path is the existing cost, not a new
        // one. (Copilot review on PR #390.)
        let listener = Listener::new(store).with_since(since);
        tracing::info!(
            actor = %self.config.actor,
            replay = self.config.replay_history,
            "substrate scheduler listening for trigger.fired / plan.run_requested",
        );
        listener.run(handler).await?;
        Ok(())
    }
}

/// Re-drive every plan left in `running` status by a prior host. Each
/// plan's source `SpecPlan` is recompiled against the current library
/// and handed to the shared [`PlanRunner`]; [`Scheduler::run`] resets
/// any non-terminal node state read from the durable store and
/// re-dispatches, so an interrupted multi-node run resumes to
/// completion (B1 #499 / B2 #500).
async fn recover_running_plans(
    store: &EventStore,
    registry: &Arc<ExecutorRegistry>,
    base_spine: &SpineEventStoreClient,
    library: &PersistedWorkflowLibrary,
    plan_store: &Arc<dyn PlanStore>,
) -> anyhow::Result<()> {
    let plans = plan_registry::list_recoverable(store.pool())
        .await
        .context("listing recoverable plans")?;
    if plans.is_empty() {
        return Ok(());
    }
    tracing::info!(
        count = plans.len(),
        "resuming in-flight plans after restart"
    );
    for rec in plans {
        let snapshot = match SchedulerLibrarySnapshot::build(&rec.spec_plan, library).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(plan_id = %rec.plan_id, "recovery library snapshot failed: {e}");
                continue;
            }
        };
        let scoped: Arc<dyn SpineClient> = Arc::new(base_spine.with_workspace(&rec.workspace_id));
        let runner = PlanRunner::new(Arc::clone(registry), Arc::clone(plan_store), scoped);
        match runner.run(&rec.plan_id, &rec.spec_plan, &snapshot).await {
            Ok(()) => {
                let _ = plan_registry::set_status(store.pool(), &rec.plan_id, "completed").await;
                tracing::info!(plan_id = %rec.plan_id, "resumed plan completed");
            }
            Err(e) => {
                let _ = plan_registry::set_status(store.pool(), &rec.plan_id, "failed").await;
                tracing::warn!(plan_id = %rec.plan_id, "resumed plan failed: {e}");
            }
        }
    }
    Ok(())
}

/// Read `MAX(id) FROM events_ext`, returning `None` when the table is
/// empty. Used as the warm-start cursor for the spine listener —
/// `EventStore::max_event_id()` returns the GREATEST across `events`
/// and `events_ext` and is wrong here (see the comment in
/// [`SchedulerService::run`]).
async fn events_ext_max_id(store: &EventStore) -> anyhow::Result<Option<i64>> {
    let row: (Option<i64>,) = sqlx::query_as("SELECT MAX(id) FROM events_ext")
        .fetch_one(store.pool())
        .await?;
    Ok(row.0)
}

/// Build the default registry the deployed binary uses.
///
/// v1 registers only [`NoOpExecutor`]. Script / Verify / Agent /
/// SubWorkflow are wired through the registry by `executor_kind`,
/// but the dispatch path (`onsager_nodes::dispatch`) runs the
/// *registered* instance — not the node's per-instance config (see
/// dispatch.rs and the RUN-02 follow-up note in `crawlab_fixture.rs`).
/// Until per-node config threading lands, registering a singleton
/// ScriptExecutor with no command would dispatch every script node
/// through that empty config — silently wrong. Registering only NoOp
/// means non-NoOp nodes fail with [`ExecutorError::UnknownKind`],
/// which is the correct loud failure; tests inject a richer registry
/// via [`SchedulerService::with_registry`].
pub fn default_executor_registry() -> ExecutorRegistry {
    let mut r = ExecutorRegistry::new();
    r.register(Arc::new(NoOpExecutor));
    r
}

/// Spine [`EventHandler`] that dispatches the scheduler's two front
/// doors — `trigger.fired` (single-spec) and `plan.run_requested`
/// (multi-spec) — through the shared [`PlanRunner`].
struct TriggerHandler {
    registry: Arc<ExecutorRegistry>,
    base_spine: SpineEventStoreClient,
    library: PersistedWorkflowLibrary,
    store: EventStore,
    plan_store: Arc<dyn PlanStore>,
}

#[async_trait]
impl EventHandler for TriggerHandler {
    async fn handle(&self, event: EventNotification) -> anyhow::Result<()> {
        match event.event_type.as_str() {
            "trigger.fired" => self.handle_trigger_fired(event).await,
            "plan.run_requested" => self.handle_plan_run_requested(event).await,
            _ => Ok(()),
        }
    }
}

impl TriggerHandler {
    async fn handle_trigger_fired(&self, event: EventNotification) -> anyhow::Result<()> {
        // Pull the row's data + workspace_id. Notifications carry
        // only stream_id + event_type; the body lives on
        // `events_ext`. workspace_id is the indexed tenant column —
        // we thread it into the per-fire spine client so node
        // lifecycle events emit under the same workspace as the
        // triggering fire (Copilot review on PR #390).
        let row = sqlx::query("SELECT data, workspace_id FROM events_ext WHERE id = $1")
            .bind(event.id)
            .fetch_optional(self.store.pool())
            .await
            .context("loading trigger.fired body")?;
        let Some(row) = row else { return Ok(()) };
        let data: serde_json::Value = row.try_get("data")?;
        let row_workspace_id: String = row.try_get("workspace_id").unwrap_or_default();
        let kind = match decode_trigger_fired(&data) {
            Some(k) => k,
            None => {
                tracing::warn!(
                    event_id = event.id,
                    "trigger.fired body did not decode as TriggerFired"
                );
                return Ok(());
            }
        };
        let FactoryEventKind::TriggerFired {
            workflow_id,
            trigger_kind: _,
            payload,
        } = kind
        else {
            return Ok(());
        };

        let meta = load_workflow_meta(&self.store, &workflow_id).await?;
        let spec_kind = crate::bridge::resolve_spec_kind_for_logging(&payload, &meta);
        let workflow = match spec_kind.as_deref() {
            Some(kind) => self
                .library
                .lookup(kind)
                .await
                .with_context(|| format!("library lookup for kind `{kind}`"))?,
            None => None,
        };
        let lookup = PreloadedWorkflow {
            kind: spec_kind.clone().unwrap_or_default(),
            workflow,
        };
        // Workspace-scope this fire's emits: prefer the value the
        // bridge resolves (workflows.workspace_id is the canonical
        // truth) and fall back to the events_ext row's column. If
        // both are empty we leave the default — the bridge / spine
        // client still emits, just to "default".
        let workspace_id = lookup_workspace(&self.store, &workflow_id)
            .await
            .unwrap_or(None)
            .or_else(|| (!row_workspace_id.is_empty()).then(|| row_workspace_id.clone()))
            .unwrap_or_else(|| "default".to_string());
        let scoped_spine: Arc<dyn SpineClient> =
            Arc::new(self.base_spine.with_workspace(&workspace_id));
        let runner = PlanRunner::new(
            Arc::clone(&self.registry),
            Arc::clone(&self.plan_store),
            scoped_spine,
        );
        let bridge = TriggerBridge::new(runner);
        match bridge
            .handle_payload(&workflow_id, &payload, &meta, lookup)
            .await
        {
            Ok(plan_id) => tracing::info!(
                event_id = event.id,
                %workflow_id,
                %workspace_id,
                plan_id = %plan_id,
                "trigger.fired dispatched",
            ),
            Err(e) => tracing::warn!(
                event_id = event.id,
                %workflow_id,
                "trigger.fired dispatch failed: {e}",
            ),
        }
        Ok(())
    }

    /// Load a persisted multi-spec `SpecPlan`, compile it, and run it
    /// to completion through the shared [`PlanRunner`] (B2 #500 / B3
    /// #501). Portal's `run_spec_plan` tool emitted the
    /// `plan.run_requested` event this consumes.
    async fn handle_plan_run_requested(&self, event: EventNotification) -> anyhow::Result<()> {
        let row = sqlx::query("SELECT data, workspace_id FROM events_ext WHERE id = $1")
            .bind(event.id)
            .fetch_optional(self.store.pool())
            .await
            .context("loading plan.run_requested body")?;
        let Some(row) = row else { return Ok(()) };
        let data: serde_json::Value = row.try_get("data")?;
        let row_workspace_id: String = row.try_get("workspace_id").unwrap_or_default();

        let Some((spec_plan_id, workspace_id)) =
            decode_spec_plan_run_requested(&data, &row_workspace_id)
        else {
            tracing::warn!(
                event_id = event.id,
                "plan.run_requested body did not decode (no spec_plan_id)"
            );
            return Ok(());
        };

        let spec_plan =
            match plan_registry::load_spec_plan(self.store.pool(), &workspace_id, &spec_plan_id)
                .await
                .context("loading stored spec plan")?
            {
                Some(plan) => plan,
                None => {
                    tracing::warn!(
                        event_id = event.id,
                        %workspace_id,
                        %spec_plan_id,
                        "plan.run_requested for unknown spec plan — dropped",
                    );
                    return Ok(());
                }
            };

        let plan_id = derive_plan_id(&workspace_id, &spec_plan_id);
        plan_registry::register_plan(
            self.store.pool(),
            &plan_id,
            &workspace_id,
            Some(&spec_plan_id),
            &spec_plan,
        )
        .await
        .context("registering plan run")?;

        let snapshot = SchedulerLibrarySnapshot::build(&spec_plan, &self.library)
            .await
            .context("building library snapshot")?;
        let scoped_spine: Arc<dyn SpineClient> =
            Arc::new(self.base_spine.with_workspace(&workspace_id));
        let runner = PlanRunner::new(
            Arc::clone(&self.registry),
            Arc::clone(&self.plan_store),
            scoped_spine,
        );
        match runner.run(&plan_id, &spec_plan, &snapshot).await {
            Ok(()) => {
                let _ = plan_registry::set_status(self.store.pool(), &plan_id, "completed").await;
                tracing::info!(
                    event_id = event.id,
                    %workspace_id,
                    %spec_plan_id,
                    plan_id = %plan_id,
                    "plan.run_requested completed",
                );
            }
            Err(e) => {
                let _ = plan_registry::set_status(self.store.pool(), &plan_id, "failed").await;
                tracing::warn!(
                    event_id = event.id,
                    %spec_plan_id,
                    "plan.run_requested failed: {e}",
                );
            }
        }
        Ok(())
    }
}

/// Extract `FactoryEventKind::TriggerFired` from an `events_ext.data`
/// column.
///
/// Three accepted shapes (in priority order — same precedent
/// `onsager_trigger::main::load_trigger_fired` set, plus the raw-
/// payload fallback Copilot flagged on PR #390):
///
/// 1. **`FactoryEvent` envelope** — `onsager-trigger` CLI and
///    `onsager-portal/src/handlers/triggers.rs` write this shape.
/// 2. **Bare `FactoryEventKind`** — historical shape some legacy
///    producers wrote.
/// 3. **Raw trigger payload** — `onsager-portal/src/mcp/tools/
///    workflows.rs` writes the trigger payload object directly,
///    with `workflow_id` (mandatory) and `trigger_kind` (optional,
///    defaults to "unknown") inline. Synthesize a `TriggerFired`
///    variant from that so the scheduler still dispatches.
fn decode_trigger_fired(data: &serde_json::Value) -> Option<FactoryEventKind> {
    if let Ok(env) = serde_json::from_value::<FactoryEvent>(data.clone()) {
        return Some(env.event);
    }
    if let Ok(kind) = serde_json::from_value::<FactoryEventKind>(data.clone()) {
        return Some(kind);
    }
    // Raw payload fallback. The payload itself becomes
    // `TriggerFired::payload` — every emitter convention seen so far
    // (manual fire, MCP run_workflow, telegram webhook) embeds
    // `workflow_id` and an optional `trigger_kind` directly in the
    // payload object, so passing the whole thing back through keeps
    // the bridge's spec_kind resolution working against the same
    // fields.
    let workflow_id = data.get("workflow_id").and_then(|v| v.as_str())?;
    let trigger_kind = data
        .get("trigger_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    Some(FactoryEventKind::TriggerFired {
        workflow_id: workflow_id.to_string(),
        trigger_kind,
        payload: data.clone(),
    })
}

/// Extract `(spec_plan_id, workspace_id)` from a `plan.run_requested`
/// `events_ext.data` column. Accepts the same three shapes as
/// [`decode_trigger_fired`]: a `FactoryEvent` envelope, a bare
/// `FactoryEventKind`, or a raw `{ spec_plan_id, workspace_id }`
/// payload (the shape portal's MCP tool writes). The events_ext row's
/// `workspace_id` column is the fallback when the payload omits it.
fn decode_spec_plan_run_requested(
    data: &serde_json::Value,
    fallback_workspace_id: &str,
) -> Option<(String, String)> {
    let from_kind = serde_json::from_value::<FactoryEvent>(data.clone())
        .map(|env| env.event)
        .ok()
        .or_else(|| serde_json::from_value::<FactoryEventKind>(data.clone()).ok());
    if let Some(FactoryEventKind::SpecPlanRunRequested {
        spec_plan_id,
        workspace_id,
    }) = from_kind
    {
        return Some((spec_plan_id, workspace_id));
    }
    // Raw payload fallback.
    let spec_plan_id = data
        .get("spec_plan_id")
        .and_then(|v| v.as_str())?
        .to_string();
    let workspace_id = data
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_workspace_id)
        .to_string();
    Some((spec_plan_id, workspace_id))
}

/// Pull the workflow row's workspace_id. Returns `Ok(None)` if no
/// such row exists — the caller falls back to the events_ext row's
/// workspace_id column (which is mandatory on append_ext).
async fn lookup_workspace(store: &EventStore, workflow_id: &str) -> anyhow::Result<Option<String>> {
    let row = sqlx::query("SELECT workspace_id FROM workflows WHERE workflow_id = $1")
        .bind(workflow_id)
        .fetch_optional(store.pool())
        .await
        .context("loading workflow workspace_id")?;
    Ok(row.and_then(|r| r.try_get::<String, _>("workspace_id").ok()))
}

/// Pull the workflow row's preset_id so the bridge can fall back to
/// it. A missing workflow row produces an empty `WorkflowMeta`; the
/// bridge then surfaces `UnresolvedSpecKind` to the caller.
async fn load_workflow_meta(store: &EventStore, workflow_id: &str) -> anyhow::Result<WorkflowMeta> {
    let row = sqlx::query("SELECT preset_id FROM workflows WHERE workflow_id = $1")
        .bind(workflow_id)
        .fetch_optional(store.pool())
        .await
        .context("loading workflow meta")?;
    let Some(row) = row else {
        return Ok(WorkflowMeta::default());
    };
    let preset_id: Option<String> = row.try_get("preset_id").ok();
    Ok(WorkflowMeta { preset_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_handles_factory_event_envelope() {
        let env = FactoryEvent {
            event: FactoryEventKind::TriggerFired {
                workflow_id: "wf-1".into(),
                trigger_kind: "manual".into(),
                payload: serde_json::json!({"hello": "world"}),
            },
            correlation_id: None,
            causation_id: None,
            actor: "test".into(),
            timestamp: chrono::Utc::now(),
        };
        let data = serde_json::to_value(&env).unwrap();
        let kind = decode_trigger_fired(&data).expect("envelope decodes");
        assert!(matches!(
            kind,
            FactoryEventKind::TriggerFired { ref workflow_id, .. } if workflow_id == "wf-1",
        ));
    }

    #[test]
    fn decode_handles_bare_kind() {
        let kind = FactoryEventKind::TriggerFired {
            workflow_id: "wf-2".into(),
            trigger_kind: "github_issue_webhook".into(),
            payload: serde_json::json!({}),
        };
        let data = serde_json::to_value(&kind).unwrap();
        let decoded = decode_trigger_fired(&data).expect("bare kind decodes");
        assert!(matches!(
            decoded,
            FactoryEventKind::TriggerFired { ref workflow_id, .. } if workflow_id == "wf-2",
        ));
    }

    /// The MCP `run_workflow` producer
    /// (`crates/onsager-portal/src/mcp/tools/workflows.rs`) writes a
    /// raw payload object directly with `workflow_id` inline. Without
    /// this fallback the scheduler logged "did not decode" and
    /// dropped the fire (Copilot review on PR #390).
    #[test]
    fn decode_falls_back_to_raw_payload() {
        let raw = serde_json::json!({
            "workflow_id": "wf-mcp",
            "trigger_kind": "manual",
            "workspace_id": "ws-7",
            "name": "run-it",
            "actor": "mcp-client",
        });
        let decoded = decode_trigger_fired(&raw).expect("raw payload decodes");
        let FactoryEventKind::TriggerFired {
            workflow_id,
            trigger_kind,
            payload,
        } = decoded
        else {
            panic!("expected TriggerFired");
        };
        assert_eq!(workflow_id, "wf-mcp");
        assert_eq!(trigger_kind, "manual");
        // The whole raw object survives as `payload` so the bridge's
        // resolve_spec_kind / spec_kind lookup can read its fields.
        assert_eq!(
            payload.get("workspace_id").and_then(|v| v.as_str()),
            Some("ws-7")
        );
    }

    #[test]
    fn decode_returns_none_for_payload_without_workflow_id() {
        let data = serde_json::json!({ "not_a_workflow": "nope" });
        assert!(decode_trigger_fired(&data).is_none());
    }

    #[test]
    fn decode_plan_run_requested_from_raw_payload() {
        // The shape portal's `run_spec_plan` MCP tool writes.
        let data = serde_json::json!({
            "spec_plan_id": "github:42",
            "workspace_id": "ws-7",
            "actor": "mcp-client",
            "source": "mcp",
        });
        let (spec_plan_id, workspace_id) =
            decode_spec_plan_run_requested(&data, "fallback").expect("decodes");
        assert_eq!(spec_plan_id, "github:42");
        assert_eq!(workspace_id, "ws-7");
    }

    #[test]
    fn decode_plan_run_requested_falls_back_to_row_workspace() {
        let data = serde_json::json!({ "spec_plan_id": "github:42" });
        let (spec_plan_id, workspace_id) =
            decode_spec_plan_run_requested(&data, "row-ws").expect("decodes");
        assert_eq!(spec_plan_id, "github:42");
        assert_eq!(
            workspace_id, "row-ws",
            "missing workspace falls back to the row column"
        );
    }

    #[test]
    fn decode_plan_run_requested_from_bare_kind() {
        let kind = FactoryEventKind::SpecPlanRunRequested {
            spec_plan_id: "github:99".into(),
            workspace_id: "ws-9".into(),
        };
        let data = serde_json::to_value(&kind).unwrap();
        let (spec_plan_id, workspace_id) =
            decode_spec_plan_run_requested(&data, "fallback").expect("decodes");
        assert_eq!(spec_plan_id, "github:99");
        assert_eq!(workspace_id, "ws-9");
    }

    #[test]
    fn decode_plan_run_requested_none_without_spec_plan_id() {
        let data = serde_json::json!({ "workspace_id": "ws" });
        assert!(decode_spec_plan_run_requested(&data, "fallback").is_none());
    }
}
