//! [`SchedulerService`] — long-running task that subscribes to the
//! spine and runs a plan on every `plan.run_requested` event (the one
//! run entry kind post-ADR 0028 / #601; portal converts trigger fires).
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

use crate::nodes::{AgentExecutor, ExecutorRegistry, NoOpExecutor, PlanStore, SpineClient};
use anyhow::Context;
use async_trait::async_trait;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};
use onsager_substrate::workflow_library::WorkflowLibrary as PersistedWorkflowLibrary;
use sqlx::Row;

use crate::scheduler::plan_registry::{self, derive_plan_id};
use crate::scheduler::plan_runner::{PlanRunner, SchedulerLibrarySnapshot, resolve_kind_versions};
use crate::scheduler::plan_store::SqlxPlanStore;
use crate::scheduler::runners::ClaudeCliRunner;
use crate::scheduler::spine_client::SpineEventStoreClient;

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
    /// AES-256-GCM credential key (hex), shared with portal / stiglab.
    /// Used to decrypt the plan-scoped session token (#536) off the
    /// `plan.run_requested` event so agent nodes can be injected with
    /// `ONSAGER_SESSION_TOKEN`. `None` disables the inject path — the
    /// plan still runs, agents just lack MCP auth (the authoritative
    /// liveness gate still applies). Read from `ONSAGER_CREDENTIAL_KEY`.
    pub credential_key: Option<String>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL").unwrap_or_default(),
            actor: "substrate-scheduler".to_string(),
            replay_history: false,
            credential_key: std::env::var("ONSAGER_CREDENTIAL_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
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
        crate::scheduler::migrate::run(store.pool())
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
            credential_key: self.config.credential_key.clone(),
            actor: self.config.actor.clone(),
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
            "substrate scheduler listening for plan.run_requested",
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
        let snapshot = match SchedulerLibrarySnapshot::build(
            &rec.spec_plan,
            library,
            &rec.kind_versions,
        )
        .await
        {
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
/// Registers [`NoOpExecutor`] and the production [`AgentExecutor`]
/// backed by [`ClaudeCliRunner`] (spec #537, closing #533's headline
/// loop). Executors are wired through the registry by `executor_kind`,
/// and the dispatch path (`crate::nodes::dispatch`) runs the
/// *registered* instance. Per-node config reaches that instance through
/// the `ExecutorContext` escape hatch the scheduler threads off each
/// substrate executor: `subworkflow_ref` for SubWorkflow (#357) and
/// `agent_config` for Agent (#534). The registered singleton
/// `AgentExecutor` therefore dispatches each agent node with *its*
/// model / prompt / tools / credential — its own placeholder fields
/// (empty model / prompt) are never used on the scheduler path. The
/// plan-scoped session token (#536) is threaded separately onto the
/// runner request for `ONSAGER_SESSION_TOKEN` injection.
///
/// Script / Verify per-node config is *not* yet threaded (those keep
/// only the compile-side round-trip assertion), so registering a
/// singleton `ScriptExecutor` with no command would still dispatch
/// every script node through that empty config — silently wrong. They
/// stay unregistered; a script / verify node therefore fails with
/// [`crate::nodes::ExecutorError::UnknownKind`], the correct loud
/// failure, until their threading lands. Tests inject a richer or
/// narrower registry via [`SchedulerService::with_registry`].
pub fn default_executor_registry() -> ExecutorRegistry {
    let mut r = ExecutorRegistry::new();
    r.register(Arc::new(NoOpExecutor));
    // Agent: one runtime singleton backed by the production
    // `ClaudeCliRunner`. The placeholder model / prompt are overridden
    // per node by `ExecutorContext::agent_config` (#534); the runner
    // binary is `claude` on PATH, overridable via `ONSAGER_CLAUDE_BIN`.
    r.register(Arc::new(
        AgentExecutor::new("", "").with_runner(Arc::new(ClaudeCliRunner::new())),
    ));
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
    /// Shared credential key (hex) for decrypting the plan-scoped
    /// session token off `plan.run_requested` (#536). `None` disables
    /// token injection.
    credential_key: Option<String>,
    /// Configured actor (`ServiceConfig.actor`, via `SCHEDULER_ACTOR`)
    /// stamped onto spine emits this handler makes directly — e.g. the
    /// plan-terminal events in [`Self::emit_plan_terminal`]. Mirrors the
    /// actor `base_spine` was built with.
    actor: String,
}

#[async_trait]
impl EventHandler for TriggerHandler {
    async fn handle(&self, event: EventNotification) -> anyhow::Result<()> {
        match event.event_type.as_str() {
            "plan.run_requested" => self.handle_plan_run_requested(event).await,
            _ => Ok(()),
        }
    }
}

impl TriggerHandler {
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

        // Decrypt the plan-scoped session token (#536) once per plan and
        // thread the plaintext to the runner; the AgentExecutor injects
        // it as `ONSAGER_SESSION_TOKEN` into every agent node. The token
        // rides the event encrypted (`encrypted_session_token`); a
        // missing token or absent credential key is non-fatal — the plan
        // runs without MCP auth and the authoritative liveness gate
        // still applies (mirrors stiglab's chat-path degradation).
        let session_token = decode_plan_session_token(&data)
            .zip(self.credential_key.as_deref())
            .and_then(
                |(enc, key)| match crate::scheduler::crypto::decrypt_credential(key, &enc) {
                    Ok(token) => Some(token),
                    Err(e) => {
                        tracing::error!(
                            plan_id = %plan_id,
                            "failed to decrypt plan session token: {e}"
                        );
                        None
                    }
                },
            );

        // Resolve the workspace's bound repo set (#555) the same way the
        // session token is resolved: portal attached it (read tokens
        // encrypted) to the event, the scheduler decrypts each and builds
        // the `ONSAGER_REPOS` value once per plan, then threads it to the
        // runner; the AgentExecutor injects it into every agent node. Only
        // an empty set (no bound repos) injects nothing, leaving
        // single-`working_dir` runs unchanged. A missing/rotated
        // credential key does NOT drop the set — each repo degrades to an
        // unauthenticated clone URL (still surfaced), so a private repo
        // fails loudly at clone time rather than vanishing silently
        // (matches `repo_env::repos_env_from_access` + stiglab). This is
        // the scheduler-path parity #555 closes: spec-plan runs now span
        // the workspace's repos, not just the one the chat path got.
        let repos_env = crate::scheduler::repo_env::repos_env_from_access(
            &decode_plan_repos(&data),
            self.credential_key.as_deref(),
        );

        // Pin the workflow-library versions this run compiles against so
        // a later recovery recompiles against the same versions, not
        // whatever is latest then (#511). Resolved once and shared with
        // both the persisted pin and the live snapshot so they cannot
        // diverge.
        let kind_versions = resolve_kind_versions(&spec_plan, &self.library)
            .await
            .context("resolving workflow-library versions")?;
        plan_registry::register_plan(
            self.store.pool(),
            &plan_id,
            &workspace_id,
            Some(&spec_plan_id),
            &spec_plan,
            &kind_versions,
        )
        .await
        .context("registering plan run")?;

        let snapshot = SchedulerLibrarySnapshot::build(&spec_plan, &self.library, &kind_versions)
            .await
            .context("building library snapshot")?;
        let scoped_spine: Arc<dyn SpineClient> =
            Arc::new(self.base_spine.with_workspace(&workspace_id));
        let runner = PlanRunner::new(
            Arc::clone(&self.registry),
            Arc::clone(&self.plan_store),
            scoped_spine,
        )
        .with_session_token(session_token)
        .with_repos_env(repos_env);
        match runner.run(&plan_id, &spec_plan, &snapshot).await {
            Ok(()) => {
                let _ = plan_registry::set_status(self.store.pool(), &plan_id, "completed").await;
                self.emit_plan_terminal(&workspace_id, plan_id.as_str(), &spec_plan_id, true)
                    .await;
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
                self.emit_plan_terminal(&workspace_id, plan_id.as_str(), &spec_plan_id, false)
                    .await;
                tracing::warn!(
                    event_id = event.id,
                    %spec_plan_id,
                    "plan.run_requested failed: {e}",
                );
            }
        }
        Ok(())
    }

    /// Emit the plan-terminal spine event (#536) so portal's revoke
    /// listener can soft-revoke the plan-scoped session token the moment
    /// the run finishes. `completed` selects
    /// `plan.run_completed` vs `plan.run_failed`. Best-effort: a failed
    /// emit logs and is not propagated — the token's expiry backstop
    /// still closes the window, this just tightens it.
    async fn emit_plan_terminal(
        &self,
        workspace_id: &str,
        plan_id: &str,
        spec_plan_id: &str,
        completed: bool,
    ) {
        let kind = if completed {
            "plan.run_completed"
        } else {
            "plan.run_failed"
        };
        let payload = serde_json::json!({
            "plan_id": plan_id,
            "workspace_id": workspace_id,
            "spec_plan_id": spec_plan_id,
        });
        let metadata = onsager_spine::EventMetadata {
            correlation_id: None,
            causation_id: None,
            actor: self.actor.clone(),
        };
        let stream_id = format!("plan:{plan_id}");
        if let Err(e) = self
            .store
            .append_ext(
                workspace_id,
                &stream_id,
                "plan",
                kind,
                payload,
                &metadata,
                None,
            )
            .await
        {
            tracing::warn!(%plan_id, kind, "failed to emit plan-terminal event: {e}");
        }
    }
}

/// Pull the `encrypted_session_token` (#536) off a `plan.run_requested`
/// body. Accepts the raw-payload shape portal's `run_spec_plan` MCP tool
/// writes (the field lives at the top level alongside `spec_plan_id`).
/// `None` when the field is absent (older producers, or no credential
/// key was configured at mint time).
fn decode_plan_session_token(data: &serde_json::Value) -> Option<String> {
    data.get("encrypted_session_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Pull the workspace's bound repo set (#555) off a `plan.run_requested`
/// body. Portal's `run_spec_plan` MCP tool attaches it as an additive
/// top-level `repos` array — one [`onsager_spine::protocol::RepoAccess`]
/// per bound repo, read tokens encrypted with the shared credential key,
/// alongside `encrypted_session_token`. Empty when the field is absent
/// (older producers, or the workspace binds no repos) or does not parse,
/// so the single-`working_dir` path stays unchanged.
fn decode_plan_repos(data: &serde_json::Value) -> Vec<onsager_spine::protocol::RepoAccess> {
    data.get("repos")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Extract `(spec_plan_id, workspace_id)` from a `plan.run_requested`
/// `events_ext.data` column. Accepts three shapes: a `FactoryEvent`
/// envelope, a bare
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

#[cfg(test)]
mod tests {
    use super::*;

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

    // -----------------------------------------------------------------
    // Spec #536 — plan-scoped session token decrypt + inject seam.
    // -----------------------------------------------------------------

    #[test]
    fn decode_plan_session_token_reads_top_level_field() {
        let data = serde_json::json!({
            "spec_plan_id": "github:42",
            "workspace_id": "ws-7",
            "encrypted_session_token": "deadbeefcafe",
        });
        assert_eq!(
            decode_plan_session_token(&data).as_deref(),
            Some("deadbeefcafe")
        );
    }

    #[test]
    fn decode_plan_session_token_none_when_absent_or_empty() {
        assert!(decode_plan_session_token(&serde_json::json!({ "spec_plan_id": "x" })).is_none());
        assert!(
            decode_plan_session_token(&serde_json::json!({ "encrypted_session_token": "" }))
                .is_none()
        );
    }

    // -----------------------------------------------------------------
    // Spec #555 — workspace repo set decode (the transport layer of the
    // scheduler-path ONSAGER_REPOS parity).
    // -----------------------------------------------------------------

    #[test]
    fn decode_plan_repos_reads_top_level_array() {
        // Portal attaches the workspace repo set as a top-level `repos`
        // array, decoded here alongside the session token. Mutation
        // guard: drop the `repos` key and the length assertion fails.
        let data = serde_json::json!({
            "spec_plan_id": "github:42",
            "workspace_id": "ws-7",
            "repos": [
                { "owner": "acme", "name": "widgets", "default_branch": "main",
                  "encrypted_read_token": "deadbeef" },
                { "owner": "acme", "name": "gadgets", "default_branch": "trunk" },
            ],
        });
        let repos = decode_plan_repos(&data);
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "widgets");
        assert_eq!(repos[0].encrypted_read_token.as_deref(), Some("deadbeef"));
        // A tokenless entry (public repo) survives — surfaced by identity.
        assert_eq!(repos[1].encrypted_read_token, None);
    }

    #[test]
    fn decode_plan_repos_empty_when_absent_or_malformed() {
        // Absent `repos` (older producer / no bound repos) → empty set,
        // so the single-`working_dir` path is unchanged.
        assert!(decode_plan_repos(&serde_json::json!({ "spec_plan_id": "x" })).is_empty());
        // A non-array `repos` does not parse → empty, never a panic.
        assert!(decode_plan_repos(&serde_json::json!({ "repos": "nope" })).is_empty());
    }

    /// End-to-end of the inject seam #536 wires up: portal's
    /// `encrypted_session_token` on the event → scheduler decrypt →
    /// the plaintext that gets threaded onto the `PlanRunner` /
    /// `Scheduler` / `ExecutorContext` for `ONSAGER_SESSION_TOKEN`
    /// injection. Encrypts with the same AES-256-GCM wire format portal
    /// uses, then asserts `decode_plan_session_token` + `decrypt` round-
    /// trips to the original token.
    ///
    /// Mutation guard: break the `.zip(self.credential_key)` /
    /// `decrypt_credential` line in `handle_plan_run_requested` and the
    /// production path no longer injects — this asserts the two helpers
    /// the production path composes do reconstruct the token.
    #[test]
    fn plan_session_token_decrypts_off_event_body() {
        use ring::aead;
        use ring::rand::{SecureRandom, SystemRandom};

        // Mirror portal's `encrypt_credential` to produce the ciphertext
        // the scheduler will decrypt (portal is the real producer).
        let rng = SystemRandom::new();
        let mut key = [0u8; 32];
        rng.fill(&mut key).unwrap();
        let key_hex = hex::encode(key);

        let plaintext = "ons_pat_plan_scoped_token";
        let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key).unwrap();
        let sealing = aead::LessSafeKey::new(unbound);
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes).unwrap();
        let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = plaintext.as_bytes().to_vec();
        sealing
            .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut in_out)
            .unwrap();
        let mut wire = nonce_bytes.to_vec();
        wire.extend_from_slice(&in_out);
        let encrypted_hex = hex::encode(wire);

        let event_body = serde_json::json!({
            "spec_plan_id": "github:42",
            "workspace_id": "ws-7",
            "plan_id": "ws-7:github:42",
            "encrypted_session_token": encrypted_hex,
        });

        // The exact composition `handle_plan_run_requested` runs.
        let injected = decode_plan_session_token(&event_body)
            .zip(Some(key_hex.as_str()))
            .and_then(|(enc, k)| crate::scheduler::crypto::decrypt_credential(k, &enc).ok());

        assert_eq!(
            injected.as_deref(),
            Some(plaintext),
            "scheduler must decrypt the plan-scoped token off the event body"
        );
    }
}
