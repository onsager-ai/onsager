use serde::{Deserialize, Serialize};

use crate::core::session::SessionState;

/// Events emitted by the system for dashboard consumption (via SSE).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    NodeRegistered {
        node_id: String,
        name: String,
    },
    NodeStatusChanged {
        node_id: String,
        status: String,
    },
    SessionStateChanged {
        session_id: String,
        state: SessionState,
    },
    SessionOutput {
        session_id: String,
        chunk: String,
    },
}
