//! Factory events — the authoritative event types emitted to the factory event
//! spine by each subsystem.
//!
//! See `specs/forge-v0.1.md §9` for the Forge event contract. Additional event
//! types from Stiglab, Synodic, and Ising are included here so that the spine
//! library provides a single typed vocabulary.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::artifact::{ArtifactId, ArtifactState, Kind, QualitySignal};

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
        artifact_id: ArtifactId,
        gate_point: GatePoint,
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
    },

    /// A session terminated with an error.
    StiglabSessionFailed {
        session_id: String,
        request_id: String,
        error: String,
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

    /// An insight was deduplicated or fell below confidence threshold (audit trail).
    IsingInsightSuppressed { insight_id: String, reason: String },

    /// An insight was packaged as a rule proposal for Synodic.
    IsingRuleProposed {
        insight_id: String,
        proposed_rule_description: String,
    },

    /// An analyzer encountered an error during its run.
    IsingAnalyzerError { analyzer: String, error: String },

    /// Ising finished catching up from a lag position.
    IsingCatchupCompleted { events_processed: u64 },

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
            Self::StiglabSessionFailed { .. } => "stiglab.session_failed",
            Self::StiglabSessionAborted { .. } => "stiglab.session_aborted",
            Self::StiglabEventUpgraded { .. } => "stiglab.event_upgraded",
            Self::StiglabNodeRegistered { .. } => "stiglab.node_registered",
            Self::StiglabNodeDeregistered { .. } => "stiglab.node_deregistered",
            Self::StiglabNodeHeartbeatMissed { .. } => "stiglab.node_heartbeat_missed",
            Self::SynodicGateEvaluated { .. } => "synodic.gate_evaluated",
            Self::SynodicGateDenied { .. } => "synodic.gate_denied",
            Self::SynodicGateModified { .. } => "synodic.gate_modified",
            Self::SynodicEscalationStarted { .. } => "synodic.escalation_started",
            Self::SynodicEscalationResolved { .. } => "synodic.escalation_resolved",
            Self::SynodicEscalationTimedOut { .. } => "synodic.escalation_timed_out",
            Self::SynodicRuleProposed { .. } => "synodic.rule_proposed",
            Self::SynodicRuleApproved { .. } => "synodic.rule_approved",
            Self::SynodicRuleDisabled { .. } => "synodic.rule_disabled",
            Self::SynodicRuleVersionCreated { .. } => "synodic.rule_version_created",
            Self::IsingInsightDetected { .. } => "ising.insight_detected",
            Self::IsingInsightSuppressed { .. } => "ising.insight_suppressed",
            Self::IsingRuleProposed { .. } => "ising.rule_proposed",
            Self::IsingAnalyzerError { .. } => "ising.analyzer_error",
            Self::IsingCatchupCompleted { .. } => "ising.catchup_completed",
            Self::TypeProposed { .. } => "registry.type_proposed",
            Self::TypeApproved { .. } => "registry.type_approved",
            Self::TypeDeprecated { .. } => "registry.type_deprecated",
            Self::AdapterRegistered { .. } => "registry.adapter_registered",
            Self::AdapterDeprecated { .. } => "registry.adapter_deprecated",
            Self::GateRegistered { .. } => "registry.gate_registered",
            Self::GateDeprecated { .. } => "registry.gate_deprecated",
            Self::ProfileRegistered { .. } => "registry.profile_registered",
            Self::ProfileDeprecated { .. } => "registry.profile_deprecated",
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
            | Self::StiglabSessionFailed { .. }
            | Self::StiglabSessionAborted { .. }
            | Self::StiglabEventUpgraded { .. }
            | Self::StiglabNodeRegistered { .. }
            | Self::StiglabNodeDeregistered { .. }
            | Self::StiglabNodeHeartbeatMissed { .. } => "stiglab",
            Self::SynodicGateEvaluated { .. }
            | Self::SynodicGateDenied { .. }
            | Self::SynodicGateModified { .. }
            | Self::SynodicEscalationStarted { .. }
            | Self::SynodicEscalationResolved { .. }
            | Self::SynodicEscalationTimedOut { .. }
            | Self::SynodicRuleProposed { .. }
            | Self::SynodicRuleApproved { .. }
            | Self::SynodicRuleDisabled { .. }
            | Self::SynodicRuleVersionCreated { .. } => "synodic",
            Self::IsingInsightDetected { .. }
            | Self::IsingInsightSuppressed { .. }
            | Self::IsingRuleProposed { .. }
            | Self::IsingAnalyzerError { .. }
            | Self::IsingCatchupCompleted { .. } => "ising",
            Self::TypeProposed { .. }
            | Self::TypeApproved { .. }
            | Self::TypeDeprecated { .. }
            | Self::AdapterRegistered { .. }
            | Self::AdapterDeprecated { .. }
            | Self::GateRegistered { .. }
            | Self::GateDeprecated { .. }
            | Self::ProfileRegistered { .. }
            | Self::ProfileDeprecated { .. } => "registry",
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
            Self::StiglabNodeRegistered { node_id, .. }
            | Self::StiglabNodeDeregistered { node_id, .. }
            | Self::StiglabNodeHeartbeatMissed { node_id, .. } => node_id.clone(),
            Self::SynodicGateEvaluated { gate_id, .. } => gate_id.clone(),
            Self::SynodicGateDenied { gate_id, .. } => gate_id.clone(),
            Self::SynodicGateModified { gate_id, .. } => gate_id.clone(),
            Self::SynodicEscalationStarted { escalation_id, .. } => escalation_id.clone(),
            Self::SynodicEscalationResolved { escalation_id, .. } => escalation_id.clone(),
            Self::SynodicEscalationTimedOut { escalation_id, .. } => escalation_id.clone(),
            Self::SynodicRuleProposed { rule_id, .. } => rule_id.clone(),
            Self::SynodicRuleApproved { rule_id, .. } => rule_id.clone(),
            Self::SynodicRuleDisabled { rule_id, .. } => rule_id.clone(),
            Self::SynodicRuleVersionCreated { rule_id, .. } => rule_id.clone(),
            Self::IsingInsightDetected { insight_id, .. } => insight_id.clone(),
            Self::IsingInsightSuppressed { insight_id, .. } => insight_id.clone(),
            Self::IsingRuleProposed { insight_id, .. } => insight_id.clone(),
            Self::IsingAnalyzerError { analyzer, .. } => analyzer.clone(),
            Self::IsingCatchupCompleted { .. } => "ising".to_string(),
            Self::TypeProposed { type_id, .. }
            | Self::TypeApproved { type_id, .. }
            | Self::TypeDeprecated { type_id, .. } => format!("type:{type_id}"),
            Self::AdapterRegistered { adapter_id, .. }
            | Self::AdapterDeprecated { adapter_id, .. } => format!("adapter:{adapter_id}"),
            Self::GateRegistered { evaluator_id, .. }
            | Self::GateDeprecated { evaluator_id, .. } => format!("gate:{evaluator_id}"),
            Self::ProfileRegistered { profile_id, .. }
            | Self::ProfileDeprecated { profile_id, .. } => format!("profile:{profile_id}"),
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
}
