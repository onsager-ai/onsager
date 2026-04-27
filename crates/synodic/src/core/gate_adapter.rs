//! Adapter between Onsager protocol types and Synodic intercept types.
//!
//! Converts `GateRequest` (from the Onsager spine) into `InterceptRequest`
//! for evaluation by the existing `InterceptEngine`, and converts the
//! resulting `InterceptResponse` back into a `GateVerdict`.

use onsager_spine::factory_event::GatePoint;
use onsager_spine::protocol::{GateRequest, GateVerdict};

use crate::core::intercept::{InterceptRequest, InterceptResponse};

/// Convert an Onsager `GateRequest` into a Synodic `InterceptRequest`.
///
/// The mapping depends on the gate point:
/// - `ToolLevel`: extracts `tool_name` and `tool_input` from the proposed
///   action payload.
/// - Other gate points: uses a synthetic `forge:*` tool name and serializes
///   the gate context as the tool input.
pub fn gate_request_to_intercept(req: &GateRequest) -> InterceptRequest {
    match req.context.gate_point {
        GatePoint::ToolLevel => {
            let tool_name = req
                .proposed_action
                .payload
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let tool_input = req
                .proposed_action
                .payload
                .get("tool_input")
                .cloned()
                .unwrap_or_else(|| req.proposed_action.payload.clone());
            InterceptRequest {
                tool_name,
                tool_input,
            }
        }
        GatePoint::PreDispatch => InterceptRequest {
            tool_name: "forge:pre_dispatch".to_string(),
            tool_input: serde_json::to_value(&req.context).unwrap_or_default(),
        },
        GatePoint::StateTransition => InterceptRequest {
            tool_name: "forge:state_transition".to_string(),
            tool_input: serde_json::to_value(&req.context).unwrap_or_default(),
        },
        GatePoint::ConsumerRouting => InterceptRequest {
            tool_name: "forge:consumer_routing".to_string(),
            tool_input: serde_json::to_value(&req.context).unwrap_or_default(),
        },
    }
}

/// Convert a Synodic `InterceptResponse` into an Onsager `GateVerdict`.
///
/// - `"allow"` maps to `GateVerdict::Allow`
/// - `"block"` maps to `GateVerdict::Deny` with the response reason
/// - Any other decision defaults to `GateVerdict::Allow` (safe default)
pub fn intercept_to_gate_verdict(resp: &InterceptResponse) -> GateVerdict {
    match resp.decision.as_str() {
        "allow" => GateVerdict::Allow,
        "block" => GateVerdict::Deny {
            reason: resp
                .reason
                .clone()
                .unwrap_or_else(|| "blocked by rule".into()),
        },
        _ => GateVerdict::Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::{ArtifactId, ArtifactState, Kind};
    use onsager_spine::protocol::{GateContext, ProposedAction};

    fn tool_level_request() -> GateRequest {
        GateRequest {
            context: GateContext {
                gate_point: GatePoint::ToolLevel,
                artifact_id: ArtifactId::new("art_test1234"),
                artifact_kind: Kind::Code,
                current_state: ArtifactState::InProgress,
                target_state: None,
                extra: None,
            },
            proposed_action: ProposedAction {
                description: "Write a file".to_string(),
                payload: serde_json::json!({
                    "tool_name": "Write",
                    "tool_input": {
                        "file_path": "/etc/passwd",
                        "content": "malicious"
                    }
                }),
            },
        }
    }

    fn pre_dispatch_request() -> GateRequest {
        GateRequest {
            context: GateContext {
                gate_point: GatePoint::PreDispatch,
                artifact_id: ArtifactId::new("art_test1234"),
                artifact_kind: Kind::Code,
                current_state: ArtifactState::Draft,
                target_state: None,
                extra: None,
            },
            proposed_action: ProposedAction {
                description: "Dispatch shaping".to_string(),
                payload: serde_json::json!({}),
            },
        }
    }

    #[test]
    fn tool_level_extracts_tool_name_and_input() {
        let req = gate_request_to_intercept(&tool_level_request());
        assert_eq!(req.tool_name, "Write");
        assert_eq!(req.tool_input["file_path"], "/etc/passwd");
    }

    #[test]
    fn pre_dispatch_uses_synthetic_tool_name() {
        let req = gate_request_to_intercept(&pre_dispatch_request());
        assert_eq!(req.tool_name, "forge:pre_dispatch");
        assert_eq!(req.tool_input["gate_point"], "pre_dispatch");
    }

    #[test]
    fn state_transition_uses_synthetic_tool_name() {
        let mut gate = pre_dispatch_request();
        gate.context.gate_point = GatePoint::StateTransition;
        let req = gate_request_to_intercept(&gate);
        assert_eq!(req.tool_name, "forge:state_transition");
    }

    #[test]
    fn consumer_routing_uses_synthetic_tool_name() {
        let mut gate = pre_dispatch_request();
        gate.context.gate_point = GatePoint::ConsumerRouting;
        let req = gate_request_to_intercept(&gate);
        assert_eq!(req.tool_name, "forge:consumer_routing");
    }

    #[test]
    fn allow_response_maps_to_allow_verdict() {
        let resp = InterceptResponse::allow();
        let verdict = intercept_to_gate_verdict(&resp);
        let json = serde_json::to_value(&verdict).unwrap();
        assert_eq!(json["verdict"], "allow");
    }

    #[test]
    fn block_response_maps_to_deny_verdict() {
        let resp = InterceptResponse::block("unsafe operation", "test-rule");
        let verdict = intercept_to_gate_verdict(&resp);
        let json = serde_json::to_value(&verdict).unwrap();
        assert_eq!(json["verdict"], "deny");
        assert_eq!(json["reason"], "unsafe operation");
    }

    #[test]
    fn block_without_reason_uses_default() {
        let resp = InterceptResponse {
            decision: "block".to_string(),
            reason: None,
            rule: None,
        };
        let verdict = intercept_to_gate_verdict(&resp);
        let json = serde_json::to_value(&verdict).unwrap();
        assert_eq!(json["reason"], "blocked by rule");
    }

    #[test]
    fn unknown_decision_defaults_to_allow() {
        let resp = InterceptResponse {
            decision: "unknown".to_string(),
            reason: None,
            rule: None,
        };
        let verdict = intercept_to_gate_verdict(&resp);
        let json = serde_json::to_value(&verdict).unwrap();
        assert_eq!(json["verdict"], "allow");
    }

    #[test]
    fn tool_level_end_to_end_with_engine() {
        use crate::core::intercept::{default_rules, InterceptEngine};

        let gate_req = tool_level_request();
        let intercept_req = gate_request_to_intercept(&gate_req);
        let engine = InterceptEngine::new(default_rules());
        let resp = engine.evaluate(&intercept_req);
        let verdict = intercept_to_gate_verdict(&resp);

        // The Write to /etc/passwd should be blocked by the default rules
        let json = serde_json::to_value(&verdict).unwrap();
        assert_eq!(json["verdict"], "deny");
    }
}
