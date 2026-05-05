//! Workflow domain model — persisted declarative production-line blueprint
//! that the dashboard surfaces and forge executes (issue #81 / parent #79).
//!
//! v1 semantics:
//! - **Trigger** kinds: `github-issue-webhook` only. Polling triggers are v2.
//! - **Stage** kinds (per stage-gate pair): `agent-session`, `external-check`,
//!   `governance`, `manual-approval`.
//! - Ordering is static — stages run in declared order, never reordered.
//!
//! A workflow is a plain DB record (not an artifact). The `workflow_stages`
//! child table holds the ordered stage chain; each stage has an opaque
//! `params: serde_json::Value` so gate kinds can carry kind-specific config
//! without forcing a schema churn when v1 adds new gates.

use chrono::{DateTime, Utc};
use onsager_spine::TriggerKind as SpineTriggerKind;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::core::error::StiglabError;

/// What fires a workflow. v1 ships only `GithubIssueWebhook`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriggerKind {
    GithubIssueWebhook,
}

impl fmt::Display for TriggerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TriggerKind::GithubIssueWebhook => write!(f, "{}", SpineTriggerKind::GithubIssueWebhook.kebab_case()),
        }
    }
}

impl FromStr for TriggerKind {
    type Err = StiglabError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            s if s == SpineTriggerKind::GithubIssueWebhook.kebab_case() => Ok(TriggerKind::GithubIssueWebhook),
            other => Err(StiglabError::InvalidState(format!(
                "invalid trigger kind: {other}"
            ))),
        }
    }
}

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
    pub trigger_kind: TriggerKind,
    /// Required for `github-issue-webhook`: the repo and label to watch.
    pub repo_owner: String,
    pub repo_name: String,
    pub trigger_label: String,
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
