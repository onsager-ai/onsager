//! Inter-subsystem protocol types — the typed request/response contracts
//! between Forge, Stiglab, Synodic, and Ising.
//!
//! See `specs/subsystem-map-v0.1.md §4.1` for the four direct protocols and
//! `specs/forge-v0.1.md §5-7` for the detailed contracts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::artifact::{ArtifactId, ArtifactState, ContentRef, Kind, QualitySignal};
use crate::factory_event::{GatePoint, InsightKind, InsightScope, ShapingOutcome};

// ===========================================================================
// Forge → Stiglab: Imperative dispatch protocol
// ===========================================================================

/// A reference to another artifact used as horizontal lineage input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: ArtifactId,
    pub version: u32,
    pub role: String,
}

/// A constraint that Stiglab must respect during shaping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    /// Constraint type (e.g., "max_tokens", "tool_allowlist", "timeout_ms").
    #[serde(rename = "type")]
    pub constraint_type: String,
    /// Constraint value (interpretation depends on type).
    pub value: serde_json::Value,
}

/// Shaping request from Forge to Stiglab (forge-v0.1 §5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapingRequest {
    /// Unique request ID (ULID). Forge never mutates this. Used for
    /// idempotent dispatch (forge invariant #6).
    pub request_id: String,
    /// Which artifact to shape.
    pub artifact_id: ArtifactId,
    /// Which version to produce (usually `current_version + 1`).
    pub target_version: u32,
    /// Structured description of what the shaping should accomplish.
    pub shaping_intent: serde_json::Value,
    /// Other artifacts used as horizontal lineage inputs.
    pub inputs: Vec<ArtifactRef>,
    /// Governance and resource constraints.
    pub constraints: Vec<Constraint>,
    /// Optional soft deadline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<DateTime<Utc>>,
}

/// Error detail attached to failed or aborted shaping results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retriable: Option<bool>,
}

/// Shaping result from Stiglab back to Forge (forge-v0.1 §5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapingResult {
    /// Echoed from the original request.
    pub request_id: String,
    /// Outcome of the shaping attempt.
    pub outcome: ShapingOutcome,
    /// Pointer to the produced content (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_ref: Option<ContentRef>,
    /// Semantic summary of what changed.
    pub change_summary: String,
    /// Quality signals produced during shaping.
    pub quality_signals: Vec<QualitySignal>,
    /// Session ID for vertical lineage. Owned by Stiglab, not Forge.
    pub session_id: String,
    /// How long the shaping took.
    pub duration_ms: u64,
    /// Error detail if outcome is Failed or Aborted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDetail>,
}

// ===========================================================================
// Forge → Synodic: Gated governance protocol
// ===========================================================================

/// Context for a gate evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateContext {
    /// Which gate point is being consulted.
    pub gate_point: GatePoint,
    /// The artifact under evaluation.
    pub artifact_id: ArtifactId,
    pub artifact_kind: Kind,
    /// Current artifact state.
    pub current_state: ArtifactState,
    /// Target state (for state transitions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_state: Option<ArtifactState>,
    /// Additional context (e.g., consumer sink for routing gates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

/// The action Forge proposes and Synodic evaluates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedAction {
    /// Human-readable description.
    pub description: String,
    /// Structured payload for the action.
    pub payload: serde_json::Value,
}

/// Gate request from Forge to Synodic (forge-v0.1 §6.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateRequest {
    pub context: GateContext,
    pub proposed_action: ProposedAction,
}

/// Escalation context — returned when Synodic cannot decide autonomously.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationContext {
    pub escalation_id: String,
    pub reason: String,
    /// Who should resolve this (user, team, system).
    pub target: String,
    /// Timeout after which the default (conservative) verdict applies.
    pub timeout_at: DateTime<Utc>,
}

/// Gate verdict from Synodic to Forge (forge-v0.1 §6.2).
///
/// Forge honors this unconditionally. There is no override mechanism.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum GateVerdict {
    Allow,
    Deny { reason: String },
    Modify { new_action: ProposedAction },
    Escalate { context: EscalationContext },
}

// ===========================================================================
// Stiglab → Synodic: Tool-level gated protocol
// ===========================================================================

/// Tool-level gate request from inside a Stiglab session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolGateRequest {
    pub session_id: String,
    pub artifact_id: ArtifactId,
    /// The tool being invoked.
    pub tool_name: String,
    /// The tool's input payload.
    pub tool_input: serde_json::Value,
}

/// Tool-level gate verdict — simpler than Forge-level.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum ToolGateVerdict {
    Allow,
    Deny { reason: String },
    Escalate { context: EscalationContext },
}

// ===========================================================================
// Ising → Forge: Advisory insight protocol
// ===========================================================================

/// A reference to a factory event used as evidence for an insight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryEventRef {
    pub event_id: i64,
    pub event_type: String,
}

/// An optional action Ising suggests based on an insight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedAction {
    pub description: String,
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Insight from Ising to Forge (forge-v0.1 §7.2).
///
/// Advisory only — Forge may or may not act on it. Ising cannot block Forge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub insight_id: String,
    pub kind: InsightKind,
    pub scope: InsightScope,
    pub observation: String,
    pub evidence: Vec<FactoryEventRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<SuggestedAction>,
    pub confidence: f64,
}

// ===========================================================================
// Scheduling kernel interface
// ===========================================================================

/// The decision output of the scheduling kernel (forge-v0.1 §4.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapingDecision {
    pub artifact_id: ArtifactId,
    pub target_version: u32,
    pub target_state: ArtifactState,
    pub shaping_intent: serde_json::Value,
    pub inputs: Vec<ArtifactRef>,
    pub constraints: Vec<Constraint>,
    pub priority: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaping_request_roundtrip() {
        let req = ShapingRequest {
            request_id: "01HXYZ".into(),
            artifact_id: ArtifactId::new("art_test1234"),
            target_version: 2,
            shaping_intent: serde_json::json!({"action": "improve_tests"}),
            inputs: vec![],
            constraints: vec![],
            deadline: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        let back: ShapingRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back.request_id, "01HXYZ");
        assert_eq!(back.target_version, 2);
    }

    #[test]
    fn gate_verdict_variants() {
        let allow = GateVerdict::Allow;
        let json = serde_json::to_string(&allow).unwrap();
        assert!(json.contains("allow"));

        let deny = GateVerdict::Deny {
            reason: "unsafe operation".into(),
        };
        let json = serde_json::to_value(&deny).unwrap();
        assert_eq!(json["verdict"], "deny");
        assert_eq!(json["reason"], "unsafe operation");
    }

    #[test]
    fn insight_serialization() {
        let insight = Insight {
            insight_id: "ins_001".into(),
            kind: InsightKind::Failure,
            scope: InsightScope::Global,
            observation: "Code artifacts failing at 40% rate".into(),
            evidence: vec![FactoryEventRef {
                event_id: 42,
                event_type: "forge.shaping_returned".into(),
            }],
            suggested_action: None,
            confidence: 0.87,
        };
        let json = serde_json::to_value(&insight).unwrap();
        assert_eq!(json["kind"], "failure");
        assert_eq!(json["confidence"], 0.87);
    }
}
