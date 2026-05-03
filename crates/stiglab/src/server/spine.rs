//! Optional event spine integration for emitting factory events to the
//! Onsager event store.

use onsager_spine::factory_event::{FactoryEventKind, TokenUsage};
use onsager_spine::{EventMetadata, EventStore};

/// Default workspace_id stamped on `events_ext` rows whose source event
/// has no resolvable tenant (system telemetry — node lifecycle, idle
/// ticks, catch-up completions). Per #183 the column is NOT NULL; this
/// is the canonical fallback for cross-workspace events.
const SYSTEM_WORKSPACE: &str = "default";

/// Emits factory events to the Onsager event spine under the "stiglab" namespace.
#[derive(Clone)]
pub struct SpineEmitter {
    store: EventStore,
}

impl SpineEmitter {
    /// Connect to the Onsager event store at the given database URL.
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let store = EventStore::connect(database_url).await?;
        Ok(Self { store })
    }

    /// Emit a factory event to the extension event table under the "stiglab"
    /// namespace. Returns the assigned event ID.
    pub async fn emit(&self, event: FactoryEventKind) -> Result<i64, sqlx::Error> {
        let metadata = EventMetadata {
            correlation_id: None,
            causation_id: None,
            actor: "stiglab".to_string(),
        };
        let data = serde_json::to_value(&event).unwrap_or_default();
        let stream_id = event.stream_id();
        let event_type = event.event_type();
        let workspace_id = self.resolve_workspace(&event).await;

        self.store
            .append_ext(
                &workspace_id,
                &stream_id,
                "stiglab",
                event_type,
                data,
                &metadata,
                None,
            )
            .await
    }

    /// Resolve the tenant scope for a factory event (#183). Per-variant
    /// match lives here so every call site shares the same policy:
    /// look up the artifact's workspace when the event names one,
    /// fall back to `"default"` for system events that genuinely have
    /// no tenant (node lifecycle, idle ticks, etc.).
    ///
    /// Lookup errors are logged distinctly from `Ok(None)` so a real
    /// DB problem doesn't silently mis-scope every event into the
    /// default workspace (Copilot review on #235).
    async fn resolve_workspace(&self, event: &FactoryEventKind) -> String {
        let Some(artifact_id) = artifact_id_for_workspace_lookup(event) else {
            return SYSTEM_WORKSPACE.to_string();
        };
        match self.store.lookup_workspace_for_artifact(artifact_id).await {
            Ok(Some(ws)) => ws,
            Ok(None) => SYSTEM_WORKSPACE.to_string(),
            Err(e) => {
                tracing::warn!(
                    artifact_id,
                    event_type = event.event_type(),
                    "spine workspace lookup failed; falling back to {SYSTEM_WORKSPACE}: {e}"
                );
                SYSTEM_WORKSPACE.to_string()
            }
        }
    }

    /// Get a reference to the underlying PostgreSQL pool for direct queries.
    pub fn pool(&self) -> &sqlx::PgPool {
        self.store.pool()
    }

    /// Clone the underlying [`EventStore`] handle so listener tasks can
    /// own a handle without borrowing through the emitter. `EventStore`
    /// is internally `Clone` over the `PgPool` it wraps, so this is
    /// cheap (one `Arc` clone per pool).
    pub fn store_clone(&self) -> EventStore {
        self.store.clone()
    }

    /// Emit a raw event to the extension event table under a given namespace.
    /// Used for events that don't map to a `FactoryEventKind` variant (e.g.,
    /// artifact registration from the dashboard).
    ///
    /// `workspace_id` (#183) is the tenant scope; pass `"default"` for
    /// system events with no tenant. `namespace` identifies the event
    /// store partition; `actor` identifies the service or user that
    /// originated the event.
    pub async fn emit_raw(
        &self,
        workspace_id: &str,
        stream_id: &str,
        namespace: &str,
        actor: &str,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<i64, sqlx::Error> {
        let metadata = EventMetadata {
            correlation_id: None,
            causation_id: None,
            actor: actor.to_string(),
        };
        self.store
            .append_ext(
                workspace_id,
                stream_id,
                namespace,
                event_type,
                data.clone(),
                &metadata,
                None,
            )
            .await
    }

    /// Emit a session-started event.
    pub async fn emit_session_started(
        &self,
        session_id: &str,
        request_id: &str,
        node_id: &str,
    ) -> Result<i64, sqlx::Error> {
        self.emit(FactoryEventKind::StiglabSessionCreated {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            node_id: node_id.to_string(),
        })
        .await
    }

    /// Emit a session-completed event.
    ///
    /// `artifact_id` links the session to the factory pipeline artifact it
    /// was shaping (issue #14 phase 2). Pass `None` for sessions that don't
    /// originate from a `ShapingRequest`. `token_usage` is the LLM accounting
    /// payload (issue #39); pass `None` when the runtime can't report it.
    /// `branch` and `pr_number` (issue #60) carry the agent's git context
    /// when available — used by `onsager-portal` for vertical lineage.
    #[allow(clippy::too_many_arguments)]
    pub async fn emit_session_completed(
        &self,
        session_id: &str,
        request_id: &str,
        duration_ms: u64,
        artifact_id: Option<&str>,
        token_usage: Option<TokenUsage>,
        branch: Option<&str>,
        pr_number: Option<u64>,
    ) -> Result<i64, sqlx::Error> {
        self.emit(FactoryEventKind::StiglabSessionCompleted {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            duration_ms,
            artifact_id: artifact_id.map(str::to_owned),
            token_usage,
            branch: branch.map(str::to_owned),
            pr_number,
        })
        .await
    }

    /// Emit a `stiglab.shaping_result_ready` event carrying the full
    /// `ShapingResult` payload so Forge's `shaping_result_listener` can
    /// resume the parked pipeline decision (spec #131 / ADR 0004
    /// Lever C, phase 3). Emitted alongside `stiglab.session_completed`:
    /// the lifecycle event signals "this session finished" (used by the
    /// dashboard); this event signals "the artifact outputs are ready
    /// for the next pipeline stage" (used by Forge's state machine).
    /// Sessions without an artifact link skip this — see
    /// `handler.rs::handle_agent_message` for the gate.
    pub async fn emit_shaping_result_ready(
        &self,
        artifact_id: onsager_artifact::ArtifactId,
        result: onsager_spine::protocol::ShapingResult,
    ) -> Result<i64, sqlx::Error> {
        self.emit(FactoryEventKind::StiglabShapingResultReady {
            artifact_id,
            result,
        })
        .await
    }

    /// Emit a session-failed event.
    ///
    /// `artifact_id` links the failure to the factory pipeline artifact
    /// the session was shaping (issue #156). Pass `None` for direct task
    /// POSTs that don't originate from a `ShapingRequest`. When `Some`,
    /// forge's workflow signal listener writes a `Failure` outcome to
    /// the agent-session signal cache so the gate fails loudly and the
    /// artifact stops re-dispatching.
    pub async fn emit_session_failed(
        &self,
        session_id: &str,
        request_id: &str,
        error: &str,
        artifact_id: Option<&str>,
    ) -> Result<i64, sqlx::Error> {
        self.emit(FactoryEventKind::StiglabSessionFailed {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            error: error.to_string(),
            artifact_id: artifact_id.map(str::to_owned),
        })
        .await
    }
}

/// Extract the artifact_id a `FactoryEventKind` is scoped to, for
/// `events_ext.workspace_id` resolution (#183). Returns `None` for
/// system events that genuinely have no tenant — node lifecycle,
/// catch-up completions, idle ticks — those land under `"default"`.
fn artifact_id_for_workspace_lookup(event: &FactoryEventKind) -> Option<&str> {
    match event {
        FactoryEventKind::ArtifactRegistered { artifact_id, .. }
        | FactoryEventKind::ArtifactStateChanged { artifact_id, .. }
        | FactoryEventKind::ArtifactVersionCreated { artifact_id, .. }
        | FactoryEventKind::ArtifactLineageExtended { artifact_id, .. }
        | FactoryEventKind::ArtifactQualityRecorded { artifact_id, .. }
        | FactoryEventKind::ArtifactRouted { artifact_id, .. }
        | FactoryEventKind::ArtifactArchived { artifact_id, .. }
        | FactoryEventKind::BundleSealed { artifact_id, .. }
        | FactoryEventKind::GitBranchCreated { artifact_id, .. }
        | FactoryEventKind::GitCommitPushed { artifact_id, .. }
        | FactoryEventKind::GitPrOpened { artifact_id, .. }
        | FactoryEventKind::GitPrReviewReceived { artifact_id, .. }
        | FactoryEventKind::GitCiCompleted { artifact_id, .. }
        | FactoryEventKind::GitPrMerged { artifact_id, .. }
        | FactoryEventKind::GitPrClosed { artifact_id, .. }
        | FactoryEventKind::ForgeShapingDispatched { artifact_id, .. }
        | FactoryEventKind::ForgeShapingReturned { artifact_id, .. }
        | FactoryEventKind::ForgeGateRequested { artifact_id, .. }
        | FactoryEventKind::ForgeGateVerdict { artifact_id, .. }
        | FactoryEventKind::ForgeDecisionMade { artifact_id, .. }
        | FactoryEventKind::StiglabShapingResultReady { artifact_id, .. }
        | FactoryEventKind::SynodicGateEvaluated { artifact_id, .. }
        | FactoryEventKind::SynodicGateDenied { artifact_id, .. }
        | FactoryEventKind::SynodicGateModified { artifact_id, .. }
        | FactoryEventKind::SynodicGateVerdict { artifact_id, .. }
        | FactoryEventKind::SynodicEscalationStarted { artifact_id, .. }
        | FactoryEventKind::SynodicEscalationResolved { artifact_id, .. }
        | FactoryEventKind::SynodicEscalationTimedOut { artifact_id, .. }
        | FactoryEventKind::SynodicGateResolutionProposed { artifact_id, .. }
        | FactoryEventKind::DeliverableUpdated { artifact_id, .. }
        | FactoryEventKind::StageEntered { artifact_id, .. }
        | FactoryEventKind::StageGatePassed { artifact_id, .. }
        | FactoryEventKind::StageGateFailed { artifact_id, .. }
        | FactoryEventKind::StageAdvanced { artifact_id, .. } => Some(artifact_id.as_str()),
        FactoryEventKind::StiglabSessionCompleted { artifact_id, .. }
        | FactoryEventKind::StiglabSessionFailed { artifact_id, .. } => artifact_id.as_deref(),
        // Variants below name no artifact — system / cross-workspace
        // telemetry. Fall back to "default".
        FactoryEventKind::DeliverySucceeded { .. }
        | FactoryEventKind::DeliveryFailed { .. }
        | FactoryEventKind::DeliverableCreated { .. }
        | FactoryEventKind::ForgeInsightObserved { .. }
        | FactoryEventKind::ForgeIdleTick
        | FactoryEventKind::ForgeStateChanged { .. }
        | FactoryEventKind::StiglabSessionCreated { .. }
        | FactoryEventKind::StiglabSessionDispatched { .. }
        | FactoryEventKind::StiglabSessionRunning { .. }
        | FactoryEventKind::StiglabSessionAborted { .. }
        | FactoryEventKind::StiglabEventUpgraded { .. }
        | FactoryEventKind::StiglabNodeRegistered { .. }
        | FactoryEventKind::StiglabNodeDeregistered { .. }
        | FactoryEventKind::StiglabNodeHeartbeatMissed { .. }
        | FactoryEventKind::SynodicRuleProposed { .. }
        | FactoryEventKind::SynodicRuleApproved { .. }
        | FactoryEventKind::SynodicRuleDisabled { .. }
        | FactoryEventKind::SynodicRuleVersionCreated { .. }
        | FactoryEventKind::IsingInsightDetected { .. }
        | FactoryEventKind::IsingInsightEmitted { .. }
        | FactoryEventKind::IsingInsightSuppressed { .. }
        | FactoryEventKind::IsingRuleProposed { .. }
        | FactoryEventKind::IsingAnalyzerError { .. }
        | FactoryEventKind::IsingCatchupCompleted { .. }
        | FactoryEventKind::IntentSubmitted { .. }
        | FactoryEventKind::RefractDecomposed { .. }
        | FactoryEventKind::RefractFailed { .. }
        | FactoryEventKind::TriggerFired { .. }
        | FactoryEventKind::TypeProposed { .. }
        | FactoryEventKind::TypeApproved { .. }
        | FactoryEventKind::TypeDeprecated { .. }
        | FactoryEventKind::AdapterRegistered { .. }
        | FactoryEventKind::AdapterDeprecated { .. }
        | FactoryEventKind::GateRegistered { .. }
        | FactoryEventKind::GateDeprecated { .. }
        | FactoryEventKind::ProfileRegistered { .. }
        | FactoryEventKind::ProfileDeprecated { .. }
        | FactoryEventKind::GateCheckUpdated { .. }
        | FactoryEventKind::GateManualApprovalSignal { .. } => None,
    }
}
