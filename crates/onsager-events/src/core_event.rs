use serde::{Deserialize, Serialize};

/// Core events — strictly enumerated, covering the essential lifecycle
/// transitions of tasks, sessions, and nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreEvent {
    // Task lifecycle
    TaskCreated {
        task_id: String,
        prompt: String,
        working_dir: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_turns: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        system_prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        permission_mode: Option<String>,
    },

    // Session lifecycle
    SessionCreated {
        session_id: String,
        task_id: String,
        node_id: String,
    },
    SessionDispatched {
        session_id: String,
    },
    SessionRunning {
        session_id: String,
    },
    SessionWaitingInput {
        session_id: String,
        prompt: String,
    },
    SessionOutput {
        session_id: String,
        chunk: String,
    },
    SessionToolUse {
        session_id: String,
        tool_name: String,
        tool_input: serde_json::Value,
    },
    SessionCompleted {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<String>,
    },
    SessionFailed {
        session_id: String,
        error: String,
    },

    // Node lifecycle
    NodeRegistered {
        node_id: String,
        name: String,
        hostname: String,
    },
    NodeHeartbeat {
        node_id: String,
        active_sessions: u32,
    },
}

impl CoreEvent {
    /// Returns the event type string used for the `event_type` column.
    pub fn event_type(&self) -> &'static str {
        match self {
            CoreEvent::TaskCreated { .. } => "task.created",
            CoreEvent::SessionCreated { .. } => "session.created",
            CoreEvent::SessionDispatched { .. } => "session.dispatched",
            CoreEvent::SessionRunning { .. } => "session.running",
            CoreEvent::SessionWaitingInput { .. } => "session.waiting_input",
            CoreEvent::SessionOutput { .. } => "session.output",
            CoreEvent::SessionToolUse { .. } => "session.tool_use",
            CoreEvent::SessionCompleted { .. } => "session.completed",
            CoreEvent::SessionFailed { .. } => "session.failed",
            CoreEvent::NodeRegistered { .. } => "node.registered",
            CoreEvent::NodeHeartbeat { .. } => "node.heartbeat",
        }
    }

    /// Returns the stream_id for this event (the primary entity it belongs to).
    pub fn stream_id(&self) -> &str {
        match self {
            CoreEvent::TaskCreated { task_id, .. } => task_id,
            CoreEvent::SessionCreated { session_id, .. }
            | CoreEvent::SessionDispatched { session_id, .. }
            | CoreEvent::SessionRunning { session_id, .. }
            | CoreEvent::SessionWaitingInput { session_id, .. }
            | CoreEvent::SessionOutput { session_id, .. }
            | CoreEvent::SessionToolUse { session_id, .. }
            | CoreEvent::SessionCompleted { session_id, .. }
            | CoreEvent::SessionFailed { session_id, .. } => session_id,
            CoreEvent::NodeRegistered { node_id, .. }
            | CoreEvent::NodeHeartbeat { node_id, .. } => node_id,
        }
    }

    /// Returns the stream_type for this event.
    pub fn stream_type(&self) -> &'static str {
        match self {
            CoreEvent::TaskCreated { .. } => "task",
            CoreEvent::SessionCreated { .. }
            | CoreEvent::SessionDispatched { .. }
            | CoreEvent::SessionRunning { .. }
            | CoreEvent::SessionWaitingInput { .. }
            | CoreEvent::SessionOutput { .. }
            | CoreEvent::SessionToolUse { .. }
            | CoreEvent::SessionCompleted { .. }
            | CoreEvent::SessionFailed { .. } => "session",
            CoreEvent::NodeRegistered { .. } | CoreEvent::NodeHeartbeat { .. } => "node",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_event_serialization_roundtrip() {
        let event = CoreEvent::SessionCreated {
            session_id: "s-1".into(),
            task_id: "t-1".into(),
            node_id: "n-1".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "session_created");
        assert_eq!(json["session_id"], "s-1");

        let deserialized: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.event_type(), "session.created");
        assert_eq!(deserialized.stream_id(), "s-1");
        assert_eq!(deserialized.stream_type(), "session");
    }

    #[test]
    fn test_task_created_optional_fields() {
        let event = CoreEvent::TaskCreated {
            task_id: "t-1".into(),
            prompt: "hello".into(),
            working_dir: "/tmp".into(),
            model: None,
            max_turns: None,
            system_prompt: None,
            permission_mode: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("model"));
        assert!(!json.contains("max_turns"));
    }

    #[test]
    fn test_session_tool_use_event() {
        let event = CoreEvent::SessionToolUse {
            session_id: "s-1".into(),
            tool_name: "Bash".into(),
            tool_input: serde_json::json!({"command": "ls -la"}),
        };
        assert_eq!(event.event_type(), "session.tool_use");
        assert_eq!(event.stream_type(), "session");
    }
}
