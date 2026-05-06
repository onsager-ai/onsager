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

// ── Session / Node / Task types (spec #222 Follow-up 3) ──────────────────────
//
// Portal takes ownership of /api/sessions/*, /api/tasks, /api/nodes.
// These types are portal-local copies of the equivalent types in
// `crates/stiglab/src/core/{session,node,task}.rs`; the wire shape is
// identical so the dashboard's existing payload contract is preserved.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub task_id: String,
    pub node_id: String,
    pub state: SessionState,
    pub prompt: String,
    pub output: Option<String>,
    pub working_dir: Option<String>,
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub artifact_version: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Pending,
    Dispatched,
    Running,
    WaitingInput,
    Done,
    Failed,
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Dispatched => write!(f, "dispatched"),
            Self::Running => write!(f, "running"),
            Self::WaitingInput => write!(f, "waiting_input"),
            Self::Done => write!(f, "done"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for SessionState {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "dispatched" => Ok(Self::Dispatched),
            "running" => Ok(Self::Running),
            "waiting_input" => Ok(Self::WaitingInput),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            _ => Err(anyhow::anyhow!("invalid session state: {s}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub status: NodeStatus,
    pub max_sessions: u32,
    pub active_sessions: u32,
    pub last_heartbeat: DateTime<Utc>,
    pub registered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Online,
    Offline,
    Draining,
}

impl std::fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Online => write!(f, "online"),
            Self::Offline => write!(f, "offline"),
            Self::Draining => write!(f, "draining"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub prompt: String,
    pub node_id: Option<String>,
    pub working_dir: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub max_turns: Option<u32>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub permission_mode: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    pub prompt: String,
    pub node_id: Option<String>,
    pub working_dir: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub max_turns: Option<u32>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

/// A single chunk of session log output (rendered in order of `seq`).
pub struct LogChunk {
    pub chunk: String,
    pub stream: String,
    pub created_at: String,
}

/// Log chunk with its sequence number (for cursor-based SSE streaming).
pub struct LogChunkWithSeq {
    pub seq: i64,
    pub chunk: String,
    pub stream: String,
}
