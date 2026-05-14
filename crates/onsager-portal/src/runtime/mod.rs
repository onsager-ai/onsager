//! Runtime abstraction (spec #337, Phase 1 substrate).
//!
//! A workflow surface (chat) and a workflow stage (gate execution)
//! each have a *runtime*. Both were implicit and Claude-coupled
//! before #337. This module names the abstraction and the
//! ToS-aware provenance metadata that the Synodic admissibility
//! gate (Phase 3) will read.
//!
//! ## Two traits, two surfaces
//!
//! - [`ChatRuntime`] — authoring surfaces (dashboard chat, future
//!   MCP-host clients). The dashboard relay refactored into
//!   [`AnthropicRelay`] is the first implementation; an
//!   `AcpRuntime` follows when the dashboard adds streaming
//!   support against a local Claude Code CLI session.
//! - [`HarnessRuntime`] — stage gate execution. Concrete ACP
//!   wrappers (`ClaudeCodeAcp`, `CodexAcp`, `CopilotAcp`) and the
//!   ToS-aware Synodic admissibility gate are Phase 3 work; this
//!   trait is declared here so the substrate is in place when
//!   those implementations land.
//!
//! ## Provenance
//!
//! Every runtime carries a [`RuntimeProvenance`] tag describing
//! its auth source (portal-held API key vs end-user CLI auth) and
//! its ToS posture (clean-commercial vs personal-use-only). The
//! Synodic admissibility gate consumes this metadata to decide
//! which environments a runtime may execute against — personal-use
//! runtimes are admitted only in `dev` / `personal` workspaces.

use std::fmt;

pub mod anthropic_relay;

pub use anthropic_relay::AnthropicRelay;

// ── Provenance ─────────────────────────────────────────────────────────

/// Auth source for a runtime. Determines who pays the upstream bill
/// and what ToS clause governs the runtime's use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSource {
    /// Portal holds a workspace-scoped API key issued under an org
    /// account. Clean commercial posture — the workspace pays per
    /// token usage to the upstream provider.
    PortalHeldApiKey,
    /// The end user's local CLI is the auth holder. Onsager never
    /// sees a token; the runtime drives the user's CLI process.
    /// Personal-use ToS — admissible only in personal / dev
    /// workspaces (enforced by the Synodic admissibility gate, see
    /// Phase 3).
    UserCliSubscription,
    /// The end user supplied a workspace credential (e.g.
    /// `CLAUDE_CODE_OAUTH_TOKEN`) that the runtime passes through
    /// to a forked harness binary. Personal-use ToS — same
    /// admissibility posture as `UserCliSubscription`.
    UserSuppliedSubscriptionToken,
}

/// Cost model classification. Cost-class indicators in the dashboard
/// read from this (see #337 "Open questions: Runtime cost surfacing").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostModel {
    /// Per-token / per-call billing against an upstream provider.
    PerToken,
    /// Flat subscription billing — the user's existing CLI plan
    /// (Claude Max, ChatGPT Pro, Copilot Pro).
    FlatSubscription,
}

/// ToS posture for a runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TosPosture {
    /// Org / commercial account — admissible in any workspace.
    CleanCommercial,
    /// Personal-use only per the provider's terms. Admissible only
    /// in personal / dev workspaces.
    PersonalUseOnly,
}

/// All-up provenance for a runtime instance. Returned by both
/// [`ChatRuntime::provenance`] and [`HarnessRuntime::provenance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeProvenance {
    /// Short stable identifier for telemetry / cost ledger
    /// attribution (e.g. `"anthropic_relay"`, `"claude_code_acp"`,
    /// `"codex_acp"`, `"copilot_acp"`).
    pub runtime_id: &'static str,
    pub auth_source: AuthSource,
    pub cost_model: CostModel,
    pub tos_posture: TosPosture,
}

impl fmt::Display for RuntimeProvenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.runtime_id)
    }
}

// ── ChatRuntime ────────────────────────────────────────────────────────

/// Result of a non-streaming chat call — the upstream provider's
/// raw JSON response. The current relay uses this shape because the
/// dashboard is non-streaming; an `AcpRuntime` adding streaming
/// support will extend the trait additively.
pub type ChatResponse = serde_json::Value;

/// Error returned by a chat runtime. Carries enough structure for
/// the HTTP handler to map upstream 4xx through verbatim and
/// upstream 5xx to a 502, the same shape the relay produced before
/// the trait refactor.
#[derive(Debug)]
pub enum ChatRuntimeError {
    /// Upstream provider returned a non-2xx HTTP status. The body
    /// is the parsed JSON when available, `{}` otherwise.
    Upstream {
        status: u16,
        body: serde_json::Value,
    },
    /// Transport error — connection refused, timeout, JSON decode
    /// failure on a 2xx response, etc.
    Transport(anyhow::Error),
}

impl fmt::Display for ChatRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChatRuntimeError::Upstream { status, .. } => {
                write!(f, "chat runtime upstream returned {status}")
            }
            ChatRuntimeError::Transport(e) => write!(f, "chat runtime transport error: {e}"),
        }
    }
}

impl std::error::Error for ChatRuntimeError {}

/// Authoring-surface runtime. The dashboard chat relay and any
/// future MCP-host client that connects through portal implement
/// this trait. The shape is intentionally narrow for v1: forward
/// a fully-formed Anthropic Messages API request, return the
/// upstream JSON response. Streaming and a typed Message/Tool
/// vocabulary are additive extensions for the ACP path.
#[async_trait::async_trait]
pub trait ChatRuntime: Send + Sync {
    /// Forward a fully-formed request payload. The runtime is
    /// responsible for auth, retries, and provider-specific
    /// transport (HTTP for the relay, ACP newline-JSON for the
    /// CLI-backed variants).
    async fn chat(&self, request: &serde_json::Value) -> Result<ChatResponse, ChatRuntimeError>;

    fn provenance(&self) -> RuntimeProvenance;
}

// ── HarnessRuntime ─────────────────────────────────────────────────────

/// Per-stage execution context handed to a [`HarnessRuntime`].
/// The concrete shape will be filled out alongside the
/// `ClaudeCodeAcp` / `CodexAcp` / `CopilotAcp` implementations
/// (Phase 3 of #337). It exists today as a forward declaration so
/// the trait can compile.
#[derive(Debug, Clone)]
pub struct StageContext {
    pub workflow_id: String,
    pub workflow_version_id: String,
    pub stage_index: u32,
    pub workspace_id: String,
}

/// Outcome of a stage harness invocation. Concrete success /
/// failure shape lives with the Phase 3 implementations; this
/// substrate keeps the trait callable without committing to a
/// design that will need to change once real harnesses land.
#[derive(Debug, Clone)]
pub enum HarnessOutcome {
    Completed { detail: serde_json::Value },
    Failed { reason: String },
}

/// Capability declared by a harness — used by the per-stage
/// `harness` binding (Phase 3) to validate that a stage's
/// requested runtime is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessCapability {
    pub kind: HarnessKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessKind {
    /// Native ACP newline-JSON over a local CLI subprocess.
    Acp,
    /// HTTP API key path. Not implemented in this monorepo; the
    /// variant is here so future code can pattern-match on the
    /// closed enum without churn.
    ApiKey,
}

/// Stage gate execution runtime. ACP-backed implementations
/// (`ClaudeCodeAcp`, `CodexAcp`, `CopilotAcp`) and the
/// commercial-grade `ApiKey` impl are Phase 3 / follow-up work;
/// this trait declares the abstraction so the substrate is in
/// place.
#[async_trait::async_trait]
pub trait HarnessRuntime: Send + Sync {
    async fn run_stage(
        &self,
        stage: &StageContext,
        intent: serde_json::Value,
    ) -> anyhow::Result<HarnessOutcome>;

    fn capability(&self) -> HarnessCapability;

    fn provenance(&self) -> RuntimeProvenance;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_display_uses_runtime_id() {
        let p = RuntimeProvenance {
            runtime_id: "anthropic_relay",
            auth_source: AuthSource::PortalHeldApiKey,
            cost_model: CostModel::PerToken,
            tos_posture: TosPosture::CleanCommercial,
        };
        assert_eq!(format!("{p}"), "anthropic_relay");
    }
}
