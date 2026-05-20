//! Typed-shape translator — the poller's path from observed GitHub
//! resources to spine `RoutedEvent`s.
//!
//! The reconciliation poller (`scheduler.rs::tick_project`) hands a
//! typed `Issue` / `Pull` straight in and gets back the events the
//! spine expects. The webhook handler keeps using the spine-side
//! `route_*` family (`onsager_spine::webhook_routing::{
//! route_issues_labeled, route_pull_request_closed, ...}`) — those
//! routers and the functions here share a contract, not a call site:
//! both produce identical `RoutedEvent.kind` shapes for the same
//! resource update, and both populate the same dedup key
//! (`(adapter_id, external_ref)`) so the webhook ↔ poller race
//! collapses on the spine migration 032 partial unique index.
//!
//! Byte-equivalence of `RoutedEvent.kind` between the two paths is
//! exercised by the
//! `webhook_and_poller_paths_produce_equivalent_routed_events_*`
//! tests in this module.
//!
//! v1 resource scope (matches #121): issues + PR closed-merged only.
//! `check_*` stays webhook-only (`route_check_event` on the spine).

use std::collections::HashMap;

use onsager_github::api::{
    issues::{Issue, Label},
    pulls::Pull,
};
use onsager_spine::webhook_routing::WorkflowTrigger;
use onsager_spine::{
    FactoryEventKind, IssueTriggerContext, RoutedEvent, WorkflowMatch, build_trigger_fired_events,
    trigger::TriggerKind, trigger_source,
};

use crate::db::{issue_external_ref, pr_external_ref};

/// Adapter identifier the GitHub translator stamps on emitted
/// events for the (adapter_id, external_ref) dedup index. Centralised
/// so the webhook and poller agree on the spelling and the partial
/// unique index actually collapses races.
pub const GITHUB_ADAPTER_ID: &str = "github";

/// Decorate the `TriggerFired` events with a per-workflow dedup key.
/// We append `:trigger:<workflow_id>` to the resource-level ref so two
/// workflows matching the same issue don't dedup against each other —
/// only webhook + poller races for the *same* (issue, workflow) pair
/// collapse. The resource-level prefix comes from
/// `crate::db::{issue_external_ref, pr_external_ref}` so a webhook
/// emit and a poller emit produce the same key.
fn trigger_external_ref(resource_ref: &str, workflow_id: &str) -> String {
    format!("{resource_ref}:trigger:{workflow_id}")
}

/// Translate an [`Issue`] into the `Vec<RoutedEvent>` the spine
/// expects. For each label on the issue that matches a workflow's
/// configured trigger label, one `TriggerFired` is produced.
///
/// `matched_workflows_by_label` is keyed on the label string — the
/// caller (webhook or poller) restricts the keyspace to the labels
/// it wants to consider:
/// - The webhook handler passes a single-entry map with the
///   just-added label, preserving today's "fire once per labeled
///   delivery" semantics.
/// - The poller passes every current label, since it has no
///   action context (only "this issue is currently shaped like
///   this"). The spine dedup index collapses the inevitable replay.
pub fn translate_issue(
    issue: &Issue,
    repo_owner: &str,
    repo_name: &str,
    project_id: Option<&str>,
    matched_workflows_by_label: &HashMap<String, Vec<WorkflowMatch>>,
) -> Vec<RoutedEvent> {
    let mut out = Vec::new();
    let resource_ref = project_id.map(|pid| issue_external_ref(pid, issue.number));
    for Label { name } in &issue.labels {
        let Some(workflows) = matched_workflows_by_label.get(name) else {
            continue;
        };
        let events = build_trigger_fired_events(
            &IssueTriggerContext {
                repo_owner,
                repo_name,
                issue_number: issue.number,
                title: &issue.title,
                label: name,
                source: trigger_source::WEBHOOK,
                replayed_by: None,
            },
            workflows,
        );
        // Decorate with per-(resource, workflow) dedup keys so a
        // webhook delivery and a poller observation of the same
        // labeled issue + workflow collapse to one events_ext row.
        for (ev, wf) in events.into_iter().zip(workflows.iter()) {
            let decorated = match resource_ref.as_deref() {
                Some(r) => ev.with_dedup(GITHUB_ADAPTER_ID, trigger_external_ref(r, &wf.id)),
                None => ev,
            };
            out.push(decorated);
        }
    }
    out
}

/// Translate a [`Pull`] into the `Vec<RoutedEvent>` the spine expects.
/// A closed+merged PR yields a `GateManualApprovalSignal` plus one
/// `TriggerFired` per matching `github_pull_request_closed` workflow
/// whose predicate accepts the delivered `merged` flag.
///
/// `matched` is the candidate set the caller queried — the
/// translator applies the per-workflow predicate (`merged`) and drops
/// non-matches, mirroring `route_pull_request_closed_workflows` on
/// the spine.
pub fn translate_pull_request(
    pull: &Pull,
    repo_owner: &str,
    repo_name: &str,
    project_id: Option<&str>,
    matched: &[WorkflowTrigger],
) -> Vec<RoutedEvent> {
    let mut out = Vec::new();
    if pull.state != "closed" {
        return out;
    }
    let merged = pull.merged_at.is_some();
    let resource_ref = project_id.map(|pid| pr_external_ref(pid, pull.number));

    if merged {
        let ev = RoutedEvent::new(FactoryEventKind::GateManualApprovalSignal {
            repo_owner: repo_owner.to_string(),
            repo_name: repo_name.to_string(),
            pr_number: pull.number,
            source: "github.pull_request.closed".to_string(),
        });
        let ev = match resource_ref.as_deref() {
            Some(r) => ev.with_dedup(GITHUB_ADAPTER_ID, format!("{r}:manual_approval")),
            None => ev,
        };
        out.push(ev);
    }

    for w in matched {
        let predicate_match = match &w.trigger {
            TriggerKind::GithubPullRequestClosed { predicate, .. } => match predicate {
                Some(p) => p.merged.is_none_or(|want| want == merged),
                None => true,
            },
            _ => false,
        };
        if !predicate_match {
            continue;
        }
        let payload = serde_json::json!({
            "repo_owner": repo_owner,
            "repo_name": repo_name,
            "pr_number": pull.number,
            "title": pull.title,
            "head_branch": pull.head.ref_name,
            "base_branch": pull.base.ref_name,
            "merged": merged,
            "workspace_id": w.workspace_id,
            "source": trigger_source::WEBHOOK,
            "trigger_kind": "github_pull_request_closed",
        });
        let ev = RoutedEvent::new(FactoryEventKind::TriggerFired {
            workflow_id: w.id.clone(),
            trigger_kind: "github_pull_request_closed".to_string(),
            payload,
        });
        let ev = match resource_ref.as_deref() {
            Some(r) => ev.with_dedup(GITHUB_ADAPTER_ID, trigger_external_ref(r, &w.id)),
            None => ev,
        };
        out.push(ev);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_github::api::issues::{Issue, Label};
    use onsager_github::api::pulls::{Pull, PullRef, PullUser};
    use onsager_spine::trigger::PullRequestClosedPredicate;

    fn sample_issue(labels: Vec<&str>) -> Issue {
        Issue {
            number: 42,
            title: "fix the bug".into(),
            state: "open".into(),
            body: None,
            labels: labels
                .into_iter()
                .map(|n| Label { name: n.into() })
                .collect(),
            pull_request: None,
            updated_at: None,
        }
    }

    fn sample_pull(state: &str, merged: bool) -> Pull {
        Pull {
            number: 11,
            title: "merge me".into(),
            state: state.into(),
            merged_at: merged.then(|| "2026-05-20T00:00:00Z".to_string()),
            merge_commit_sha: merged.then(|| "abc123".into()),
            updated_at: None,
            head: PullRef {
                ref_name: "feature".into(),
                sha: "deadbeef".into(),
            },
            base: PullRef {
                ref_name: "main".into(),
                sha: "cafebabe".into(),
            },
            html_url: "https://github.com/acme/widgets/pull/11".into(),
            user: PullUser {
                login: "alice".into(),
            },
        }
    }

    fn sample_match(id: &str) -> WorkflowMatch {
        WorkflowMatch {
            id: id.into(),
            workspace_id: "ws1".into(),
            trigger_kind_tag: "github_issue_webhook".into(),
        }
    }

    #[test]
    fn translate_issue_fires_per_matched_label() {
        let issue = sample_issue(vec!["spec", "feat"]);
        let mut by_label = HashMap::new();
        by_label.insert("spec".into(), vec![sample_match("wf_a")]);
        // No entry for "feat" → ignored.
        let out = translate_issue(&issue, "acme", "widgets", Some("proj_x"), &by_label);
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            FactoryEventKind::TriggerFired { workflow_id, .. } => assert_eq!(workflow_id, "wf_a"),
            _ => panic!("expected TriggerFired"),
        }
        assert_eq!(out[0].adapter_id.as_deref(), Some(GITHUB_ADAPTER_ID));
        assert_eq!(
            out[0].external_ref.as_deref(),
            Some("github:project:proj_x:issue:42:trigger:wf_a")
        );
    }

    #[test]
    fn translate_issue_dedup_blank_without_project_id() {
        let issue = sample_issue(vec!["spec"]);
        let mut by_label = HashMap::new();
        by_label.insert("spec".into(), vec![sample_match("wf_a")]);
        let out = translate_issue(&issue, "acme", "widgets", None, &by_label);
        assert_eq!(out.len(), 1);
        assert!(out[0].external_ref.is_none());
        assert!(out[0].adapter_id.is_none());
    }

    fn pr_workflow(merged_predicate: Option<bool>) -> WorkflowTrigger {
        WorkflowTrigger {
            id: "wf_pr".into(),
            workspace_id: "ws1".into(),
            trigger: TriggerKind::GithubPullRequestClosed {
                repo: "acme/widgets".into(),
                predicate: merged_predicate.map(|m| PullRequestClosedPredicate { merged: Some(m) }),
            },
        }
    }

    #[test]
    fn translate_pull_request_merged_emits_manual_approval_and_trigger() {
        let pull = sample_pull("closed", true);
        let workflows = vec![pr_workflow(Some(true))];
        let out = translate_pull_request(&pull, "acme", "widgets", Some("proj_x"), &workflows);
        assert_eq!(out.len(), 2);

        let manual_approval = out
            .iter()
            .find(|e| matches!(e.kind, FactoryEventKind::GateManualApprovalSignal { .. }))
            .expect("manual approval");
        assert_eq!(
            manual_approval.external_ref.as_deref(),
            Some("github:project:proj_x:pr:11:manual_approval")
        );

        let trigger = out
            .iter()
            .find(|e| matches!(e.kind, FactoryEventKind::TriggerFired { .. }))
            .expect("trigger fired");
        assert_eq!(
            trigger.external_ref.as_deref(),
            Some("github:project:proj_x:pr:11:trigger:wf_pr")
        );
    }

    #[test]
    fn translate_pull_request_unmerged_drops_manual_approval() {
        let pull = sample_pull("closed", false);
        let workflows = vec![pr_workflow(None)];
        let out = translate_pull_request(&pull, "acme", "widgets", Some("proj_x"), &workflows);
        // Manual-approval is merged-only. Trigger fires (predicate None
        // accepts any merged flag).
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            FactoryEventKind::TriggerFired { payload, .. } => {
                assert_eq!(payload.get("merged").and_then(|v| v.as_bool()), Some(false));
            }
            _ => panic!("expected TriggerFired"),
        }
    }

    #[test]
    fn translate_pull_request_open_drops_everything() {
        let pull = sample_pull("open", false);
        let out = translate_pull_request(
            &pull,
            "acme",
            "widgets",
            Some("proj_x"),
            &[pr_workflow(None)],
        );
        assert!(out.is_empty());
    }

    #[test]
    fn webhook_and_poller_paths_produce_equivalent_routed_events_for_labeled_issue() {
        // The spec's "byte-equivalent RoutedEvent sets" contract:
        // given matching input shapes, the webhook router and the
        // poller translator must produce the same `kind`. The dedup
        // fields differ by design (only the poller stamps them in
        // unit-test scope; the webhook handler does it later via
        // `decorate_routed_with_dedup`) so we compare on `kind`.
        let payload = serde_json::json!({
            "action": "labeled",
            "issue": {
                "number": 42,
                "title": "fix the bug",
                "state": "open",
                "labels": [{"name": "spec"}],
            },
            "label": {"name": "spec"},
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        let workflows = vec![sample_match("wf_a")];
        let from_webhook = onsager_spine::route_issues_labeled(&payload, &workflows);

        let issue = sample_issue(vec!["spec"]);
        let mut by_label = HashMap::new();
        by_label.insert("spec".to_string(), workflows.clone());
        let from_poller = translate_issue(&issue, "acme", "widgets", None, &by_label);

        assert_eq!(from_webhook.len(), 1);
        assert_eq!(from_poller.len(), 1);
        assert_eq!(
            from_webhook[0].kind, from_poller[0].kind,
            "webhook and poller must emit identical TriggerFired kinds"
        );
    }

    #[test]
    fn webhook_and_poller_paths_produce_equivalent_routed_events_for_merged_pr() {
        let payload = serde_json::json!({
            "action": "closed",
            "pull_request": {"number": 11, "merged": true},
            "repository": {"name": "widgets", "owner": {"login": "acme"}},
        });
        let from_webhook = onsager_spine::route_pull_request_closed(&payload);

        let pull = sample_pull("closed", true);
        let from_poller = translate_pull_request(&pull, "acme", "widgets", None, &[]);

        // Webhook path returns Option<RoutedEvent> for the manual-
        // approval signal; poller returns Vec. Find the manual-
        // approval event on each side and compare kinds.
        let webhook_kind = &from_webhook.expect("webhook emits manual approval").kind;
        let poller_kind = &from_poller
            .iter()
            .find(|e| matches!(e.kind, FactoryEventKind::GateManualApprovalSignal { .. }))
            .expect("poller emits manual approval")
            .kind;
        assert_eq!(webhook_kind, poller_kind);
    }

    #[test]
    fn translate_pull_request_predicate_misses_drops_trigger() {
        let pull = sample_pull("closed", false);
        // Predicate says "merged only".
        let workflows = vec![pr_workflow(Some(true))];
        let out = translate_pull_request(&pull, "acme", "widgets", Some("proj_x"), &workflows);
        assert!(out.is_empty());
    }
}
