pub mod adapter;
pub mod error;
pub mod event;
pub mod github_app_installation;
pub mod node;
pub mod project;
pub mod protocol;
pub mod session;
pub mod task;
pub mod tenant;
pub mod tenant_member;
pub mod user;

pub use error::StiglabError;
pub use event::Event;
pub use github_app_installation::{GitHubAccountType, GitHubAppInstallation};
pub use node::{Node, NodeInfo, NodeStatus};
pub use project::Project;
pub use protocol::{AgentMessage, ServerMessage};
pub use session::{Session, SessionState};
pub use task::{Task, TaskRequest};
pub use tenant::Tenant;
pub use tenant_member::TenantMember;
pub use user::User;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_state_valid_transitions() {
        assert!(SessionState::Pending.can_transition_to(SessionState::Dispatched));
        assert!(SessionState::Dispatched.can_transition_to(SessionState::Running));
        assert!(SessionState::Running.can_transition_to(SessionState::WaitingInput));
        assert!(SessionState::Running.can_transition_to(SessionState::Done));
        assert!(SessionState::Running.can_transition_to(SessionState::Failed));
        assert!(SessionState::WaitingInput.can_transition_to(SessionState::Running));
        assert!(SessionState::WaitingInput.can_transition_to(SessionState::Done));
    }

    #[test]
    fn test_session_state_invalid_transitions() {
        assert!(!SessionState::Done.can_transition_to(SessionState::Running));
        assert!(!SessionState::Failed.can_transition_to(SessionState::Running));
        assert!(!SessionState::Pending.can_transition_to(SessionState::Running));
        assert!(!SessionState::Done.can_transition_to(SessionState::WaitingInput));
    }

    #[test]
    fn test_session_state_transition_to() {
        let state = SessionState::Pending;
        assert!(state.transition_to(SessionState::Dispatched).is_ok());
        assert!(state.transition_to(SessionState::Running).is_err());
    }

    #[test]
    fn test_session_state_display() {
        assert_eq!(SessionState::Pending.to_string(), "pending");
        assert_eq!(SessionState::WaitingInput.to_string(), "waiting_input");
        assert_eq!(SessionState::Done.to_string(), "done");
    }

    #[test]
    fn test_session_state_from_str() {
        assert_eq!(
            "pending".parse::<SessionState>().unwrap(),
            SessionState::Pending
        );
        assert_eq!(
            "waiting_input".parse::<SessionState>().unwrap(),
            SessionState::WaitingInput
        );
        assert!("invalid".parse::<SessionState>().is_err());
    }

    #[test]
    fn test_node_status_display_and_parse() {
        assert_eq!(NodeStatus::Online.to_string(), "online");
        assert_eq!(
            "draining".parse::<NodeStatus>().unwrap(),
            NodeStatus::Draining
        );
        assert!("invalid".parse::<NodeStatus>().is_err());
    }

    #[test]
    fn test_protocol_serialization() {
        let msg = AgentMessage::Heartbeat { active_sessions: 3 };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("heartbeat"));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentMessage::Heartbeat { active_sessions } => assert_eq!(active_sessions, 3),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_server_message_serialization() {
        let msg = ServerMessage::Registered {
            node_id: "node-1".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        match deserialized {
            ServerMessage::Registered { node_id } => assert_eq!(node_id, "node-1"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_github_account_type_display_and_parse() {
        assert_eq!(GitHubAccountType::User.to_string(), "user");
        assert_eq!(GitHubAccountType::Organization.to_string(), "organization");
        assert_eq!(
            "user".parse::<GitHubAccountType>().unwrap(),
            GitHubAccountType::User
        );
        assert_eq!(
            "Organization".parse::<GitHubAccountType>().unwrap(),
            GitHubAccountType::Organization
        );
        assert!("bot".parse::<GitHubAccountType>().is_err());
    }

    #[test]
    fn test_github_account_type_serde() {
        let user = GitHubAccountType::User;
        let json = serde_json::to_string(&user).unwrap();
        assert_eq!(json, "\"user\"");
        let parsed: GitHubAccountType = serde_json::from_str("\"organization\"").unwrap();
        assert_eq!(parsed, GitHubAccountType::Organization);
    }
}
