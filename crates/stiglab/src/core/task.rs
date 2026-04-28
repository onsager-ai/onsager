use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    /// Optional workspace-owned project this session is scoped to (issue #59).
    /// The server validates the caller is a member of the project's workspace;
    /// omitted for personal sessions.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Optional explicit workspace scope (#164). When set without
    /// `project_id`, the session is filed under this workspace and
    /// shows up in `/api/sessions?workspace=` listings + receives the
    /// workspace's credential bundle at dispatch. When `project_id`
    /// is set, that wins (the project owns its workspace context); a
    /// disagreement between the two surfaces as a 400. Omitting both
    /// preserves the legacy "personal session" path that doesn't
    /// surface in any workspace listing.
    #[serde(default)]
    pub workspace_id: Option<String>,
}
