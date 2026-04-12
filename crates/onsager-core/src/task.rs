use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A task — the unit of work submitted to the factory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub prompt: String,
    pub working_dir: String,
    pub model: Option<String>,
    pub max_turns: Option<u32>,
    pub system_prompt: Option<String>,
    pub permission_mode: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Request to create a new task (input model, no id or timestamp).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    pub prompt: String,
    pub working_dir: String,
    pub model: Option<String>,
    pub max_turns: Option<u32>,
    pub system_prompt: Option<String>,
    pub permission_mode: Option<String>,
}
