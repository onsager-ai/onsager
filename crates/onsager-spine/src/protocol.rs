//! Inter-subsystem request/response payload types — the typed shapes that
//! Legacy 0.1 subsystem exchange payloads carried on spine events.
//!
//! These types previously lived in the `onsager-protocol` crate, where they
//! described HTTP request/response contracts. Per spec #131 / ADR 0004
//! Lever C, the HTTP transport between subsystems is gone and these shapes
//! now travel as spine event payloads. The crate moved here so the wire
//! format and the carrier (the spine) live together.
//!
//! See `specs/forge-v0.1.md §5-7` for the field-level contracts.

use chrono::{DateTime, Utc};
use onsager_artifact::{ArtifactId, ArtifactState, Kind};
use serde::{Deserialize, Serialize};

use crate::factory_event::{GatePoint, InsightKind, InsightScope};

// ===========================================================================
// Legacy 0.1 shaping payloads (forge-v0.1 §5)
// ===========================================================================
//
// The `ShapingRequest` / `ShapingResult` exchange types retired with
// stiglab's shaping path (ADR 0027 / spec #583); only the building
// blocks still referenced by `ShapingDecision` remain.

/// A reference to another artifact used as horizontal lineage input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: ArtifactId,
    pub version: u32,
    pub role: String,
}

/// A constraint a shaping session must respect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    /// Constraint type (e.g., "max_tokens", "tool_allowlist", "timeout_ms").
    #[serde(rename = "type")]
    pub constraint_type: String,
    /// Constraint value (interpretation depends on type).
    pub value: serde_json::Value,
}

// ===========================================================================
// Portal → scheduler: multi-repo session-launch handoff (spec #546)
// ===========================================================================

/// One repo handed to an agent session at launch, with read-scoped
/// access (spec #546, Part of #544).
///
/// A workspace-scoped run carries one entry per bound repo (resolved via
/// `list_workspace_repo_installs`); the repos may span multiple GitHub
/// orgs, each minted its own installation token. The single-repo / pinned
/// case degenerates to a one-element vec, and a workspace with no bound
/// repos to an empty vec — both reproduce today's single-`working_dir`
/// behavior exactly.
///
/// The token is **read-scoped** (`contents:read` + `metadata:read`) and
/// **encrypted** with the shared credential key — never the raw token on
/// the wire. The consumer (the scheduler's agent launch path) decrypts it
/// and builds the authenticated clone URL before surfacing it to the
/// agent. *Writes* to origin are out of scope here and gated separately
/// (#548).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoAccess {
    /// GitHub repo owner (org or user login).
    pub owner: String,
    /// GitHub repo name (short name, no owner prefix).
    pub name: String,
    /// The repo's default branch, so the agent can base work off it
    /// without an extra API round-trip.
    pub default_branch: String,
    /// Read-scoped GitHub App installation token, **encrypted** with the
    /// shared credential key. `None` when the App is unconfigured or the
    /// mint failed — the repo is still surfaced by identity (a public
    /// repo can be cloned unauthenticated; a private one will fail loudly
    /// rather than silently).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_read_token: Option<String>,
}

// ===========================================================================
// Legacy gate evaluation payloads (0.1 gate protocol)
// ===========================================================================

/// Context for a gate evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// The action a gate request proposes for evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedAction {
    /// Human-readable description.
    pub description: String,
    /// Structured payload for the action.
    pub payload: serde_json::Value,
}

/// Gate request payload (forge-v0.1 §6.2; legacy).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateRequest {
    pub context: GateContext,
    pub proposed_action: ProposedAction,
}

/// Escalation context — returned when a gate cannot decide autonomously.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EscalationContext {
    pub escalation_id: String,
    pub reason: String,
    /// Who should resolve this (user, team, system).
    pub target: String,
    /// Timeout after which the default (conservative) verdict applies.
    pub timeout_at: DateTime<Utc>,
}

/// Gate verdict payload (forge-v0.1 §6.2; legacy).
///
/// Forge honors this unconditionally. There is no override mechanism.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum GateVerdict {
    Allow,
    Deny { reason: String },
    Modify { new_action: ProposedAction },
    Escalate { context: EscalationContext },
}

// ===========================================================================
// Tool-level gate payloads (legacy)
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
// Ising → Forge: advisory insight payloads
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
    fn repo_access_roundtrip_and_token_wire_optional() {
        // Spec #546: the read token rides encrypted and is omitted from
        // the wire when absent (public-repo / unconfigured-App path).
        let with = RepoAccess {
            owner: "acme".into(),
            name: "widgets".into(),
            default_branch: "main".into(),
            encrypted_read_token: Some("enc_abc".into()),
        };
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["encrypted_read_token"], "enc_abc");
        let back: RepoAccess = serde_json::from_value(json).unwrap();
        assert_eq!(back, with);

        let without = RepoAccess {
            encrypted_read_token: None,
            ..with
        };
        let json = serde_json::to_value(&without).unwrap();
        assert!(
            !json
                .as_object()
                .unwrap()
                .contains_key("encrypted_read_token"),
            "a None read token must be omitted from the wire"
        );
        // Legacy / tokenless payloads still parse (serde default).
        let parsed: RepoAccess = serde_json::from_value(serde_json::json!({
            "owner": "acme", "name": "widgets", "default_branch": "main"
        }))
        .expect("tokenless RepoAccess must deserialize");
        assert!(parsed.encrypted_read_token.is_none());
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
