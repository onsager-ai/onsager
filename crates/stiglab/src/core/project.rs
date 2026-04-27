use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A repo linked to both a workspace and a specific GitHub App
/// installation.  Opt-in per repo: installing the App on an org does not
/// auto-mirror all of that org's repos; the user explicitly adds each one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub workspace_id: String,
    pub github_app_installation_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub default_branch: String,
    pub created_at: DateTime<Utc>,
}
