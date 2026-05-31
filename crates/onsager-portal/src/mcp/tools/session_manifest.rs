//! MCP tools for the agent session output manifest (spec #520 §4a /
//! #521 — the addressable-output foundation).
//!
//! An agent session runs in an untrusted execution environment that
//! holds **no DB credentials** — these three MCP tools are its only
//! path to record (and inspect) what it produced. The credential-
//! holding portal MCP server is the only thing that touches the
//! database, exactly like every other tool in this directory.
//!
//! The manifest wire contract — the `events_ext` namespace / stream /
//! event-type / data-field conventions, the parser, and the store-level
//! append/read helpers — is the **single source of truth** in
//! [`onsager_spine::session_manifest`]. It lives there (a crate both
//! sides of the seam already depend on) because the substrate scheduler
//! reads the same manifest from the other side of the seam to run the
//! authoritative liveness gate (spec #520 §4c). These tools own only the
//! MCP surface on top of it: argument schemas, workspace-scoped auth,
//! and the inline content-storage path the agent's `emit_artifact` runs
//! before recording a row.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ring::digest;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::auth::AuthUser;
use crate::state::AppState;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

// Re-export the shared store-level helpers + types so existing callers
// (the integration test, the tool wrappers below) keep one import path.
// The contract itself is owned by `onsager-spine`.
pub use onsager_spine::session_manifest::{
    EmitStatus, append_declared_empty, append_emit, read_status,
};

/// Inline content-storage URI scheme. First cut matches the Script
/// executor's convention (`crates/onsager-nodes/src/script.rs`):
/// base64-encoded body behind an `inline:` prefix. A real warehouse-
/// backed scheme replaces this once content storage is formalized
/// (RUN-01 #359 — the same gap the Script executor flags).
const INLINE_URI_PREFIX: &str = "inline:base64,";

// ---------------------------------------------------------------------------
// emit_artifact
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmitArtifactArgs {
    /// Workspace that owns this session. The caller's MCP token scope
    /// is enforced against this value before anything is written.
    pub workspace_id: String,
    /// The session emitting the output — the manifest stream key
    /// (`session:<session_id>`).
    pub session_id: String,
    /// Raw deliverable content. Persisted to addressable storage; the
    /// returned `content_ref` points back at it.
    pub content: String,
    /// Artifact kind tag (`code` / `document` / `pull_request` /
    /// `github_issue` / a custom string). Stored verbatim.
    pub kind: String,
    /// Human-readable summary of what was emitted.
    pub summary: String,
}

/// Persist `content` to addressable storage and return its
/// `ContentRef` shape (`{ uri, checksum }`). First cut: inline base64
/// URI + sha256 checksum, matching the Script executor path.
fn store_content(content: &str) -> Value {
    let bytes = content.as_bytes();
    let uri = format!("{INLINE_URI_PREFIX}{}", BASE64.encode(bytes));
    let checksum = hex::encode(digest::digest(&digest::SHA256, bytes).as_ref());
    serde_json::json!({ "uri": uri, "checksum": checksum })
}

pub async fn emit_artifact(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: EmitArtifactArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid emit_artifact args: {e}")))?;
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let content_ref = store_content(&args.content);
    let artifact_id = append_emit(
        &state.spine,
        &args.workspace_id,
        &args.session_id,
        content_ref.clone(),
        &args.kind,
        &args.summary,
    )
    .await
    .map_err(|e| {
        tracing::error!("mcp emit_artifact append_ext failed: {e}");
        ToolError::Internal(format!("failed to record emitted artifact: {e}"))
    })?;

    Ok(serde_json::json!({
        "artifact_id": artifact_id,
        "content_ref": content_ref,
    }))
}

// ---------------------------------------------------------------------------
// declare_no_output
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeclareNoOutputArgs {
    /// Workspace that owns this session (token-scope enforced).
    pub workspace_id: String,
    /// The session declaring it produced nothing.
    pub session_id: String,
    /// Why the session produced no deliverable. Surfaced to operators
    /// and the authoritative liveness gate so "delivered nothing on
    /// purpose" is distinguishable from "delivered nothing".
    pub reason: String,
}

pub async fn declare_no_output(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: DeclareNoOutputArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid declare_no_output args: {e}")))?;
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    append_declared_empty(
        &state.spine,
        &args.workspace_id,
        &args.session_id,
        &args.reason,
    )
    .await
    .map_err(|e| {
        tracing::error!("mcp declare_no_output append_ext failed: {e}");
        ToolError::Internal(format!("failed to record no-output declaration: {e}"))
    })?;

    Ok(serde_json::json!({ "ok": true }))
}

// ---------------------------------------------------------------------------
// read_emit_status
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadEmitStatusArgs {
    /// Workspace that owns this session (token-scope enforced). Manifest
    /// rows are matched against this so a session-id collision across
    /// workspaces can never leak counts.
    pub workspace_id: String,
    /// The session whose manifest to summarize.
    pub session_id: String,
}

pub async fn read_emit_status(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: ReadEmitStatusArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid read_emit_status args: {e}")))?;
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let status = read_status(&state.spine, &args.workspace_id, &args.session_id)
        .await
        .map_err(|e| {
            tracing::error!("mcp read_emit_status query_ext_stream failed: {e}");
            ToolError::Internal(format!("failed to read session manifest: {e}"))
        })?;

    serde_json::to_value(&status)
        .map_err(|e| ToolError::Internal(format!("response serialization failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_content_round_trips_via_inline_scheme() {
        let cr = store_content("hello world");
        let uri = cr["uri"].as_str().unwrap();
        let payload = uri.strip_prefix(INLINE_URI_PREFIX).unwrap();
        let decoded = BASE64.decode(payload).unwrap();
        assert_eq!(decoded, b"hello world");
        // Checksum present and sha256-shaped (64 hex chars).
        assert_eq!(cr["checksum"].as_str().unwrap().len(), 64);
    }
}
