use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// High-level trigger taxonomy used by workflow runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerCategory {
    Schedule,
    Event,
    Request,
    Manual,
}

/// Canonical trigger kind registry key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    GithubIssueWebhook,
}

impl TriggerKind {
    pub const fn snake_case(self) -> &'static str {
        match self {
            TriggerKind::GithubIssueWebhook => "github_issue_webhook",
        }
    }

    pub const fn kebab_case(self) -> &'static str {
        match self {
            TriggerKind::GithubIssueWebhook => "github-issue-webhook",
        }
    }

    pub const fn category(self) -> TriggerCategory {
        match self {
            TriggerKind::GithubIssueWebhook => TriggerCategory::Event,
        }
    }

    pub const fn all() -> &'static [TriggerKind] {
        &[TriggerKind::GithubIssueWebhook]
    }
}

impl FromStr for TriggerKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github_issue_webhook" | "github-issue-webhook" => Ok(TriggerKind::GithubIssueWebhook),
            other => Err(format!("invalid trigger kind: {other}")),
        }
    }
}
