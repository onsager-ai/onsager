use onsager_events::{EventMetadata, EventStore};

use crate::intercept::{Decision, InterceptEngine, InterceptRequest, InterceptResponse};

/// The policy processor — evaluates tool-use events against the intercept engine
/// and emits extension events back to the stream.
///
/// In Level 1, this is observational only: it logs allow/deny decisions
/// but does not block tool execution.
pub struct PolicyProcessor {
    engine: InterceptEngine,
}

impl PolicyProcessor {
    pub fn new(engine: InterceptEngine) -> Self {
        Self { engine }
    }

    /// Evaluate a tool call and emit the result as an extension event.
    pub async fn evaluate(
        &self,
        store: &EventStore,
        session_id: &str,
        ref_event_id: i64,
        tool_name: &str,
        tool_input: &serde_json::Value,
        metadata: &EventMetadata,
    ) -> anyhow::Result<InterceptResponse> {
        let request = InterceptRequest {
            tool_name: tool_name.to_string(),
            tool_input: tool_input.clone(),
        };

        let response = self.engine.evaluate(&request);

        let ext_event_type = match response.decision {
            Decision::Allow => "allowed",
            Decision::Block => "denied",
        };

        store
            .append_ext(
                session_id,
                "synodic.policy",
                ext_event_type,
                serde_json::to_value(&response)?,
                metadata,
                Some(ref_event_id),
            )
            .await?;

        if response.decision == Decision::Block {
            tracing::warn!(
                session_id = session_id,
                tool = tool_name,
                rule = response.rule.as_deref().unwrap_or("unknown"),
                "policy violation detected (observational)"
            );
        }

        Ok(response)
    }

    /// Get a reference to the intercept engine.
    pub fn engine(&self) -> &InterceptEngine {
        &self.engine
    }
}

/// Implement PolicyEvaluator trait so the executor can use the processor.
#[async_trait::async_trait]
impl onsager_core::executor::PolicyEvaluator for PolicyProcessor {
    async fn evaluate(
        &self,
        store: &EventStore,
        session_id: &str,
        ref_event_id: i64,
        tool_name: &str,
        tool_input: &serde_json::Value,
        metadata: &EventMetadata,
    ) -> anyhow::Result<()> {
        PolicyProcessor::evaluate(
            self,
            store,
            session_id,
            ref_event_id,
            tool_name,
            tool_input,
            metadata,
        )
        .await?;
        Ok(())
    }
}
