use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

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

impl fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeStatus::Online => write!(f, "online"),
            NodeStatus::Offline => write!(f, "offline"),
            NodeStatus::Draining => write!(f, "draining"),
        }
    }
}

impl std::str::FromStr for NodeStatus {
    type Err = crate::core::error::StiglabError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "online" => Ok(NodeStatus::Online),
            "offline" => Ok(NodeStatus::Offline),
            "draining" => Ok(NodeStatus::Draining),
            _ => Err(crate::core::error::StiglabError::InvalidState(format!(
                "invalid node status: {s}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub name: String,
    pub hostname: String,
    pub max_sessions: u32,
}
