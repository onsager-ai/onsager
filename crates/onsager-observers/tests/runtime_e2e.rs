//! End-to-end runtime tests for `onsager-observers` (issue #361).
//!
//! Three scenarios, all of them DATABASE_URL-gated so they no-op in
//! CI when no Postgres is available:
//!
//! 1. **Subscription matches and persists** — a `CountingObserver`
//!    subscribed to `artifact.*` receives an `artifact.state_changed`
//!    event end-to-end through the spine and writes one row to
//!    `observer_outputs`.
//! 2. **Non-matching events are filtered out** — the same observer
//!    must NOT see a `node.started` event written to the same spine.
//! 3. **Observer work does not block the substrate writer** — even
//!    when the observer blocks for 500ms, the writer that emits the
//!    event returns immediately (constitutive property 1 from
//!    ADR 0013).
//!
//! Multiple tests run against the same DB; cross-test contamination
//! is prevented by tagging each test's events with a unique
//! `artifact_id` prefix and having every observer filter
//! payload-side on that prefix. Each test's observer only counts /
//! emits for its own events even when other tests' events stream
//! past on the shared spine.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use onsager_artifact::{ArtifactId, ArtifactState, Kind};
use onsager_observers::{
    EventPattern, Insight, Observer, ObserverOutput, ObserverOutputKind, ObserverOutputStore,
    ObserverRuntime, SpineEvent,
};
use onsager_spine::{EventMetadata, EventStore, FactoryEvent, FactoryEventKind};
use tokio::sync::oneshot;

/// Observer that counts events whose payload artifact_id starts with
/// `tag`. Cross-test isolation is payload-side, not pattern-side,
/// because multiple tests subscribe to the same `artifact.*` channel.
struct CountingObserver {
    seen: Arc<AtomicUsize>,
    sleep_ms: u64,
    patterns: Vec<EventPattern>,
    /// Only count / emit for events whose payload's `artifact_id`
    /// starts with `art_<tag>` (`tag` is the test's per-run ULID).
    tag: String,
}

impl CountingObserver {
    fn payload_matches_tag(&self, ev: &SpineEvent) -> bool {
        match &ev.payload.event {
            FactoryEventKind::ArtifactStateChanged { artifact_id, .. }
            | FactoryEventKind::ArtifactRegistered { artifact_id, .. }
            | FactoryEventKind::ArtifactArchived { artifact_id, .. } => artifact_id
                .as_str()
                .starts_with(&format!("art_{}", self.tag)),
            _ => false,
        }
    }
}

#[async_trait]
impl Observer for CountingObserver {
    fn subscriptions(&self) -> Vec<EventPattern> {
        self.patterns.clone()
    }

    async fn on_event(&mut self, ev: &SpineEvent) -> Vec<ObserverOutput> {
        if !self.payload_matches_tag(ev) {
            return Vec::new();
        }
        if self.sleep_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
        }
        let n = self.seen.fetch_add(1, Ordering::SeqCst) + 1;
        vec![ObserverOutput::Insight(Insight::new(
            format!("seen #{n} from {}", ev.event_type),
            0.5,
        ))]
    }
}

async fn write_artifact_state_changed(store: &EventStore, tag: &str) -> Result<i64, sqlx::Error> {
    let event = FactoryEvent {
        event: FactoryEventKind::ArtifactStateChanged {
            artifact_id: ArtifactId::new(format!("art_{tag}")),
            from_state: ArtifactState::Draft,
            to_state: ArtifactState::InProgress,
        },
        correlation_id: None,
        causation_id: None,
        actor: "obs-test".into(),
        timestamp: Utc::now(),
    };
    store
        .append_factory_event(
            &event,
            &EventMetadata {
                actor: "obs-test".into(),
                ..Default::default()
            },
        )
        .await
}

async fn write_artifact_registered(store: &EventStore, tag: &str) -> Result<i64, sqlx::Error> {
    let event = FactoryEvent {
        event: FactoryEventKind::ArtifactRegistered {
            artifact_id: ArtifactId::new(format!("art_{tag}")),
            kind: Kind::Document,
            name: "obs test".into(),
            owner: "obs-test".into(),
        },
        correlation_id: None,
        causation_id: None,
        actor: "obs-test".into(),
        timestamp: Utc::now(),
    };
    store
        .append_factory_event(
            &event,
            &EventMetadata {
                actor: "obs-test".into(),
                ..Default::default()
            },
        )
        .await
}

async fn cleanup(store: &EventStore, observer_id: &str, event_ids: &[i64]) {
    let _ = sqlx::query("DELETE FROM observer_outputs WHERE observer_id = $1")
        .bind(observer_id)
        .execute(store.pool())
        .await;
    if !event_ids.is_empty() {
        let _ = sqlx::query("DELETE FROM events WHERE id = ANY($1)")
            .bind(event_ids)
            .execute(store.pool())
            .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observer_receives_matching_event_and_persists_output() {
    let Some(db_url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };

    let event_store = EventStore::connect(&db_url).await.unwrap();
    let pool = event_store.pool().clone();
    let output_store = ObserverOutputStore::new(pool.clone());

    let tag = ulid::Ulid::new().to_string();
    let observer_id = format!("obs_e2e_match_{}", tag);
    let seen = Arc::new(AtomicUsize::new(0));

    let runtime = ObserverRuntime::new(event_store.clone(), output_store.clone()).register(
        &observer_id,
        CountingObserver {
            seen: Arc::clone(&seen),
            sleep_ms: 0,
            patterns: vec![EventPattern::new("artifact.*")],
            tag: tag.clone(),
        },
    );

    let (ready_tx, ready_rx) = oneshot::channel();
    let handle = tokio::spawn(async move { runtime.run_with_ready(Some(ready_tx)).await });

    // Wait for the subscription to actually attach — replaces the
    // flaky 200ms sleep.
    ready_rx.await.unwrap();

    let event_id = write_artifact_state_changed(&event_store, &tag)
        .await
        .unwrap();

    // Poll for the observer output. Bounded so we fail fast on regression.
    let mut found = Vec::new();
    for _ in 0..30 {
        found = output_store
            .list_by_observer(&observer_id, 10)
            .await
            .unwrap();
        if !found.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    handle.abort();

    assert_eq!(
        found.len(),
        1,
        "expected exactly one persisted output for matched event"
    );
    assert_eq!(found[0].observer_id, observer_id);
    assert_eq!(found[0].kind, ObserverOutputKind::Insight);
    assert_eq!(found[0].triggered_by_event_id, Some(event_id));
    assert_eq!(
        seen.load(Ordering::SeqCst),
        1,
        "observer saw exactly one event"
    );

    cleanup(&event_store, &observer_id, &[event_id]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observer_ignores_non_matching_event() {
    let Some(db_url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };

    let event_store = EventStore::connect(&db_url).await.unwrap();
    let output_store = ObserverOutputStore::new(event_store.pool().clone());

    // This observer only listens to "node.*"; we send an "artifact.*" event.
    let tag = ulid::Ulid::new().to_string();
    let observer_id = format!("obs_e2e_filter_{}", tag);
    let seen = Arc::new(AtomicUsize::new(0));
    let runtime = ObserverRuntime::new(event_store.clone(), output_store.clone()).register(
        &observer_id,
        CountingObserver {
            seen: Arc::clone(&seen),
            sleep_ms: 0,
            patterns: vec![EventPattern::new("node.*")],
            tag: tag.clone(),
        },
    );
    let (ready_tx, ready_rx) = oneshot::channel();
    let handle = tokio::spawn(async move { runtime.run_with_ready(Some(ready_tx)).await });
    ready_rx.await.unwrap();

    let event_id = write_artifact_registered(&event_store, &tag).await.unwrap();

    // After the runtime is ready and the event has been written,
    // poll briefly — if the runtime were going to fire wrongly it
    // would have spawned a task in the same event-loop tick that
    // pulled the notification off pg_notify, so a short bounded
    // poll is enough.
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let rows = output_store
            .list_by_observer(&observer_id, 10)
            .await
            .unwrap();
        assert!(
            rows.is_empty(),
            "non-matching event must not produce output, got: {:?}",
            rows
        );
    }
    handle.abort();
    assert_eq!(
        seen.load(Ordering::SeqCst),
        0,
        "observer must not have been invoked"
    );

    cleanup(&event_store, &observer_id, &[event_id]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_observer_does_not_block_event_writer() {
    let Some(db_url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };

    let event_store = EventStore::connect(&db_url).await.unwrap();
    let output_store = ObserverOutputStore::new(event_store.pool().clone());

    let tag = ulid::Ulid::new().to_string();
    let observer_id = format!("obs_e2e_slow_{}", tag);
    let seen = Arc::new(AtomicUsize::new(0));
    let runtime = ObserverRuntime::new(event_store.clone(), output_store.clone()).register(
        &observer_id,
        CountingObserver {
            seen: Arc::clone(&seen),
            sleep_ms: 500, // Observer blocks for 500ms per event
            patterns: vec![EventPattern::new("artifact.*")],
            tag: tag.clone(),
        },
    );
    let (ready_tx, ready_rx) = oneshot::channel();
    let handle = tokio::spawn(async move { runtime.run_with_ready(Some(ready_tx)).await });
    ready_rx.await.unwrap();

    // Time how long it takes to write the event onto the spine.
    let writer_start = Instant::now();
    let event_id = write_artifact_state_changed(&event_store, &tag)
        .await
        .unwrap();
    let writer_elapsed = writer_start.elapsed();

    // The writer must return in well under 500ms — observers run in
    // separate tasks, so the writer never waits on observer work.
    assert!(
        writer_elapsed < Duration::from_millis(200),
        "spine writer was blocked by observer (took {:?})",
        writer_elapsed
    );

    // Eventually the (slow) observer completes and persists its output.
    let mut found = Vec::new();
    for _ in 0..30 {
        found = output_store
            .list_by_observer(&observer_id, 10)
            .await
            .unwrap();
        if !found.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    handle.abort();

    assert_eq!(found.len(), 1, "slow observer eventually completed");
    cleanup(&event_store, &observer_id, &[event_id]).await;
}
