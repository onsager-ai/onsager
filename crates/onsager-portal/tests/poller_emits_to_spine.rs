//! Integration test for spec #430 — poller wires through to the spine.
//!
//! Simulates a missed webhook delivery: an issue gets a workflow-
//! matching label on GitHub, but the webhook never arrives. The
//! reconciliation poller observes the issue via REST (here, via the
//! shared translator with hand-built shapes — the GitHub HTTP layer
//! is not exercised), translates to `Vec<RoutedEvent>`, and emits
//! through `emit_routed_events`. This test pins the contract that
//! the resulting `events_ext` row exists and carries the
//! `(adapter_id, external_ref)` dedup key from spine migration 032.
//!
//! Skipped when `DATABASE_URL` is unset — the contract lives in the
//! spine schema.
//!
//! The test does NOT spin up the scheduler's per-project tokio loop
//! (that would couple this test to project / workspace seed data and
//! a live HTTP layer); it exercises the same `translate → emit`
//! pipeline that `tick_project` calls. That keeps the test
//! deterministic and tightly focused on the spec's delivery: a
//! missed webhook turns into a spine row.

use std::collections::HashMap;

use onsager_github::api::issues::{Issue, Label};
use onsager_portal::reconciliation::{GITHUB_ADAPTER_ID, emit_routed_events, translate_issue};
use onsager_spine::{EventStore, WorkflowMatch};
use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn try_setup() -> Option<(PgPool, EventStore)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.expect("spine connect");
    let spine = EventStore::connect(&url).await.expect("spine open");
    Some((pool, spine))
}

#[tokio::test]
async fn poller_emit_lands_with_dedup_key() {
    let Some((pool, spine)) = try_setup().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };

    let workspace_id = format!("ws-{}", Uuid::new_v4());
    let project_id = format!("proj-{}", Uuid::new_v4());
    let issue_number: u64 = 4242;

    let issue = Issue {
        number: issue_number,
        title: "fix the missed delivery".into(),
        state: "open".into(),
        body: None,
        labels: vec![Label {
            name: "spec".into(),
        }],
        pull_request: None,
        updated_at: None,
    };

    let mut by_label = HashMap::new();
    by_label.insert(
        "spec".to_string(),
        vec![WorkflowMatch {
            id: format!("wf-{}", Uuid::new_v4()),
            workspace_id: workspace_id.clone(),
            trigger_kind_tag: "github_issue_webhook".into(),
        }],
    );

    let routed = translate_issue(&issue, "acme", "widgets", Some(&project_id), &by_label);
    assert_eq!(routed.len(), 1, "translator must produce one TriggerFired");
    let outcome = emit_routed_events(&spine, routed, &workspace_id, "test").await;
    assert_eq!(outcome.written, 1, "emit must persist the routed event");
    assert_eq!(outcome.failed, 0);

    // Pin: the row carries the dedup key the partial unique index
    // (adapter_id, external_ref) checks. A second emit must collapse.
    let expected_external_ref = format!(
        "github:project:{project_id}:issue:{issue_number}:trigger:{}",
        by_label.get("spec").unwrap()[0].id
    );
    let row = sqlx::query(
        "SELECT adapter_id, external_ref FROM events_ext \
         WHERE workspace_id = $1 AND adapter_id = $2 AND external_ref = $3",
    )
    .bind(&workspace_id)
    .bind(GITHUB_ADAPTER_ID)
    .bind(&expected_external_ref)
    .fetch_optional(&pool)
    .await
    .expect("query");
    let row = row.expect("emitted row must exist");
    assert_eq!(
        row.try_get::<String, _>("external_ref").unwrap(),
        expected_external_ref
    );

    // A second emit of the same translator output collapses on the
    // dedup index — the load-bearing property the webhook/reconciler
    // race relies on.
    let routed_again = translate_issue(&issue, "acme", "widgets", Some(&project_id), &by_label);
    let second = emit_routed_events(&spine, routed_again, &workspace_id, "test").await;
    assert_eq!(second.written, 0, "no new rows on re-emit");
    assert_eq!(
        second.deduped, 1,
        "re-emit must be deduped by the partial unique index"
    );
    assert_eq!(second.failed, 0);

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_ext \
         WHERE workspace_id = $1 AND adapter_id = $2 AND external_ref = $3",
    )
    .bind(&workspace_id)
    .bind(GITHUB_ADAPTER_ID)
    .bind(&expected_external_ref)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(count, 1, "exactly one row survives the race");

    // Cleanup.
    sqlx::query("DELETE FROM events_ext WHERE workspace_id = $1")
        .bind(&workspace_id)
        .execute(&pool)
        .await
        .ok();
}
