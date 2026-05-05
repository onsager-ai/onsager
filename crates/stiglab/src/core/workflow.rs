//! Workflow domain model — persisted declarative production-line blueprint
//! that the dashboard surfaces and forge executes (issue #81 / parent #79).
//!
//! Trigger kinds and per-kind config live in [`onsager_spine::TriggerKind`]
//! (the canonical registry-backed type, spec #237). Stage / gate kinds are
//! still stiglab-local.
//!
//! v1 semantics:
//! - **Trigger** kinds: `github_issue_webhook` only.
//! - **Stage** kinds (per stage-gate pair): `agent-session`, `external-check`,
//!   `governance`, `manual-approval`.
//! - Ordering is static — stages run in declared order, never reordered.
//!
//! A workflow is a plain DB record (not an artifact). The `workflow_stages`
//! child table holds the ordered stage chain; each stage has an opaque
//! `params: serde_json::Value` so gate kinds can carry kind-specific config
//! without forcing a schema churn when v1 adds new gates.

use chrono::{DateTime, Utc};
pub use onsager_spine::TriggerKind;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::core::error::StiglabError;

/// Gate kind attached to a workflow stage. These map to the four gate runtime
/// implementations forge ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GateKind {
    AgentSession,
    ExternalCheck,
    Governance,
    ManualApproval,
}

impl fmt::Display for GateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateKind::AgentSession => write!(f, "agent-session"),
            GateKind::ExternalCheck => write!(f, "external-check"),
            GateKind::Governance => write!(f, "governance"),
            GateKind::ManualApproval => write!(f, "manual-approval"),
        }
    }
}

impl FromStr for GateKind {
    type Err = StiglabError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "agent-session" => Ok(GateKind::AgentSession),
            "external-check" => Ok(GateKind::ExternalCheck),
            "governance" => Ok(GateKind::Governance),
            "manual-approval" => Ok(GateKind::ManualApproval),
            other => Err(StiglabError::InvalidState(format!(
                "invalid gate kind: {other}"
            ))),
        }
    }
}

/// One ordered stage in a workflow's stage chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStage {
    pub id: String,
    pub workflow_id: String,
    /// 0-based position in the chain. UNIQUE per workflow; stages walk in
    /// ascending `seq` order.
    pub seq: i32,
    pub gate_kind: GateKind,
    /// Gate-kind-specific config (e.g. agent profile, check name). Stored as
    /// free-form JSON so the schema doesn't need to know every param shape.
    pub params: serde_json::Value,
}

/// A persisted workflow blueprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    /// Trigger kind + per-kind config. Persisted as
    /// `(workflows.trigger_kind, workflows.trigger_config)`.
    pub trigger: TriggerKind,
    /// Workspace GitHub App install that fires this workflow (reused from
    /// issue #70 — no per-workflow install flow).
    pub install_id: i64,
    /// Preset id this workflow was expanded from (e.g. `github-issue-to-pr`).
    /// `None` for custom workflows built in the card editor.
    pub preset_id: Option<String>,
    pub active: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Workflow {
    /// `(repo_owner, repo_name)` extracted from the trigger config when the
    /// kind is `GithubIssueWebhook`. The webhook activation / deactivation
    /// helpers use this; for any other kind they should not be calling.
    pub fn github_repo(&self) -> Option<(&str, &str)> {
        match &self.trigger {
            TriggerKind::GithubIssueWebhook { repo, .. } => repo.split_once('/'),
        }
    }

    /// The label this workflow filters on, when the trigger is a GitHub
    /// `issues.labeled` webhook.
    pub fn github_label(&self) -> Option<&str> {
        match &self.trigger {
            TriggerKind::GithubIssueWebhook { label, .. } => Some(label.as_str()),
        }
    }
}
