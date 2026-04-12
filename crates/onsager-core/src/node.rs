use serde::{Deserialize, Serialize};

/// A node — a machine that runs agent sessions.
/// In Level 1, there is always exactly one local node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub active_sessions: u32,
}

impl Node {
    /// Create a local node with auto-detected hostname.
    pub fn local(id: String, name: String) -> Self {
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        Self {
            id,
            name,
            hostname,
            active_sessions: 0,
        }
    }
}
