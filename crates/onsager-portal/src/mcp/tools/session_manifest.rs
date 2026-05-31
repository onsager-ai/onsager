//! MCP tools for the agent session output manifest (spec #520 §4a /
//! #521 — the addressable-output foundation).
//!
//! An agent session runs in an untrusted execution environment that
//! holds **no DB credentials** — these three MCP tools are its only
//! path to record (and inspect) what it produced. The credential-
//! holding portal MCP server is the only thing that touches the
//! database, exactly like every other tool in this directory.
//!
//! Storage is `events_ext` (not a new table, not a new
//! `FactoryEventKind`) via [`EventStore::append_ext`] /
//! [`EventStore::query_ext_stream`]. Those rows are already on the
//! `onsager_events` pg_notify stream, so a future Observer (OBS-01,
//! #361) can subscribe to the same manifest with no extra plumbing.
//! The conventions below are locked in spec #520 §3 — do not
//! re-litigate:
//!
//! - `namespace = "stiglab"` (the emit happens inside a stiglab-
//!   orchestrated session; reuse the existing validated namespace).
//! - `stream_id = "session:<session_id>"`.
//! - `event_type = "artifact.emitted"` for a real output;
//!   `"output.declared_empty"` for an explicit no-output declaration.
//!
//! **Provenance never goes into the manifest.** Records carry
//! `content_ref / kind / summary` (plus the minted `artifact_id`
//! identity) only — the agent must not be able to self-declare
//! `Deterministic`. `AgentExecutor` stamps `Provenance::Uncertain`
//! when it reads records back (spec #520 §4c, a later layer).

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ring::digest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use onsager_spine::EventMetadata;
use onsager_spine::EventStore;
use onsager_spine::extension_event::ExtensionEventRecord;

use crate::auth::AuthUser;
use crate::state::AppState;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

/// Namespace the manifest rows live under (spec #520 §3).
const MANIFEST_NAMESPACE: &str = "stiglab";
/// `events_ext.event_type` for a real emitted output.
const EVENT_ARTIFACT_EMITTED: &str = "artifact.emitted";
/// `events_ext.event_type` for an explicit "I produced nothing".
const EVENT_OUTPUT_DECLARED_EMPTY: &str = "output.declared_empty";

/// Inline content-storage URI scheme. First cut matches the Script
/// executor's convention (`crates/onsager-nodes/src/script.rs`):
/// base64-encoded body behind an `inline:` prefix. A real warehouse-
/// backed scheme replaces this once content storage is formalized
/// (RUN-01 #359 — the same gap the Script executor flags).
const INLINE_URI_PREFIX: &str = "inline:base64,";

/// Build the manifest stream key for a session.
fn session_stream_id(session_id: &str) -> String {
    format!("session:{session_id}")
}

/// The actor stamp on every manifest write — the session itself, never
/// a human or another subsystem.
fn session_actor(session_id: &str) -> String {
    format!("session:{session_id}")
}

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

/// Append an `artifact.emitted` row to the session manifest. Returns
/// the minted `artifact_id`. Store-level helper so the integration
/// test can exercise the round-trip without standing up auth.
pub async fn append_emit(
    store: &EventStore,
    workspace_id: &str,
    session_id: &str,
    content_ref: Value,
    kind: &str,
    summary: &str,
) -> Result<String, sqlx::Error> {
    // Mint a stable id so the authoritative gate (spec #520 §4c) can
    // reference this output when it packages it onto the typed DAG.
    let artifact_id = ulid::Ulid::new().to_string();
    let data = serde_json::json!({
        "artifact_id": artifact_id,
        "content_ref": content_ref,
        "kind": kind,
        "summary": summary,
    });
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: session_actor(session_id),
    };
    store
        .append_ext(
            workspace_id,
            &session_stream_id(session_id),
            MANIFEST_NAMESPACE,
            EVENT_ARTIFACT_EMITTED,
            data,
            &metadata,
            None,
        )
        .await?;
    Ok(artifact_id)
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

/// Append an `output.declared_empty` row to the session manifest.
/// Store-level helper, mirroring [`append_emit`].
pub async fn append_declared_empty(
    store: &EventStore,
    workspace_id: &str,
    session_id: &str,
    reason: &str,
) -> Result<(), sqlx::Error> {
    let data = serde_json::json!({ "reason": reason });
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: session_actor(session_id),
    };
    store
        .append_ext(
            workspace_id,
            &session_stream_id(session_id),
            MANIFEST_NAMESPACE,
            EVENT_OUTPUT_DECLARED_EMPTY,
            data,
            &metadata,
            None,
        )
        .await?;
    Ok(())
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

/// Pure tally of a session's manifest records, scoped to a workspace.
/// Counts `artifact.emitted` rows and detects any
/// `output.declared_empty`. Separated from the DB read so it is
/// unit-testable without Postgres.
pub fn tally_status(records: &[ExtensionEventRecord], workspace_id: &str) -> EmitStatus {
    let mut emit_count: u64 = 0;
    let mut declared_empty = false;
    for r in records {
        // Defensive: the session stream is workspace-private by
        // construction, but never count a row that names a different
        // workspace.
        if r.workspace_id != workspace_id {
            continue;
        }
        match r.event_type.as_str() {
            EVENT_ARTIFACT_EMITTED => emit_count += 1,
            EVENT_OUTPUT_DECLARED_EMPTY => declared_empty = true,
            _ => {}
        }
    }
    EmitStatus {
        emit_count,
        declared_empty,
    }
}

/// The manifest summary `read_emit_status` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmitStatus {
    pub emit_count: u64,
    pub declared_empty: bool,
}

/// Read + tally a session's manifest. Store-level helper so the
/// integration test can call it directly. Pure read — safe to call
/// repeatedly.
pub async fn read_status(
    store: &EventStore,
    workspace_id: &str,
    session_id: &str,
) -> Result<EmitStatus, sqlx::Error> {
    let records = store
        .query_ext_stream(&session_stream_id(session_id))
        .await?;
    Ok(tally_status(&records, workspace_id))
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
    use chrono::Utc;

    fn record(workspace_id: &str, event_type: &str) -> ExtensionEventRecord {
        ExtensionEventRecord {
            id: 0,
            stream_id: session_stream_id("s1"),
            namespace: MANIFEST_NAMESPACE.to_string(),
            event_type: event_type.to_string(),
            data: serde_json::json!({}),
            metadata: serde_json::json!({}),
            ref_event_id: None,
            created_at: Utc::now(),
            correlation_id: None,
            workspace_id: workspace_id.to_string(),
        }
    }

    #[test]
    fn tally_counts_emits_and_detects_declared_empty() {
        let recs = vec![
            record("ws1", EVENT_ARTIFACT_EMITTED),
            record("ws1", EVENT_ARTIFACT_EMITTED),
            record("ws1", EVENT_OUTPUT_DECLARED_EMPTY),
        ];
        let status = tally_status(&recs, "ws1");
        assert_eq!(status.emit_count, 2);
        assert!(status.declared_empty);
    }

    #[test]
    fn tally_untouched_session_is_zero_and_not_declared() {
        let status = tally_status(&[], "ws1");
        assert_eq!(status.emit_count, 0);
        assert!(!status.declared_empty);
    }

    #[test]
    fn tally_ignores_rows_from_a_different_workspace() {
        let recs = vec![
            record("other", EVENT_ARTIFACT_EMITTED),
            record("other", EVENT_OUTPUT_DECLARED_EMPTY),
        ];
        let status = tally_status(&recs, "ws1");
        assert_eq!(status.emit_count, 0);
        assert!(!status.declared_empty);
    }

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
