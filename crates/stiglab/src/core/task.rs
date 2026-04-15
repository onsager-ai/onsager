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
}
