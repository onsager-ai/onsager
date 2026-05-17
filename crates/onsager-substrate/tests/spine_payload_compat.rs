//! Cross-crate compat: every substrate event payload struct in
//! `onsager_substrate::events` serializes to the same JSON object the
//! corresponding `FactoryEventKind` variant emits (modulo the
//! enum's `"type"` discriminator).
//!
//! Two halves carry the same payload shape (RUN-02, #360):
//!
//! - `onsager-substrate::events` — typed authoring surface.
//! - `onsager-spine::FactoryEventKind::*` — wire vocabulary.
//!
//! Field-name / field-type drift between the two would silently break
//! the dashboard run timeline (which reads `FactoryEventKind`) and any
//! future scheduler→spine adapter (which authors from the substrate
//! side). The asserts below pin the shape so a typo or rename surfaces
//! at `cargo test`, not in production.
//!
//! The check is structural: we serialize both halves, strip the
//! `FactoryEventKind` `"type"` tag, and assert the resulting JSON
//! objects are equal. Adding an optional field to the substrate struct
//! without also adding it to the variant would diff here; same for the
//! reverse.

use onsager_artifact::{ArtifactId, NodeId};
use onsager_spine::factory_event::{self as fe, FactoryEventKind};
use onsager_substrate::events as se;
use serde_json::Value;

/// Drop the `"type"` discriminator the `FactoryEventKind` enum
/// injects via `#[serde(tag = "type")]` so the remaining payload
/// matches the substrate struct's serialization byte-for-byte.
fn strip_type(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("type");
    }
    v
}

fn pid() -> String {
    "plan_test_42".into()
}
fn nid() -> NodeId {
    NodeId::new(uuid::Uuid::nil())
}

#[test]
fn node_started_payload_matches_variant() {
    let s = se::NodeStarted {
        plan_id: pid(),
        node_id: nid(),
        executor_kind: "script".into(),
    };
    let v = FactoryEventKind::NodeStarted {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        executor_kind: s.executor_kind.clone(),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

#[test]
fn node_completed_payload_matches_variant() {
    let s = se::NodeCompleted {
        plan_id: pid(),
        node_id: nid(),
        output_artifact_ids: vec![ArtifactId::new("art_1"), ArtifactId::new("art_2")],
    };
    let v = FactoryEventKind::NodeCompleted {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        output_artifact_ids: s.output_artifact_ids.clone(),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

#[test]
fn node_failed_payload_matches_variant() {
    let s = se::NodeFailed {
        plan_id: pid(),
        node_id: nid(),
        error: "boom".into(),
    };
    let v = FactoryEventKind::NodeFailed {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        error: s.error.clone(),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

#[test]
fn node_awaiting_human_payload_matches_variant() {
    let s = se::NodeAwaitingHuman {
        plan_id: pid(),
        node_id: nid(),
        prompt: "approve?".into(),
    };
    let v = FactoryEventKind::NodeAwaitingHuman {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        prompt: s.prompt.clone(),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

#[test]
fn node_human_approved_payload_matches_variant() {
    let s = se::NodeHumanApproved {
        plan_id: pid(),
        node_id: nid(),
        approved_by: "human:42".into(),
    };
    let v = FactoryEventKind::NodeHumanApproved {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        approved_by: s.approved_by.clone(),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

#[test]
fn node_human_rejected_payload_matches_variant_with_and_without_reason() {
    // With reason.
    let s = se::NodeHumanRejected {
        plan_id: pid(),
        node_id: nid(),
        rejected_by: "human:42".into(),
        reason: Some("scope creep".into()),
    };
    let v = FactoryEventKind::NodeHumanRejected {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        rejected_by: s.rejected_by.clone(),
        reason: s.reason.clone(),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );

    // Without reason — the optional field must be omitted on both
    // sides (same `skip_serializing_if = "Option::is_none"` behavior).
    let s = se::NodeHumanRejected { reason: None, ..s };
    let v = FactoryEventKind::NodeHumanRejected {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        rejected_by: s.rejected_by.clone(),
        reason: None,
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

#[test]
fn synodic_verdict_payload_matches_variant() {
    let s = se::SynodicVerdict {
        plan_id: pid(),
        node_id: nid(),
        passed: false,
        check_results: vec![
            se::VerifyCheckResult {
                name: "cargo_test".into(),
                passed: true,
            },
            se::VerifyCheckResult {
                name: "clippy".into(),
                passed: false,
            },
        ],
    };
    let v = FactoryEventKind::SynodicVerdict {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        passed: s.passed,
        check_results: s
            .check_results
            .iter()
            .map(|c| fe::VerifyCheckResult {
                name: c.name.clone(),
                passed: c.passed,
            })
            .collect(),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

#[test]
fn agent_session_started_payload_matches_variant() {
    let s = se::AgentSessionStarted {
        plan_id: pid(),
        node_id: nid(),
        session_id: "sess_42".into(),
        model: "claude-sonnet-4-6".into(),
    };
    let v = FactoryEventKind::AgentSessionStarted {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        session_id: s.session_id.clone(),
        model: s.model.clone(),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

#[test]
fn agent_session_completed_payload_matches_variant() {
    // With token_usage.
    let s = se::AgentSessionCompleted {
        plan_id: pid(),
        node_id: nid(),
        session_id: "sess_42".into(),
        token_usage: Some(se::TokenUsage {
            input_tokens: 1_200,
            output_tokens: 340,
            cache_read_tokens: 800,
            cache_write_tokens: 0,
            model: Some("claude-sonnet-4-6".into()),
        }),
    };
    let v = FactoryEventKind::AgentSessionCompleted {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        session_id: s.session_id.clone(),
        token_usage: s.token_usage.as_ref().map(|t| fe::TokenUsage {
            input_tokens: t.input_tokens,
            output_tokens: t.output_tokens,
            cache_read_tokens: t.cache_read_tokens,
            cache_write_tokens: t.cache_write_tokens,
            model: t.model.clone(),
        }),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );

    // Without token_usage — both sides must omit the field.
    let s = se::AgentSessionCompleted {
        token_usage: None,
        ..s
    };
    let v = FactoryEventKind::AgentSessionCompleted {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        session_id: s.session_id.clone(),
        token_usage: None,
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

#[test]
fn agent_session_failed_payload_matches_variant() {
    let s = se::AgentSessionFailed {
        plan_id: pid(),
        node_id: nid(),
        session_id: "sess_42".into(),
        error: "api went down".into(),
    };
    let v = FactoryEventKind::AgentSessionFailed {
        plan_id: s.plan_id.clone(),
        node_id: s.node_id,
        session_id: s.session_id.clone(),
        error: s.error.clone(),
    };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        strip_type(serde_json::to_value(&v).unwrap()),
    );
}

/// Wire-kind constants in the substrate module match
/// `FactoryEventKind::event_type()` for the corresponding variant —
/// otherwise dashboards filtering by kind string would miss substrate
/// emissions silently.
#[test]
fn wire_kind_constants_agree_with_factory_event_kind() {
    use FactoryEventKind as F;
    let pairs: &[(&str, F)] = &[
        (
            se::KIND_NODE_STARTED,
            F::NodeStarted {
                plan_id: pid(),
                node_id: nid(),
                executor_kind: "script".into(),
            },
        ),
        (
            se::KIND_NODE_COMPLETED,
            F::NodeCompleted {
                plan_id: pid(),
                node_id: nid(),
                output_artifact_ids: vec![],
            },
        ),
        (
            se::KIND_NODE_FAILED,
            F::NodeFailed {
                plan_id: pid(),
                node_id: nid(),
                error: "x".into(),
            },
        ),
        (
            se::KIND_NODE_AWAITING_HUMAN,
            F::NodeAwaitingHuman {
                plan_id: pid(),
                node_id: nid(),
                prompt: "x".into(),
            },
        ),
        (
            se::KIND_NODE_HUMAN_APPROVED,
            F::NodeHumanApproved {
                plan_id: pid(),
                node_id: nid(),
                approved_by: "x".into(),
            },
        ),
        (
            se::KIND_NODE_HUMAN_REJECTED,
            F::NodeHumanRejected {
                plan_id: pid(),
                node_id: nid(),
                rejected_by: "x".into(),
                reason: None,
            },
        ),
        (
            se::KIND_SYNODIC_VERDICT,
            F::SynodicVerdict {
                plan_id: pid(),
                node_id: nid(),
                passed: true,
                check_results: vec![],
            },
        ),
        (
            se::KIND_AGENT_SESSION_STARTED,
            F::AgentSessionStarted {
                plan_id: pid(),
                node_id: nid(),
                session_id: "x".into(),
                model: "x".into(),
            },
        ),
        (
            se::KIND_AGENT_SESSION_COMPLETED,
            F::AgentSessionCompleted {
                plan_id: pid(),
                node_id: nid(),
                session_id: "x".into(),
                token_usage: None,
            },
        ),
        (
            se::KIND_AGENT_SESSION_FAILED,
            F::AgentSessionFailed {
                plan_id: pid(),
                node_id: nid(),
                session_id: "x".into(),
                error: "x".into(),
            },
        ),
    ];
    for (kind, variant) in pairs {
        assert_eq!(*kind, variant.event_type(), "wire kind drift on `{kind}`");
    }
    // Plus artifact.state_changed (which already existed before #360,
    // but the substrate module's KIND_ARTIFACT_STATE_CHANGED still has
    // to match the canonical variant).
    let v = F::ArtifactStateChanged {
        artifact_id: ArtifactId::new("art_1"),
        from_state: onsager_artifact::ArtifactState::Draft,
        to_state: onsager_artifact::ArtifactState::InProgress,
    };
    assert_eq!(se::KIND_ARTIFACT_STATE_CHANGED, v.event_type());
}
