//! Stateless webhook routing logic (issue #81).
//!
//! Takes a parsed GitHub webhook payload + the matched set of active
//! workflows and decides which spine events to emit. Kept isolated from the
//! HTTP handler so unit tests can exercise the routing rules without spinning
//! up Axum, a DB, or a real spine.
//!
//! Rules:
//! - `issues.labeled` → one `workflow.trigger_fired` per matched workflow
//!   (label matched a workflow's configured trigger label). Zero matches
//!   produces zero events; the caller ignores the webhook.
//! - `check_suite.completed` / `check_run.completed` / `status` →
//!   `gate.check_updated` keyed by repo + PR. Emitted only when a PR number
//!   is resolvable from the payload.
//! - `pull_request.closed` with `merged=true` → `gate.manual_approval_signal`
//!   keyed by repo + PR.
//! - Anything else → no events (caller returns 202 so GitHub stops retrying).

use onsager_spine::factory_event::FactoryEventKind;
use serde_json::Value;

use crate::core::workflow::Workflow;

/// A single spine event the webhook handler should emit.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedEvent {
    pub kind: FactoryEventKind,
}

/// Inspect an `issues` payload; if action is `labeled`, return one
/// `TriggerFired` per matching workflow.
///
/// `workflows` should already be filtered to the caller's label-match set —
/// the router emits one event per entry without re-checking.
pub fn route_issues_labeled(payload: &Value, matched: &[Workflow]) -> Vec<RoutedEvent> {
    if payload.get("action").and_then(Value::as_str) != Some("labeled") {
        return Vec::new();
    }
    let issue = match payload.get("issue") {
        Some(v) => v,
        None => return Vec::new(),
    };
    let repo = match payload.get("repository") {
        Some(v) => v,
        None => return Vec::new(),
    };

    let repo_owner = repo
        .pointer("/owner/login")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let repo_name = repo
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let issue_number = issue.get("number").and_then(Value::as_u64).unwrap_or(0);
    let title = issue
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let label_name = payload
        .pointer("/label/name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    matched
        .iter()
        .map(|w| RoutedEvent {
            kind: FactoryEventKind::TriggerFired {
                workflow_id: w.id.clone(),
                trigger_kind: w.trigger_kind.to_string(),
                payload: serde_json::json!({
                    "repo_owner": repo_owner,
                    "repo_name": repo_name,
                    "issue_number": issue_number,
                    "title": title,
                    "label": label_name,
                    "tenant_id": w.tenant_id,
                }),
            },
        })
        .collect()
}

/// Inspect a `check_suite`, `check_run`, or `status` payload and produce a
/// `gate.check_updated` event when a PR number can be resolved.
///
/// For `check_suite` / `check_run` we only act on `action == "completed"` —
/// `requested` / `rerequested` / `created` / `in_progress` carry no verdict
/// and shouldn't advance or block a gate.
pub fn route_check_event(event: &str, payload: &Value) -> Option<RoutedEvent> {
    let repo = payload.get("repository")?;
    let repo_owner = repo
        .pointer("/owner/login")
        .and_then(Value::as_str)?
        .to_string();
    let repo_name = repo.get("name").and_then(Value::as_str)?.to_string();

    let (check_name, conclusion, pr_number) = match event {
        "check_suite" => {
            if payload.get("action").and_then(Value::as_str) != Some("completed") {
                return None;
            }
            let cs = payload.get("check_suite")?;
            // Pull the first PR number; GitHub includes the full PR array on
            // check_suite deliveries for the head sha.
            let pr_number = cs
                .get("pull_requests")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(|pr| pr.get("number"))
                .and_then(Value::as_u64)?;
            let conclusion = cs
                .get("conclusion")
                .and_then(Value::as_str)
                .unwrap_or("neutral")
                .to_string();
            (
                format!(
                    "suite/{}",
                    cs.get("id").and_then(Value::as_i64).unwrap_or(0)
                ),
                conclusion,
                pr_number,
            )
        }
        "check_run" => {
            if payload.get("action").and_then(Value::as_str) != Some("completed") {
                return None;
            }
            let cr = payload.get("check_run")?;
            let pr_number = cr
                .get("pull_requests")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(|pr| pr.get("number"))
                .and_then(Value::as_u64)?;
            let conclusion = cr
                .get("conclusion")
                .and_then(Value::as_str)
                .unwrap_or("neutral")
                .to_string();
            let name = cr
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("check_run")
                .to_string();
            (name, conclusion, pr_number)
        }
        "status" => {
            // `status` events don't include a PR number — forge's
            // external-check gate is expected to cross-reference on
            // commit SHA. We skip emission here rather than fabricate
            // one.
            return None;
        }
        _ => return None,
    };

    Some(RoutedEvent {
        kind: FactoryEventKind::GateCheckUpdated {
            repo_owner,
            repo_name,
            pr_number,
            check_name,
            conclusion,
        },
    })
}

/// Inspect a `pull_request` payload; when it's `closed` with `merged=true`
/// return a `gate.manual_approval_signal` event.
pub fn route_pull_request_closed(payload: &Value) -> Option<RoutedEvent> {
    if payload.get("action").and_then(Value::as_str) != Some("closed") {
        return None;
    }
    let pr = payload.get("pull_request")?;
    let merged = pr.get("merged").and_then(Value::as_bool).unwrap_or(false);
    if !merged {
        return None;
    }
    let repo = payload.get("repository")?;
    let repo_owner = repo
        .pointer("/owner/login")
        .and_then(Value::as_str)?
        .to_string();
    let repo_name = repo.get("name").and_then(Value::as_str)?.to_string();
    let pr_number = pr.get("number").and_then(Value::as_u64)?;

    Some(RoutedEvent {
        kind: FactoryEventKind::GateManualApprovalSignal {
            repo_owner,
            repo_name,
            pr_number,
            source: "github.pull_request.closed".to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::workflow::TriggerKind;
    use chrono::Utc;
    use serde_json::json;

    fn sample_workflow(label: &str) -> Workflow {
        Workflow {
            id: "wf_1".to_string(),
            tenant_id: "t1".to_string(),
            name: "sdd".to_string(),
            trigger_kind: TriggerKind::GithubIssueWebhook,
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            trigger_label: label.to_string(),
            install_id: 42,
            preset_id: Some("github-issue-to-pr".to_string()),
            active: true,
            created_by: "u1".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn issues_labeled_emits_trigger_fired_per_match() {
        let payload = json!({
            "action": "labeled",
            "issue": {"number": 123, "title": "fix the bug"},
            "label": {"name": "spec"},
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        let workflows = vec![sample_workflow("spec"), sample_workflow("spec")];
        let out = route_issues_labeled(&payload, &workflows);
        assert_eq!(out.len(), 2);
        for ev in &out {
            match &ev.kind {
                FactoryEventKind::TriggerFired { payload, .. } => {
                    assert_eq!(
                        payload.get("issue_number").and_then(Value::as_u64),
                        Some(123)
                    );
                    assert_eq!(payload.get("label").and_then(Value::as_str), Some("spec"));
                }
                _ => panic!("wrong event kind"),
            }
        }
    }

    #[test]
    fn issues_labeled_with_empty_matches_emits_nothing() {
        let payload = json!({
            "action": "labeled",
            "issue": {"number": 1},
            "label": {"name": "bug"},
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        assert!(route_issues_labeled(&payload, &[]).is_empty());
    }

    #[test]
    fn non_labeled_action_is_ignored() {
        let payload = json!({
            "action": "opened",
            "issue": {"number": 1},
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        let workflows = vec![sample_workflow("spec")];
        assert!(route_issues_labeled(&payload, &workflows).is_empty());
    }

    #[test]
    fn check_run_emits_check_updated() {
        let payload = json!({
            "action": "completed",
            "check_run": {
                "name": "ci",
                "conclusion": "success",
                "pull_requests": [{"number": 7}],
            },
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        let out = route_check_event("check_run", &payload).expect("should emit");
        match out.kind {
            FactoryEventKind::GateCheckUpdated {
                pr_number,
                conclusion,
                check_name,
                ..
            } => {
                assert_eq!(pr_number, 7);
                assert_eq!(conclusion, "success");
                assert_eq!(check_name, "ci");
            }
            _ => panic!("wrong event kind"),
        }
    }

    #[test]
    fn check_run_without_pr_is_ignored() {
        let payload = json!({
            "action": "completed",
            "check_run": {"name": "ci", "conclusion": "success", "pull_requests": []},
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        assert!(route_check_event("check_run", &payload).is_none());
    }

    #[test]
    fn check_run_rerequested_is_ignored() {
        let payload = json!({
            "action": "rerequested",
            "check_run": {
                "name": "ci",
                "conclusion": "success",
                "pull_requests": [{"number": 7}],
            },
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        assert!(route_check_event("check_run", &payload).is_none());
    }

    #[test]
    fn check_suite_requested_is_ignored() {
        let payload = json!({
            "action": "requested",
            "check_suite": {
                "id": 1,
                "conclusion": null,
                "pull_requests": [{"number": 7}],
            },
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        assert!(route_check_event("check_suite", &payload).is_none());
    }

    #[test]
    fn pr_merged_emits_manual_approval_signal() {
        let payload = json!({
            "action": "closed",
            "pull_request": {"number": 11, "merged": true},
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        let out = route_pull_request_closed(&payload).expect("should emit");
        match out.kind {
            FactoryEventKind::GateManualApprovalSignal {
                repo_owner,
                repo_name,
                pr_number,
                ..
            } => {
                assert_eq!(repo_owner, "acme");
                assert_eq!(repo_name, "widgets");
                assert_eq!(pr_number, 11);
            }
            _ => panic!("wrong event kind"),
        }
    }

    #[test]
    fn pr_closed_unmerged_is_ignored() {
        let payload = json!({
            "action": "closed",
            "pull_request": {"number": 11, "merged": false},
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        assert!(route_pull_request_closed(&payload).is_none());
    }
}
