//! In-session liveness gate — the main-node endpoint a Claude Code
//! **Stop hook** consults before an agent session is allowed to finish
//! (spec #520 §4d / #525, the best-effort half of the two-gate design).
//!
//! The agent execution environment is untrusted and holds no DB
//! credentials. Its `claude` process is spawned with a Stop hook
//! (`crates/stiglab/src/agent/session/process.rs`) pointing here; on
//! every stop attempt the hook POSTs and we evaluate the session
//! manifest:
//!
//! - `emit_count >= 1 || declared_empty` → **allow** (`200`, empty body,
//!   no decision) — the session emitted a deliverable or explicitly
//!   declared it has none.
//! - otherwise → **block** (`200 { "decision": "block", "reason": ... }`).
//!   The reason is fed back to the agent as its next instruction, so it
//!   names the exact tools to call (`emit_artifact` / `declare_no_output`).
//!
//! Claude Code's HTTP hooks fail open on any non-2xx / timeout, which is
//! exactly the desired network-blip behavior: a missing or unauthorized
//! token (the `AuthUser` extractor returns `401` until session-token
//! provisioning lands — a sibling of #520's MCP-config wiring) lets the
//! session continue. The **authoritative** gate inside `AgentExecutor`
//! (spec #520 §4c / #523) re-reads the same manifest after the session
//! ends and does not depend on this hook firing — the two gates compose.
//!
//! Loop bound: consecutive blocks are capped by Claude Code's
//! `CLAUDE_CODE_STOP_HOOK_BLOCK_CAP` (default 8). We deliberately do
//! **not** short-circuit on any `stop_hook_active` flag — the
//! allow/block decision is recomputed from the manifest on every call,
//! so a session that is still empty stays blocked until the cap, never
//! waved through.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::handlers::workspaces::require_workspace_access;
use crate::mcp::tools::session_manifest::{EmitStatus, read_status};
use crate::state::AppState;

/// Query scope for the liveness check. The agent's Stop hook carries
/// Claude Code's own `session_id` in the POST *body*; the identity we
/// need (our stiglab session + its workspace) is baked into the hook
/// **URL** at spawn time, so we read it from the query string and
/// ignore the body.
#[derive(Debug, Deserialize)]
pub struct LivenessQuery {
    /// Workspace that owns the session (token-scope + membership
    /// enforced before any manifest read).
    pub workspace_id: String,
    /// The stiglab session whose manifest gates the stop — the manifest
    /// stream key is `session:<session_id>`.
    pub session_id: String,
}

/// The corrective instruction handed back to a blocked agent. Names the
/// exact tool to emit a deliverable and the explicit escape hatch so the
/// next turn is an unambiguous task, not a vague nudge.
const BLOCK_REASON: &str = "This session has not recorded any output yet. \
Before finishing, call the `emit_artifact` MCP tool to record each deliverable \
you produced (content + kind + summary). If this session genuinely has nothing \
to deliver, call `declare_no_output` with a reason instead. Do one of these, \
then stop.";

/// The liveness decision: a session may finish once it has emitted at
/// least one artifact or explicitly declared it has none. Anything else
/// is blocked so the agent fixes it in-context. Pure so the rule is
/// unit-testable without a DB or auth.
fn should_block(status: &EmitStatus) -> bool {
    status.emit_count == 0 && !status.declared_empty
}

/// POST /api/internal/session-liveness — Stop-hook manifest evaluator.
///
/// Authenticated via the `AuthUser` extractor (PAT bearer), exactly like
/// the MCP tools that write the manifest. A `401`/`403`/`404` is a
/// fail-open signal to the hook (non-2xx → session continues); the
/// authoritative gate (#523) is the real guard.
pub async fn evaluate(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<LivenessQuery>,
) -> Response {
    if let Err(resp) = require_workspace_access(&state.pool, &auth_user, &q.workspace_id).await {
        return resp;
    }

    let status = match read_status(&state.spine, &q.workspace_id, &q.session_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("session-liveness read_status failed: {e}");
            // Fail open: a manifest read error must not trap the agent.
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if should_block(&status) {
        // Block: feed the corrective task back to the agent.
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "decision": "block",
                "reason": BLOCK_REASON,
            })),
        )
            .into_response()
    } else {
        // Allow: empty 2xx body, no decision.
        StatusCode::OK.into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(emit_count: u64, declared_empty: bool) -> EmitStatus {
        EmitStatus {
            emit_count,
            declared_empty,
        }
    }

    #[test]
    fn empty_and_not_declared_blocks() {
        assert!(should_block(&status(0, false)));
    }

    #[test]
    fn an_emit_allows() {
        assert!(!should_block(&status(1, false)));
        assert!(!should_block(&status(3, false)));
    }

    #[test]
    fn declared_empty_allows() {
        assert!(!should_block(&status(0, true)));
    }

    #[test]
    fn block_reason_names_both_tools() {
        // The reason is the agent's next instruction — it must name the
        // tool to emit and the explicit escape, not just nudge.
        assert!(BLOCK_REASON.contains("emit_artifact"));
        assert!(BLOCK_REASON.contains("declare_no_output"));
    }
}
