//! [`SchedulerService`] — long-running task that subscribes to the
//! spine and drives [`TriggerBridge`] on every `trigger.fired` event.
//!
//! The service is the binary's main loop. It:
//!
//! 1. Connects to the spine (sqlx via [`EventStore`]) and to the
//!    persisted [`onsager_substrate::workflow_library::WorkflowLibrary`].
//! 2. Subscribes a [`Listener`] to the `workflow` namespace (where
//!    `trigger.fired` events stream) with a `with_since` cursor that
//!    starts from the current tail — production replays prior fires
//!    only when explicitly asked, not on every restart.
//! 3. Dispatches each `trigger.fired` notification through
//!    [`TriggerBridge::handle_payload`]. Failures log and continue
//!    (one bad trigger does not stop the loop).

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use onsager_nodes::{ExecutorRegistry, NoOpExecutor, SpineClient};
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener, Namespace};
use onsager_substrate::workflow_library::WorkflowLibrary as PersistedWorkflowLibrary;
use sqlx::Row;

use crate::bridge::{PreloadedWorkflow, TriggerBridge, WorkflowMeta};
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
        let library = PersistedWorkflowLibrary::new(store.pool().clone());
        let spine: Arc<dyn SpineClient> = Arc::new(SpineEventStoreClient::new(
            store.clone(),
            &self.config.actor,
        ));
        let registry = self
            .registry
            .unwrap_or_else(|| Arc::new(default_executor_registry()));
        let bridge = TriggerBridge::new(registry, spine);
        let handler = TriggerHandler {
            bridge,
            library,
            store: store.clone(),
        };

        // Default: live-only. `replay_history = true` rewinds to id=0
        // so the listener replays every prior trigger.fired event.
        let since = if self.config.replay_history {
            Some(0)
        } else {
            store.max_event_id().await.context("reading spine cursor")?
        };

        let listener = Listener::new(store)
            .subscribe(Namespace::workflow())
            .with_since(since);
        tracing::info!(
            actor = %self.config.actor,
            replay = self.config.replay_history,
            "substrate scheduler listening for trigger.fired",
        );
        listener.run(handler).await?;
        Ok(())
    }
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

/// Spine [`EventHandler`] that dispatches `trigger.fired` events
/// through the bridge.
struct TriggerHandler {
    bridge: TriggerBridge,
    library: PersistedWorkflowLibrary,
    store: EventStore,
}

#[async_trait]
impl EventHandler for TriggerHandler {
    async fn handle(&self, event: EventNotification) -> anyhow::Result<()> {
        if event.event_type != "trigger.fired" {
            return Ok(());
        }
        // Pull the row's data column so we can decode the typed
        // payload. Notifications carry only stream_id + event_type;
        // the body lives on `events_ext`.
        let row = sqlx::query("SELECT data FROM events_ext WHERE id = $1")
            .bind(event.id)
            .fetch_optional(self.store.pool())
            .await
            .context("loading trigger.fired body")?;
        let Some(row) = row else { return Ok(()) };
        let data: serde_json::Value = row.try_get("data")?;
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
        match self
            .bridge
            .handle_payload(&workflow_id, &payload, &meta, lookup)
            .await
        {
            Ok(plan_id) => tracing::info!(
                event_id = event.id,
                %workflow_id,
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
}

/// Extract `FactoryEventKind::TriggerFired` from an `events_ext.data`
/// column. The column may carry either the bare kind or the
/// [`FactoryEvent`] envelope (per `onsager_trigger`'s precedent).
fn decode_trigger_fired(data: &serde_json::Value) -> Option<FactoryEventKind> {
    if let Ok(env) = serde_json::from_value::<FactoryEvent>(data.clone()) {
        return Some(env.event);
    }
    serde_json::from_value::<FactoryEventKind>(data.clone()).ok()
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
