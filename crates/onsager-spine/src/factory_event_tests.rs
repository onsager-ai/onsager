#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::factory_event::*;

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
    fn forge_shaping_dispatched_request_field_serde_compat() {
        use crate::protocol::ShapingRequest;

        // 1. With request = None, the field is omitted from the wire form
        //    (skip_serializing_if), keeping legacy JSON shape on emit.
        let event_without = FactoryEventKind::ForgeShapingDispatched {
            request_id: "req_no_payload".into(),
            artifact_id: ArtifactId::new("art_legacy_shape"),
            target_version: 3,
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
            "type": "forge_shaping_dispatched",
            "request_id": "req_legacy",
            "artifact_id": "art_legacy_shape",
            "target_version": 1,
        });
        let parsed: FactoryEventKind = serde_json::from_value(legacy_json).unwrap();
        match parsed {
            FactoryEventKind::ForgeShapingDispatched { request, .. } => {
                assert!(
                    request.is_none(),
                    "legacy JSON must default request to None"
                );
            }
            other => panic!("expected ForgeShapingDispatched, got {other:?}"),
        }

        // 3. With request = Some(...), full payload round-trips. The
        //    Stiglab listener depends on the inner ShapingRequest staying
        //    byte-stable across upgrades.
        let event_with = FactoryEventKind::ForgeShapingDispatched {
            request_id: "req_full".into(),
            artifact_id: ArtifactId::new("art_full_shape"),
            target_version: 5,
            request: Some(ShapingRequest {
                request_id: "req_full".into(),
                artifact_id: ArtifactId::new("art_full_shape"),
                target_version: 5,
                shaping_intent: serde_json::json!({"prompt": "do the thing"}),
                inputs: vec![],
                constraints: vec![],
                deadline: None,
                created_by: Some("user_42".into()),
            }),
        };
        let json_with = serde_json::to_value(&event_with).unwrap();
        assert_eq!(json_with["type"], "forge_shaping_dispatched");
        assert_eq!(json_with["request"]["request_id"], "req_full");
        assert_eq!(
            json_with["request"]["shaping_intent"]["prompt"],
            "do the thing"
        );
        assert_eq!(json_with["request"]["created_by"], "user_42");

        let back: FactoryEventKind = serde_json::from_value(json_with).unwrap();
        assert_eq!(back, event_with);
        assert_eq!(back.event_type(), "forge.shaping_dispatched");
    }

    #[test]
    fn stiglab_session_result_ready_roundtrip() {
        use crate::protocol::ShapingResult;
        use onsager_artifact::ContentRef;

        let event = FactoryEventKind::StiglabSessionResultReady {
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
        assert_eq!(event.event_type(), "stiglab.session_result_ready");
        assert_eq!(event.stream_type(), "stiglab");
        assert_eq!(event.stream_id(), "art_shaped");

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "stiglab_session_result_ready");
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
