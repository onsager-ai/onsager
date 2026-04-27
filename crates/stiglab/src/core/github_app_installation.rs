use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::core::error::StiglabError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubAccountType {
    User,
    Organization,
}

impl fmt::Display for GitHubAccountType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitHubAccountType::User => write!(f, "user"),
            GitHubAccountType::Organization => write!(f, "organization"),
        }
    }
}

impl FromStr for GitHubAccountType {
    type Err = StiglabError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" | "User" => Ok(GitHubAccountType::User),
            "organization" | "Organization" => Ok(GitHubAccountType::Organization),
            other => Err(StiglabError::InvalidState(format!(
                "invalid github account type: {other}"
            ))),
        }
    }
}

/// A GitHub App installation linked to a workspace.  A workspace may have
/// 0..N installations (typical: exactly one; but cross-org workspaces can
/// link more).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubAppInstallation {
    pub id: String,
    pub workspace_id: String,
    pub install_id: i64,
    pub account_login: String,
    pub account_type: GitHubAccountType,
    pub created_at: DateTime<Utc>,
}
