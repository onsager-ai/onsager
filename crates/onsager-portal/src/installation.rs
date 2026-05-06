//! Domain types for GitHub App installations linked to workspaces.
//!
//! Spec #222 Slice 3b moved the routes and schema from stiglab to
//! portal. Stiglab keeps its own `GitHubAccountType` /
//! `GitHubAppInstallation` types in `crates/stiglab/src/core/` for the
//! in-process needs of session orchestration and live-data hydration;
//! the wire shape is identical so the dashboard's existing payload
//! contract is preserved.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use onsager_github::credential::AccountKind;

/// Whether the GitHub account that installed the App is a personal
/// account or an organization. Mirrors stiglab's `core::GitHubAccountType`
/// byte-for-byte so the migration is wire-compatible.
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

impl GitHubAccountType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" | "User" => Some(GitHubAccountType::User),
            "organization" | "Organization" => Some(GitHubAccountType::Organization),
            _ => None,
        }
    }
}

impl From<AccountKind> for GitHubAccountType {
    fn from(k: AccountKind) -> Self {
        match k {
            AccountKind::User => GitHubAccountType::User,
            AccountKind::Organization => GitHubAccountType::Organization,
        }
    }
}

/// A GitHub App installation linked to a workspace. A workspace may
/// have 0..N installations (typical: exactly one; cross-org workspaces
/// can link more).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubAppInstallation {
    pub id: String,
    pub workspace_id: String,
    pub install_id: i64,
    pub account_login: String,
    pub account_type: GitHubAccountType,
    pub created_at: DateTime<Utc>,
}
