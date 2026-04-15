use crate::core::session::SessionState;

#[derive(Debug, thiserror::Error)]
pub enum StiglabError {
    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("invalid state transition from {from} to {to}")]
    InvalidTransition {
        from: SessionState,
        to: SessionState,
    },

    #[error("node not found: {0}")]
    NodeNotFound(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("no available nodes for dispatch")]
    NoAvailableNodes,

    #[error("node at capacity: {0}")]
    NodeAtCapacity(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("websocket error: {0}")]
    WebSocket(String),
}
