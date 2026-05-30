//! `POST /api/workflows` request body + validation (spec #509 / ADR 0024
//! item 4).
//!
//! The trigger arrives as a structured [`TriggerKind`] — the registry-backed
//! discriminated union, the same shape the MCP `propose_workflow` tool
//! accepts — so any trigger kind (GitHub webhook, Telegram, Manual,
//! Schedule, spine-event) is creatable through one shape. There is no
//! `install_id is required` gate; GitHub workflows resolve their install
//! through the project layer at activation.

use serde::Deserialize;
use uuid::Uuid;

use crate::preset::{PRESET_IDS, resolve_preset};
use crate::workflow::{GateKind, TriggerKind, WorkflowStage};
use onsager_registry::TRIGGERS;

/// Request body for `POST /api/workflows`.
#[derive(Debug, Deserialize)]
pub struct CreateWorkflowBody {
    pub workspace_id: String,
    pub name: String,
    pub trigger: TriggerKind,
    /// Optional GitHub App installation id, persisted as a token-mint cache
    /// only (ADR 0024). `None` for non-GitHub triggers and for GitHub
    /// triggers whose install is resolved live through the project layer at
    /// activation. No longer a create-time gate.
    #[serde(default)]
    pub install_id: Option<i64>,
    #[serde(default)]
    pub preset_id: Option<String>,
    /// Optional explicit stage chain. Required when `preset_id` is not set.
    #[serde(default)]
    pub stages: Option<Vec<CreateStageBody>>,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateStageBody {
    pub gate_kind: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// `(owner, name)` for any GitHub-shaped webhook trigger, or `None` for
/// non-GitHub kinds. Mirrors [`crate::workflow::Workflow::github_repo_any`]
/// but operates on a bare [`TriggerKind`] (before a `Workflow` row exists).
fn trigger_github_repo(trigger: &TriggerKind) -> Option<(&str, &str)> {
    match trigger {
        TriggerKind::GithubIssueWebhook { repo, .. }
        | TriggerKind::GithubPullRequestClosed { repo, .. }
        | TriggerKind::GithubWorkflowRunCompleted { repo, .. } => repo.split_once('/'),
        _ => None,
    }
}

/// Validate a create-workflow body and expand its stage chain.
///
/// The trigger must be a registry-known kind; GitHub webhook kinds must
/// carry a well-formed `owner/name` slug (the activation pipeline needs it).
/// Any trigger kind is accepted — there is no `install_id is required` gate
/// (ADR 0024 item 4).
pub fn validate_create_body(
    body: &CreateWorkflowBody,
) -> Result<(TriggerKind, Vec<WorkflowStage>), String> {
    if body.name.trim().is_empty() {
        return Err("name is required".into());
    }

    let trigger = body.trigger.clone();
    let kind_tag = trigger.kind_tag();
    if TRIGGERS.lookup(kind_tag).is_none() {
        return Err(format!("invalid trigger kind: {kind_tag}"));
    }
    // GitHub webhook kinds need a well-formed `owner/name` slug for the
    // activation pipeline (label create / webhook register).
    if let Some((owner, name)) = trigger_github_repo(&trigger) {
        if owner.trim().is_empty() || name.trim().is_empty() {
            return Err("github trigger requires a non-empty owner/name repo".into());
        }
    } else if matches!(
        trigger,
        TriggerKind::GithubIssueWebhook { .. }
            | TriggerKind::GithubPullRequestClosed { .. }
            | TriggerKind::GithubWorkflowRunCompleted { .. }
    ) {
        return Err("github trigger repo must be in `owner/name` form".into());
    }

    // Reject requests that ship both a preset and explicit stages — the
    // two are mutually exclusive and silently preferring one is surprising.
    if body.preset_id.is_some() && body.stages.as_ref().is_some_and(|s| !s.is_empty()) {
        return Err("provide either preset_id or stages, not both".into());
    }

    let stages = if let Some(preset_id) = body.preset_id.as_deref() {
        if !PRESET_IDS.contains(&preset_id) {
            return Err(format!("unknown preset_id: {preset_id}"));
        }
        let expansion =
            resolve_preset(preset_id).ok_or_else(|| format!("unknown preset_id: {preset_id}"))?;
        if expansion.trigger_kind_tag != trigger.kind_tag() {
            return Err(format!(
                "preset {preset_id} expects trigger_kind {} but got {}",
                expansion.trigger_kind_tag,
                trigger.kind_tag()
            ));
        }
        expansion
            .stages
            .into_iter()
            .enumerate()
            .map(|(i, s)| WorkflowStage {
                id: Uuid::new_v4().to_string(),
                workflow_id: String::new(),
                seq: i as i32,
                gate_kind: s.gate_kind,
                params: s.params,
            })
            .collect()
    } else {
        let explicit = body
            .stages
            .as_ref()
            .ok_or_else(|| "stages or preset_id required".to_string())?;
        if explicit.is_empty() {
            return Err("at least one stage is required".into());
        }
        let mut out = Vec::with_capacity(explicit.len());
        for (i, s) in explicit.iter().enumerate() {
            let gate_kind = s.gate_kind.parse::<GateKind>().map_err(|e| e.to_string())?;
            out.push(WorkflowStage {
                id: Uuid::new_v4().to_string(),
                workflow_id: String::new(),
                seq: i as i32,
                gate_kind,
                params: s.params.clone().unwrap_or_else(|| serde_json::json!({})),
            });
        }
        out
    };

    Ok((trigger, stages))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_body() -> CreateWorkflowBody {
        CreateWorkflowBody {
            workspace_id: "w1".into(),
            name: "sdd".into(),
            trigger: TriggerKind::GithubIssueWebhook {
                repo: "acme/widgets".into(),
                label: "spec".into(),
            },
            install_id: Some(42),
            preset_id: Some("github-issue-to-pr".into()),
            stages: None,
            active: false,
        }
    }

    #[test]
    fn validate_preset_expands_to_stage_chain() {
        let (trigger, stages) = validate_create_body(&base_body()).unwrap();
        assert_eq!(
            trigger,
            TriggerKind::GithubIssueWebhook {
                repo: "acme/widgets".into(),
                label: "spec".into(),
            }
        );
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].gate_kind, GateKind::AgentSession);
        assert_eq!(stages[0].seq, 0);
    }

    #[test]
    fn validate_rejects_malformed_github_repo() {
        let mut body = base_body();
        body.trigger = TriggerKind::GithubIssueWebhook {
            repo: "no-slash".into(),
            label: "spec".into(),
        };
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.to_lowercase().contains("owner/name"));
    }

    /// ADR 0024 item 4: any registry trigger kind is creatable — a
    /// Manual-only workflow with no install is accepted. Mutating
    /// `validate_create_body` to re-add a GitHub-only gate fails here.
    #[test]
    fn validate_accepts_manual_trigger_without_install() {
        let mut body = base_body();
        body.trigger = TriggerKind::Manual {
            name: "kickoff".into(),
        };
        body.preset_id = None;
        body.install_id = None;
        body.stages = Some(vec![CreateStageBody {
            gate_kind: "agent-session".into(),
            params: None,
        }]);
        let (trigger, stages) = validate_create_body(&body).unwrap();
        assert_eq!(
            trigger,
            TriggerKind::Manual {
                name: "kickoff".into()
            }
        );
        assert_eq!(stages.len(), 1);
    }

    #[test]
    fn validate_rejects_unknown_preset() {
        let mut body = base_body();
        body.preset_id = Some("nope".into());
        body.stages = None;
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.contains("unknown preset_id"));
    }

    #[test]
    fn validate_accepts_explicit_stages_when_no_preset() {
        let mut body = base_body();
        body.preset_id = None;
        body.stages = Some(vec![
            CreateStageBody {
                gate_kind: "agent-session".into(),
                params: Some(serde_json::json!({"profile": "implementer"})),
            },
            CreateStageBody {
                gate_kind: "external-check".into(),
                params: None,
            },
            CreateStageBody {
                gate_kind: "manual-approval".into(),
                params: None,
            },
        ]);
        let (_t, stages) = validate_create_body(&body).unwrap();
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].gate_kind, GateKind::AgentSession);
        assert_eq!(stages[1].gate_kind, GateKind::ExternalCheck);
        assert_eq!(stages[2].gate_kind, GateKind::ManualApproval);
    }

    #[test]
    fn validate_rejects_empty_stage_chain_without_preset() {
        let mut body = base_body();
        body.preset_id = None;
        body.stages = Some(vec![]);
        assert!(validate_create_body(&body).is_err());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut body = base_body();
        body.name = "  ".into();
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.contains("name"));
    }

    #[test]
    fn validate_rejects_both_preset_and_stages() {
        let mut body = base_body();
        body.preset_id = Some("github-issue-to-pr".into());
        body.stages = Some(vec![CreateStageBody {
            gate_kind: "agent-session".into(),
            params: None,
        }]);
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.contains("preset_id or stages"));
    }
}
