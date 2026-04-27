//! Adapter between Onsager protocol types and Stiglab domain types.

use chrono::Utc;
use uuid::Uuid;

use crate::core::session::{Session, SessionState};
use crate::core::task::Task;

/// Convert an Onsager `ShapingRequest` into a Stiglab `Task`.
pub fn shaping_request_to_task(req: &onsager_spine::protocol::ShapingRequest) -> Task {
    let prompt = req
        .shaping_intent
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let working_dir = req
        .shaping_intent
        .get("working_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let model = find_constraint_str(&req.constraints, "model");
    let max_turns = find_constraint_u32(&req.constraints, "max_turns");
    let permission_mode = find_constraint_str(&req.constraints, "permission_mode");

    Task {
        id: req.request_id.clone(),
        prompt,
        node_id: None,
        working_dir,
        allowed_tools: None,
        max_turns,
        model,
        system_prompt: None,
        permission_mode,
        created_at: Utc::now(),
    }
}

/// Convert a completed `Session` into an Onsager `ShapingResult`.
pub fn session_to_shaping_result(
    req: &onsager_spine::protocol::ShapingRequest,
    session: &Session,
    duration_ms: u64,
) -> onsager_spine::protocol::ShapingResult {
    let outcome = match session.state {
        SessionState::Done => onsager_spine::factory_event::ShapingOutcome::Completed,
        SessionState::Failed => onsager_spine::factory_event::ShapingOutcome::Failed,
        _ => onsager_spine::factory_event::ShapingOutcome::Partial,
    };

    let content_ref = session
        .working_dir
        .as_ref()
        .map(|dir| onsager_artifact::ContentRef {
            uri: format!("file://{dir}"),
            checksum: None,
        });

    let change_summary = session.output.clone().unwrap_or_default();

    let error = if session.state == SessionState::Failed {
        Some(onsager_spine::protocol::ErrorDetail {
            code: "session_failed".to_string(),
            message: change_summary.clone(),
            retriable: Some(true),
        })
    } else {
        None
    };

    onsager_spine::protocol::ShapingResult {
        request_id: req.request_id.clone(),
        outcome,
        content_ref,
        change_summary,
        quality_signals: vec![],
        session_id: session.id.clone(),
        duration_ms,
        error,
    }
}

/// Create a new `Session` from a `Task` and a target node.
pub fn task_to_session(task: &Task, node_id: &str) -> Session {
    Session {
        id: Uuid::new_v4().to_string(),
        task_id: task.id.clone(),
        node_id: node_id.to_string(),
        state: SessionState::Pending,
        prompt: task.prompt.clone(),
        output: None,
        working_dir: task.working_dir.clone(),
        artifact_id: None,
        artifact_version: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn find_constraint_str(
    constraints: &[onsager_spine::protocol::Constraint],
    name: &str,
) -> Option<String> {
    constraints
        .iter()
        .find(|c| c.constraint_type == name)
        .and_then(|c| c.value.as_str())
        .map(|s| s.to_string())
}

fn find_constraint_u32(
    constraints: &[onsager_spine::protocol::Constraint],
    name: &str,
) -> Option<u32> {
    constraints
        .iter()
        .find(|c| c.constraint_type == name)
        .and_then(|c| c.value.as_u64())
        .map(|v| v as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> onsager_spine::protocol::ShapingRequest {
        onsager_spine::protocol::ShapingRequest {
            request_id: "req_001".to_string(),
            artifact_id: onsager_artifact::ArtifactId::new("art_test1234"),
            target_version: 1,
            shaping_intent: serde_json::json!({
                "prompt": "Fix the login bug",
                "working_dir": "/home/user/project"
            }),
            inputs: vec![],
            constraints: vec![
                onsager_spine::protocol::Constraint {
                    constraint_type: "model".to_string(),
                    value: serde_json::json!("claude-sonnet-4-20250514"),
                },
                onsager_spine::protocol::Constraint {
                    constraint_type: "max_turns".to_string(),
                    value: serde_json::json!(10),
                },
                onsager_spine::protocol::Constraint {
                    constraint_type: "permission_mode".to_string(),
                    value: serde_json::json!("auto"),
                },
            ],
            deadline: None,
        }
    }

    #[test]
    fn test_shaping_request_to_task() {
        let req = sample_request();
        let task = shaping_request_to_task(&req);

        assert_eq!(task.id, "req_001");
        assert_eq!(task.prompt, "Fix the login bug");
        assert_eq!(task.working_dir.as_deref(), Some("/home/user/project"));
        assert_eq!(task.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(task.max_turns, Some(10));
        assert_eq!(task.permission_mode.as_deref(), Some("auto"));
        assert!(task.node_id.is_none());
    }

    #[test]
    fn test_shaping_request_to_task_minimal() {
        let req = onsager_spine::protocol::ShapingRequest {
            request_id: "req_002".to_string(),
            artifact_id: onsager_artifact::ArtifactId::new("art_abcd1234"),
            target_version: 1,
            shaping_intent: serde_json::json!({}),
            inputs: vec![],
            constraints: vec![],
            deadline: None,
        };
        let task = shaping_request_to_task(&req);

        assert_eq!(task.id, "req_002");
        assert_eq!(task.prompt, "");
        assert!(task.working_dir.is_none());
        assert!(task.model.is_none());
        assert!(task.max_turns.is_none());
    }

    #[test]
    fn test_session_to_shaping_result_done() {
        let req = sample_request();
        let session = Session {
            id: "sess_001".to_string(),
            task_id: "req_001".to_string(),
            node_id: "node_1".to_string(),
            state: SessionState::Done,
            prompt: "Fix the login bug".to_string(),
            output: Some("Fixed the authentication flow".to_string()),
            working_dir: Some("/home/user/project".to_string()),
            artifact_id: None,
            artifact_version: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let result = session_to_shaping_result(&req, &session, 5000);

        assert_eq!(result.request_id, "req_001");
        assert_eq!(
            result.outcome,
            onsager_spine::factory_event::ShapingOutcome::Completed
        );
        assert_eq!(result.change_summary, "Fixed the authentication flow");
        assert_eq!(result.session_id, "sess_001");
        assert_eq!(result.duration_ms, 5000);
        assert!(result.content_ref.is_some());
        assert!(result.error.is_none());
        assert!(result.quality_signals.is_empty());
    }

    #[test]
    fn test_task_to_session() {
        let task = Task {
            id: "task_001".to_string(),
            prompt: "Implement feature X".to_string(),
            node_id: None,
            working_dir: Some("/home/user/project".to_string()),
            allowed_tools: None,
            max_turns: Some(10),
            model: Some("claude-sonnet-4-20250514".to_string()),
            system_prompt: None,
            permission_mode: None,
            created_at: Utc::now(),
        };

        let session = task_to_session(&task, "node_42");

        assert_eq!(session.task_id, "task_001");
        assert_eq!(session.node_id, "node_42");
        assert_eq!(session.state, SessionState::Pending);
        assert_eq!(session.prompt, "Implement feature X");
        assert_eq!(session.working_dir.as_deref(), Some("/home/user/project"));
        assert!(session.output.is_none());
        // Session ID should be a valid UUID
        assert!(uuid::Uuid::parse_str(&session.id).is_ok());
    }

    #[test]
    fn test_session_to_shaping_result_partial() {
        let req = sample_request();
        let session = Session {
            id: "sess_003".to_string(),
            task_id: "req_001".to_string(),
            node_id: "node_1".to_string(),
            state: SessionState::Running,
            prompt: "Fix the login bug".to_string(),
            output: Some("Partial progress".to_string()),
            working_dir: None,
            artifact_id: None,
            artifact_version: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let result = session_to_shaping_result(&req, &session, 10000);

        assert_eq!(
            result.outcome,
            onsager_spine::factory_event::ShapingOutcome::Partial
        );
        assert!(result.error.is_none());
    }

    #[test]
    fn test_session_to_shaping_result_failed() {
        let req = sample_request();
        let session = Session {
            id: "sess_002".to_string(),
            task_id: "req_001".to_string(),
            node_id: "node_1".to_string(),
            state: SessionState::Failed,
            prompt: "Fix the login bug".to_string(),
            output: Some("Agent timed out".to_string()),
            working_dir: None,
            artifact_id: None,
            artifact_version: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let result = session_to_shaping_result(&req, &session, 30000);

        assert_eq!(
            result.outcome,
            onsager_spine::factory_event::ShapingOutcome::Failed
        );
        assert!(result.content_ref.is_none());
        assert!(result.error.is_some());
        let err = result.error.unwrap();
        assert_eq!(err.code, "session_failed");
    }
}
