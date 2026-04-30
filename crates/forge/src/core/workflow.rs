//! Workflow domain model (issue #80).
//!
//! A workflow is a declarative production-line blueprint: a trigger plus an
//! ordered chain of stages. Each stage lists the gates that must resolve
//! before the artifact advances. Stages walk in strict declared order — no
//! skipping, no reordering. Adaptive scheduling (the kernel) still applies
//! *within* a stage, never *across* stages.
//!
//! This module is persistence-agnostic domain. The stage runner in
//! `stage_runner.rs` drives it each tick; the sqlx-backed loader in
//! `persistence.rs` rebuilds it on startup.

use onsager_artifact::ArtifactState;
use serde::{Deserialize, Serialize};

/// A workflow-runtime trigger. v1 ships only the GitHub-issue webhook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerSpec {
    /// A GitHub `issues.labeled` webhook whose label matches `label`.
    GithubIssueWebhook { repo: String, label: String },
}

impl TriggerSpec {
    /// Stable string used as the `trigger_kind` column value.
    pub fn kind_tag(&self) -> &'static str {
        match self {
            TriggerSpec::GithubIssueWebhook { .. } => "github_issue_webhook",
        }
    }
}

/// One gate in a stage's gate set. All gates in a stage must pass before
/// the runner advances.
///
/// `agent-session`, `external-check`, `governance`, `manual-approval` are
/// all v1 kinds (issue #80). Every kind evaluates to one of
/// [`GateOutcome::Pass`], [`GateOutcome::Fail`], [`GateOutcome::Pending`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GateSpec {
    /// Dispatch a shaping session to stiglab; resolve when the
    /// `stiglab.session_completed` event arrives for this artifact.
    AgentSession {
        /// Free-form intent payload handed to the shaping request.
        #[serde(default)]
        shaping_intent: serde_json::Value,
    },

    /// Wait for a GitHub check/status event on the spine. `check_name` is
    /// the check identifier (e.g. `"ci/test"`); when a `git.ci_completed`
    /// event with matching check_name lands, resolve Pass or Fail based on
    /// its conclusion.
    ExternalCheck { check_name: String },

    /// Invoke the Synodic gate with the standard [`GateRequest`] shape.
    Governance {
        /// Optional gate-point label to tag the request with. Defaults to
        /// `StateTransition` when absent.
        #[serde(default)]
        gate_point: Option<String>,
    },

    /// Block until an explicit completion signal arrives on the spine.
    /// `signal_kind` is the signal vocabulary the runner matches against:
    /// `"pr_merged"` is the GitHub merge webhook; `"dashboard_approve"`
    /// is the dashboard-button action.
    ManualApproval { signal_kind: String },
}

impl GateSpec {
    /// Stable string used in event payloads and logs.
    pub fn kind_tag(&self) -> &'static str {
        match self {
            GateSpec::AgentSession { .. } => "agent_session",
            GateSpec::ExternalCheck { .. } => "external_check",
            GateSpec::Governance { .. } => "governance",
            GateSpec::ManualApproval { .. } => "manual_approval",
        }
    }
}

/// Result of evaluating a single gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// Gate resolved successfully — runner can consider this gate done.
    Pass,
    /// Gate resolved unsuccessfully — runner parks the artifact in
    /// `under_review` with the failure reason until the condition is
    /// cleared (e.g. a re-run of the check passes).
    Fail(String),
    /// Gate hasn't resolved yet — runner leaves the artifact at this
    /// stage and rechecks next tick.
    Pending,
}

/// One stage in a workflow.
///
/// `target_state` is the state the artifact transitions to on entering
/// this stage. `None` leaves the state unchanged (useful when multiple
/// stages share the same `UnderReview` state but differ in their gates).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageSpec {
    pub name: String,
    #[serde(default)]
    pub target_state: Option<ArtifactState>,
    #[serde(default)]
    pub gates: Vec<GateSpec>,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A complete workflow definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workflow {
    pub workflow_id: String,
    pub name: String,
    pub trigger: TriggerSpec,
    pub stages: Vec<StageSpec>,
    pub active: bool,
    /// Workspace this workflow belongs to. Stamped into every stage event
    /// `data` payload so the workspace-scoped `/api/spine/events` filter
    /// (`data->>'workspace_id' = $1`) can find them — without this, stage
    /// events land in `events_ext` but are silently invisible to the
    /// dashboard's workflow detail page (#230). Spine column is `NOT NULL`
    /// post-#149, so this is required.
    pub workspace_id: String,
    #[serde(default)]
    pub preset_id: Option<String>,
    #[serde(default)]
    pub install_id: Option<String>,
    /// User this workflow runs on behalf of (issue #156). Threaded into
    /// `ShapingRequest.created_by` so stiglab can decrypt the matching
    /// `CLAUDE_CODE_OAUTH_TOKEN`. `None` for workflows that pre-date the
    /// migration; their dispatches fail loudly via `stiglab.session_failed`
    /// until the owner re-activates the workflow (which re-mirrors the row).
    #[serde(default)]
    pub created_by: Option<String>,
}

impl Workflow {
    /// Fetch a stage by its 0-based index. Returns `None` past the end of
    /// the chain — the caller treats that as "workflow complete for this
    /// artifact".
    pub fn stage(&self, index: usize) -> Option<&StageSpec> {
        self.stages.get(index)
    }

    /// The artifact kind produced by this workflow's trigger.
    ///
    /// v1 only has `github_issue_webhook` → produces `github-issue`
    /// artifacts. When more trigger kinds arrive, this grows into a
    /// match on `TriggerSpec`.
    pub fn trigger_artifact_kind(&self) -> &'static str {
        match self.trigger {
            TriggerSpec::GithubIssueWebhook { .. } => "github-issue",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_kind_tag_matches_db_check() {
        let trigger = TriggerSpec::GithubIssueWebhook {
            repo: "onsager-ai/onsager".into(),
            label: "ai-implementable".into(),
        };
        assert_eq!(trigger.kind_tag(), "github_issue_webhook");
    }

    #[test]
    fn gate_kind_tags_are_stable() {
        assert_eq!(
            GateSpec::AgentSession {
                shaping_intent: serde_json::Value::Null,
            }
            .kind_tag(),
            "agent_session"
        );
        assert_eq!(
            GateSpec::ExternalCheck {
                check_name: "ci".into(),
            }
            .kind_tag(),
            "external_check"
        );
        assert_eq!(
            GateSpec::Governance { gate_point: None }.kind_tag(),
            "governance"
        );
        assert_eq!(
            GateSpec::ManualApproval {
                signal_kind: "pr_merged".into(),
            }
            .kind_tag(),
            "manual_approval"
        );
    }

    #[test]
    fn trigger_artifact_kind_is_github_issue_for_v1() {
        let wf = Workflow {
            workflow_id: "wf_1".into(),
            name: "test".into(),
            trigger: TriggerSpec::GithubIssueWebhook {
                repo: "a/b".into(),
                label: "x".into(),
            },
            stages: vec![],
            active: true,
            workspace_id: "ws_test".into(),
            preset_id: None,
            install_id: None,
            created_by: None,
        };
        assert_eq!(wf.trigger_artifact_kind(), "github-issue");
    }

    #[test]
    fn stages_are_indexable_and_past_end_is_none() {
        let wf = Workflow {
            workflow_id: "wf_2".into(),
            name: "test".into(),
            trigger: TriggerSpec::GithubIssueWebhook {
                repo: "a/b".into(),
                label: "x".into(),
            },
            stages: vec![
                StageSpec {
                    name: "first".into(),
                    target_state: Some(ArtifactState::InProgress),
                    gates: vec![],
                    params: serde_json::Value::Null,
                },
                StageSpec {
                    name: "second".into(),
                    target_state: Some(ArtifactState::UnderReview),
                    gates: vec![],
                    params: serde_json::Value::Null,
                },
            ],
            active: true,
            workspace_id: "ws_test".into(),
            preset_id: None,
            install_id: None,
            created_by: None,
        };

        assert_eq!(wf.stage(0).map(|s| s.name.as_str()), Some("first"));
        assert_eq!(wf.stage(1).map(|s| s.name.as_str()), Some("second"));
        assert!(wf.stage(2).is_none());
    }

    #[test]
    fn workflow_roundtrip_serde() {
        // The dashboard edits workflows as JSON; a roundtrip keeps the
        // wire contract honest.
        let wf = Workflow {
            workflow_id: "wf_3".into(),
            name: "issue-to-pr".into(),
            trigger: TriggerSpec::GithubIssueWebhook {
                repo: "x/y".into(),
                label: "ai".into(),
            },
            stages: vec![StageSpec {
                name: "implement".into(),
                target_state: Some(ArtifactState::InProgress),
                gates: vec![
                    GateSpec::AgentSession {
                        shaping_intent: serde_json::json!({"role": "coder"}),
                    },
                    GateSpec::ExternalCheck {
                        check_name: "ci/test".into(),
                    },
                ],
                params: serde_json::json!({"timeout_minutes": 30}),
            }],
            active: true,
            workspace_id: "ws_test".into(),
            preset_id: Some("github_issue_to_pr".into()),
            install_id: Some("install_42".into()),
            created_by: Some("user_42".into()),
        };

        let json = serde_json::to_value(&wf).unwrap();
        let back: Workflow = serde_json::from_value(json).unwrap();
        assert_eq!(back, wf);
    }
}
