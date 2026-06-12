//! Workflow domain model — persisted declarative production-line blueprint
//! that the dashboard surfaces and forge executes (issue #81 / parent #79).
//!
//! Trigger kinds and per-kind config live in [`onsager_spine::TriggerKind`]
//! (the canonical registry-backed type, spec #237). Stage / gate kinds are
//! still workflow-local.
//!
//! v1 semantics:
//! - **Trigger** kinds: `github_issue_webhook` only.
//! - **Stage** kinds (per stage-gate pair): `agent-session`, `external-check`,
//!   `manual-approval`.
//! - Ordering is static — stages run in declared order, never reordered.
//!
//! A workflow is a plain DB record (not an artifact). The `workflow_stages`
//! child table holds the ordered stage chain; each stage has an opaque
//! `params: serde_json::Value` so gate kinds can carry kind-specific config
//! without forcing a schema churn when v1 adds new gates.

use chrono::{DateTime, Utc};
pub use onsager_spine::TriggerKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use ts_rs::TS;

/// Gate kind attached to a workflow stage. These map to the four gate runtime
/// implementations forge ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, rename = "WorkflowGateKind")]
pub enum GateKind {
    AgentSession,
    ExternalCheck,
    ManualApproval,
}

impl fmt::Display for GateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateKind::AgentSession => write!(f, "agent-session"),
            GateKind::ExternalCheck => write!(f, "external-check"),
            GateKind::ManualApproval => write!(f, "manual-approval"),
        }
    }
}

impl FromStr for GateKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "agent-session" => Ok(GateKind::AgentSession),
            "external-check" => Ok(GateKind::ExternalCheck),
            "manual-approval" => Ok(GateKind::ManualApproval),
            other => Err(anyhow::anyhow!("invalid gate kind: {other}")),
        }
    }
}

/// One ordered stage in a workflow's stage chain.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub struct WorkflowStage {
    pub id: String,
    pub workflow_id: String,
    /// 0-based position in the chain. UNIQUE per workflow; stages walk in
    /// ascending `seq` order.
    pub seq: i32,
    pub gate_kind: GateKind,
    /// Whether this stage's gate kind has a registered engine executor
    /// today (ADR 0028 / #602). `external-check` / `manual-approval`
    /// stages stay in the authored definition but are excluded from the
    /// executable DAG until their substrate executors land.
    #[serde(default)]
    pub executable: bool,
    /// Gate-kind-specific config (e.g. agent profile, check name). Stored as
    /// free-form JSON so the schema doesn't need to know every param shape.
    pub params: serde_json::Value,
}

/// A persisted workflow blueprint.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub struct Workflow {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    /// Trigger kind + per-kind config. Persisted as
    /// `(workflows.trigger_kind, workflows.trigger_config)`.
    pub trigger: TriggerKind,
    /// GitHub App installation id used to mint tokens when this workflow's
    /// trigger is a GitHub webhook. Per ADR 0024 this is a **token-mint
    /// cache only** — it does NOT participate in lifecycle status
    /// (`readiness` / `autofire`), and it is `None` for workflows whose
    /// trigger needs no GitHub install (Telegram / Manual / Schedule /
    /// spine-event) and for GitHub workflows whose install is resolved
    /// live through `(workspace_id, repo) → project → installation`.
    ///
    /// Annotated `ts(type = "number | null")` because `JSON.parse` in the
    /// dashboard returns a JS `number` for integers, not a `bigint` —
    /// ts-rs v12's default `bigint` mapping for i64 would mis-type the
    /// wire value. GitHub installation IDs fit comfortably in
    /// `Number.MAX_SAFE_INTEGER` (2^53), so the precision loss is
    /// theoretical only.
    #[ts(type = "number | null")]
    pub install_id: Option<i64>,
    /// Preset id this workflow was expanded from (e.g. `github-issue-to-pr`).
    /// `None` for custom workflows built in the card editor.
    pub preset_id: Option<String>,
    pub active: bool,
    /// Lifecycle **readiness** axis (ADR 0024): `"drafted"` → `"ready"`.
    /// `ready` ⇔ the workflow has ≥1 trigger binding of any kind. Per
    /// spec #512 a `ready` workflow earns the manual "run a batch"
    /// button; the dashboard gates that button on this field rather than
    /// re-deriving from `active`.
    pub readiness: String,
    /// Lifecycle **autofire** axis (ADR 0024): `"off"` / `"active"` /
    /// `"paused"`. Orthogonal to `readiness` — a `ready` workflow may be
    /// `off` (manual-only), `active` (auto-firing), or `paused`. `active`
    /// (the legacy firing pin) is kept in sync with this by portal's
    /// single writer path.
    pub autofire: String,
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
            _ => None,
        }
    }

    /// The label this workflow filters on, when the trigger is a GitHub
    /// `issues.labeled` webhook.
    pub fn github_label(&self) -> Option<&str> {
        match &self.trigger {
            TriggerKind::GithubIssueWebhook { label, .. } => Some(label.as_str()),
            _ => None,
        }
    }

    /// `(repo_owner, repo_name)` extracted from the trigger config for any
    /// GitHub-shaped webhook trigger (issues, PR-closed, workflow-run-completed).
    /// Returns `None` for non-GitHub kinds.
    pub fn github_repo_any(&self) -> Option<(&str, &str)> {
        match &self.trigger {
            TriggerKind::GithubIssueWebhook { repo, .. }
            | TriggerKind::GithubPullRequestClosed { repo, .. }
            | TriggerKind::GithubWorkflowRunCompleted { repo, .. } => repo.split_once('/'),
            _ => None,
        }
    }
}
