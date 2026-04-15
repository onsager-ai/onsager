use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::core::error::StiglabError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub task_id: String,
    pub node_id: String,
    pub state: SessionState,
    pub prompt: String,
    pub output: Option<String>,
    pub working_dir: Option<String>,
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

impl SessionState {
    /// Validate that a state transition is allowed.
    pub fn can_transition_to(self, next: SessionState) -> bool {
        matches!(
            (self, next),
            (SessionState::Pending, SessionState::Dispatched)
                | (SessionState::Dispatched, SessionState::Running)
                | (SessionState::Running, SessionState::WaitingInput)
                | (SessionState::Running, SessionState::Done)
                | (SessionState::Running, SessionState::Failed)
                | (SessionState::WaitingInput, SessionState::Running)
                | (SessionState::WaitingInput, SessionState::Done)
                | (SessionState::WaitingInput, SessionState::Failed)
                // Allow any state to transition to Failed (e.g., on timeout)
                | (SessionState::Pending, SessionState::Failed)
                | (SessionState::Dispatched, SessionState::Failed)
        )
    }

    /// Attempt a state transition, returning an error if invalid.
    pub fn transition_to(self, next: SessionState) -> Result<SessionState, StiglabError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(StiglabError::InvalidTransition {
                from: self,
                to: next,
            })
        }
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionState::Pending => write!(f, "pending"),
            SessionState::Dispatched => write!(f, "dispatched"),
            SessionState::Running => write!(f, "running"),
            SessionState::WaitingInput => write!(f, "waiting_input"),
            SessionState::Done => write!(f, "done"),
            SessionState::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for SessionState {
    type Err = StiglabError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(SessionState::Pending),
            "dispatched" => Ok(SessionState::Dispatched),
            "running" => Ok(SessionState::Running),
            "waiting_input" => Ok(SessionState::WaitingInput),
            "done" => Ok(SessionState::Done),
            "failed" => Ok(SessionState::Failed),
            _ => Err(StiglabError::InvalidState(format!(
                "invalid session state: {s}"
            ))),
        }
    }
}
