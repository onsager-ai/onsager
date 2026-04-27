//! Factory events — the authoritative event types emitted to the factory event
//! spine by each subsystem.
//!
//! See `specs/forge-v0.1.md §9` for the Forge event contract. Additional event
//! types from Stiglab, Synodic, and Ising are included here so that the spine
//! library provides a single typed vocabulary.

use chrono::{DateTime, Utc};
use onsager_artifact::{
    ArtifactId, ArtifactState, BundleId, DeliverableId, Kind, KindId, QualitySignal, WorkflowRunId,
};
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
/// ## Stiglab events (session lifecycle upgrades)
///
/// ## Synodic events (rule and escalation outcomes)
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

    /// New version committed for an artifact.
    ArtifactVersionCreated {
        artifact_id: ArtifactId,
        version: u32,
        content_ref_uri: String,
        change_summary: String,
        session_id: String,
    },

    /// New vertical or horizontal lineage entry recorded.
    ArtifactLineageExtended {
        artifact_id: ArtifactId,
        lineage_type: LineageType,
        detail: serde_json::Value,
    },

    /// New quality signal appended.
    ArtifactQualityRecorded {
        artifact_id: ArtifactId,
        signal: QualitySignal,
    },

    /// Released artifact dispatched to a consumer sink.
    ArtifactRouted {
        artifact_id: ArtifactId,
        consumer_id: String,
        sink: String,
    },

    /// Artifact reached terminal state (archived).
    ArtifactArchived {
        artifact_id: ArtifactId,
        reason: String,
    },

    // -- Warehouse & Delivery (warehouse-and-delivery-v0.1) -----------------
    /// A new bundle was sealed for an artifact (§5.1).
    BundleSealed {
        artifact_id: ArtifactId,
        bundle_id: BundleId,
        version: u32,
    },

    /// A delivery attempt succeeded; the receipt is stored on the delivery row (§5.3).
    DeliverySucceeded {
        bundle_id: BundleId,
        consumer_id: String,
    },

    /// A delivery attempt failed; includes whether the worker will retry or
    /// has marked the delivery `Abandoned` (§5.3).
    DeliveryFailed {
        bundle_id: BundleId,
        consumer_id: String,
        reason: String,
        /// Whether the delivery has been abandoned (terminal) or will retry.
        abandoned: bool,
    },

    // -- Deliverable (workflow-run output, issue #100/#101) -----------------
    /// A workflow run produced its first artifact reference. Emitted once per
    /// run; subsequent additions flow through `DeliverableUpdated`.
    DeliverableCreated {
        deliverable_id: DeliverableId,
        workflow_run_id: WorkflowRunId,
    },

    /// A workflow run added an artifact reference to its deliverable under a
    /// given kind. Replay is idempotent on exact `(kind, artifact_id)`.
    DeliverableUpdated {
        deliverable_id: DeliverableId,
        workflow_run_id: WorkflowRunId,
        kind: KindId,
        artifact_id: ArtifactId,
    },

    // -- Git lifecycle events -------------------------------------------------
    GitBranchCreated {
        artifact_id: ArtifactId,
        repo: String,
        branch: String,
    },
    GitCommitPushed {
        artifact_id: ArtifactId,
        sha: String,
        message: String,
        session_id: String,
    },
    GitPrOpened {
        artifact_id: ArtifactId,
        repo: String,
        pr_number: u64,
        url: String,
    },
    GitPrReviewReceived {
        artifact_id: ArtifactId,
        pr_number: u64,
        reviewer: String,
        state: String,
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
    /// ShapingRequest sent to Stiglab.
    ForgeShapingDispatched {
        request_id: String,
        artifact_id: ArtifactId,
        target_version: u32,
    },

    /// ShapingResult received from Stiglab.
    ForgeShapingReturned {
        request_id: String,
        artifact_id: ArtifactId,
        outcome: ShapingOutcome,
    },

    /// GateRequest sent to Synodic.
    ForgeGateRequested {
        /// Correlation ID generated by Forge when emitting the request.
        /// Synodic echoes it back on `synodic.gate_verdict` so Forge can
        /// match the verdict to its parked pipeline decision.
        ///
        /// `#[serde(default)]` allows older events written before spec
        /// #131 / ADR 0004 Lever C to deserialize with an empty id.
        #[serde(default)]
        gate_id: String,
        artifact_id: ArtifactId,
        gate_point: GatePoint,
        /// Full gate-evaluation payload — the same shape Synodic
        /// previously consumed as the `POST /api/gate` request body.
        /// `None` on events written before this field was added (spec
        /// #131 / ADR 0004 Lever C phase 2). When `None`, Synodic's
        /// listener cannot evaluate and must skip the request.
        ///
        /// Top-level `gate_id`, `artifact_id`, and `gate_point` are
        /// kept for stream indexing and dashboard filtering; the
        /// duplication with `request.context` is intentional.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<crate::protocol::GateRequest>,
    },

    /// GateVerdict received from Synodic.
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

    /// Scheduling kernel returned None (idle, emitted at reduced frequency).
    ForgeIdleTick,

    /// Forge process state machine transitioned.
    ForgeStateChanged {
        from_state: ForgeProcessState,
        to_state: ForgeProcessState,
    },

    // -- Stiglab events (session and node lifecycle) -------------------------
    /// A new session was allocated for a shaping request.
    StiglabSessionCreated {
        session_id: String,
        request_id: String,
        node_id: String,
    },

    /// A session was dispatched to a Stiglab node.
    StiglabSessionDispatched { session_id: String, node_id: String },

    /// A session began active execution.
    StiglabSessionRunning { session_id: String },

    /// A session finished successfully.
    StiglabSessionCompleted {
        session_id: String,
        request_id: String,
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

    /// Full `ShapingResult` produced by a Stiglab session, ready for
    /// Forge to act on. Replaces the legacy synchronous
    /// `forge → stiglab POST /api/shaping` HTTP response per spec #131
    /// / ADR 0004 Lever C.
    ///
    /// Emitted in addition to (not instead of) `stiglab.session_completed`:
    /// the lifecycle event signals "this session finished" (used by the
    /// dashboard and node telemetry); this event signals "the artifact
    /// outputs are ready for the next pipeline stage" (used by Forge's
    /// state machine to advance / seal). Stiglab emits it only for
    /// sessions that were dispatched as shaping requests — direct task
    /// POSTs that produce no `ShapingResult` skip it.
    StiglabShapingResultReady {
        /// Artifact this shaping was for. Hoisted so `stream_id()` can
        /// route the event without parsing the embedded result.
        artifact_id: ArtifactId,
        /// Full result payload — the same shape Forge previously
        /// consumed as the `POST /api/shaping` response body.
        /// `request_id` inside this struct correlates back to the
        /// originating `forge.shaping_dispatched`.
        result: crate::protocol::ShapingResult,
    },

    /// A session terminated with an error.
    StiglabSessionFailed {
        session_id: String,
        request_id: String,
        error: String,
        /// Artifact this session was shaping (issue #156). Optional so
        /// non-shaping sessions (direct task POSTs) don't emit a
        /// meaningless id. When present, forge's workflow signal
        /// listener writes a `Failure` outcome to the agent-session
        /// signal cache so the gate fails loudly instead of stalling.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<String>,
    },

    /// A session was aborted (e.g. node lost, deadline exceeded).
    StiglabSessionAborted { session_id: String, reason: String },

    /// A session-internal event was promoted to a factory event.
    StiglabEventUpgraded {
        session_id: String,
        original_event_type: String,
        reason: String,
    },

    /// A new Stiglab node joined the pool.
    StiglabNodeRegistered {
        node_id: String,
        name: String,
        hostname: String,
    },

    /// A Stiglab node left the pool.
    StiglabNodeDeregistered { node_id: String, reason: String },

    /// A node missed its expected heartbeat.
    StiglabNodeHeartbeatMissed { node_id: String },

    // -- Synodic events (governance) ----------------------------------------
    /// A gate request was evaluated and a verdict issued.
    SynodicGateEvaluated {
        gate_id: String,
        artifact_id: ArtifactId,
        verdict: VerdictSummary,
    },

    /// A gate request was denied (subset of gate_evaluated, for easy filtering).
    SynodicGateDenied {
        gate_id: String,
        artifact_id: ArtifactId,
        reason: String,
    },

    /// A gate request verdict was Modify (subset of gate_evaluated).
    SynodicGateModified {
        gate_id: String,
        artifact_id: ArtifactId,
    },

    /// Full gate verdict issued by Synodic in response to a
    /// `forge.gate_requested` event. Carries the complete `GateVerdict`
    /// payload — Allow, Deny{reason}, Modify{new_action}, or
    /// Escalate{context} — so Forge can act on it without a follow-up
    /// HTTP roundtrip.
    ///
    /// Replaces the legacy synchronous `forge → synodic POST /api/gate`
    /// response per spec #131 / ADR 0004 Lever C. The summary variants
    /// above (`SynodicGateEvaluated`, `SynodicGateDenied`,
    /// `SynodicGateModified`) remain for dashboard filtering; this
    /// variant is the one consumers act on.
    SynodicGateVerdict {
        /// Correlation ID echoed from the originating
        /// `forge.gate_requested` event.
        gate_id: String,
        artifact_id: ArtifactId,
        gate_point: GatePoint,
        verdict: crate::protocol::GateVerdict,
    },

    /// An escalation was initiated.
    SynodicEscalationStarted {
        escalation_id: String,
        artifact_id: ArtifactId,
    },

    /// An escalation was resolved (by human, delegate, or timeout).
    SynodicEscalationResolved {
        escalation_id: String,
        artifact_id: ArtifactId,
        resolution: EscalationResolution,
    },

    /// An escalation timed out and the default verdict was applied.
    SynodicEscalationTimedOut {
        escalation_id: String,
        artifact_id: ArtifactId,
    },

    /// A delegate (human or supervisor agent) proposed a resolution for an
    /// active escalation (issue #37). The resolution is not applied until
    /// accepted — this event is the proposal itself, not the final verdict.
    SynodicGateResolutionProposed {
        escalation_id: String,
        artifact_id: ArtifactId,
        /// Who's proposing the resolution (`"supervisor"`, `"human:<id>"`).
        proposer: String,
        /// The verdict being proposed.
        proposed_verdict: VerdictSummary,
        /// Free-form justification for the audit trail.
        rationale: String,
    },

    /// A crystallization candidate rule was created.
    SynodicRuleProposed {
        rule_id: String,
        description: String,
    },

    /// A proposed rule was approved and entered the active set.
    SynodicRuleApproved { rule_id: String },

    /// A rule was disabled.
    SynodicRuleDisabled { rule_id: String, reason: String },

    /// A rule was modified, producing a new version.
    SynodicRuleVersionCreated { rule_id: String, version: u32 },

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
    /// it. Forge tails these into `WorldState.insights`; a future Synodic
    /// consumer will treat high-confidence emissions as rule-proposal input.
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

    /// An insight was packaged as a rule proposal for Synodic (issue #36
    /// Step 2). Unlike the legacy form, this variant carries enough
    /// structure that a Synodic consumer can route it without looking up
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

    // -- Refract events (issue #35 — intent decomposition) ------------------
    /// A new intent was submitted for decomposition. Intents are the
    /// high-level units of work a Refract decomposer expands into artifact
    /// trees (e.g. `"migrate all legacy auth callers to the new SDK"` →
    /// one artifact per file-touchpoint).
    IntentSubmitted {
        /// Opaque unique id — used as the correlation handle for every
        /// downstream `refract.*` event.
        intent_id: String,
        /// Stable class identifier — maps 1:1 to a registered decomposer
        /// (e.g. `"file_migration"`, `"spec_rollout"`).
        intent_class: String,
        /// Free-form description of the intent, shown in the UI and
        /// preserved as audit trail.
        description: String,
        /// Who or what submitted the intent.
        submitter: String,
    },

    /// A decomposer produced an artifact tree for an intent.
    RefractDecomposed {
        intent_id: String,
        /// Name of the decomposer that handled the intent (the Refract
        /// equivalent of Ising's `signal_kind`).
        decomposer: String,
        /// Newly registered artifact ids produced by the decomposition.
        artifact_ids: Vec<String>,
    },

    /// Decomposition failed — either no decomposer matched, or the matched
    /// decomposer errored out.
    RefractFailed { intent_id: String, reason: String },

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

    /// A workflow-tagged artifact entered a new stage.
    StageEntered {
        artifact_id: ArtifactId,
        workflow_id: String,
        stage_index: u32,
        stage_name: String,
    },

    /// A gate on the current stage resolved successfully.
    StageGatePassed {
        artifact_id: ArtifactId,
        workflow_id: String,
        stage_index: u32,
        gate_kind: String,
    },

    /// A gate on the current stage failed. The artifact is parked in
    /// `under_review` until the gate-failure condition is cleared (e.g. a
    /// new CI run succeeds, a reviewer approves).
    StageGateFailed {
        artifact_id: ArtifactId,
        workflow_id: String,
        stage_index: u32,
        gate_kind: String,
        reason: String,
    },

    /// All gates on a stage resolved and the artifact advanced to the next
    /// stage (or reached terminal state when this was the last stage).
    StageAdvanced {
        artifact_id: ArtifactId,
        workflow_id: String,
        from_stage_index: u32,
        /// `None` when the artifact has just completed the final stage.
        to_stage_index: Option<u32>,
    },

    // -- Registry events (factory pipeline foundations, issue #14) ----------
    /// A new artifact type was proposed (not yet active).
    TypeProposed {
        type_id: String,
        workspace_id: String,
        revision: i32,
    },

    /// A proposed type was approved and entered the active catalog.
    TypeApproved {
        type_id: String,
        workspace_id: String,
        revision: i32,
    },

    /// A type was deprecated (retained for audit, not used for new artifacts).
    TypeDeprecated {
        type_id: String,
        workspace_id: String,
        reason: String,
    },

    /// An adapter implementation was registered in the catalog.
    AdapterRegistered {
        adapter_id: String,
        workspace_id: String,
        revision: i32,
    },

    /// An adapter was deprecated.
    AdapterDeprecated {
        adapter_id: String,
        workspace_id: String,
        reason: String,
    },

    /// A gate evaluator was registered.
    GateRegistered {
        evaluator_id: String,
        workspace_id: String,
        revision: i32,
    },

    /// A gate evaluator was deprecated.
    GateDeprecated {
        evaluator_id: String,
        workspace_id: String,
        reason: String,
    },

    /// An agent profile was registered.
    ProfileRegistered {
        profile_id: String,
        workspace_id: String,
        revision: i32,
    },

    /// An agent profile was deprecated.
    ProfileDeprecated {
        profile_id: String,
        workspace_id: String,
        reason: String,
    },

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
}

impl FactoryEventKind {
    /// Returns the dot-separated event type string (e.g., "artifact.registered").
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ArtifactRegistered { .. } => "artifact.registered",
            Self::ArtifactStateChanged { .. } => "artifact.state_changed",
            Self::ArtifactVersionCreated { .. } => "artifact.version_created",
            Self::ArtifactLineageExtended { .. } => "artifact.lineage_extended",
            Self::ArtifactQualityRecorded { .. } => "artifact.quality_recorded",
            Self::ArtifactRouted { .. } => "artifact.routed",
            Self::ArtifactArchived { .. } => "artifact.archived",
            Self::BundleSealed { .. } => "warehouse.bundle_sealed",
            Self::DeliverySucceeded { .. } => "delivery.succeeded",
            Self::DeliveryFailed { .. } => "delivery.failed",
            Self::DeliverableCreated { .. } => "deliverable.created",
            Self::DeliverableUpdated { .. } => "deliverable.updated",
            Self::GitBranchCreated { .. } => "git.branch_created",
            Self::GitCommitPushed { .. } => "git.commit_pushed",
            Self::GitPrOpened { .. } => "git.pr_opened",
            Self::GitPrReviewReceived { .. } => "git.pr_review_received",
            Self::GitCiCompleted { .. } => "git.ci_completed",
            Self::GitPrMerged { .. } => "git.pr_merged",
            Self::GitPrClosed { .. } => "git.pr_closed",
            Self::ForgeShapingDispatched { .. } => "forge.shaping_dispatched",
            Self::ForgeShapingReturned { .. } => "forge.shaping_returned",
            Self::ForgeGateRequested { .. } => "forge.gate_requested",
            Self::ForgeGateVerdict { .. } => "forge.gate_verdict",
            Self::ForgeInsightObserved { .. } => "forge.insight_observed",
            Self::ForgeDecisionMade { .. } => "forge.decision_made",
            Self::ForgeIdleTick => "forge.idle_tick",
            Self::ForgeStateChanged { .. } => "forge.state_changed",
            Self::StiglabSessionCreated { .. } => "stiglab.session_created",
            Self::StiglabSessionDispatched { .. } => "stiglab.session_dispatched",
            Self::StiglabSessionRunning { .. } => "stiglab.session_running",
            Self::StiglabSessionCompleted { .. } => "stiglab.session_completed",
            Self::StiglabShapingResultReady { .. } => "stiglab.shaping_result_ready",
            Self::StiglabSessionFailed { .. } => "stiglab.session_failed",
            Self::StiglabSessionAborted { .. } => "stiglab.session_aborted",
            Self::StiglabEventUpgraded { .. } => "stiglab.event_upgraded",
            Self::StiglabNodeRegistered { .. } => "stiglab.node_registered",
            Self::StiglabNodeDeregistered { .. } => "stiglab.node_deregistered",
            Self::StiglabNodeHeartbeatMissed { .. } => "stiglab.node_heartbeat_missed",
            Self::SynodicGateEvaluated { .. } => "synodic.gate_evaluated",
            Self::SynodicGateDenied { .. } => "synodic.gate_denied",
            Self::SynodicGateModified { .. } => "synodic.gate_modified",
            Self::SynodicGateVerdict { .. } => "synodic.gate_verdict",
            Self::SynodicEscalationStarted { .. } => "synodic.escalation_started",
            Self::SynodicEscalationResolved { .. } => "synodic.escalation_resolved",
            Self::SynodicEscalationTimedOut { .. } => "synodic.escalation_timed_out",
            Self::SynodicGateResolutionProposed { .. } => "synodic.gate_resolution_proposed",
            Self::SynodicRuleProposed { .. } => "synodic.rule_proposed",
            Self::SynodicRuleApproved { .. } => "synodic.rule_approved",
            Self::SynodicRuleDisabled { .. } => "synodic.rule_disabled",
            Self::SynodicRuleVersionCreated { .. } => "synodic.rule_version_created",
            Self::IsingInsightDetected { .. } => "ising.insight_detected",
            Self::IsingInsightEmitted { .. } => "ising.insight_emitted",
            Self::IsingInsightSuppressed { .. } => "ising.insight_suppressed",
            Self::IsingRuleProposed { .. } => "ising.rule_proposed",
            Self::IsingAnalyzerError { .. } => "ising.analyzer_error",
            Self::IsingCatchupCompleted { .. } => "ising.catchup_completed",
            Self::IntentSubmitted { .. } => "refract.intent_submitted",
            Self::RefractDecomposed { .. } => "refract.decomposed",
            Self::RefractFailed { .. } => "refract.failed",
            Self::TriggerFired { .. } => "trigger.fired",
            Self::StageEntered { .. } => "stage.entered",
            Self::StageGatePassed { .. } => "stage.gate_passed",
            Self::StageGateFailed { .. } => "stage.gate_failed",
            Self::StageAdvanced { .. } => "stage.advanced",
            Self::TypeProposed { .. } => "registry.type_proposed",
            Self::TypeApproved { .. } => "registry.type_approved",
            Self::TypeDeprecated { .. } => "registry.type_deprecated",
            Self::AdapterRegistered { .. } => "registry.adapter_registered",
            Self::AdapterDeprecated { .. } => "registry.adapter_deprecated",
            Self::GateRegistered { .. } => "registry.gate_registered",
            Self::GateDeprecated { .. } => "registry.gate_deprecated",
            Self::ProfileRegistered { .. } => "registry.profile_registered",
            Self::ProfileDeprecated { .. } => "registry.profile_deprecated",
            Self::GateCheckUpdated { .. } => "gate.check_updated",
            Self::GateManualApprovalSignal { .. } => "gate.manual_approval_signal",
        }
    }

    /// Returns the stream_type for this event.
    pub fn stream_type(&self) -> &'static str {
        match self {
            Self::ArtifactRegistered { .. }
            | Self::ArtifactStateChanged { .. }
            | Self::ArtifactVersionCreated { .. }
            | Self::ArtifactLineageExtended { .. }
            | Self::ArtifactQualityRecorded { .. }
            | Self::ArtifactRouted { .. }
            | Self::ArtifactArchived { .. } => "artifact",
            Self::BundleSealed { .. } => "warehouse",
            Self::DeliverySucceeded { .. } | Self::DeliveryFailed { .. } => "delivery",
            Self::DeliverableCreated { .. } | Self::DeliverableUpdated { .. } => "deliverable",
            Self::GitBranchCreated { .. }
            | Self::GitCommitPushed { .. }
            | Self::GitPrOpened { .. }
            | Self::GitPrReviewReceived { .. }
            | Self::GitCiCompleted { .. }
            | Self::GitPrMerged { .. }
            | Self::GitPrClosed { .. } => "git",
            Self::ForgeShapingDispatched { .. }
            | Self::ForgeShapingReturned { .. }
            | Self::ForgeGateRequested { .. }
            | Self::ForgeGateVerdict { .. }
            | Self::ForgeInsightObserved { .. }
            | Self::ForgeDecisionMade { .. }
            | Self::ForgeIdleTick
            | Self::ForgeStateChanged { .. } => "forge",
            Self::StiglabSessionCreated { .. }
            | Self::StiglabSessionDispatched { .. }
            | Self::StiglabSessionRunning { .. }
            | Self::StiglabSessionCompleted { .. }
            | Self::StiglabShapingResultReady { .. }
            | Self::StiglabSessionFailed { .. }
            | Self::StiglabSessionAborted { .. }
            | Self::StiglabEventUpgraded { .. }
            | Self::StiglabNodeRegistered { .. }
            | Self::StiglabNodeDeregistered { .. }
            | Self::StiglabNodeHeartbeatMissed { .. } => "stiglab",
            Self::SynodicGateEvaluated { .. }
            | Self::SynodicGateDenied { .. }
            | Self::SynodicGateModified { .. }
            | Self::SynodicGateVerdict { .. }
            | Self::SynodicEscalationStarted { .. }
            | Self::SynodicEscalationResolved { .. }
            | Self::SynodicEscalationTimedOut { .. }
            | Self::SynodicGateResolutionProposed { .. }
            | Self::SynodicRuleProposed { .. }
            | Self::SynodicRuleApproved { .. }
            | Self::SynodicRuleDisabled { .. }
            | Self::SynodicRuleVersionCreated { .. } => "synodic",
            Self::IsingInsightDetected { .. }
            | Self::IsingInsightEmitted { .. }
            | Self::IsingInsightSuppressed { .. }
            | Self::IsingRuleProposed { .. }
            | Self::IsingAnalyzerError { .. }
            | Self::IsingCatchupCompleted { .. } => "ising",
            Self::IntentSubmitted { .. }
            | Self::RefractDecomposed { .. }
            | Self::RefractFailed { .. } => "refract",
            Self::TriggerFired { .. }
            | Self::StageEntered { .. }
            | Self::StageGatePassed { .. }
            | Self::StageGateFailed { .. }
            | Self::StageAdvanced { .. } => "workflow",
            Self::TypeProposed { .. }
            | Self::TypeApproved { .. }
            | Self::TypeDeprecated { .. }
            | Self::AdapterRegistered { .. }
            | Self::AdapterDeprecated { .. }
            | Self::GateRegistered { .. }
            | Self::GateDeprecated { .. }
            | Self::ProfileRegistered { .. }
            | Self::ProfileDeprecated { .. } => "registry",
            Self::GateCheckUpdated { .. } | Self::GateManualApprovalSignal { .. } => "gate",
        }
    }

    /// Returns the primary entity ID this event relates to.
    pub fn stream_id(&self) -> String {
        match self {
            Self::ArtifactRegistered { artifact_id, .. }
            | Self::ArtifactStateChanged { artifact_id, .. }
            | Self::ArtifactVersionCreated { artifact_id, .. }
            | Self::ArtifactLineageExtended { artifact_id, .. }
            | Self::ArtifactQualityRecorded { artifact_id, .. }
            | Self::ArtifactRouted { artifact_id, .. }
            | Self::ArtifactArchived { artifact_id, .. } => artifact_id.to_string(),
            Self::BundleSealed { bundle_id, .. } => bundle_id.to_string(),
            Self::DeliverySucceeded { bundle_id, .. } | Self::DeliveryFailed { bundle_id, .. } => {
                bundle_id.to_string()
            }
            Self::DeliverableCreated { deliverable_id, .. }
            | Self::DeliverableUpdated { deliverable_id, .. } => deliverable_id.to_string(),
            Self::GitBranchCreated { artifact_id, .. }
            | Self::GitCommitPushed { artifact_id, .. }
            | Self::GitPrOpened { artifact_id, .. }
            | Self::GitPrReviewReceived { artifact_id, .. }
            | Self::GitCiCompleted { artifact_id, .. }
            | Self::GitPrMerged { artifact_id, .. }
            | Self::GitPrClosed { artifact_id, .. } => artifact_id.to_string(),
            Self::ForgeShapingDispatched { request_id, .. } => request_id.clone(),
            Self::ForgeShapingReturned { request_id, .. } => request_id.clone(),
            Self::ForgeGateRequested { artifact_id, .. } => artifact_id.to_string(),
            Self::ForgeGateVerdict { artifact_id, .. } => artifact_id.to_string(),
            Self::ForgeInsightObserved { insight_id, .. } => insight_id.clone(),
            Self::ForgeDecisionMade { artifact_id, .. } => artifact_id.to_string(),
            Self::ForgeIdleTick => "forge".to_string(),
            Self::ForgeStateChanged { .. } => "forge".to_string(),
            Self::StiglabSessionCreated { session_id, .. }
            | Self::StiglabSessionDispatched { session_id, .. }
            | Self::StiglabSessionRunning { session_id, .. }
            | Self::StiglabSessionCompleted { session_id, .. }
            | Self::StiglabSessionFailed { session_id, .. }
            | Self::StiglabSessionAborted { session_id, .. }
            | Self::StiglabEventUpgraded { session_id, .. } => session_id.clone(),
            Self::StiglabShapingResultReady { artifact_id, .. } => artifact_id.to_string(),
            Self::StiglabNodeRegistered { node_id, .. }
            | Self::StiglabNodeDeregistered { node_id, .. }
            | Self::StiglabNodeHeartbeatMissed { node_id, .. } => node_id.clone(),
            Self::SynodicGateEvaluated { gate_id, .. } => gate_id.clone(),
            Self::SynodicGateDenied { gate_id, .. } => gate_id.clone(),
            Self::SynodicGateModified { gate_id, .. } => gate_id.clone(),
            Self::SynodicGateVerdict { gate_id, .. } => gate_id.clone(),
            Self::SynodicEscalationStarted { escalation_id, .. } => escalation_id.clone(),
            Self::SynodicEscalationResolved { escalation_id, .. } => escalation_id.clone(),
            Self::SynodicEscalationTimedOut { escalation_id, .. } => escalation_id.clone(),
            Self::SynodicGateResolutionProposed { escalation_id, .. } => escalation_id.clone(),
            Self::SynodicRuleProposed { rule_id, .. } => rule_id.clone(),
            Self::SynodicRuleApproved { rule_id, .. } => rule_id.clone(),
            Self::SynodicRuleDisabled { rule_id, .. } => rule_id.clone(),
            Self::SynodicRuleVersionCreated { rule_id, .. } => rule_id.clone(),
            Self::IsingInsightDetected { insight_id, .. } => insight_id.clone(),
            Self::IsingInsightEmitted { subject_ref, .. } => subject_ref.clone(),
            Self::IsingInsightSuppressed { insight_id, .. } => insight_id.clone(),
            Self::IsingRuleProposed { insight_id, .. } => insight_id.clone(),
            Self::IsingAnalyzerError { analyzer, .. } => analyzer.clone(),
            Self::IsingCatchupCompleted { .. } => "ising".to_string(),
            Self::IntentSubmitted { intent_id, .. }
            | Self::RefractDecomposed { intent_id, .. }
            | Self::RefractFailed { intent_id, .. } => intent_id.clone(),
            Self::TriggerFired { workflow_id, .. } => format!("workflow:{workflow_id}"),
            Self::StageEntered { artifact_id, .. }
            | Self::StageGatePassed { artifact_id, .. }
            | Self::StageGateFailed { artifact_id, .. }
            | Self::StageAdvanced { artifact_id, .. } => format!("workflow:{artifact_id}"),
            Self::TypeProposed { type_id, .. }
            | Self::TypeApproved { type_id, .. }
            | Self::TypeDeprecated { type_id, .. } => format!("type:{type_id}"),
            Self::AdapterRegistered { adapter_id, .. }
            | Self::AdapterDeprecated { adapter_id, .. } => format!("adapter:{adapter_id}"),
            Self::GateRegistered { evaluator_id, .. }
            | Self::GateDeprecated { evaluator_id, .. } => format!("gate:{evaluator_id}"),
            Self::ProfileRegistered { profile_id, .. }
            | Self::ProfileDeprecated { profile_id, .. } => format!("profile:{profile_id}"),
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
        }
    }
}

// ---------------------------------------------------------------------------
// Supporting enums
// ---------------------------------------------------------------------------

/// Whether a lineage entry is vertical (session→version) or horizontal
/// (artifact→artifact).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageType {
    Vertical,
    Horizontal,
}

/// Outcome of a shaping request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapingOutcome {
    Completed,
    Failed,
    Partial,
    Aborted,
}

/// Gate points where Synodic is consulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatePoint {
    PreDispatch,
    StateTransition,
    ConsumerRouting,
    ToolLevel,
}

/// Summary of a Synodic verdict (for event spine recording).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictSummary {
    Allow,
    Deny,
    Modify,
    Escalate,
}

/// Forge process states.
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

/// How an escalation was resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationResolution {
    Approved,
    Denied,
    TimedOut,
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

/// LLM token usage carried on [`FactoryEventKind::StiglabSessionCompleted`]
/// (issue #39). Accounting primitives only — USD cost is resolved downstream
/// by the budget consumer, not on the event, so we don't have to version the
/// pricing table every time a model changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cache-read input tokens (Anthropic-style prompt caching). Zero for
    /// providers without a cache concept.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Cache-creation input tokens.
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Model identifier (`"claude-sonnet-4-6"`, etc.) so the downstream
    /// pricing table can resolve cost without guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// What kind of rule change an [`FactoryEventKind::IsingRuleProposed`] is
/// asking Synodic to make.
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

/// How a rule proposal should be handled by Synodic.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_event_type_strings() {
        let event = FactoryEventKind::ArtifactRegistered {
            artifact_id: ArtifactId::new("art_test1234"),
            kind: Kind::Code,
            name: "my-service".into(),
            owner: "marvin".into(),
        };
        assert_eq!(event.event_type(), "artifact.registered");
        assert_eq!(event.stream_type(), "artifact");
        assert_eq!(event.stream_id(), "art_test1234");
    }

    #[test]
    fn forge_event_types() {
        assert_eq!(
            FactoryEventKind::ForgeIdleTick.event_type(),
            "forge.idle_tick"
        );
        assert_eq!(FactoryEventKind::ForgeIdleTick.stream_type(), "forge");
    }

    #[test]
    fn git_event_types_and_streams() {
        let event = FactoryEventKind::GitPrOpened {
            artifact_id: ArtifactId::new("art_git123"),
            repo: "onsager-ai/onsager".into(),
            pr_number: 42,
            url: "https://github.com/onsager-ai/onsager/pull/42".into(),
        };
        assert_eq!(event.event_type(), "git.pr_opened");
        assert_eq!(event.stream_type(), "git");
        assert_eq!(event.stream_id(), "art_git123");
    }

    #[test]
    fn serialization_roundtrip() {
        let event = FactoryEventKind::ArtifactStateChanged {
            artifact_id: ArtifactId::new("art_abcd1234"),
            from_state: ArtifactState::Draft,
            to_state: ArtifactState::InProgress,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "artifact_state_changed");
        assert_eq!(json["from_state"], "draft");
        assert_eq!(json["to_state"], "in_progress");

        let deserialized: FactoryEventKind = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.event_type(), "artifact.state_changed");
    }

    #[test]
    fn shaping_outcome_serde() {
        let outcome = ShapingOutcome::Completed;
        let json = serde_json::to_string(&outcome).unwrap();
        assert_eq!(json, r#""completed""#);
    }

    #[test]
    fn insight_scope_variants() {
        let global = InsightScope::Global;
        let json = serde_json::to_string(&global).unwrap();
        assert!(json.contains("global"));

        let specific = InsightScope::SpecificArtifact(ArtifactId::new("art_12345678"));
        let json = serde_json::to_string(&specific).unwrap();
        assert!(json.contains("art_12345678"));
    }

    #[test]
    fn ising_insight_emitted_roundtrip() {
        // Regression: the event_type / stream_type / stream_id triple must
        // survive a roundtrip so the listener can filter on `ising:<subject>`
        // and the dashboard can query by `event_type = "ising.insight_emitted"`.
        let event = FactoryEventKind::IsingInsightEmitted {
            signal_kind: "repeated_gate_override".into(),
            subject_ref: "code".into(),
            evidence: vec![
                EventRef {
                    event_id: 101,
                    event_type: "forge.gate_verdict".into(),
                },
                EventRef {
                    event_id: 103,
                    event_type: "forge.gate_verdict".into(),
                },
            ],
            confidence: 0.82,
        };
        assert_eq!(event.event_type(), "ising.insight_emitted");
        assert_eq!(event.stream_type(), "ising");
        assert_eq!(event.stream_id(), "code");

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "ising_insight_emitted");
        assert_eq!(json["signal_kind"], "repeated_gate_override");
        assert_eq!(json["subject_ref"], "code");
        assert_eq!(json["evidence"][0]["event_id"], 101);

        let back: FactoryEventKind = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn ising_rule_proposed_carries_routing_fields() {
        // Issue #36 Step 2 contract: a Synodic consumer must be able to
        // route the proposal without looking up the producing insight. The
        // event_type / stream_type / stream_id triple pins the dashboard
        // query path.
        let event = FactoryEventKind::IsingRuleProposed {
            insight_id: "ins_spine_101".into(),
            signal_kind: "repeated_gate_override".into(),
            subject_ref: "code".into(),
            proposed_action: RuleProposalAction::Retire {
                rule_id: "noisy-rule".into(),
            },
            class: RuleProposalClass::ReviewRequired,
            rationale: "80% override rate over 40 verdicts".into(),
            confidence: 0.85,
        };
        assert_eq!(event.event_type(), "ising.rule_proposed");
        assert_eq!(event.stream_type(), "ising");
        assert_eq!(event.stream_id(), "ins_spine_101");

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["class"], "review_required");
        assert_eq!(json["proposed_action"]["action"], "retire");
        let back: FactoryEventKind = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn refract_events_round_trip() {
        let submitted = FactoryEventKind::IntentSubmitted {
            intent_id: "int_abc".into(),
            intent_class: "file_migration".into(),
            description: "Migrate auth callers".into(),
            submitter: "marvin".into(),
        };
        assert_eq!(submitted.event_type(), "refract.intent_submitted");
        assert_eq!(submitted.stream_type(), "refract");
        assert_eq!(submitted.stream_id(), "int_abc");

        let decomposed = FactoryEventKind::RefractDecomposed {
            intent_id: "int_abc".into(),
            decomposer: "file_migration".into(),
            artifact_ids: vec!["art_1".into(), "art_2".into()],
        };
        assert_eq!(decomposed.event_type(), "refract.decomposed");
        assert_eq!(decomposed.stream_type(), "refract");

        let failed = FactoryEventKind::RefractFailed {
            intent_id: "int_abc".into(),
            reason: "no decomposer matched".into(),
        };
        assert_eq!(failed.event_type(), "refract.failed");
    }

    #[test]
    fn gate_resolution_proposed_round_trip() {
        let event = FactoryEventKind::SynodicGateResolutionProposed {
            escalation_id: "esc_42".into(),
            artifact_id: ArtifactId::new("art_ri"),
            proposer: "supervisor".into(),
            proposed_verdict: VerdictSummary::Allow,
            rationale: "supervisor reviewed the evidence".into(),
        };
        assert_eq!(event.event_type(), "synodic.gate_resolution_proposed");
        assert_eq!(event.stream_type(), "synodic");
        assert_eq!(event.stream_id(), "esc_42");
        let back: FactoryEventKind =
            serde_json::from_value(serde_json::to_value(&event).unwrap()).expect("round trip");
        assert_eq!(back, event);
    }

    #[test]
    fn token_usage_on_session_completed_is_optional() {
        // Without token_usage (legacy shape)
        let without = FactoryEventKind::StiglabSessionCompleted {
            session_id: "sess_1".into(),
            request_id: "req_1".into(),
            duration_ms: 123,
            artifact_id: None,
            token_usage: None,
            branch: None,
            pr_number: None,
        };
        let json = serde_json::to_value(&without).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("token_usage"),
            "None token_usage must be omitted for wire compatibility"
        );
        assert!(
            !json.as_object().unwrap().contains_key("branch"),
            "None branch must be omitted for wire compatibility"
        );
        assert!(
            !json.as_object().unwrap().contains_key("pr_number"),
            "None pr_number must be omitted for wire compatibility"
        );

        // With token_usage populated
        let with = FactoryEventKind::StiglabSessionCompleted {
            session_id: "sess_2".into(),
            request_id: "req_2".into(),
            duration_ms: 42,
            artifact_id: Some("art_x".into()),
            token_usage: Some(TokenUsage {
                input_tokens: 1_000,
                output_tokens: 500,
                cache_read_tokens: 200,
                cache_write_tokens: 100,
                model: Some("claude-sonnet-4-6".into()),
            }),
            branch: Some("claude/feature".into()),
            pr_number: Some(42),
        };
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["token_usage"]["input_tokens"], 1_000);
        assert_eq!(json["token_usage"]["model"], "claude-sonnet-4-6");
        assert_eq!(json["branch"], "claude/feature");
        assert_eq!(json["pr_number"], 42);
        let back: FactoryEventKind = serde_json::from_value(json).unwrap();
        assert_eq!(back, with);
    }

    #[test]
    fn git_events_serialize_deserialize() {
        let event = FactoryEventKind::GitCiCompleted {
            artifact_id: ArtifactId::new("art_pr_ci"),
            pr_number: 7,
            check_name: "ci/test".into(),
            conclusion: "success".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "git_ci_completed");
        assert_eq!(json["pr_number"], 7);

        let deserialized: FactoryEventKind = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, event);
        assert_eq!(deserialized.event_type(), "git.ci_completed");
    }

    // -- Phase 2 (Lever C) wire-format regressions ------------------------
    //
    // Phase-3 listeners will rely on these events keeping their shape across
    // upgrades; pin the additive-schema and round-trip behavior here.

    #[test]
    fn forge_gate_requested_request_field_serde_compat() {
        use crate::protocol::{GateContext, GateRequest, ProposedAction};

        // 1. With request = None, the field is omitted from the wire form
        //    (skip_serializing_if), keeping legacy JSON shape on emit.
        let event_without = FactoryEventKind::ForgeGateRequested {
            gate_id: "gate_no_request".into(),
            artifact_id: ArtifactId::new("art_legacy_shape"),
            gate_point: GatePoint::PreDispatch,
            request: None,
        };
        let json_without = serde_json::to_value(&event_without).unwrap();
        assert!(
            json_without.get("request").is_none(),
            "request: None must be omitted on serialization, got: {json_without}"
        );

        // 2. Legacy JSON lacking the `request` field deserializes with
        //    request = None — the #[serde(default)] contract that lets
        //    pre-Lever-C events still parse.
        let legacy_json = serde_json::json!({
            "type": "forge_gate_requested",
            "gate_id": "gate_legacy",
            "artifact_id": "art_legacy_shape",
            "gate_point": "pre_dispatch",
        });
        let parsed: FactoryEventKind = serde_json::from_value(legacy_json).unwrap();
        match parsed {
            FactoryEventKind::ForgeGateRequested { request, .. } => {
                assert!(
                    request.is_none(),
                    "legacy JSON must default request to None"
                );
            }
            other => panic!("expected ForgeGateRequested, got {other:?}"),
        }

        // 3. With request = Some(...), full payload round-trips. Phase 3
        //    consumers depend on the inner GateRequest staying byte-stable.
        let event_with = FactoryEventKind::ForgeGateRequested {
            gate_id: "gate_full".into(),
            artifact_id: ArtifactId::new("art_full_shape"),
            gate_point: GatePoint::StateTransition,
            request: Some(GateRequest {
                context: GateContext {
                    gate_point: GatePoint::StateTransition,
                    artifact_id: ArtifactId::new("art_full_shape"),
                    artifact_kind: Kind::Code,
                    current_state: ArtifactState::InProgress,
                    target_state: Some(ArtifactState::UnderReview),
                    extra: None,
                },
                proposed_action: ProposedAction {
                    description: "advance art_full_shape to UnderReview".into(),
                    payload: serde_json::json!({"summary": "ok"}),
                },
            }),
        };
        let json_with = serde_json::to_value(&event_with).unwrap();
        assert_eq!(json_with["type"], "forge_gate_requested");
        assert_eq!(
            json_with["request"]["context"]["gate_point"],
            "state_transition"
        );
        assert_eq!(
            json_with["request"]["proposed_action"]["description"],
            "advance art_full_shape to UnderReview"
        );

        let back: FactoryEventKind = serde_json::from_value(json_with).unwrap();
        assert_eq!(back, event_with);
        assert_eq!(back.event_type(), "forge.gate_requested");
    }

    #[test]
    fn stiglab_shaping_result_ready_roundtrip() {
        use crate::protocol::ShapingResult;
        use onsager_artifact::ContentRef;

        let event = FactoryEventKind::StiglabShapingResultReady {
            artifact_id: ArtifactId::new("art_shaped"),
            result: ShapingResult {
                request_id: "req_shaping_42".into(),
                outcome: ShapingOutcome::Completed,
                content_ref: Some(ContentRef {
                    uri: "git://repo@abc123".into(),
                    checksum: None,
                }),
                change_summary: "added auth middleware".into(),
                quality_signals: vec![],
                session_id: "sess_42".into(),
                duration_ms: 12_500,
                error: None,
            },
        };

        // event_type / stream routing — phase-3 listeners filter on these.
        assert_eq!(event.event_type(), "stiglab.shaping_result_ready");
        assert_eq!(event.stream_type(), "stiglab");
        assert_eq!(event.stream_id(), "art_shaped");

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "stiglab_shaping_result_ready");
        assert_eq!(json["artifact_id"], "art_shaped");
        assert_eq!(json["result"]["request_id"], "req_shaping_42");
        assert_eq!(json["result"]["outcome"], "completed");
        assert_eq!(json["result"]["content_ref"]["uri"], "git://repo@abc123");
        // checksum and error are skip_serializing_if Option::is_none
        assert!(json["result"]["content_ref"].get("checksum").is_none());
        assert!(json["result"].get("error").is_none());

        let back: FactoryEventKind = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}
