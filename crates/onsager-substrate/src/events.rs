//! Substrate event payload structs (RUN-02, [#360]).
//!
//! This module is the authoring surface for events the 0.2 substrate
//! emits onto the spine — the scheduler ([`onsager-nodes`]) and the
//! executor catalog ([`Script`], [`Agent`], [`Verify`], [`Human`],
//! [`SubWorkflow`]). Each struct mirrors a [`FactoryEventKind`] variant
//! over in `onsager-spine`; the two halves intentionally carry the same
//! payload shape:
//!
//! - This module defines the typed authoring shape substrate-side code
//!   reaches for ("emit a `NodeStarted { ... }`").
//! - `onsager-spine::FactoryEventKind` carries the wire vocabulary the
//!   spine read API hands to the dashboard and (post OBS-01, #361) to
//!   Observers.
//!
//! Keeping the structs here — rather than embedding `FactoryEventKind`
//! variants by reference — avoids forcing `onsager-spine` to depend on
//! `onsager-substrate`. The two are tied together by an integration
//! test that asserts byte-identical JSON for every event type (see
//! `tests/spine_payload_compat.rs` in this crate).
//!
//! ## Schema stability
//!
//! Every struct here uses `#[serde(default,
//! skip_serializing_if = "Option::is_none")]` on optional fields, so
//! adding a new optional field is wire-compatible — old deserializers
//! ignore the new field, new deserializers fill the field with
//! `None` for old payloads. Renaming or removing fields is a
//! schema-version bump in the registry manifest (see
//! `crates/onsager-registry/src/events.rs`).
//!
//! ## Catalog
//!
//! | Wire kind                    | Struct                                  |
//! |------------------------------|-----------------------------------------|
//! | `artifact.state_changed`     | [`ArtifactStateChanged`]                |
//! | `node.started`               | [`NodeStarted`]                         |
//! | `node.completed`             | [`NodeCompleted`]                       |
//! | `node.failed`                | [`NodeFailed`]                          |
//! | `node.awaiting_human`        | [`NodeAwaitingHuman`]                   |
//! | `node.human_approved`        | [`NodeHumanApproved`]                   |
//! | `node.human_rejected`        | [`NodeHumanRejected`]                   |
//! | `synodic.verdict`            | [`SynodicVerdict`]                      |
//! | `agent.session_started`      | [`AgentSessionStarted`]                 |
//! | `agent.session_completed`    | [`AgentSessionCompleted`]               |
//! | `agent.session_failed`       | [`AgentSessionFailed`]                  |
//!
//! [#360]: https://github.com/onsager-ai/onsager/issues/360
//! [`onsager-nodes`]: https://docs.rs/onsager-nodes
//! [`Script`]: https://docs.rs/onsager-nodes/latest/onsager_nodes/script/struct.ScriptExecutor.html
//! [`Agent`]: https://docs.rs/onsager-nodes/latest/onsager_nodes/agent/struct.AgentExecutor.html
//! [`Verify`]: https://docs.rs/onsager-nodes/latest/onsager_nodes/verify/struct.VerifyExecutor.html
//! [`Human`]: https://docs.rs/onsager-nodes/latest/onsager_nodes/human/struct.HumanExecutor.html
//! [`SubWorkflow`]: https://docs.rs/onsager-nodes/latest/onsager_nodes/subworkflow/struct.SubWorkflowExecutor.html
//! [`FactoryEventKind`]: https://docs.rs/onsager-spine/latest/onsager_spine/enum.FactoryEventKind.html

use onsager_artifact::{ArtifactId, ArtifactState, NodeId};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Wire-kind constants
// ---------------------------------------------------------------------------
//
// One per struct, matching `FactoryEventKind::event_type()`. Constants
// (not derived from struct names) so a typo on either side surfaces as a
// compile-time mismatch in the compat tests rather than a silent
// rename.

pub const KIND_ARTIFACT_STATE_CHANGED: &str = "artifact.state_changed";
pub const KIND_NODE_STARTED: &str = "node.started";
pub const KIND_NODE_COMPLETED: &str = "node.completed";
pub const KIND_NODE_FAILED: &str = "node.failed";
pub const KIND_NODE_AWAITING_HUMAN: &str = "node.awaiting_human";
pub const KIND_NODE_HUMAN_APPROVED: &str = "node.human_approved";
pub const KIND_NODE_HUMAN_REJECTED: &str = "node.human_rejected";
pub const KIND_SYNODIC_VERDICT: &str = "synodic.verdict";
pub const KIND_AGENT_SESSION_STARTED: &str = "agent.session_started";
pub const KIND_AGENT_SESSION_COMPLETED: &str = "agent.session_completed";
pub const KIND_AGENT_SESSION_FAILED: &str = "agent.session_failed";

// ---------------------------------------------------------------------------
// Substrate event payloads
// ---------------------------------------------------------------------------

/// Artifact moved to a new lifecycle state.
///
/// Wire-equivalent to
/// [`FactoryEventKind::ArtifactStateChanged`](https://docs.rs/onsager-spine/latest/onsager_spine/enum.FactoryEventKind.html#variant.ArtifactStateChanged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactStateChanged {
    pub artifact_id: ArtifactId,
    pub from_state: ArtifactState,
    pub to_state: ArtifactState,
}

impl ArtifactStateChanged {
    pub fn kind(&self) -> &'static str {
        KIND_ARTIFACT_STATE_CHANGED
    }
}

/// Substrate scheduler dispatched a node — execution began.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStarted {
    /// `PlanId` for this Execution Plan run (carried as a `String` so
    /// the substrate crate does not depend on the runtime; deserialize
    /// via `PlanId::new(...)` on the consumer side).
    pub plan_id: String,
    pub node_id: NodeId,
    /// Executor catalog key (`"script"`, `"agent"`, `"verify"`,
    /// `"human"`, `"sub_workflow"`, `"noop"`).
    pub executor_kind: String,
}

impl NodeStarted {
    pub fn kind(&self) -> &'static str {
        KIND_NODE_STARTED
    }
}

/// A node finished successfully; the scheduler persisted each output
/// artifact under its declared edge `ArtifactId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCompleted {
    pub plan_id: String,
    pub node_id: NodeId,
    /// `ArtifactId`s the executor materialized — order matches the
    /// node's declared output edges.
    pub output_artifact_ids: Vec<ArtifactId>,
}

impl NodeCompleted {
    pub fn kind(&self) -> &'static str {
        KIND_NODE_COMPLETED
    }
}

/// A node's executor returned Err. The scheduler aborts the plan
/// (v1; no retries).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeFailed {
    pub plan_id: String,
    pub node_id: NodeId,
    /// Free-text error message from the executor.
    pub error: String,
}

impl NodeFailed {
    pub fn kind(&self) -> &'static str {
        KIND_NODE_FAILED
    }
}

/// A Human executor is parked waiting on an out-of-band approval
/// decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAwaitingHuman {
    pub plan_id: String,
    pub node_id: NodeId,
    /// Free-text prompt shown to the human reviewer.
    pub prompt: String,
}

impl NodeAwaitingHuman {
    pub fn kind(&self) -> &'static str {
        KIND_NODE_AWAITING_HUMAN
    }
}

/// A pending Human executor node received an approval decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeHumanApproved {
    pub plan_id: String,
    pub node_id: NodeId,
    /// Actor identifier — `"human:<id>"` for a dashboard user,
    /// `"supervisor"` for a delegate agent.
    pub approved_by: String,
}

impl NodeHumanApproved {
    pub fn kind(&self) -> &'static str {
        KIND_NODE_HUMAN_APPROVED
    }
}

/// A pending Human executor node received a rejection decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeHumanRejected {
    pub plan_id: String,
    pub node_id: NodeId,
    /// Actor identifier — same shape as `approved_by` above.
    pub rejected_by: String,
    /// Free-text justification carried into the audit trail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl NodeHumanRejected {
    pub fn kind(&self) -> &'static str {
        KIND_NODE_HUMAN_REJECTED
    }
}

/// Verify executor produced a verdict — pass / fail outcome with
/// per-check details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SynodicVerdict {
    pub plan_id: String,
    pub node_id: NodeId,
    /// True when every check the executor ran passed.
    pub passed: bool,
    /// Per-check results — one entry per [`Check`] the executor ran.
    ///
    /// [`Check`]: https://docs.rs/onsager-nodes/latest/onsager_nodes/verify/enum.Check.html
    pub check_results: Vec<VerifyCheckResult>,
}

impl SynodicVerdict {
    pub fn kind(&self) -> &'static str {
        KIND_SYNODIC_VERDICT
    }
}

/// One check's outcome carried on [`SynodicVerdict`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyCheckResult {
    /// Check name (e.g. `"cargo_test"`, `"clippy"`, `"schema_lint"`).
    pub name: String,
    /// `true` when this check passed.
    pub passed: bool,
}

/// Agent executor opened an LLM session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionStarted {
    pub plan_id: String,
    pub node_id: NodeId,
    /// Session identifier minted by the agent runner.
    pub session_id: String,
    /// Model name (`"claude-sonnet-4-6"`, etc.).
    pub model: String,
}

impl AgentSessionStarted {
    pub fn kind(&self) -> &'static str {
        KIND_AGENT_SESSION_STARTED
    }
}

/// Agent executor's LLM session finished successfully.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSessionCompleted {
    pub plan_id: String,
    pub node_id: NodeId,
    pub session_id: String,
    /// Token usage for this session — `None` when the runner does not
    /// report it (stub / mock runners).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsage>,
}

impl AgentSessionCompleted {
    pub fn kind(&self) -> &'static str {
        KIND_AGENT_SESSION_COMPLETED
    }
}

/// LLM token usage carried on [`AgentSessionCompleted`]. Mirrors the
/// shape used by `onsager-spine::TokenUsage` so the dashboard and
/// budget consumers see the same fields whether the row originated
/// here or from the legacy `stiglab.session_completed` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cache-read input tokens (Anthropic-style prompt caching). Zero
    /// for providers without a cache concept.
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

/// Agent executor's LLM session terminated with an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionFailed {
    pub plan_id: String,
    pub node_id: NodeId,
    pub session_id: String,
    /// Free-text error from the runner.
    pub error: String,
}

impl AgentSessionFailed {
    pub fn kind(&self) -> &'static str {
        KIND_AGENT_SESSION_FAILED
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pid() -> String {
        "plan_test_0001".to_string()
    }
    fn nid() -> NodeId {
        NodeId::new(uuid::Uuid::nil())
    }

    #[test]
    fn kinds_match_wire_names() {
        assert_eq!(KIND_NODE_STARTED, "node.started");
        assert_eq!(KIND_NODE_COMPLETED, "node.completed");
        assert_eq!(KIND_NODE_FAILED, "node.failed");
        assert_eq!(KIND_NODE_AWAITING_HUMAN, "node.awaiting_human");
        assert_eq!(KIND_NODE_HUMAN_APPROVED, "node.human_approved");
        assert_eq!(KIND_NODE_HUMAN_REJECTED, "node.human_rejected");
        assert_eq!(KIND_SYNODIC_VERDICT, "synodic.verdict");
        assert_eq!(KIND_AGENT_SESSION_STARTED, "agent.session_started");
        assert_eq!(KIND_AGENT_SESSION_COMPLETED, "agent.session_completed");
        assert_eq!(KIND_AGENT_SESSION_FAILED, "agent.session_failed");
        assert_eq!(KIND_ARTIFACT_STATE_CHANGED, "artifact.state_changed");
    }

    #[test]
    fn node_started_roundtrip() {
        let ev = NodeStarted {
            plan_id: pid(),
            node_id: nid(),
            executor_kind: "script".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["plan_id"], pid());
        assert_eq!(json["executor_kind"], "script");
        let back: NodeStarted = serde_json::from_value(json).unwrap();
        assert_eq!(back, ev);
        assert_eq!(ev.kind(), "node.started");
    }

    #[test]
    fn node_completed_roundtrip() {
        let ev = NodeCompleted {
            plan_id: pid(),
            node_id: nid(),
            output_artifact_ids: vec![ArtifactId::new("art_1"), ArtifactId::new("art_2")],
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["output_artifact_ids"][0], "art_1");
        let back: NodeCompleted = serde_json::from_value(json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn node_failed_roundtrip() {
        let ev = NodeFailed {
            plan_id: pid(),
            node_id: nid(),
            error: "boom".into(),
        };
        let back: NodeFailed = serde_json::from_value(serde_json::to_value(&ev).unwrap()).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn synodic_verdict_roundtrip() {
        let ev = SynodicVerdict {
            plan_id: pid(),
            node_id: nid(),
            passed: false,
            check_results: vec![
                VerifyCheckResult {
                    name: "cargo_test".into(),
                    passed: true,
                },
                VerifyCheckResult {
                    name: "clippy".into(),
                    passed: false,
                },
            ],
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["check_results"][1]["name"], "clippy");
        assert_eq!(json["check_results"][1]["passed"], false);
        let back: SynodicVerdict = serde_json::from_value(json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn agent_session_completed_omits_none_token_usage() {
        let ev = AgentSessionCompleted {
            plan_id: pid(),
            node_id: nid(),
            session_id: "sess_x".into(),
            token_usage: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert!(
            json.get("token_usage").is_none(),
            "None token_usage must be omitted, got: {json}"
        );
    }

    /// Schema-stability check from issue #360 verification: adding a new
    /// optional field to a payload must not break consumers
    /// deserializing the older shape. We exercise the rule by
    /// deserializing a payload that lacks the future field — `None` is
    /// the correct decoded value.
    #[test]
    fn schema_stable_optional_fields_default_to_none() {
        // `NodeHumanRejected.reason` is the canonical optional field on
        // a substrate event. Producing a payload without it (the way
        // an older emitter would) must still deserialize cleanly.
        let json = serde_json::json!({
            "plan_id": pid(),
            "node_id": nid(),
            "rejected_by": "human:42",
        });
        let ev: NodeHumanRejected = serde_json::from_value(json).unwrap();
        assert_eq!(ev.reason, None);

        // Mirror for `AgentSessionCompleted.token_usage`.
        let json = serde_json::json!({
            "plan_id": pid(),
            "node_id": nid(),
            "session_id": "sess_42",
        });
        let ev: AgentSessionCompleted = serde_json::from_value(json).unwrap();
        assert_eq!(ev.token_usage, None);
    }
}
