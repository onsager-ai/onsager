// budget-allow: factory event enum (~40 variants after spec #285 prune); macro/derive compression deferred to follow-up spec per #261
//! Factory events — the authoritative event types emitted to the factory event
//! spine by each subsystem.
//!
//! See `specs/forge-v0.1.md §9` for the Forge event contract. Additional event
//! types from the other subsystems are included here so that the spine
//! library provides a single typed vocabulary.

use chrono::{DateTime, Utc};
use onsager_artifact::{ArtifactId, ArtifactState, Kind, NodeId};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Factory event envelope
// ---------------------------------------------------------------------------

/// A factory event as written to the event spine.
///
/// All subsystems write events through this envelope. The `event` field carries
/// the typed payload; the wrapper carries tracing metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryEvent {
    /// The typed event payload.
    pub event: FactoryEventKind,
    /// Correlation ID for tracing a causal chain of events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// ID of the event that caused this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<i64>,
    /// The subsystem or actor that produced this event.
    pub actor: String,
    /// Timestamp (usually DB-assigned, but included for serialization).
    pub timestamp: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Typed event payloads
// ---------------------------------------------------------------------------

/// All factory event types across all subsystems.
///
/// ## Forge events (authoritative per forge-v0.1 §9)
///
/// ## Session events (chat-session lifecycle — portal's in-process runner)
///
/// ## Ising events (insight records)
///
/// `f64` fields (confidence) block `Eq`; only `PartialEq` is derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FactoryEventKind {
    // -- Artifact lifecycle (Forge) -----------------------------------------
    /// New artifact accepted and ID assigned.
    ArtifactRegistered {
        artifact_id: ArtifactId,
        kind: Kind,
        name: String,
        owner: String,
    },

    /// Artifact transitioned between lifecycle states.
    ArtifactStateChanged {
        artifact_id: ArtifactId,
        from_state: ArtifactState,
        to_state: ArtifactState,
    },

    /// Artifact reached terminal state (archived).
    ArtifactArchived {
        artifact_id: ArtifactId,
        reason: String,
    },

    // -- Git lifecycle events -------------------------------------------------
    GitPrOpened {
        artifact_id: ArtifactId,
        repo: String,
        pr_number: u64,
        url: String,
    },
    GitCiCompleted {
        artifact_id: ArtifactId,
        pr_number: u64,
        check_name: String,
        conclusion: String,
    },
    GitPrMerged {
        artifact_id: ArtifactId,
        pr_number: u64,
        merge_sha: String,
    },
    GitPrClosed {
        artifact_id: ArtifactId,
        pr_number: u64,
    },

    // -- Forge process events -----------------------------------------------
    /// Legacy 0.1 shaping-result record. Kept for historical spine rows
    /// and the deprecated observers runtime (`shape_retry`, retiring
    /// with #584); nothing emits it post-ADR 0027.
    ForgeShapingReturned {
        request_id: String,
        artifact_id: ArtifactId,
        outcome: ShapingOutcome,
    },

    /// GateVerdict observed by the legacy forge gate path. Kept for
    /// historical spine rows; nothing emits it post-0.2.
    ForgeGateVerdict {
        artifact_id: ArtifactId,
        gate_point: GatePoint,
        verdict: VerdictSummary,
    },

    /// Insight forwarded to the scheduling kernel.
    ForgeInsightObserved {
        insight_id: String,
        insight_kind: InsightKind,
        scope: InsightScope,
    },

    /// Scheduling kernel produced a ShapingDecision.
    ForgeDecisionMade {
        artifact_id: ArtifactId,
        target_version: u32,
        priority: i32,
    },

    // -- Chat-session lifecycle (portal's in-process runner, ADR 0027) ------
    /// A chat agent session finished successfully. Emitted by portal's
    /// in-process session runner (spec #583; formerly
    /// `stiglab.session_completed`).
    SessionCompleted {
        session_id: String,
        duration_ms: u64,
        /// Artifact this session was shaping (issue #14 phase 2). Optional so
        /// non-shaping sessions (e.g. direct task POSTs) don't emit a
        /// meaningless id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<String>,
        /// LLM token usage for this session (issue #39). Optional so
        /// pre-accounting sessions and mock dispatchers don't fabricate a
        /// zero bill — `None` means "not reported", not "cost nothing".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_usage: Option<TokenUsage>,
        /// Working-tree branch the agent pushed at session completion (issue
        /// #60). Used by `onsager-portal` to attach `vertical_lineage` when
        /// the matching PR webhook arrives. Optional — sessions that don't
        /// touch a git working dir leave it `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// PR number the agent opened, when known at completion time.
        /// Optional for the same reasons as `branch`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pr_number: Option<u64>,
    },

    /// A chat agent session terminated with an error. Emitted by portal's
    /// in-process session runner (spec #583; formerly
    /// `stiglab.session_failed`).
    SessionFailed {
        session_id: String,
        error: String,
        /// Artifact this session was shaping (issue #156). Optional so
        /// non-shaping sessions (direct task POSTs) don't emit a
        /// meaningless id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<String>,
    },

    // -- Portal intents (dashboard → session runner) -------------------------
    /// Dashboard task request from portal. Diagnostic timeline record —
    /// dispatch is in-process post-ADR 0027 (spec #583); portal's
    /// `SessionRunner` spawns the session directly.
    PortalSessionRequested { session_id: String },

    /// Portal failed to open a GitHub PR for a completed session (spec #273).
    /// Emitted as a diagnostic signal when the GitHub API call fails, the
    /// session pushed an empty branch, or the GitHub App is not configured.
    /// No retry is attempted inside the listener — the operator investigates
    /// via the dashboard event timeline.
    PortalPrOpenFailed {
        /// Issue artifact the session was shaping, if known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<ArtifactId>,
        /// Branch the session pushed to, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// Workspace the session belongs to.
        workspace_id: String,
        /// Short reason code: `"empty_branch"`, `"github_api_error"`,
        /// `"no_installation"`, `"app_not_configured"`, etc.
        reason: String,
    },

    /// Dashboard cancel-session request from portal (spec #303). Diagnostic
    /// timeline record — the cancel itself is applied in-process by
    /// portal's `SessionRunner` (spec #583). Best-effort: a terminal
    /// session makes the cancel a no-op.
    PortalSessionCancelRequested {
        session_id: String,
        /// Actor identifier captured for audit / debugging.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
    },

    // -- Ising events (observation) -----------------------------------------
    /// An insight passed validation and was recorded on the spine.
    IsingInsightDetected {
        insight_id: String,
        kind: InsightKind,
        scope: InsightScope,
        observation: String,
        confidence: f64,
    },

    /// A structured signal Ising surfaces on the spine for other subsystems
    /// to consume (issue #36 — close the feedback loop). Unlike
    /// `IsingInsightDetected`, which carries a human-readable observation,
    /// this variant is the machine-readable edge: each emission names the
    /// signal kind, the subject it attaches to, and the events that evidence
    /// it. Forge tailed these into `WorldState.insights`; no consumer
    /// remains post-ADR 0027.
    IsingInsightEmitted {
        /// Stable signal identifier (e.g. `"repeated_gate_override"`,
        /// `"shape_retry_spike"`). Consumers match on this string.
        signal_kind: String,
        /// What the signal is about — an artifact kind, artifact id, rule id,
        /// or intent class — serialized as a free-form string so the event
        /// contract doesn't need to know every possible subject type.
        subject_ref: String,
        /// Spine events that evidence this signal. Invariant: non-empty
        /// (enforced by the producer).
        evidence: Vec<EventRef>,
        /// 0.0..=1.0; downstream consumers use this to route (advisory vs.
        /// crystallization threshold).
        confidence: f64,
    },

    /// An insight was deduplicated or fell below confidence threshold (audit trail).
    IsingInsightSuppressed { insight_id: String, reason: String },

    /// An insight was packaged as a rule proposal (issue #36 Step 2).
    /// Unlike the legacy form, this variant carries enough structure
    /// that a downstream consumer can route it without looking up
    /// the original insight — `signal_kind` + `subject_ref` identify the
    /// evidence, `proposed_action` names what rule change is being asked
    /// for, and `class` decides whether the proposal auto-activates
    /// (`safe_auto`) or enters the review queue (`review_required`).
    IsingRuleProposed {
        /// ID of the insight that motivated the proposal.
        insight_id: String,
        /// Copy of the producing analyzer's signal kind (e.g.
        /// `"repeated_gate_override"`) so consumers can dedupe against the
        /// `insight_emitted` stream.
        signal_kind: String,
        /// What the proposal is about (artifact kind, rule id, etc.).
        subject_ref: String,
        /// Kind of change being proposed.
        proposed_action: RuleProposalAction,
        /// How the proposal should be handled downstream.
        class: RuleProposalClass,
        /// Human-readable justification for the audit trail.
        rationale: String,
        /// Confidence copied from the backing insight (0.0..=1.0).
        confidence: f64,
    },

    /// An analyzer encountered an error during its run.
    IsingAnalyzerError { analyzer: String, error: String },

    /// Ising finished catching up from a lag position.
    IsingCatchupCompleted { events_processed: u64 },

    // -- Workflow runtime events (issue #80) --------------------------------
    /// A trigger (e.g. a GitHub issue webhook) fired and produced a payload
    /// the trigger subscriber will translate into an artifact registration.
    /// Emitted by the stiglab webhook receiver; consumed by forge.
    TriggerFired {
        /// Workflow whose trigger fired.
        workflow_id: String,
        /// Trigger classification (matches the `workflows.trigger_kind`
        /// column). v1 always `"github_issue_webhook"`.
        trigger_kind: String,
        /// Free-form payload the subscriber needs to translate the trigger
        /// into an artifact (e.g. issue number, title, body, repo).
        payload: serde_json::Value,
    },

    /// Audit record for a manual / CLI / replay trigger fire (#241).
    /// Emitted alongside the underlying `TriggerFired` so audit views
    /// can attribute fires to a user. `event_subtype` distinguishes
    /// surfaces (`"ui_fire"`, `"ui_replay"`, `"cli_fire"`,
    /// `"cli_replay"`); `detail` carries source-specific fields
    /// (manual name, source event id).
    WorkflowManualTriggered {
        workflow_id: String,
        workspace_id: String,
        actor: String,
        event_subtype: String,
        detail: serde_json::Value,
    },

    /// A workflow-tagged artifact entered a new stage.
    StageEntered {
        artifact_id: ArtifactId,
        workflow_id: String,
        stage_index: u32,
        stage_name: String,
    },

    /// All gates on a stage resolved and the artifact advanced to the next
    /// stage (or reached terminal state when this was the last stage).
    ///
    /// Per spec #285 the redundant per-gate `stage.gate_passed` /
    /// `stage.gate_failed` events are gone; the run timeline reconstructs
    /// gate outcomes from `verify.verdict` plus this advancement
    /// signal.
    StageAdvanced {
        artifact_id: ArtifactId,
        workflow_id: String,
        from_stage_index: u32,
        /// `None` when the artifact has just completed the final stage.
        to_stage_index: Option<u32>,
    },

    /// A persisted multi-spec `SpecPlan` was requested to run (#501,
    /// ADR 0023). Emitted by portal's `run_spec_plan` MCP tool;
    /// consumed by the substrate scheduler host, which loads the stored
    /// plan from `spec_plans`, compiles it, and drives it to
    /// completion. Portal does not run the plan itself — the seam rule
    /// (portal is the edge, the scheduler host is the runtime).
    SpecPlanRunRequested {
        /// `(workspace_id, spec_plan_id)` keys the `spec_plans` row the
        /// scheduler loads and compiles.
        spec_plan_id: String,
        workspace_id: String,
    },

    /// A plan run reached a terminal state (#536). Emitted by the
    /// substrate scheduler when [`SpecPlanRunRequested`] finishes;
    /// portal consumes it to revoke the plan-scoped session token. The
    /// `plan_id` is the derived run id (`derive_plan_id`), not the
    /// spec_plan_id.
    PlanRunCompleted {
        plan_id: String,
        workspace_id: String,
        spec_plan_id: String,
    },

    /// A plan run failed terminally (#536). Same producer/consumer as
    /// [`PlanRunCompleted`] — portal revokes the plan-scoped token on
    /// either terminal signal.
    PlanRunFailed {
        plan_id: String,
        workspace_id: String,
        spec_plan_id: String,
    },

    // -- Registry events removed by spec #285 -------------------------------
    // The nine `registry.*` mutation events (`type_proposed`, `type_approved`,
    // `type_deprecated`, `adapter_registered`, `adapter_deprecated`,
    // `gate_registered`, `gate_deprecated`, `profile_registered`,
    // `profile_deprecated`) had no in-tree consumer or dashboard reader;
    // the registry tables are still the source of truth, but mutations no
    // longer publish a spine event. Add a variant back here when there is
    // a real consumer.

    // -- Workflow events (issue #81 — stiglab workflow CRUD + webhook) ------
    // `TriggerFired` is defined above in the issue #80 block; the stiglab
    // webhook router emits the same variant so forge's trigger subscriber can
    // consume it with no translation.
    /// A GitHub `check_suite`, `check_run`, or `status` event arrived for a
    /// PR we care about. Forge's external-check gate consumes this to advance
    /// or block artifacts whose current stage is `external-check`.
    GateCheckUpdated {
        repo_owner: String,
        repo_name: String,
        pr_number: u64,
        check_name: String,
        conclusion: String,
    },

    /// A manual-approval gate received a signal (e.g. the PR was merged).
    /// Forge's manual-approval gate advances when this arrives.
    GateManualApprovalSignal {
        repo_owner: String,
        repo_name: String,
        pr_number: u64,
        source: String,
    },

    // -- Substrate runtime (RUN-02, #360) -----------------------------------
    // Lifecycle events the 0.2 substrate scheduler and the executor
    // catalog emit. The on-wire shapes mirror the typed payload structs
    // in `onsager-substrate::events`; the structs there are the
    // authoring surface for substrate-side code, this enum is the
    // single wire vocabulary the spine read API hands to the dashboard
    // and (post OBS-01 / #361) to Observers.
    //
    // `plan_id` is the externally-assigned identifier for one
    // Execution Plan run (`onsager-nodes::scheduler::PlanId`) carried
    // as a `String` here so the spine crate doesn't need to depend on
    // the runtime; downstream consumers parse it back into `PlanId`
    // via `PlanId::new(...)`.
    /// Substrate scheduler dispatched a node — execution began.
    NodeStarted {
        plan_id: String,
        node_id: NodeId,
        /// Executor catalog key (`"script"`, `"agent"`, `"verify"`,
        /// `"human"`, `"sub_workflow"`, `"noop"`).
        executor_kind: String,
    },

    /// A node finished successfully; the scheduler persisted each output
    /// artifact under its declared edge `ArtifactId`.
    NodeCompleted {
        plan_id: String,
        node_id: NodeId,
        /// `ArtifactId`s the executor materialized — order matches the
        /// node's declared output edges.
        output_artifact_ids: Vec<ArtifactId>,
        /// `Some(true)` when the executor declared it produced nothing
        /// on purpose (an intentional empty delivery), distinguishing
        /// it from "delivered nothing" (`output_artifact_ids` empty,
        /// field absent). Populated by the authoritative gate (§4c,
        /// #520); `None` until then so behavior is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        declared_empty: Option<bool>,
    },

    /// A node's executor returned Err. The scheduler aborts the plan
    /// (v1; no retries).
    NodeFailed {
        plan_id: String,
        node_id: NodeId,
        /// Free-text error message from the executor.
        error: String,
    },

    /// A Human executor is parked waiting on an out-of-band approval
    /// decision. The dashboard's HITL inbox renders this; the
    /// substrate's own Human executor (#357) resolves it inline by
    /// observing the matching `node.human_approved` /
    /// `node.human_rejected`.
    NodeAwaitingHuman {
        plan_id: String,
        node_id: NodeId,
        /// Free-text prompt shown to the human reviewer.
        prompt: String,
    },

    /// A pending Human executor node received an approval decision.
    NodeHumanApproved {
        plan_id: String,
        node_id: NodeId,
        /// Actor identifier — `"human:<id>"` for a dashboard user,
        /// `"supervisor"` for a delegate agent.
        approved_by: String,
    },

    /// A pending Human executor node received a rejection decision.
    NodeHumanRejected {
        plan_id: String,
        node_id: NodeId,
        /// Actor identifier — same shape as `approved_by` above.
        rejected_by: String,
        /// Free-text justification carried into the audit trail.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// Verify executor produced a verdict — pass / fail outcome with
    /// per-check details. Per spec #360 this is the substrate-native
    /// verdict event (renamed when the governance subsystem retired,
    /// ADR 0027 / spec #582).
    VerifyVerdict {
        plan_id: String,
        node_id: NodeId,
        /// True when every check the executor ran passed.
        passed: bool,
        /// Per-check (name, passed) results so the dashboard can
        /// render the failure surface without a follow-up read.
        check_results: Vec<VerifyCheckResult>,
    },

    /// Agent executor opened an LLM session.
    AgentSessionStarted {
        plan_id: String,
        node_id: NodeId,
        /// Session identifier minted by the agent runner.
        session_id: String,
        /// Model name (`"claude-sonnet-4-6"`, etc.).
        model: String,
    },

    /// Agent executor's LLM session finished successfully.
    AgentSessionCompleted {
        plan_id: String,
        node_id: NodeId,
        session_id: String,
        /// Token usage for this session — `None` when the runner does
        /// not report it (stub / mock runners).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_usage: Option<TokenUsage>,
    },

    /// Agent executor's LLM session terminated with an error.
    AgentSessionFailed {
        plan_id: String,
        node_id: NodeId,
        session_id: String,
        /// Free-text error from the runner.
        error: String,
    },
}

impl FactoryEventKind {
    /// Returns the dot-separated event type string (e.g., "artifact.registered").
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ArtifactRegistered { .. } => "artifact.registered",
            Self::ArtifactStateChanged { .. } => "artifact.state_changed",
            Self::ArtifactArchived { .. } => "artifact.archived",
            Self::GitPrOpened { .. } => "git.pr_opened",
            Self::GitCiCompleted { .. } => "git.ci_completed",
            Self::GitPrMerged { .. } => "git.pr_merged",
            Self::GitPrClosed { .. } => "git.pr_closed",
            Self::ForgeShapingReturned { .. } => "forge.shaping_returned",
            Self::ForgeGateVerdict { .. } => "forge.gate_verdict",
            Self::ForgeInsightObserved { .. } => "forge.insight_observed",
            Self::ForgeDecisionMade { .. } => "forge.decision_made",
            Self::SessionCompleted { .. } => "session.completed",
            Self::SessionFailed { .. } => "session.failed",
            Self::PortalSessionRequested { .. } => "portal.session_requested",
            Self::PortalSessionCancelRequested { .. } => "portal.session_cancel_requested",
            Self::PortalPrOpenFailed { .. } => "portal.pr_open_failed",
            Self::IsingInsightDetected { .. } => "ising.insight_detected",
            Self::IsingInsightEmitted { .. } => "ising.insight_emitted",
            Self::IsingInsightSuppressed { .. } => "ising.insight_suppressed",
            Self::IsingRuleProposed { .. } => "ising.rule_proposed",
            Self::IsingAnalyzerError { .. } => "ising.analyzer_error",
            Self::IsingCatchupCompleted { .. } => "ising.catchup_completed",
            Self::TriggerFired { .. } => "trigger.fired",
            Self::WorkflowManualTriggered { .. } => "workflow.manual_triggered",
            Self::StageEntered { .. } => "stage.entered",
            Self::StageAdvanced { .. } => "stage.advanced",
            Self::SpecPlanRunRequested { .. } => "plan.run_requested",
            Self::PlanRunCompleted { .. } => "plan.run_completed",
            Self::PlanRunFailed { .. } => "plan.run_failed",
            Self::GateCheckUpdated { .. } => "gate.check_updated",
            Self::GateManualApprovalSignal { .. } => "gate.manual_approval_signal",
            Self::NodeStarted { .. } => "node.started",
            Self::NodeCompleted { .. } => "node.completed",
            Self::NodeFailed { .. } => "node.failed",
            Self::NodeAwaitingHuman { .. } => "node.awaiting_human",
            Self::NodeHumanApproved { .. } => "node.human_approved",
            Self::NodeHumanRejected { .. } => "node.human_rejected",
            Self::VerifyVerdict { .. } => "verify.verdict",
            Self::AgentSessionStarted { .. } => "agent.session_started",
            Self::AgentSessionCompleted { .. } => "agent.session_completed",
            Self::AgentSessionFailed { .. } => "agent.session_failed",
        }
    }

    /// Returns the stream_type for this event.
    pub fn stream_type(&self) -> &'static str {
        match self {
            Self::ArtifactRegistered { .. }
            | Self::ArtifactStateChanged { .. }
            | Self::ArtifactArchived { .. } => "artifact",
            Self::GitPrOpened { .. }
            | Self::GitCiCompleted { .. }
            | Self::GitPrMerged { .. }
            | Self::GitPrClosed { .. } => "git",
            Self::ForgeShapingReturned { .. }
            | Self::ForgeGateVerdict { .. }
            | Self::ForgeInsightObserved { .. }
            | Self::ForgeDecisionMade { .. } => "forge",
            Self::SessionCompleted { .. } | Self::SessionFailed { .. } => "session",
            Self::PortalSessionRequested { .. }
            | Self::PortalSessionCancelRequested { .. }
            | Self::PortalPrOpenFailed { .. } => "portal",
            Self::IsingInsightDetected { .. }
            | Self::IsingInsightEmitted { .. }
            | Self::IsingInsightSuppressed { .. }
            | Self::IsingRuleProposed { .. }
            | Self::IsingAnalyzerError { .. }
            | Self::IsingCatchupCompleted { .. } => "ising",
            Self::TriggerFired { .. } | Self::StageEntered { .. } | Self::StageAdvanced { .. } => {
                "workflow"
            }
            // Plan-run intent (portal → scheduler host) and the
            // plan-terminal signals (scheduler → portal, #536). Their
            // own namespace so the dashboard can scope plan-run
            // lifecycle without mixing it with per-node substrate
            // lifecycle.
            Self::SpecPlanRunRequested { .. }
            | Self::PlanRunCompleted { .. }
            | Self::PlanRunFailed { .. } => "plan",
            // Audit-only fan-out for manual fires (#241). Lives in the
            // `audit` namespace so audit views can filter without
            // mixing with first-class workflow runtime events.
            Self::WorkflowManualTriggered { .. } => "audit",
            Self::GateCheckUpdated { .. } | Self::GateManualApprovalSignal { .. } => "gate",
            // Substrate runtime — node lifecycle, Verify verdicts, and
            // agent-session signals share the `substrate` stream so the
            // dashboard / Observer (#361) can subscribe with one prefix.
            Self::NodeStarted { .. }
            | Self::NodeCompleted { .. }
            | Self::NodeFailed { .. }
            | Self::NodeAwaitingHuman { .. }
            | Self::NodeHumanApproved { .. }
            | Self::NodeHumanRejected { .. }
            | Self::VerifyVerdict { .. }
            | Self::AgentSessionStarted { .. }
            | Self::AgentSessionCompleted { .. }
            | Self::AgentSessionFailed { .. } => "substrate",
        }
    }

    /// Returns the primary entity ID this event relates to.
    pub fn stream_id(&self) -> String {
        match self {
            Self::ArtifactRegistered { artifact_id, .. }
            | Self::ArtifactStateChanged { artifact_id, .. }
            | Self::ArtifactArchived { artifact_id, .. } => artifact_id.to_string(),
            Self::GitPrOpened { artifact_id, .. }
            | Self::GitCiCompleted { artifact_id, .. }
            | Self::GitPrMerged { artifact_id, .. }
            | Self::GitPrClosed { artifact_id, .. } => artifact_id.to_string(),
            Self::ForgeShapingReturned { request_id, .. } => request_id.clone(),
            Self::ForgeGateVerdict { artifact_id, .. } => artifact_id.to_string(),
            Self::ForgeInsightObserved { insight_id, .. } => insight_id.clone(),
            Self::ForgeDecisionMade { artifact_id, .. } => artifact_id.to_string(),
            Self::SessionCompleted { session_id, .. } | Self::SessionFailed { session_id, .. } => {
                session_id.clone()
            }
            Self::PortalSessionRequested { session_id, .. }
            | Self::PortalSessionCancelRequested { session_id, .. } => session_id.clone(),
            Self::PortalPrOpenFailed {
                workspace_id,
                branch,
                ..
            } => branch
                .as_deref()
                .map(|b| format!("portal:pr_open_failed:{b}"))
                .unwrap_or_else(|| format!("portal:pr_open_failed:{workspace_id}")),
            Self::IsingInsightDetected { insight_id, .. } => insight_id.clone(),
            Self::IsingInsightEmitted { subject_ref, .. } => subject_ref.clone(),
            Self::IsingInsightSuppressed { insight_id, .. } => insight_id.clone(),
            Self::IsingRuleProposed { insight_id, .. } => insight_id.clone(),
            Self::IsingAnalyzerError { analyzer, .. } => analyzer.clone(),
            Self::IsingCatchupCompleted { .. } => "ising".to_string(),
            Self::TriggerFired { workflow_id, .. } => format!("workflow:{workflow_id}"),
            Self::WorkflowManualTriggered { workflow_id, .. } => {
                format!("audit:workflow:{workflow_id}")
            }
            Self::StageEntered { artifact_id, .. } | Self::StageAdvanced { artifact_id, .. } => {
                format!("workflow:{artifact_id}")
            }
            Self::SpecPlanRunRequested {
                workspace_id,
                spec_plan_id,
            } => format!("plan:{workspace_id}:{spec_plan_id}"),
            // Keyed by the derived run id so portal's revoke listener
            // reads `plan_id` straight off the stream (#536).
            Self::PlanRunCompleted { plan_id, .. } | Self::PlanRunFailed { plan_id, .. } => {
                format!("plan:{plan_id}")
            }
            Self::GateCheckUpdated {
                repo_owner,
                repo_name,
                pr_number,
                ..
            }
            | Self::GateManualApprovalSignal {
                repo_owner,
                repo_name,
                pr_number,
                ..
            } => format!("{repo_owner}/{repo_name}#{pr_number}"),
            // Substrate runtime — `(plan_id, node_id)` is the natural
            // composite key so the dashboard run timeline can scope
            // by plan and node without parsing the payload.
            Self::NodeStarted {
                plan_id, node_id, ..
            }
            | Self::NodeCompleted {
                plan_id, node_id, ..
            }
            | Self::NodeFailed {
                plan_id, node_id, ..
            }
            | Self::NodeAwaitingHuman {
                plan_id, node_id, ..
            }
            | Self::NodeHumanApproved {
                plan_id, node_id, ..
            }
            | Self::NodeHumanRejected {
                plan_id, node_id, ..
            }
            | Self::VerifyVerdict {
                plan_id, node_id, ..
            }
            | Self::AgentSessionStarted {
                plan_id, node_id, ..
            }
            | Self::AgentSessionCompleted {
                plan_id, node_id, ..
            }
            | Self::AgentSessionFailed {
                plan_id, node_id, ..
            } => format!("plan:{plan_id}:{node_id}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Supporting enums
// ---------------------------------------------------------------------------

/// Outcome of a shaping request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapingOutcome {
    Completed,
    Failed,
    Partial,
    Aborted,
}

/// Gate points in the legacy 0.1 gate protocol. Kept for historical
/// spine rows; the governance subsystem retired with ADR 0027.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatePoint {
    PreDispatch,
    StateTransition,
    ConsumerRouting,
    ToolLevel,
    /// A run wants to *write* (push + open PR) to a bound-but-unpinned
    /// repo it selected mid-run (spec #548, Part of #544). Binding a repo
    /// to a workspace grants candidacy, not blanket write consent — this
    /// gate is the consent point. The write-scoped token is minted only
    /// after the verdict is `Allow`. The pinned repo (named in the
    /// trigger) is pre-approved and never reaches this point.
    RepoWrite,
}

/// Summary of a gate verdict (for event spine recording).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictSummary {
    Allow,
    Deny,
    Modify,
    Escalate,
}

/// Forge process states. Used by forge's internal state machine; no longer
/// carried on a `FactoryEventKind` variant after spec #272 retired
/// `ForgeStateChanged`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgeProcessState {
    Running,
    Paused,
    Draining,
    Stopped,
}

/// Insight categories from Ising.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightKind {
    Failure,
    Waste,
    Win,
    Anomaly,
}

/// Scope of an insight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightScope {
    ArtifactKind(String),
    SpecificArtifact(ArtifactId),
    Global,
}

/// Reference to a spine event used as evidence for a signal (`insight.emitted`
/// variant carries a `Vec<EventRef>`). This is the spine-native counterpart to
/// `onsager_spine::protocol::FactoryEventRef` — kept in the spine crate so the event
/// vocabulary has no protocol-crate dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRef {
    /// The `id` column in the `events` or `events_ext` table.
    pub event_id: i64,
    /// The `event_type` string (e.g. `"forge.gate_verdict"`), for quick
    /// consumer-side filtering without a second lookup.
    pub event_type: String,
}

/// LLM token usage carried on [`FactoryEventKind::SessionCompleted`]
/// (issue #39). Accounting primitives only — USD cost is resolved downstream
/// by the budget consumer, not on the event, so we don't have to version the
/// pricing table every time a model changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export)]
pub struct TokenUsage {
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    /// Cache-read input tokens (Anthropic-style prompt caching). Zero for
    /// providers without a cache concept.
    #[serde(default)]
    #[ts(type = "number")]
    pub cache_read_tokens: u64,
    /// Cache-creation input tokens.
    #[serde(default)]
    #[ts(type = "number")]
    pub cache_write_tokens: u64,
    /// Model identifier (`"claude-sonnet-4-6"`, etc.) so the downstream
    /// pricing table can resolve cost without guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// What kind of rule change an [`FactoryEventKind::IsingRuleProposed`] is
/// asking for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RuleProposalAction {
    /// Disable or retire an existing rule — typically when override rate is
    /// so high the rule is more friction than value.
    Retire { rule_id: String },
    /// Rewrite an existing rule's condition — typically when the rule is
    /// tripping on false positives.
    Rewrite {
        rule_id: String,
        suggested_condition: Option<String>,
    },
    /// Register a new rule for a subject that currently has none.
    Introduce {
        subject_ref: String,
        suggested_condition: Option<String>,
    },
}

/// One check's outcome carried on [`FactoryEventKind::VerifyVerdict`].
/// The Verify executor's [`Check`](../../../crates/onsager-nodes/src/verify.rs)
/// list maps 1:1 onto this struct list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyCheckResult {
    /// Check name (e.g. `"cargo_test"`, `"clippy"`, `"schema_lint"`).
    pub name: String,
    /// `true` when this check passed.
    pub passed: bool,
}

/// How a rule proposal should be handled downstream.
///
/// `SafeAuto` proposals carry a narrow, reversible change with enough
/// confidence that blocking on a human is pure friction. `ReviewRequired`
/// proposals land in the review queue on the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleProposalClass {
    SafeAuto,
    ReviewRequired,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[path = "factory_event_tests.rs"]
#[cfg(test)]
mod tests;
