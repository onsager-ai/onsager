//! Stateless GitHub-webhook → spine-event routing.
//!
//! Lives on the spine because both **portal** (the live webhook ingress at
//! `POST /webhooks/github`) and **stiglab** (the dashboard-driven manual
//! replay route) translate webhook payloads into the same
//! [`FactoryEventKind`] variants. Hosting the routing rules here keeps a
//! single source of truth so the two paths can't drift in subtle shape
//! differences — only in the `source` field of `TriggerFired`.
//!
//! Rules (issue #81):
//! - `issues.labeled` → one `workflow.trigger_fired` per matched workflow
//!   (label matched a workflow's configured trigger label). Zero matches
//!   produces zero events; the caller ignores the webhook.
//! - `check_suite.completed` / `check_run.completed` → `gate.check_updated`
//!   keyed by repo + PR. Emitted only when a PR number is resolvable.
//!   `status` events carry no PR number and are skipped here — forge's
//!   external-check gate cross-references on commit SHA instead.
//! - `pull_request.closed` with `merged=true` → `gate.manual_approval_signal`
//!   keyed by repo + PR.
//! - Anything else → no events (caller returns 202 so GitHub stops retrying).

use serde_json::Value;

use crate::factory_event::FactoryEventKind;

/// A single spine event the webhook handler should emit.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedEvent {
    pub kind: FactoryEventKind,
}

/// Source tag stamped into every `workflow.trigger_fired` payload so
/// downstream consumers (and audit views) can tell a real GitHub-webhook
/// fire apart from a manual replay invoked from the dashboard.
pub mod trigger_source {
    pub const WEBHOOK: &str = "github_webhook";
    pub const MANUAL_REPLAY: &str = "manual_replay";
}

/// Already-resolved fields for one `issues.labeled`-shaped trigger event.
/// The webhook path extracts these from the GitHub payload; the manual
/// replay route builds them from the project + the live issue.
pub struct IssueTriggerContext<'a> {
    pub repo_owner: &'a str,
    pub repo_name: &'a str,
    pub issue_number: u64,
    pub title: &'a str,
    pub label: &'a str,
    pub source: &'a str,
    pub replayed_by: Option<&'a str>,
}

/// Minimal portable view of a workflow that the routing functions need —
/// just enough to build a `TriggerFired` payload. Both portal and stiglab
/// project their richer `Workflow` types into this shape so the routing
/// stays subsystem-agnostic.
#[derive(Debug, Clone)]
pub struct WorkflowMatch {
    pub id: String,
    pub workspace_id: String,
    pub trigger_kind_tag: String,
}

/// Build one `TriggerFired` event per matching workflow from an
/// already-resolved context. Shared by the live webhook path and the
/// manual replay route so both produce identical payload shapes — the
/// only difference is the `source` field (and an optional `replayed_by`).
pub fn build_trigger_fired_events(
    ctx: &IssueTriggerContext<'_>,
    matched: &[WorkflowMatch],
) -> Vec<RoutedEvent> {
    matched
        .iter()
        .map(|w| {
            let mut payload = serde_json::json!({
                "repo_owner": ctx.repo_owner,
                "repo_name": ctx.repo_name,
                "issue_number": ctx.issue_number,
                "title": ctx.title,
                "label": ctx.label,
                "workspace_id": w.workspace_id,
                "source": ctx.source,
            });
            if let Some(uid) = ctx.replayed_by {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("replayed_by".into(), Value::String(uid.to_string()));
                }
            }
            RoutedEvent {
                kind: FactoryEventKind::TriggerFired {
                    workflow_id: w.id.clone(),
                    trigger_kind: w.trigger_kind_tag.clone(),
                    payload,
                },
            }
        })
        .collect()
}

/// Inspect an `issues` payload; if action is `labeled`, return one
/// `TriggerFired` per matching workflow.
///
/// `matched` should already be filtered to the caller's label-match set —
/// the router emits one event per entry without re-checking.
pub fn route_issues_labeled(payload: &Value, matched: &[WorkflowMatch]) -> Vec<RoutedEvent> {
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
        .unwrap_or_default();
    let repo_name = repo.get("name").and_then(Value::as_str).unwrap_or_default();
    let issue_number = issue.get("number").and_then(Value::as_u64).unwrap_or(0);
    let title = issue.get("title").and_then(Value::as_str).unwrap_or("");
    let label_name = payload
        .pointer("/label/name")
        .and_then(Value::as_str)
        .unwrap_or("");

    build_trigger_fired_events(
        &IssueTriggerContext {
            repo_owner,
            repo_name,
            issue_number,
            title,
            label: label_name,
            source: trigger_source::WEBHOOK,
            replayed_by: None,
        },
        matched,
    )
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
        "status" => return None,
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

/// Namespace partition for webhook-sourced spine events. Both the live
/// webhook handler (portal) and the manual-replay route (stiglab) write
/// events through this so consumer streams stay unified.
pub fn spine_namespace(kind: &FactoryEventKind) -> &'static str {
    match kind {
        FactoryEventKind::TriggerFired { .. } => "workflow",
        FactoryEventKind::GateCheckUpdated { .. }
        | FactoryEventKind::GateManualApprovalSignal { .. } => "gate",
        _ => "stiglab",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_match(_label: &str) -> WorkflowMatch {
        WorkflowMatch {
            id: "wf_1".to_string(),
            workspace_id: "w1".to_string(),
            trigger_kind_tag: "github_issue_webhook".to_string(),
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
        let workflows = vec![sample_match("spec"), sample_match("spec")];
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
                    assert_eq!(
                        payload.get("source").and_then(Value::as_str),
                        Some(trigger_source::WEBHOOK)
                    );
                    assert!(payload.get("replayed_by").is_none());
                }
                _ => panic!("wrong event kind"),
            }
        }
    }

    #[test]
    fn manual_replay_matches_webhook_payload_except_source() {
        let payload = json!({
            "action": "labeled",
            "issue": {"number": 123, "title": "fix the bug"},
            "label": {"name": "spec"},
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        let workflows = vec![sample_match("spec")];
        let from_webhook = route_issues_labeled(&payload, &workflows);
        let from_replay = build_trigger_fired_events(
            &IssueTriggerContext {
                repo_owner: "acme",
                repo_name: "widgets",
                issue_number: 123,
                title: "fix the bug",
                label: "spec",
                source: trigger_source::MANUAL_REPLAY,
                replayed_by: Some("u_abc"),
            },
            &workflows,
        );
        assert_eq!(from_webhook.len(), 1);
        assert_eq!(from_replay.len(), 1);
        let (
            FactoryEventKind::TriggerFired { payload: w, .. },
            FactoryEventKind::TriggerFired { payload: r, .. },
        ) = (&from_webhook[0].kind, &from_replay[0].kind)
        else {
            panic!("wrong event kind");
        };
        for key in [
            "repo_owner",
            "repo_name",
            "issue_number",
            "title",
            "label",
            "workspace_id",
        ] {
            assert_eq!(w.get(key), r.get(key), "mismatch on {key}");
        }
        assert_eq!(
            w.get("source").and_then(Value::as_str),
            Some(trigger_source::WEBHOOK)
        );
        assert_eq!(
            r.get("source").and_then(Value::as_str),
            Some(trigger_source::MANUAL_REPLAY)
        );
        assert_eq!(r.get("replayed_by").and_then(Value::as_str), Some("u_abc"));
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
        let workflows = vec![sample_match("spec")];
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
