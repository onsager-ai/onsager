//! MCP diagnostic tools — read paths that AI clients need to navigate
//! failed runs without dead-ending in "something went wrong" (ADR
//! 0007's first-class diagnostic-surface commitment).
//!
//! Four tools:
//!
//! - `inspect_run` — structured snapshot of an artifact's current
//!   state plus recent spine events touching it.
//! - `get_stage_logs` — ordered log chunks from an agent-session
//!   stage's `sessions` row.
//! - `propose_remediation` — server-side AI reasoning over the failed
//!   run's state and logs (#312). Reads the caller's
//!   `ANTHROPIC_API_KEY` workspace credential, calls the Anthropic
//!   Messages API with prompt caching, and returns `proposed_actions`
//!   the client AI can review via HitlCard. Falls back to the v1 stub
//!   envelope when the call short-circuits (no credential, monthly
//!   budget exhausted, model error) so clients can detect the degraded
//!   path instead of dead-ending.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use sqlx::FromRow;

use crate::anthropic::{
    AnthropicClient, CacheControl, MAX_OUTPUT_TOKENS, MessagesRequest, ModelPricing, SystemBlock,
    UserMessage, collect_text, resolve_model,
};
use crate::auth::{AuthUser, decrypt_credential};
use crate::remediation_db::{self, BudgetStatus};
use crate::session_db;
use crate::state::AppState;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

// =============================================================================
// inspect_run
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InspectRunArgs {
    /// Artifact id flowing through the workflow (one artifact == one
    /// run).
    pub artifact_id: String,
    /// Max recent spine events to include. Clamped to `[1, 200]`.
    /// Defaults to 50.
    #[serde(default)]
    pub event_limit: Option<i64>,
}

#[derive(Debug, FromRow, serde::Serialize)]
struct InspectArtifactRow {
    artifact_id: String,
    workspace_id: String,
    kind: String,
    state: String,
    workflow_id: Option<String>,
    current_stage_index: Option<i32>,
    workflow_parked_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow, serde::Serialize)]
struct RecentEventRow {
    id: i64,
    namespace: String,
    event_type: String,
    actor: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn inspect_run(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: InspectRunArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid inspect_run args: {e}")))?;

    let spine = state.spine.pool();
    let row = sqlx::query_as::<_, InspectArtifactRow>(
        "SELECT artifact_id, workspace_id, kind, state, workflow_id, \
                current_stage_index, workflow_parked_reason, created_at, updated_at \
         FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&args.artifact_id)
    .fetch_optional(spine)
    .await
    .map_err(|e| {
        tracing::error!("mcp inspect_run artifact lookup failed: {e}");
        ToolError::Internal(format!("artifact lookup failed: {e}"))
    })?;

    let Some(row) = row else {
        return Err(ToolError::NotFound(format!(
            "artifact `{}` not found",
            args.artifact_id
        )));
    };
    require_workspace_access(&state.pool, auth_user, &row.workspace_id).await?;

    let limit = args.event_limit.unwrap_or(50).clamp(1, 200);
    let events = sqlx::query_as::<_, RecentEventRow>(
        "SELECT id, namespace, event_type, \
                COALESCE(metadata->>'actor', '') AS actor, created_at \
         FROM events_ext WHERE stream_id = $1 \
         ORDER BY id DESC LIMIT $2",
    )
    .bind(&row.artifact_id)
    .bind(limit)
    .fetch_all(spine)
    .await
    .map_err(|e| {
        tracing::error!("mcp inspect_run events query failed: {e}");
        ToolError::Internal(format!("failed to query events: {e}"))
    })?;

    Ok(serde_json::json!({
        "artifact": row,
        "recent_events": events,
    }))
}

// =============================================================================
// get_stage_logs
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetStageLogsArgs {
    /// Session id for the agent-session stage. `inspect_run` surfaces
    /// these via `stiglab.session_*` events on the artifact stream.
    pub session_id: String,
    /// Skip chunks with seq <= `since_seq`. Defaults to 0 (return
    /// everything).
    #[serde(default)]
    pub since_seq: Option<i64>,
}

pub async fn get_stage_logs(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: GetStageLogsArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid get_stage_logs args: {e}")))?;

    // Authorize: walk session → workspace → membership. Sessions
    // without a workspace fall back to owner-equals-caller (legacy
    // personal sessions); MCP requires workspace-scoped sessions to
    // keep the auth check uniform.
    let workspace_id = session_db::get_session_workspace(&state.pool, &args.session_id)
        .await
        .map_err(|e| {
            tracing::error!("mcp get_stage_logs session lookup failed: {e}");
            ToolError::Internal(format!("session lookup failed: {e}"))
        })?;
    let Some(workspace_id) = workspace_id else {
        return Err(ToolError::NotFound(format!(
            "session `{}` not found or not workspace-scoped",
            args.session_id
        )));
    };
    require_workspace_access(&state.pool, auth_user, &workspace_id).await?;

    let session = session_db::get_session(&state.pool, &args.session_id)
        .await
        .map_err(|e| {
            tracing::error!("mcp get_stage_logs get_session failed: {e}");
            ToolError::Internal(format!("session lookup failed: {e}"))
        })?
        .ok_or_else(|| ToolError::NotFound(format!("session `{}` not found", args.session_id)))?;

    let since = args.since_seq.unwrap_or(0);
    let chunks = session_db::get_session_logs_after(&state.pool, &args.session_id, since)
        .await
        .map_err(|e| {
            tracing::error!("mcp get_stage_logs chunk query failed: {e}");
            ToolError::Internal(format!("log chunk query failed: {e}"))
        })?;

    let chunks_json: Vec<Value> = chunks
        .iter()
        .map(|c| {
            serde_json::json!({
                "seq": c.seq,
                "stream": c.stream,
                "chunk": c.chunk,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "session_id": session.id,
        "state": session.state,
        "chunks": chunks_json,
    }))
}

// =============================================================================
// propose_remediation
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeRemediationArgs {
    /// Artifact id for the failed (or stuck) run.
    pub artifact_id: String,
    /// Optional model selector. Accepts the convenience aliases
    /// `"sonnet"` / `"opus"`, or a canonical Anthropic model id like
    /// `"claude-opus-4-7"`. Defaults to Sonnet for cost — Opus is the
    /// "hard cases" escape hatch.
    #[serde(default)]
    pub model: Option<String>,
}

/// Pointer to a session attached to the failed run's artifact stream.
/// Surfaced verbatim in both the AI-success and stub fallback envelopes
/// so clients can chain `get_stage_logs` with no special-casing.
#[derive(FromRow)]
struct SessionPointer {
    session_id: Option<String>,
    event_type: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Server-side remediation tool.
///
/// Reads the failed run's state via `inspect_run`, gathers recent log
/// tails for the artifact's most recent agent sessions, and asks
/// Claude what the next step should be. The response is a
/// `proposed_actions` array — each entry names a registered MCP tool
/// plus concrete arguments — which the client surfaces as `HitlCard`s
/// for the user to commit.
///
/// Short-circuits to the v1 stub envelope when:
///   1. The workspace has no `ANTHROPIC_API_KEY` credential.
///   2. The credential is unreadable (no encryption key configured;
///      decryption fails).
///   3. The workspace's per-month spend cap has been reached.
///   4. The Anthropic call errors.
///
/// In every short-circuit the client sees the same shape
/// (`v1_stub: true`, populated `failure_summary` and `log_pointers`,
/// `suggested_next_tools` to chain), so the chat experience degrades
/// instead of dead-ending.
pub async fn propose_remediation(
    state: &AppState,
    auth_user: &AuthUser,
    args: Value,
) -> ToolResult {
    let args: ProposeRemediationArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid propose_remediation args: {e}")))?;

    // Workspace authz happens inside inspect_run; reuse it for the
    // canonical failure-summary payload and the workspace_id lookup.
    let summary = inspect_run(
        state,
        auth_user,
        serde_json::json!({
            "artifact_id": args.artifact_id,
            "event_limit": 100,
        }),
    )
    .await?;

    let workspace_id = summary
        .get("artifact")
        .and_then(|a| a.get("workspace_id"))
        .and_then(|w| w.as_str())
        .ok_or_else(|| ToolError::Internal("inspect_run did not return a workspace_id".into()))?
        .to_string();

    let spine = state.spine.pool();
    let pointers = sqlx::query_as::<_, SessionPointer>(
        "SELECT data->>'session_id' AS session_id, event_type, created_at \
         FROM events_ext \
         WHERE stream_id = $1 AND namespace = 'stiglab' \
         ORDER BY id DESC LIMIT 20",
    )
    .bind(&args.artifact_id)
    .fetch_all(spine)
    .await
    .map_err(|e| {
        tracing::error!("mcp propose_remediation pointers query failed: {e}");
        ToolError::Internal(format!("failed to query session pointers: {e}"))
    })?;

    let log_pointers: Vec<Value> = pointers
        .iter()
        .filter(|p| p.session_id.is_some())
        .map(|p| {
            serde_json::json!({
                "session_id": p.session_id,
                "event_type": p.event_type,
                "created_at": p.created_at,
            })
        })
        .collect();

    // Decide whether to go to the model. Each guard returns the stub
    // envelope with a specific `stub_reason` so the dashboard can
    // tell the user why their request degraded.
    let api_key = match load_workspace_anthropic_key(state, auth_user, &workspace_id).await {
        Ok(Some(k)) => k,
        Ok(None) => {
            return Ok(stub_envelope(
                &summary,
                &log_pointers,
                "no ANTHROPIC_API_KEY credential set for this workspace",
            ));
        }
        Err(reason) => {
            return Ok(stub_envelope(&summary, &log_pointers, &reason));
        }
    };

    let cap_usd = state.config.remediation_monthly_cap_usd;
    match remediation_db::check_budget(&state.pool, &workspace_id, cap_usd).await {
        Ok(BudgetStatus::OverCap { spent_usd, cap_usd }) => {
            return Ok(stub_envelope(
                &summary,
                &log_pointers,
                &format!(
                    "workspace monthly remediation budget exceeded ({:.2} USD / {:.2} USD cap)",
                    spent_usd, cap_usd
                ),
            ));
        }
        Ok(BudgetStatus::Ok { .. }) => {}
        Err(e) => {
            tracing::warn!("propose_remediation budget check failed: {e}");
            // Fail open — budget query failures shouldn't deny a
            // tool call. The next call after the next successful
            // ledger insert will re-check.
        }
    }

    // Pull log tails for the most recent agent sessions. Keep this
    // bounded — the prompt is the cost driver.
    let recent_logs = collect_recent_log_tails(state, &pointers).await;

    let model = resolve_model(args.model.as_deref()).to_string();
    let user_prompt = build_user_prompt(&args.artifact_id, &summary, &log_pointers, &recent_logs);

    let client = match AnthropicClient::new(api_key) {
        Ok(c) => c,
        Err(e) => {
            return Ok(stub_envelope(
                &summary,
                &log_pointers,
                &format!("anthropic client init failed: {e}"),
            ));
        }
    };

    let messages_req = MessagesRequest {
        model: &model,
        max_tokens: MAX_OUTPUT_TOKENS,
        system: vec![SystemBlock {
            kind: "text",
            text: REMEDIATION_SYSTEM_PROMPT,
            cache_control: Some(CacheControl::EPHEMERAL),
        }],
        messages: vec![UserMessage {
            role: "user",
            content: &user_prompt,
        }],
    };

    let response = match client.messages(&messages_req).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("propose_remediation anthropic call failed: {e}");
            return Ok(stub_envelope(
                &summary,
                &log_pointers,
                &format!("anthropic call failed: {e}"),
            ));
        }
    };

    let pricing = ModelPricing::for_model(&model);
    let cost_usd = pricing.estimate_usd(&response.usage);
    if let Err(e) = remediation_db::record_call(
        &state.pool,
        &workspace_id,
        &auth_user.user_id,
        &args.artifact_id,
        &model,
        &response.usage,
        cost_usd,
    )
    .await
    {
        // Ledger failure is logged but doesn't block the response —
        // the AI call already happened and the user is waiting on it.
        tracing::warn!("propose_remediation ledger insert failed: {e}");
    }

    let raw_text = collect_text(&response.content);
    let (proposed_actions, ai_rationale) = parse_remediation_response(&raw_text);

    Ok(serde_json::json!({
        "v1_stub": false,
        "failure_summary": summary,
        "log_pointers": log_pointers,
        "proposed_actions": proposed_actions,
        "rationale": ai_rationale,
        "model": model,
        "usage": {
            "input_tokens": response.usage.input_tokens,
            "output_tokens": response.usage.output_tokens,
            "cache_creation_input_tokens": response.usage.cache_creation_input_tokens,
            "cache_read_input_tokens": response.usage.cache_read_input_tokens,
            "estimated_cost_usd": cost_usd,
        },
    }))
}

/// System prompt — workspace-invariant, prompt-cached. Carries the
/// tool registry summary and the response-shape contract. Any change
/// here invalidates the cache for every workspace; keep it stable.
const REMEDIATION_SYSTEM_PROMPT: &str = r#"You are an operations agent for Onsager, an AI factory that drives software-engineering artifacts (GitHub issues, pull requests, etc.) through workflows. A "run" is one workflow execution against an artifact; runs can park on a stage failure, an agent-session error, or a gate verdict.

You will receive a JSON blob describing a failed or stuck run: its artifact state, the most recent spine events, structured pointers to agent sessions, and trailing log excerpts. Your job is to recommend concrete next actions the operator can review and commit.

Available MCP tools the operator can invoke. Recommend by tool name plus concrete arguments:
- get_stage_logs(session_id, since_seq?) — fetch more session log chunks when the failure cause is not yet clear.
- inspect_run(artifact_id, event_limit?) — re-read the run's state, useful after another action runs.
- get_artifact(artifact_id) — read a specific artifact's metadata.
- list_runs(workflow_id, limit?) — list recent runs of a workflow to spot patterns.
- list_workflows(workspace_id) — list workflows when the failure looks like a workflow-shape problem.
- cancel_run(artifact_id) — archive the artifact; irreversible. Use only when the run cannot be recovered and the operator should abandon it.
- propose_workflow(...) / edit_workflow(...) / schedule_workflow(...) — mutate workflow definitions. Use only when the failure is rooted in the workflow definition itself, not a transient session error.
- run_workflow(workflow_id, trigger_name) — fire a manual trigger to retry, when the failure looked transient and the workflow has a manual trigger.

Respond with a single JSON object, no surrounding markdown, no prose outside the JSON. Shape:
{
  "rationale": "1-3 sentences naming the failure cause as you read it",
  "proposed_actions": [
    {
      "tool": "<one of the tool names above>",
      "arguments": { <concrete JSON args matching that tool> },
      "reason": "1 sentence — why this action, what it tells us"
    },
    ...
  ]
}

Rules:
- Prefer diagnostic actions (get_stage_logs, inspect_run) before destructive ones (cancel_run).
- Never invent tool names or arguments not listed above.
- Two or three actions is typical; one is fine if the cause is obvious; never more than five.
- If you cannot tell what happened, propose a get_stage_logs call against the most recent session.
- If the run is clearly unrecoverable (agent crashed deterministically; the underlying GitHub artifact is gone), propose cancel_run as the sole action.
- Output ONLY the JSON object. No code fences, no commentary."#;

/// Build the per-call user prompt — failure summary + log tails. Not
/// cached; this is the bit that varies per artifact.
fn build_user_prompt(
    artifact_id: &str,
    summary: &Value,
    log_pointers: &[Value],
    recent_logs: &[(String, String)],
) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str("Failed run report:\n\n");
    out.push_str(&format!("artifact_id: {}\n\n", artifact_id));
    out.push_str("=== Artifact + recent spine events (from inspect_run) ===\n");
    out.push_str(
        &serde_json::to_string_pretty(summary)
            .unwrap_or_else(|_| "<summary serialization failed>".into()),
    );
    out.push_str("\n\n=== Session pointers ===\n");
    out.push_str(&serde_json::to_string_pretty(log_pointers).unwrap_or_else(|_| "[]".into()));
    if recent_logs.is_empty() {
        out.push_str("\n\n=== Session log tails ===\n(none available)\n");
    } else {
        out.push_str("\n\n=== Session log tails (most recent first) ===\n");
        for (session_id, tail) in recent_logs {
            out.push_str(&format!("--- session {session_id} (tail) ---\n"));
            out.push_str(tail);
            out.push_str("\n\n");
        }
    }
    out.push_str("\nReturn the JSON object as instructed.");
    out
}

/// Pull the most recent log tails for up to three distinct sessions
/// surfaced in the artifact's `stiglab.*` events. Each tail is capped
/// to ~1500 characters to keep the prompt bounded; we keep the last
/// N chars rather than the first, because failure messages live at
/// the end of the log.
async fn collect_recent_log_tails(
    state: &AppState,
    pointers: &[SessionPointer],
) -> Vec<(String, String)> {
    const MAX_SESSIONS: usize = 3;
    const MAX_TAIL_CHARS: usize = 1500;

    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for p in pointers {
        if out.len() >= MAX_SESSIONS {
            break;
        }
        let Some(sid) = p.session_id.as_deref() else {
            continue;
        };
        if !seen.insert(sid.to_string()) {
            continue;
        }
        let chunks = match session_db::get_session_logs_after(&state.pool, sid, 0).await {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(session_id = sid, "log tail fetch failed: {e}");
                continue;
            }
        };
        if chunks.is_empty() {
            continue;
        }
        let mut joined = String::new();
        for c in &chunks {
            joined.push_str(&c.chunk);
        }
        let tail = if joined.len() > MAX_TAIL_CHARS {
            joined
                .char_indices()
                .rev()
                .nth(MAX_TAIL_CHARS)
                .map(|(idx, _)| &joined[idx..])
                .unwrap_or(&joined)
                .to_string()
        } else {
            joined
        };
        out.push((sid.to_string(), tail));
    }
    out
}

/// Resolve the caller's `ANTHROPIC_API_KEY` for `workspace_id`.
/// Returns `Ok(None)` when the user has no such credential (clean
/// stub fallback); `Err(reason)` when the credential exists but
/// can't be decrypted (something is wrong server-side and we should
/// surface a different stub_reason).
async fn load_workspace_anthropic_key(
    state: &AppState,
    auth_user: &AuthUser,
    workspace_id: &str,
) -> Result<Option<String>, String> {
    #[derive(FromRow)]
    struct EncRow {
        encrypted_value: String,
    }
    let row = sqlx::query_as::<_, EncRow>(
        "SELECT encrypted_value FROM user_credentials \
         WHERE workspace_id = $1 AND user_id = $2 AND name = 'ANTHROPIC_API_KEY'",
    )
    .bind(workspace_id)
    .bind(&auth_user.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| format!("credential lookup failed: {e}"))?;

    let Some(row) = row else { return Ok(None) };
    let Some(key) = state.config.credential_key.as_deref() else {
        return Err("portal credential_key is not configured; cannot decrypt".into());
    };
    decrypt_credential(key, &row.encrypted_value)
        .map(Some)
        .map_err(|e| format!("ANTHROPIC_API_KEY decryption failed: {e}"))
}

/// Parse the model's response. The system prompt asks for a JSON
/// object with `rationale` + `proposed_actions`; we accept either
/// that shape directly or any JSON object found inside the response.
/// On parse failure we surface the raw text as rationale and an
/// empty actions list — clients render the rationale and the user
/// can decide.
fn parse_remediation_response(raw: &str) -> (Vec<Value>, String) {
    let trimmed = raw.trim();
    // Strip an accidental markdown code fence if the model emitted one.
    let stripped = if let Some(rest) = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
    {
        rest.trim_start_matches('\n')
            .strip_suffix("```")
            .unwrap_or(rest)
            .trim()
    } else {
        trimmed
    };

    match serde_json::from_str::<Value>(stripped) {
        Ok(v) => {
            let actions = v
                .get("proposed_actions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let rationale = v
                .get("rationale")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (actions, rationale)
        }
        Err(_) => (Vec::new(), trimmed.to_string()),
    }
}

/// Stub envelope shared by every short-circuit path. Mirrors the v1
/// shape so existing clients keep working — only the `stub_reason`
/// string changes.
fn stub_envelope(summary: &Value, log_pointers: &[Value], reason: &str) -> Value {
    serde_json::json!({
        "v1_stub": true,
        "stub_reason": reason,
        "failure_summary": summary,
        "log_pointers": log_pointers,
        "suggested_next_tools": [
            "get_stage_logs",
            "get_artifact",
            "inspect_run",
        ],
    })
}

#[cfg(test)]
mod propose_remediation_tests {
    use super::*;

    #[test]
    fn parse_handles_clean_json_object() {
        let raw = r#"{"rationale": "cause", "proposed_actions": [{"tool":"inspect_run","arguments":{"artifact_id":"a"},"reason":"r"}]}"#;
        let (actions, rat) = parse_remediation_response(raw);
        assert_eq!(rat, "cause");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["tool"], "inspect_run");
    }

    #[test]
    fn parse_strips_markdown_fence() {
        let raw = "```json\n{\"rationale\": \"x\", \"proposed_actions\": []}\n```";
        let (actions, rat) = parse_remediation_response(raw);
        assert_eq!(rat, "x");
        assert!(actions.is_empty());
    }

    #[test]
    fn parse_falls_back_to_rationale_on_garbage() {
        let raw = "I think you should restart everything";
        let (actions, rat) = parse_remediation_response(raw);
        assert!(actions.is_empty());
        assert!(rat.contains("restart"));
    }

    #[test]
    fn stub_envelope_carries_reason_and_pointers() {
        let summary = serde_json::json!({"artifact": {"workspace_id":"w1"}});
        let pointers = vec![serde_json::json!({"session_id":"s1"})];
        let env = stub_envelope(&summary, &pointers, "no key");
        assert_eq!(env["v1_stub"], true);
        assert_eq!(env["stub_reason"], "no key");
        assert_eq!(env["failure_summary"]["artifact"]["workspace_id"], "w1");
        assert_eq!(env["log_pointers"][0]["session_id"], "s1");
        // Stub keeps the legacy suggested_next_tools so degraded
        // clients can still chain.
        assert!(env["suggested_next_tools"].is_array());
    }

    #[test]
    fn build_user_prompt_includes_required_sections() {
        let summary = serde_json::json!({"artifact": {"state": "parked"}});
        let pointers = vec![serde_json::json!({"session_id":"s1"})];
        let logs = vec![("s1".to_string(), "boom".to_string())];
        let p = build_user_prompt("a-1", &summary, &pointers, &logs);
        assert!(p.contains("artifact_id: a-1"));
        assert!(p.contains("=== Artifact"));
        assert!(p.contains("=== Session pointers"));
        assert!(p.contains("=== Session log tails"));
        assert!(p.contains("session s1"));
        assert!(p.contains("boom"));
    }

    #[test]
    fn build_user_prompt_marks_empty_logs() {
        let summary = serde_json::json!({});
        let pointers: Vec<Value> = vec![];
        let logs: Vec<(String, String)> = vec![];
        let p = build_user_prompt("a-1", &summary, &pointers, &logs);
        assert!(p.contains("(none available)"));
    }
}
