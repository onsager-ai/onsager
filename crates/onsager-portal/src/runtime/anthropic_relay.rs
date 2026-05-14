//! Anthropic relay as a [`ChatRuntime`] (spec #337, Phase 1).
//!
//! Wraps the hand-rolled [`crate::anthropic::AnthropicClient`] so the
//! dashboard chat handler talks to the runtime trait rather than a
//! concrete provider client. Today's only `ChatRuntime` implementation;
//! the ACP-backed `AcpRuntime` (lazy detection of the user's local
//! Claude Code CLI) lands as a follow-up.

use anyhow::Context;

use super::{
    AuthSource, ChatResponse, ChatRuntime, ChatRuntimeError, CostModel, RuntimeProvenance,
    TosPosture,
};
use crate::anthropic::{AnthropicClient, AnthropicUpstreamError};

/// `ChatRuntime` backed by a portal-held Anthropic API key. The key
/// is workspace-scoped — resolved per-call by the chat handler from
/// the workspace's `anthropic` credential. The relay itself is
/// stateless apart from the `reqwest::Client` housed inside
/// `AnthropicClient`.
pub struct AnthropicRelay {
    inner: AnthropicClient,
}

impl AnthropicRelay {
    pub fn new(api_key: String) -> anyhow::Result<Self> {
        let inner = AnthropicClient::new(api_key).context("build Anthropic relay")?;
        Ok(Self { inner })
    }
}

#[async_trait::async_trait]
impl ChatRuntime for AnthropicRelay {
    async fn chat(&self, request: &serde_json::Value) -> Result<ChatResponse, ChatRuntimeError> {
        match self.inner.forward(request).await {
            Ok(v) => Ok(v),
            Err(e) => match e.downcast::<AnthropicUpstreamError>() {
                Ok(upstream) => Err(ChatRuntimeError::Upstream {
                    status: upstream.status,
                    body: upstream.body,
                }),
                Err(other) => Err(ChatRuntimeError::Transport(other)),
            },
        }
    }

    fn provenance(&self) -> RuntimeProvenance {
        RuntimeProvenance {
            runtime_id: "anthropic_relay",
            auth_source: AuthSource::PortalHeldApiKey,
            cost_model: CostModel::PerToken,
            tos_posture: TosPosture::CleanCommercial,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_is_clean_commercial() {
        // Doesn't make a network call — `AnthropicClient::new` just
        // builds the reqwest client. The provenance shape is what
        // the Synodic admissibility gate (Phase 3) consumes.
        let relay = AnthropicRelay::new("test-key".into()).unwrap();
        let p = relay.provenance();
        assert_eq!(p.runtime_id, "anthropic_relay");
        assert_eq!(p.auth_source, AuthSource::PortalHeldApiKey);
        assert_eq!(p.cost_model, CostModel::PerToken);
        assert_eq!(p.tos_posture, TosPosture::CleanCommercial);
    }
}
