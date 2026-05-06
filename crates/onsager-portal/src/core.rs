//! Domain types for the portal-owned external surface.
//!
//! Spec #222 Slice 3a moved workspace / membership / project CRUD from
//! stiglab to portal. The corresponding domain types move with them so
//! portal's handlers don't import from stiglab (forbidden by the seam
//! rule). Stiglab keeps its own copies under `crates/stiglab/src/core/`
//! for the in-process needs of session orchestration; the wire shape is
//! identical so the dashboard's existing payload contract is preserved.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An Onsager-native workspace identity. Owns membership, GitHub
/// installations, and projects. Identity is owned by Onsager (not borrowed
/// from any external provider), so future source providers can hang off
/// the same workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

/// Many-to-many join between users and workspaces. v1 has no `role`
/// column — every member has equal access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMember {
    pub workspace_id: String,
    pub user_id: String,
    pub joined_at: DateTime<Utc>,
}

/// `WorkspaceMember` enriched with the member's GitHub profile so the
/// dashboard can render `@login` + avatar instead of an opaque user UUID.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceMemberWithUser {
    pub workspace_id: String,
    pub user_id: String,
    pub joined_at: DateTime<Utc>,
    pub github_login: Option<String>,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
}

/// A repo linked to both a workspace and a specific GitHub App
/// installation. Opt-in per repo: installing the App on an org does not
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
