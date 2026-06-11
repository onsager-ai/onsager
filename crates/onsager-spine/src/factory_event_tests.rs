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
    fn insight_scope_variants() {
        let global = InsightScope::Global;
        let json = serde_json::to_string(&global).unwrap();
        assert!(json.contains("global"));

        let specific = InsightScope::SpecificArtifact(ArtifactId::new("art_12345678"));
        let json = serde_json::to_string(&specific).unwrap();
        assert!(json.contains("art_12345678"));
    }

    #[test]
    fn token_usage_on_session_completed_is_optional() {
        // Without token_usage (legacy shape)
        let without = FactoryEventKind::SessionCompleted {
            session_id: "sess_1".into(),
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
        let with = FactoryEventKind::SessionCompleted {
            session_id: "sess_2".into(),
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

    // -- Chat-session lifecycle routing (spec #583) -------------------------

    #[test]
    fn session_lifecycle_events_route_by_session_id() {
        // Portal's listeners (PR opener, token revoke) and the dashboard
        // filter on this triple — pin it. Renamed from the
        // `stiglab.session_*` names when stiglab folded into portal
        // (ADR 0027 / spec #583).
        let completed = FactoryEventKind::SessionCompleted {
            session_id: "sess_42".into(),
            duration_ms: 12_500,
            artifact_id: Some("art_x".into()),
            token_usage: None,
            branch: Some("claude/feature".into()),
            pr_number: None,
        };
        assert_eq!(completed.event_type(), "session.completed");
        assert_eq!(completed.stream_type(), "session");
        assert_eq!(completed.stream_id(), "sess_42");

        let failed = FactoryEventKind::SessionFailed {
            session_id: "sess_43".into(),
            error: "boom".into(),
            artifact_id: None,
        };
        assert_eq!(failed.event_type(), "session.failed");
        assert_eq!(failed.stream_type(), "session");
        assert_eq!(failed.stream_id(), "sess_43");

        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["type"], "session_failed");
        let back: FactoryEventKind = serde_json::from_value(json).unwrap();
        assert_eq!(back, failed);
    }
}
