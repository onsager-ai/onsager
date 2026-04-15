//! Forge process state machine (forge-v0.1 §8).
//!
//! ```text
//! running ──pause──► paused
//!    ▲                  │
//!    └────── resume ────┘
//!    │
//!    ▼
//! draining ──drained──► stopped
//! ```

use onsager_spine::factory_event::ForgeProcessState;

/// Forge process state machine.
///
/// Invariant: pausing never loses in-flight work (forge invariant #9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForgeState {
    current: ForgeProcessState,
}

impl ForgeState {
    pub fn new() -> Self {
        Self {
            current: ForgeProcessState::Running,
        }
    }

    pub fn current(&self) -> ForgeProcessState {
        self.current
    }

    /// Attempt a state transition. Returns the previous state on success,
    /// or an error if the transition is invalid.
    pub fn transition(
        &mut self,
        target: ForgeProcessState,
    ) -> Result<ForgeProcessState, ForgeStateError> {
        use ForgeProcessState::*;
        let valid = matches!(
            (self.current, target),
            (Running, Paused) | (Paused, Running) | (Running, Draining) | (Draining, Stopped)
        );
        if valid {
            let prev = self.current;
            self.current = target;
            Ok(prev)
        } else {
            Err(ForgeStateError::InvalidTransition {
                from: self.current,
                to: target,
            })
        }
    }

    /// Whether the kernel should produce decisions in this state.
    pub fn should_decide(&self) -> bool {
        self.current == ForgeProcessState::Running
    }

    /// Whether in-flight work should still be processed.
    pub fn should_process_results(&self) -> bool {
        matches!(
            self.current,
            ForgeProcessState::Running | ForgeProcessState::Paused | ForgeProcessState::Draining
        )
    }
}

impl Default for ForgeState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ForgeStateError {
    #[error("invalid forge state transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: ForgeProcessState,
        to: ForgeProcessState,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_spine::factory_event::ForgeProcessState::*;

    #[test]
    fn initial_state_is_running() {
        let state = ForgeState::new();
        assert_eq!(state.current(), Running);
        assert!(state.should_decide());
    }

    #[test]
    fn pause_and_resume() {
        let mut state = ForgeState::new();
        assert!(state.transition(Paused).is_ok());
        assert_eq!(state.current(), Paused);
        assert!(!state.should_decide());
        assert!(state.should_process_results());

        assert!(state.transition(Running).is_ok());
        assert_eq!(state.current(), Running);
    }

    #[test]
    fn drain_and_stop() {
        let mut state = ForgeState::new();
        assert!(state.transition(Draining).is_ok());
        assert!(!state.should_decide());
        assert!(state.should_process_results());

        assert!(state.transition(Stopped).is_ok());
        assert!(!state.should_decide());
        assert!(!state.should_process_results());
    }

    #[test]
    fn invalid_transitions() {
        let mut state = ForgeState::new();
        // Can't go directly to Stopped
        assert!(state.transition(Stopped).is_err());

        // Can't go from Paused to Draining
        state.transition(Paused).unwrap();
        assert!(state.transition(Draining).is_err());

        // Can't go from Stopped to anything
        let mut state = ForgeState::new();
        state.transition(Draining).unwrap();
        state.transition(Stopped).unwrap();
        assert!(state.transition(Running).is_err());
        assert!(state.transition(Paused).is_err());
    }

    #[test]
    fn transition_returns_previous_state() {
        let mut state = ForgeState::new();
        let prev = state.transition(Paused).unwrap();
        assert_eq!(prev, Running);
    }
}
