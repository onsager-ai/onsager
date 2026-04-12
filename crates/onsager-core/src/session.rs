use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use onsager_events::CoreEvent;

/// Session state machine — ported from stiglab-core with the same
/// six states and validated transitions.
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
    /// Check whether a transition to `next` is valid.
    pub fn can_transition_to(&self, next: SessionState) -> bool {
        matches!(
            (self, next),
            (SessionState::Pending, SessionState::Dispatched)
                | (SessionState::Pending, SessionState::Failed)
                | (SessionState::Dispatched, SessionState::Running)
                | (SessionState::Dispatched, SessionState::Failed)
                | (SessionState::Running, SessionState::WaitingInput)
                | (SessionState::Running, SessionState::Done)
                | (SessionState::Running, SessionState::Failed)
                | (SessionState::WaitingInput, SessionState::Running)
                | (SessionState::WaitingInput, SessionState::Done)
                | (SessionState::WaitingInput, SessionState::Failed)
        )
    }

    /// Attempt a validated transition. Returns the new state or an error.
    pub fn transition_to(self, next: SessionState) -> Result<SessionState, SessionError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(SessionError::InvalidTransition {
                from: self,
                to: next,
            })
        }
    }

    /// Whether this state is terminal (no further transitions possible).
    pub fn is_terminal(&self) -> bool {
        matches!(self, SessionState::Done | SessionState::Failed)
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
    type Err = SessionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(SessionState::Pending),
            "dispatched" => Ok(SessionState::Dispatched),
            "running" => Ok(SessionState::Running),
            "waiting_input" => Ok(SessionState::WaitingInput),
            "done" => Ok(SessionState::Done),
            "failed" => Ok(SessionState::Failed),
            _ => Err(SessionError::InvalidState(s.to_string())),
        }
    }
}

/// Errors related to session state management.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("invalid state transition: {from} -> {to}")]
    InvalidTransition {
        from: SessionState,
        to: SessionState,
    },
    #[error("invalid state: {0}")]
    InvalidState(String),
}

/// A session — materialized from the event stream.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub task_id: String,
    pub node_id: String,
    pub state: SessionState,
    pub prompt: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    /// Apply a core event to update session state (event folding).
    pub fn apply(&mut self, event: &CoreEvent) {
        match event {
            CoreEvent::SessionCreated {
                session_id,
                task_id,
                node_id,
            } => {
                self.id.clone_from(session_id);
                self.task_id.clone_from(task_id);
                self.node_id.clone_from(node_id);
                self.state = SessionState::Pending;
            }
            CoreEvent::SessionDispatched { .. } => {
                self.state = SessionState::Dispatched;
            }
            CoreEvent::SessionRunning { .. } => {
                self.state = SessionState::Running;
            }
            CoreEvent::SessionWaitingInput { .. } => {
                self.state = SessionState::WaitingInput;
            }
            CoreEvent::SessionCompleted { .. } => {
                self.state = SessionState::Done;
            }
            CoreEvent::SessionFailed { .. } => {
                self.state = SessionState::Failed;
            }
            _ => {}
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self {
            id: String::new(),
            task_id: String::new(),
            node_id: String::new(),
            state: SessionState::Pending,
            prompt: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_transitions() {
        assert!(SessionState::Pending.can_transition_to(SessionState::Dispatched));
        assert!(SessionState::Dispatched.can_transition_to(SessionState::Running));
        assert!(SessionState::Running.can_transition_to(SessionState::WaitingInput));
        assert!(SessionState::Running.can_transition_to(SessionState::Done));
        assert!(SessionState::Running.can_transition_to(SessionState::Failed));
        assert!(SessionState::WaitingInput.can_transition_to(SessionState::Running));
        assert!(SessionState::WaitingInput.can_transition_to(SessionState::Done));
    }

    #[test]
    fn test_invalid_transitions() {
        assert!(!SessionState::Pending.can_transition_to(SessionState::Running));
        assert!(!SessionState::Done.can_transition_to(SessionState::Running));
        assert!(!SessionState::Failed.can_transition_to(SessionState::Running));
        assert!(!SessionState::Done.can_transition_to(SessionState::Failed));
    }

    #[test]
    fn test_transition_to() {
        assert!(SessionState::Pending
            .transition_to(SessionState::Dispatched)
            .is_ok());
        assert!(SessionState::Done
            .transition_to(SessionState::Running)
            .is_err());
    }

    #[test]
    fn test_display_and_parse() {
        assert_eq!(SessionState::Pending.to_string(), "pending");
        assert_eq!(SessionState::WaitingInput.to_string(), "waiting_input");
        assert_eq!("done".parse::<SessionState>().unwrap(), SessionState::Done);
        assert!("invalid".parse::<SessionState>().is_err());
    }

    #[test]
    fn test_is_terminal() {
        assert!(SessionState::Done.is_terminal());
        assert!(SessionState::Failed.is_terminal());
        assert!(!SessionState::Running.is_terminal());
        assert!(!SessionState::Pending.is_terminal());
    }

    #[test]
    fn test_session_apply_event_folding() {
        let mut session = Session::default();

        session.apply(&CoreEvent::SessionCreated {
            session_id: "s-1".into(),
            task_id: "t-1".into(),
            node_id: "n-1".into(),
        });
        assert_eq!(session.id, "s-1");
        assert_eq!(session.state, SessionState::Pending);

        session.apply(&CoreEvent::SessionDispatched {
            session_id: "s-1".into(),
        });
        assert_eq!(session.state, SessionState::Dispatched);

        session.apply(&CoreEvent::SessionRunning {
            session_id: "s-1".into(),
        });
        assert_eq!(session.state, SessionState::Running);

        session.apply(&CoreEvent::SessionCompleted {
            session_id: "s-1".into(),
            output: Some("done".into()),
        });
        assert_eq!(session.state, SessionState::Done);
    }
}
