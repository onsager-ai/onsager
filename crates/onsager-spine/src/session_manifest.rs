//! Session output manifest — the `events_ext`-backed record of what an
//! agent session produced (spec #520).
//!
//! This module is the **single source of truth** for the manifest wire
//! contract: the namespace / stream / event-type / data-field
//! conventions locked in spec #520 §3, the parse from raw
//! [`ExtensionEventRecord`] rows into a typed [`SessionManifest`], and
//! the store-level append/read helpers. Two callers sit on opposite
//! sides of the seam and share it from here rather than each carrying
//! their own copy:
//!
//! - **portal's MCP tools** (`crates/onsager-portal/src/mcp/tools/
//!   session_manifest.rs`, spec #520 §4a / #521) — the agent's only
//!   path to *write* the manifest (the execution environment holds no
//!   DB credentials).
//! - **the substrate scheduler's [`SpineClient`]** (`onsager-scheduler`,
//!   spec #520 §4c / #523) — the authoritative liveness gate *reads* the
//!   manifest after the agent session ends.
//!
//! Keeping the contract here (a crate both sides already depend on)
//! avoids the parallel-schema drift the seam rule forbids: one set of
//! constants, one parser, one data shape.
//!
//! Conventions (locked in spec #520 §3 — do not re-litigate):
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
//! `Deterministic`. The `AgentExecutor` stamps
//! `Provenance::Uncertain { source: Agent }` when it reads records back
//! (spec #520 §4c).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use onsager_artifact::ContentRef;

use crate::extension_event::ExtensionEventRecord;
use crate::store::{EventMetadata, EventStore};

/// Namespace the manifest rows live under (spec #520 §3).
pub const MANIFEST_NAMESPACE: &str = "stiglab";
/// `events_ext.event_type` for a real emitted output.
pub const EVENT_ARTIFACT_EMITTED: &str = "artifact.emitted";
/// `events_ext.event_type` for an explicit "I produced nothing".
pub const EVENT_OUTPUT_DECLARED_EMPTY: &str = "output.declared_empty";

/// Build the manifest stream key for a session.
pub fn session_stream_id(session_id: &str) -> String {
    format!("session:{session_id}")
}

/// The actor stamp on every manifest write — the session itself, never
/// a human or another subsystem.
pub fn session_actor(session_id: &str) -> String {
    format!("session:{session_id}")
}

// ---------------------------------------------------------------------------
// Typed manifest shapes
// ---------------------------------------------------------------------------

/// One output a session emitted, as recorded on an `artifact.emitted`
/// row. Carries identity + content pointer + agent-supplied metadata —
/// never provenance (see module docs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmittedRecord {
    /// Stable id minted at emit time (`art_<ulid>`), so the
    /// authoritative gate can reference this output when packaging it
    /// onto the typed DAG.
    pub artifact_id: String,
    /// Pointer to the addressable content the session produced.
    pub content_ref: ContentRef,
    /// Artifact kind tag (`code` / `document` / `pull_request` /
    /// `github_issue` / a custom string). Stored verbatim.
    pub kind: String,
    /// Human-readable summary of what was emitted.
    pub summary: String,
}

/// The deserialization shape of an `artifact.emitted` row's `data`
/// field. `artifact_id` and `content_ref` are required — the contract
/// relies on the minted id as a stable identity and on the ref as the
/// content pointer, so a row missing either is malformed and dropped
/// (see [`parse_manifest`]). `kind` / `summary` default so a row that
/// only omits the soft metadata is still salvageable.
#[derive(Debug, Deserialize)]
struct EmitData {
    artifact_id: String,
    content_ref: ContentRef,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    summary: String,
}

/// A session's full output manifest: every emitted output plus whether
/// the session explicitly declared it produced nothing.
///
/// `emitted.is_empty() && !declared_empty` is the liveness-gate trip
/// condition (spec #520 §4c) — the session ended having neither
/// delivered nor declared empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionManifest {
    pub emitted: Vec<EmittedRecord>,
    pub declared_empty: bool,
}

impl SessionManifest {
    /// Count of real emitted outputs.
    pub fn emit_count(&self) -> u64 {
        self.emitted.len() as u64
    }

    /// The narrow count-only summary `read_emit_status` returns.
    pub fn status(&self) -> EmitStatus {
        EmitStatus {
            emit_count: self.emit_count(),
            declared_empty: self.declared_empty,
        }
    }
}

/// The narrow manifest summary the `read_emit_status` MCP tool returns —
/// counts only, no content. Pure projection of [`SessionManifest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmitStatus {
    pub emit_count: u64,
    pub declared_empty: bool,
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

/// Parse a session's raw `events_ext` rows into a typed
/// [`SessionManifest`], scoped to a workspace.
///
/// Two defensive filters guard the read. The session stream is
/// workspace-private by construction, but rows from a different
/// workspace are skipped so a session-id collision across workspaces
/// can never leak outputs. `query_ext_stream` also returns every
/// extension row on the stream regardless of namespace, so rows outside
/// the manifest namespace (`stiglab`) are skipped too — another
/// subsystem reusing the same `event_type` on the same stream must not
/// be miscounted as a manifest record. An `artifact.emitted` row whose
/// `data` fails to parse (missing `artifact_id` / `content_ref`) is
/// dropped with a warning rather than failing the whole read — a
/// malformed row must not strand a live session.
pub fn parse_manifest(records: &[ExtensionEventRecord], workspace_id: &str) -> SessionManifest {
    let mut emitted = Vec::new();
    let mut declared_empty = false;
    for r in records {
        if r.workspace_id != workspace_id || r.namespace != MANIFEST_NAMESPACE {
            continue;
        }
        match r.event_type.as_str() {
            EVENT_ARTIFACT_EMITTED => match serde_json::from_value::<EmitData>(r.data.clone()) {
                Ok(d) => emitted.push(EmittedRecord {
                    artifact_id: d.artifact_id,
                    content_ref: d.content_ref,
                    kind: d.kind,
                    summary: d.summary,
                }),
                Err(e) => tracing::warn!(
                    stream = %r.stream_id,
                    "skipping malformed artifact.emitted manifest row: {e}"
                ),
            },
            EVENT_OUTPUT_DECLARED_EMPTY => declared_empty = true,
            _ => {}
        }
    }
    SessionManifest {
        emitted,
        declared_empty,
    }
}

// ---------------------------------------------------------------------------
// Store helpers (write / read)
// ---------------------------------------------------------------------------

/// Append an `artifact.emitted` row to the session manifest, recording a
/// pre-stored `content_ref`. Returns the minted `artifact_id`.
///
/// Content persistence is the caller's concern (portal's MCP tool runs
/// the inline/object-store path before calling this); this helper owns
/// only the manifest row.
pub async fn append_emit(
    store: &EventStore,
    workspace_id: &str,
    session_id: &str,
    content_ref: Value,
    kind: &str,
    summary: &str,
) -> Result<String, sqlx::Error> {
    // Mint a stable id so the authoritative gate (spec #520 §4c) can
    // reference this output when it packages it onto the typed DAG. Use
    // the canonical `art_<ulid>` shape so manifest rows match every
    // other id that flows through the artifact model.
    let artifact_id = onsager_artifact::ArtifactId::generate()
        .as_str()
        .to_string();
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

/// Append an `output.declared_empty` row to the session manifest.
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

/// Read + parse a session's full manifest. Pure read — safe to call
/// repeatedly.
pub async fn read_manifest(
    store: &EventStore,
    workspace_id: &str,
    session_id: &str,
) -> Result<SessionManifest, sqlx::Error> {
    let records = store
        .query_ext_stream(&session_stream_id(session_id))
        .await?;
    Ok(parse_manifest(&records, workspace_id))
}

/// Read a session's narrow status summary (counts only). Pure read.
pub async fn read_status(
    store: &EventStore,
    workspace_id: &str,
    session_id: &str,
) -> Result<EmitStatus, sqlx::Error> {
    Ok(read_manifest(store, workspace_id, session_id)
        .await?
        .status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn record(workspace_id: &str, event_type: &str, data: Value) -> ExtensionEventRecord {
        ExtensionEventRecord {
            id: 0,
            stream_id: session_stream_id("s1"),
            namespace: MANIFEST_NAMESPACE.to_string(),
            event_type: event_type.to_string(),
            data,
            metadata: serde_json::json!({}),
            ref_event_id: None,
            created_at: Utc::now(),
            correlation_id: None,
            workspace_id: workspace_id.to_string(),
        }
    }

    fn emit_data(uri: &str, kind: &str, summary: &str) -> Value {
        serde_json::json!({
            "artifact_id": "art_abc",
            "content_ref": { "uri": uri, "checksum": "deadbeef" },
            "kind": kind,
            "summary": summary,
        })
    }

    #[test]
    fn parse_counts_emits_and_detects_declared_empty() {
        let recs = vec![
            record(
                "ws1",
                EVENT_ARTIFACT_EMITTED,
                emit_data("inline:a", "code", "one"),
            ),
            record(
                "ws1",
                EVENT_ARTIFACT_EMITTED,
                emit_data("inline:b", "document", "two"),
            ),
            record(
                "ws1",
                EVENT_OUTPUT_DECLARED_EMPTY,
                serde_json::json!({"reason": "x"}),
            ),
        ];
        let m = parse_manifest(&recs, "ws1");
        assert_eq!(m.emit_count(), 2);
        assert!(m.declared_empty);
        assert_eq!(m.emitted[0].kind, "code");
        assert_eq!(m.emitted[0].content_ref.uri, "inline:a");
        assert_eq!(m.emitted[1].summary, "two");
        assert_eq!(m.status().emit_count, 2);
    }

    #[test]
    fn parse_untouched_session_is_zero_and_not_declared() {
        let m = parse_manifest(&[], "ws1");
        assert_eq!(m.emit_count(), 0);
        assert!(!m.declared_empty);
    }

    #[test]
    fn parse_ignores_rows_from_a_different_workspace() {
        let recs = vec![
            record(
                "other",
                EVENT_ARTIFACT_EMITTED,
                emit_data("inline:a", "code", "one"),
            ),
            record(
                "other",
                EVENT_OUTPUT_DECLARED_EMPTY,
                serde_json::json!({"reason": "x"}),
            ),
        ];
        let m = parse_manifest(&recs, "ws1");
        assert_eq!(m.emit_count(), 0);
        assert!(!m.declared_empty);
    }

    #[test]
    fn parse_drops_malformed_emit_row_without_failing_read() {
        // Missing `content_ref` — the row is dropped, the declared-empty
        // row still registers, and the read does not error.
        let recs = vec![
            record(
                "ws1",
                EVENT_ARTIFACT_EMITTED,
                serde_json::json!({"kind": "code"}),
            ),
            record(
                "ws1",
                EVENT_ARTIFACT_EMITTED,
                emit_data("inline:ok", "code", "good"),
            ),
        ];
        let m = parse_manifest(&recs, "ws1");
        assert_eq!(m.emit_count(), 1);
        assert_eq!(m.emitted[0].content_ref.uri, "inline:ok");
    }

    #[test]
    fn parse_drops_emit_row_missing_artifact_id() {
        // `artifact_id` is required — a row carrying a valid content_ref
        // but no minted id is malformed (the contract relies on the id
        // as stable identity for downstream packaging) and is dropped.
        let recs = vec![record(
            "ws1",
            EVENT_ARTIFACT_EMITTED,
            serde_json::json!({
                "content_ref": { "uri": "inline:x", "checksum": "z" },
                "kind": "code",
                "summary": "no id",
            }),
        )];
        let m = parse_manifest(&recs, "ws1");
        assert_eq!(m.emit_count(), 0, "row missing artifact_id must be dropped");
    }

    #[test]
    fn parse_ignores_rows_outside_the_manifest_namespace() {
        // `query_ext_stream` returns every extension row on the stream
        // regardless of namespace; a row reusing `artifact.emitted`
        // under a different namespace must not be miscounted.
        let mut foreign = record(
            "ws1",
            EVENT_ARTIFACT_EMITTED,
            emit_data("inline:a", "code", "one"),
        );
        foreign.namespace = "telegramable".to_string();
        let m = parse_manifest(&[foreign], "ws1");
        assert_eq!(m.emit_count(), 0, "foreign-namespace row must be ignored");
    }
}
