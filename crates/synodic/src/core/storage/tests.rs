//! Tests for the storage layer (runs against in-memory SQLite).

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::core::storage::*;
    use chrono::Utc;
    use uuid::Uuid;

    /// Create an in-memory SQLite store for testing.
    async fn test_store() -> Box<dyn Storage> {
        let store = sqlite::SqliteStorage::connect("sqlite::memory:")
            .await
            .expect("connect to in-memory SQLite");
        store.migrate().await.expect("run migrations");
        Box::new(store)
    }

    // -- Seed data ----------------------------------------------------------

    #[tokio::test]
    async fn seed_threat_categories() {
        let store = test_store().await;
        let cats = store.get_threat_categories().await.unwrap();
        assert_eq!(cats.len(), 10, "expected 10 seeded threat categories");

        // Check critical ones exist
        let critical: Vec<_> = cats.iter().filter(|c| c.severity == "critical").collect();
        assert_eq!(critical.len(), 2, "expected 2 critical categories");
        assert!(cats.iter().any(|c| c.id == "data-destruction"));
        assert!(cats.iter().any(|c| c.id == "credential-exposure"));
    }

    #[tokio::test]
    async fn seed_default_rules() {
        let store = test_store().await;
        let rules = store.get_rules(false).await.unwrap();
        assert_eq!(rules.len(), 5, "expected 5 seeded default rules");

        let ids: Vec<_> = rules.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"destructive-git"));
        assert!(ids.contains(&"secrets-in-args"));
        assert!(ids.contains(&"writes-outside-project"));
        assert!(ids.contains(&"writes-to-system"));
        assert!(ids.contains(&"dangerous-rm"));
    }

    #[tokio::test]
    async fn seed_rules_are_active() {
        let store = test_store().await;
        let rules = store.get_rules(true).await.unwrap();
        assert_eq!(rules.len(), 5, "all seeded rules should be active");
        for rule in &rules {
            assert!(rule.enabled);
            assert_eq!(rule.lifecycle, Lifecycle::Active);
        }
    }

    // -- Rule CRUD ----------------------------------------------------------

    #[tokio::test]
    async fn create_and_get_rule() {
        let store = test_store().await;

        let rule = store
            .create_rule(CreateRule {
                id: "test-supply-chain".to_string(),
                description: "Block curl | bash".to_string(),
                category_id: "supply-chain".to_string(),
                tools: vec!["Bash".to_string()],
                condition_type: "command".to_string(),
                condition_value: r"curl\s+.*\|\s*bash".to_string(),
                lifecycle: Lifecycle::Candidate,
                prior_alpha: 1,
                prior_beta: 1,
                project_id: None,
            })
            .await
            .unwrap();

        assert_eq!(rule.id, "test-supply-chain");
        assert_eq!(rule.lifecycle, Lifecycle::Candidate);
        assert_eq!(rule.alpha, 1);
        assert_eq!(rule.beta, 1);

        let fetched = store.get_rule("test-supply-chain").await.unwrap().unwrap();
        assert_eq!(fetched.description, "Block curl | bash");
        assert_eq!(fetched.tools, vec!["Bash"]);
    }

    #[tokio::test]
    async fn get_rules_active_only_filters() {
        let store = test_store().await;

        // Create a disabled rule
        store
            .create_rule(CreateRule {
                id: "disabled-rule".to_string(),
                description: "Disabled".to_string(),
                category_id: "supply-chain".to_string(),
                tools: vec![],
                condition_type: "pattern".to_string(),
                condition_value: "test".to_string(),
                lifecycle: Lifecycle::Candidate,
                prior_alpha: 1,
                prior_beta: 1,
                project_id: None,
            })
            .await
            .unwrap();

        // Disable it
        store
            .update_rule(
                "disabled-rule",
                UpdateRule {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let all = store.get_rules(false).await.unwrap();
        let active = store.get_rules(true).await.unwrap();

        assert_eq!(all.len(), 6); // 5 seeded + 1 new
        assert_eq!(active.len(), 5); // Only seeded (new one disabled)
    }

    #[tokio::test]
    async fn update_rule_lifecycle() {
        let store = test_store().await;

        store
            .update_rule(
                "destructive-git",
                UpdateRule {
                    lifecycle: Some(Lifecycle::Tuned),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let rule = store.get_rule("destructive-git").await.unwrap().unwrap();
        assert_eq!(rule.lifecycle, Lifecycle::Tuned);
    }

    #[tokio::test]
    async fn update_rule_alpha_beta_increments() {
        let store = test_store().await;

        let before = store.get_rule("destructive-git").await.unwrap().unwrap();

        // Simulate 3 confirmed blocks and 1 override
        store
            .update_rule(
                "destructive-git",
                UpdateRule {
                    alpha_increment: Some(3),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        store
            .update_rule(
                "destructive-git",
                UpdateRule {
                    beta_increment: Some(1),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let after = store.get_rule("destructive-git").await.unwrap().unwrap();
        assert_eq!(after.alpha, before.alpha + 3);
        assert_eq!(after.beta, before.beta + 1);
    }

    #[tokio::test]
    async fn delete_rule() {
        let store = test_store().await;

        // Delete feedback referencing the rule first (FK constraint)
        store.delete_rule("dangerous-rm").await.unwrap();

        let rule = store.get_rule("dangerous-rm").await.unwrap();
        assert!(rule.is_none());

        let rules = store.get_rules(false).await.unwrap();
        assert_eq!(rules.len(), 4);
    }

    #[tokio::test]
    async fn rules_revision_changes_on_mutation() {
        // Cache invalidation contract for issue #32: every mutation that the
        // gate handler cares about must shift the revision token returned by
        // get_rules_revision so the cache rebuilds.
        let store = test_store().await;
        let v0 = store.get_rules_revision(true).await.unwrap();

        // Sleep enough that the RFC3339 timestamp definitely advances on
        // SQLite (which truncates to seconds).
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        store
            .update_rule(
                "destructive-git",
                UpdateRule {
                    lifecycle: Some(Lifecycle::Tuned),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let v1 = store.get_rules_revision(true).await.unwrap();
        assert_ne!(v0, v1, "update should shift the revision");

        // Reading without mutating should NOT shift the revision.
        let v1b = store.get_rules_revision(true).await.unwrap();
        assert_eq!(v1, v1b);

        // Deleting an enabled rule shifts both the count and the timestamp.
        store.delete_rule("dangerous-rm").await.unwrap();
        let v2 = store.get_rules_revision(true).await.unwrap();
        assert_ne!(v1, v2, "delete should shift the revision");
    }

    // -- Feedback events ----------------------------------------------------

    #[tokio::test]
    async fn record_and_query_feedback() {
        let store = test_store().await;

        let event = FeedbackEvent {
            id: Uuid::new_v4(),
            signal_type: "override".to_string(),
            rule_id: "destructive-git".to_string(),
            session_id: Some("session-123".to_string()),
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "git push --force"}),
            override_reason: Some("test environment".to_string()),
            failure_type: None,
            evidence_url: None,
            project_id: None,
            created_at: Utc::now(),
        };

        store.record_feedback(event.clone()).await.unwrap();

        // Query by rule
        let events = store
            .get_feedback(FeedbackFilters {
                rule_id: Some("destructive-git".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].signal_type, "override");
        assert_eq!(
            events[0].override_reason.as_deref(),
            Some("test environment")
        );
    }

    #[tokio::test]
    async fn feedback_filter_by_signal_type() {
        let store = test_store().await;

        // Insert override
        store
            .record_feedback(FeedbackEvent {
                id: Uuid::new_v4(),
                signal_type: "override".to_string(),
                rule_id: "destructive-git".to_string(),
                session_id: None,
                tool_name: "Bash".to_string(),
                tool_input: serde_json::json!({}),
                override_reason: None,
                failure_type: None,
                evidence_url: None,
                project_id: None,
                created_at: Utc::now(),
            })
            .await
            .unwrap();

        // Insert confirmed
        store
            .record_feedback(FeedbackEvent {
                id: Uuid::new_v4(),
                signal_type: "confirmed".to_string(),
                rule_id: "destructive-git".to_string(),
                session_id: None,
                tool_name: "Bash".to_string(),
                tool_input: serde_json::json!({}),
                override_reason: None,
                failure_type: None,
                evidence_url: None,
                project_id: None,
                created_at: Utc::now(),
            })
            .await
            .unwrap();

        let overrides = store
            .get_feedback(FeedbackFilters {
                signal_type: Some("override".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(overrides.len(), 1);

        let confirmed = store
            .get_feedback(FeedbackFilters {
                signal_type: Some("confirmed".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(confirmed.len(), 1);

        let all = store
            .get_feedback(FeedbackFilters::default())
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
    }

    // -- Scoring snapshots --------------------------------------------------

    #[tokio::test]
    async fn record_and_query_scores() {
        let store = test_store().await;

        let scores = GovernanceScores {
            id: Uuid::new_v4(),
            project_id: None,
            safety_score: 0.82,
            friction_score: 0.018,
            blocks_count: 5,
            override_count: 1,
            total_tool_calls: 250,
            coverage_score: 0.75,
            covered_categories: 7,
            total_categories: 10,
            converged: false,
            rule_churn_rate: 0.02,
            created_at: Utc::now(),
        };

        store.record_scores(scores).await.unwrap();

        let history = store
            .get_scores(None, Utc::now() - chrono::Duration::hours(1))
            .await
            .unwrap();

        assert_eq!(history.len(), 1);
        assert!((history[0].safety_score - 0.82).abs() < 0.001);
        assert!((history[0].coverage_score - 0.75).abs() < 0.001);
    }

    // -- Probe results ------------------------------------------------------

    #[tokio::test]
    async fn record_and_query_probes() {
        let store = test_store().await;

        let probe = ProbeResult {
            id: Uuid::new_v4(),
            rule_id: "destructive-git".to_string(),
            strategy: "syntactic-variation".to_string(),
            probe_input: serde_json::json!({"command": "git push -f"}),
            bypassed: true,
            proposed_expansion: Some(r"git\s+push\s+(--force|-f)".to_string()),
            expansion_precision_drop: Some(0.0),
            expansion_approved: None,
            created_at: Utc::now(),
        };

        store.record_probe(probe).await.unwrap();

        let probes = store.get_probes("destructive-git").await.unwrap();
        assert_eq!(probes.len(), 1);
        assert!(probes[0].bypassed);
        assert!(probes[0].proposed_expansion.is_some());
    }

    // -- Pipeline runs -------------------------------------------------------

    #[tokio::test]
    async fn record_and_query_pipeline_runs() {
        let store = test_store().await;

        let run = PipelineRun {
            id: Uuid::new_v4().to_string(),
            prompt: "add rate limiting".to_string(),
            branch: Some("synodic/20260403".to_string()),
            outcome: "passed".to_string(),
            attempts: 2,
            model: Some("sonnet".to_string()),
            build_duration_ms: Some(5000),
            build_cost_usd: Some(0.15),
            inspect_duration_ms: Some(1200),
            total_duration_ms: 7500,
            project_id: None,
            created_at: Utc::now(),
        };

        store.record_pipeline_run(run).await.unwrap();

        let runs = store.get_pipeline_runs(None, None).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].prompt, "add rate limiting");
        assert_eq!(runs[0].outcome, "passed");
        assert_eq!(runs[0].attempts, 2);
        assert_eq!(runs[0].build_duration_ms, Some(5000));
        assert!((runs[0].build_cost_usd.unwrap() - 0.15).abs() < 0.001);
    }

    #[tokio::test]
    async fn pipeline_runs_with_no_runs_recorded_returns_empty() {
        let store = test_store().await;
        let runs = store.get_pipeline_runs(None, Some(10)).await.unwrap();
        assert!(runs.is_empty());
    }

    #[tokio::test]
    async fn record_ci_pass_feedback() {
        let store = test_store().await;

        // ci_pass is now a valid signal type
        store
            .record_feedback(FeedbackEvent {
                id: Uuid::new_v4(),
                signal_type: "ci_pass".to_string(),
                rule_id: "destructive-git".to_string(),
                session_id: None,
                tool_name: "synodic-run".to_string(),
                tool_input: serde_json::json!({"check": "format", "exit_code": 0}),
                override_reason: None,
                failure_type: None,
                evidence_url: None,
                project_id: None,
                created_at: Utc::now(),
            })
            .await
            .unwrap();

        let events = store
            .get_feedback(FeedbackFilters {
                signal_type: Some("ci_pass".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
    }

    // -- Storage Rule → InterceptRule conversion ----------------------------

    #[tokio::test]
    async fn storage_rules_convert_to_intercept_rules() {
        use crate::core::intercept::{InterceptEngine, InterceptRequest};

        let store = test_store().await;
        let rules = store.get_rules(true).await.unwrap();

        // Convert storage rules to intercept rules
        let intercept_rules: Vec<_> = rules.iter().map(|r| r.into()).collect();
        let engine = InterceptEngine::new(intercept_rules);

        // Should block destructive git
        let resp = engine.evaluate(&InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "git push --force origin main"}),
        });
        assert_eq!(resp.decision, "block");

        // Should allow normal git
        let resp = engine.evaluate(&InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "git push -u origin feature"}),
        });
        assert_eq!(resp.decision, "allow");
    }

    // -- Governance events --------------------------------------------------

    #[tokio::test]
    async fn create_and_list_governance_events() {
        let store = test_store().await;

        let event = store
            .create_governance_event(CreateGovernanceEvent {
                event_type: "tool_call_error".to_string(),
                title: "Bash tool failed".to_string(),
                severity: Some("high".to_string()),
                source: Some("cli".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(event.event_type, "tool_call_error");
        assert_eq!(event.title, "Bash tool failed");
        assert_eq!(event.severity, "high");
        assert_eq!(event.source, "cli");
        assert!(!event.resolved);
        assert!(event.resolution_notes.is_none());
        assert!(event.resolved_at.is_none());

        let events = store
            .get_governance_events(GovernanceEventFilters::default())
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn get_governance_event_by_id() {
        let store = test_store().await;

        let created = store
            .create_governance_event(CreateGovernanceEvent {
                event_type: "hallucination".to_string(),
                title: "Model hallucinated a file path".to_string(),
                severity: None,
                source: None,
            })
            .await
            .unwrap();

        let fetched = store
            .get_governance_event(&created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.title, "Model hallucinated a file path");
        assert_eq!(fetched.severity, "medium"); // default
        assert_eq!(fetched.source, "api"); // default
    }

    #[tokio::test]
    async fn get_nonexistent_governance_event_returns_none() {
        let store = test_store().await;
        let event = store.get_governance_event("nonexistent").await.unwrap();
        assert!(event.is_none());
    }

    #[tokio::test]
    async fn filter_governance_events_by_type() {
        let store = test_store().await;

        store
            .create_governance_event(CreateGovernanceEvent {
                event_type: "tool_call_error".to_string(),
                title: "Error 1".to_string(),
                severity: None,
                source: None,
            })
            .await
            .unwrap();

        store
            .create_governance_event(CreateGovernanceEvent {
                event_type: "hallucination".to_string(),
                title: "Error 2".to_string(),
                severity: None,
                source: None,
            })
            .await
            .unwrap();

        let filtered = store
            .get_governance_events(GovernanceEventFilters {
                event_type: Some("tool_call_error".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event_type, "tool_call_error");

        let all = store
            .get_governance_events(GovernanceEventFilters::default())
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn resolve_governance_event() {
        let store = test_store().await;

        let event = store
            .create_governance_event(CreateGovernanceEvent {
                event_type: "misalignment".to_string(),
                title: "Agent deviated from instructions".to_string(),
                severity: Some("critical".to_string()),
                source: None,
            })
            .await
            .unwrap();

        assert!(!event.resolved);

        store
            .resolve_governance_event(&event.id, Some("False positive".to_string()))
            .await
            .unwrap();

        let resolved = store
            .get_governance_event(&event.id)
            .await
            .unwrap()
            .unwrap();
        assert!(resolved.resolved);
        assert_eq!(resolved.resolution_notes.as_deref(), Some("False positive"));
        assert!(resolved.resolved_at.is_some());
    }

    // -- Threat categories --------------------------------------------------

    #[tokio::test]
    async fn get_threat_category_by_id() {
        let store = test_store().await;

        let cat = store
            .get_threat_category("data-destruction")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(cat.severity, "critical");
        assert!((cat.severity_weight - 1.0).abs() < 0.001);
        assert!(!cat.examples.is_empty());
    }

    #[tokio::test]
    async fn get_nonexistent_category_returns_none() {
        let store = test_store().await;
        let cat = store.get_threat_category("nonexistent").await.unwrap();
        assert!(cat.is_none());
    }

    #[tokio::test]
    async fn get_nonexistent_rule_returns_none() {
        let store = test_store().await;
        let rule = store.get_rule("nonexistent").await.unwrap();
        assert!(rule.is_none());
    }
}
