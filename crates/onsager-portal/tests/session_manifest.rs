//! Integration test for spec #520 §4a / #521 — the session output
//! manifest backed by `events_ext`.
//!
//! Exercises the store-level helpers (`append_emit`,
//! `append_declared_empty`, `read_status`) directly so the assertions
//! pin the `append_ext` → `query_ext_stream` round-trip without
//! standing up workspace membership / auth. The tool wrappers add only
//! `require_workspace_access` on top of these.
//!
//! Skipped when `DATABASE_URL` is unset — the manifest rows live on the
//! spine, which only the Postgres-backed harness exercises.

use onsager_portal::mcp::tools::session_manifest::{
    append_declared_empty, append_emit, read_status,
};
use onsager_spine::EventStore;

async fn try_store() -> Option<EventStore> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(EventStore::connect(&url).await.expect("spine connect"))
}

async fn cleanup(store: &EventStore, session_id: &str) {
    let _ = sqlx::query("DELETE FROM events_ext WHERE stream_id = $1")
        .bind(format!("session:{session_id}"))
        .execute(store.pool())
        .await;
}

#[tokio::test]
async fn emit_then_read_status_reports_emit_count_one() {
    let Some(store) = try_store().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = format!("ws_{}", ulid::Ulid::new());
    let session_id = ulid::Ulid::new().to_string();

    let content_ref = serde_json::json!({ "uri": "inline:base64,aGk=", "checksum": "abc" });
    let artifact_id = append_emit(
        &store,
        &workspace_id,
        &session_id,
        content_ref,
        "document",
        "first deliverable",
    )
    .await
    .expect("append_emit");
    assert!(
        artifact_id.starts_with("art_"),
        "emit mints a canonical `art_<ulid>` id, got {artifact_id}"
    );

    let status = read_status(&store, &workspace_id, &session_id)
        .await
        .expect("read_status");
    assert_eq!(status.emit_count, 1);
    assert!(!status.declared_empty);

    cleanup(&store, &session_id).await;
}

#[tokio::test]
async fn declare_no_output_then_read_status_reports_declared_empty() {
    let Some(store) = try_store().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = format!("ws_{}", ulid::Ulid::new());
    let session_id = ulid::Ulid::new().to_string();

    append_declared_empty(&store, &workspace_id, &session_id, "nothing to do")
        .await
        .expect("append_declared_empty");

    let status = read_status(&store, &workspace_id, &session_id)
        .await
        .expect("read_status");
    assert_eq!(status.emit_count, 0);
    assert!(status.declared_empty);

    cleanup(&store, &session_id).await;
}

#[tokio::test]
async fn read_status_untouched_session_is_zero_and_not_declared() {
    let Some(store) = try_store().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = format!("ws_{}", ulid::Ulid::new());
    let session_id = ulid::Ulid::new().to_string();

    let status = read_status(&store, &workspace_id, &session_id)
        .await
        .expect("read_status");
    assert_eq!(status.emit_count, 0);
    assert!(!status.declared_empty);
}

#[tokio::test]
async fn round_trips_workspace_id_and_is_scoped_to_it() {
    let Some(store) = try_store().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = format!("ws_{}", ulid::Ulid::new());
    let other_workspace = format!("ws_{}", ulid::Ulid::new());
    let session_id = ulid::Ulid::new().to_string();

    let content_ref = serde_json::json!({ "uri": "inline:base64,aGk=", "checksum": "abc" });
    append_emit(
        &store,
        &workspace_id,
        &session_id,
        content_ref,
        "code",
        "deliverable",
    )
    .await
    .expect("append_emit");

    // Same stream, but the read is scoped to a different workspace: the
    // row must not be counted (defensive cross-workspace isolation).
    let other = read_status(&store, &other_workspace, &session_id)
        .await
        .expect("read_status other workspace");
    assert_eq!(other.emit_count, 0, "row leaked across workspace scope");

    // The owning workspace sees its row.
    let owner = read_status(&store, &workspace_id, &session_id)
        .await
        .expect("read_status owner workspace");
    assert_eq!(owner.emit_count, 1);

    cleanup(&store, &session_id).await;
}
