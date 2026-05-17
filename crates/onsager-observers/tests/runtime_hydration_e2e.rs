//! End-to-end hydration tests for `onsager-observers` (spec #392).
//!
//! Three scenarios, all DATABASE_URL-gated so they no-op in CI when
//! no Postgres is available:
//!
//! 1. **Restart correctness** — write an `artifact.registered` event,
//!    start a fresh runtime so it has to hydrate, then write a burst
//!    of `forge.gate_verdict` rows post-"restart". The
//!    [`GateOverrideObserver`] must group the live verdicts by the
//!    kind learned from the hydrated registration, exactly as if it
//!    had been online the whole time.
//! 2. **Hydration is silent** — historical events written before the
//!    runtime starts must NOT cause `observer_outputs` rows to be
//!    written. Output suppression is the key correctness rule from
//!    the spec.
//! 3. **Window is bounded** — events older than the observer's
//!    declared `hydration_window` are not replayed even if they
//!    matched.
//!
//! All three reuse the cross-test isolation trick from
//! `runtime_e2e.rs`: every test tags its events with a unique
//! per-run ULID and filters payload-side, so multiple tests can
//! share the same spine without contaminating each other.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use onsager_artifact::{ArtifactId, Kind};
use onsager_observers::{
    GateOverrideObserver, ObserverOutputStore, ObserverRuntime, gate_override::TAG as OVERRIDE_TAG,
};
use onsager_spine::factory_event::{FactoryEventKind, GatePoint, VerdictSummary};
use onsager_spine::{EventMetadata, EventStore, FactoryEvent};
use tokio::sync::oneshot;

async fn write_registered(
    store: &EventStore,
    id: &ArtifactId,
    kind: Kind,
) -> Result<i64, sqlx::Error> {
    let event = FactoryEvent {
        event: FactoryEventKind::ArtifactRegistered {
            artifact_id: id.clone(),
            kind,
            name: "hyd test".into(),
            owner: "obs-hyd".into(),
        },
        correlation_id: None,
        causation_id: None,
        actor: "obs-hyd".into(),
        timestamp: Utc::now(),
    };
    store
        .append_factory_event(
            &event,
            &EventMetadata {
                actor: "obs-hyd".into(),
                ..Default::default()
            },
        )
        .await
}

async fn write_verdict(
    store: &EventStore,
    id: &ArtifactId,
    v: VerdictSummary,
) -> Result<i64, sqlx::Error> {
    let event = FactoryEvent {
        event: FactoryEventKind::ForgeGateVerdict {
            artifact_id: id.clone(),
            gate_point: GatePoint::PreDispatch,
            verdict: v,
        },
        correlation_id: None,
        causation_id: None,
        actor: "obs-hyd".into(),
        timestamp: Utc::now(),
    };
    store
        .append_factory_event(
            &event,
            &EventMetadata {
                actor: "obs-hyd".into(),
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

/// Restart correctness: an artifact registered *before* the runtime
/// starts must still drive grouping for verdicts that arrive *after*
/// the runtime is up. Without hydration the observer's
/// artifact-id → kind index would be empty after restart and the
/// override-rate insight would never fire.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hydration_replays_artifact_registered_so_post_restart_verdicts_group() {
    let Some(db_url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let event_store = EventStore::connect(&db_url).await.unwrap();
    let output_store = ObserverOutputStore::new(event_store.pool().clone());
    let tag = ulid::Ulid::new().to_string();
    let observer_id = format!("obs_hyd_restart_{}", tag);
    let art = ArtifactId::new(format!("art_{}", tag));

    // Pre-"restart" history: just the registration.
    let registered_id = write_registered(&event_store, &art, Kind::Code)
        .await
        .unwrap();

    // Bring up the runtime. `run_with_ready` only signals once
    // hydration is complete, so the post-ready writes below are
    // guaranteed to land on a hydrated observer.
    let runtime = ObserverRuntime::new(event_store.clone(), output_store.clone())
        .register(&observer_id, GateOverrideObserver::default());
    let (ready_tx, ready_rx) = oneshot::channel();
    let handle = tokio::spawn(async move { runtime.run_with_ready(Some(ready_tx)).await });
    ready_rx.await.unwrap();

    // Post-"restart" live verdicts: 4 deny + 1 allow against the
    // hydrated kind. The default config wants 5 samples at >50%
    // override — 4/5 = 80% trips it.
    let mut verdict_ids = Vec::new();
    for v in [
        VerdictSummary::Deny,
        VerdictSummary::Deny,
        VerdictSummary::Deny,
        VerdictSummary::Deny,
        VerdictSummary::Allow,
    ] {
        verdict_ids.push(write_verdict(&event_store, &art, v).await.unwrap());
    }

    // Poll for the override-rate insight. Bounded so a regression
    // fails fast.
    let mut found = Vec::new();
    for _ in 0..30 {
        found = output_store
            .list_by_observer(&observer_id, 10)
            .await
            .unwrap();
        if !found.is_empty() {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(100)).await;
    }
    handle.abort();

    let insights: Vec<_> = found
        .iter()
        .filter_map(|r| match r.clone().into_output().ok()? {
            onsager_observers::ObserverOutput::Insight(i)
                if i.tag.as_deref() == Some(OVERRIDE_TAG) =>
            {
                Some(i)
            }
            _ => None,
        })
        .collect();
    assert!(
        !insights.is_empty(),
        "hydration should have rebuilt the kind index so live verdicts trip the rate; got {found:?}"
    );

    let mut all_ids = verdict_ids;
    all_ids.push(registered_id);
    cleanup(&event_store, &observer_id, &all_ids).await;
}

/// Hydration is silent: replaying historical events that *would*
/// have produced insights must not write to `observer_outputs`.
/// We pre-stage enough history to trip the override-rate threshold,
/// start a fresh runtime, wait for ready, and assert no rows landed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hydration_does_not_emit_outputs_for_replayed_events() {
    let Some(db_url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let event_store = EventStore::connect(&db_url).await.unwrap();
    let output_store = ObserverOutputStore::new(event_store.pool().clone());
    let tag = ulid::Ulid::new().to_string();
    let observer_id = format!("obs_hyd_silent_{}", tag);
    let art = ArtifactId::new(format!("art_{}", tag));

    // Pre-stage: registration + 5 denies. Same shape that the
    // live-only test trips on.
    let mut event_ids = Vec::new();
    event_ids.push(
        write_registered(&event_store, &art, Kind::Code)
            .await
            .unwrap(),
    );
    for v in [
        VerdictSummary::Deny,
        VerdictSummary::Deny,
        VerdictSummary::Deny,
        VerdictSummary::Deny,
        VerdictSummary::Deny,
    ] {
        event_ids.push(write_verdict(&event_store, &art, v).await.unwrap());
    }

    let runtime = ObserverRuntime::new(event_store.clone(), output_store.clone())
        .register(&observer_id, GateOverrideObserver::default());
    let (ready_tx, ready_rx) = oneshot::channel();
    let handle = tokio::spawn(async move { runtime.run_with_ready(Some(ready_tx)).await });
    ready_rx.await.unwrap();

    // After ready, the runtime has hydrated. Give it a beat to
    // settle any in-flight writes — there shouldn't be any from
    // hydration, but if there were a regression we want to see it.
    tokio::time::sleep(StdDuration::from_millis(200)).await;

    let rows = output_store
        .list_by_observer(&observer_id, 50)
        .await
        .unwrap();
    handle.abort();

    assert!(
        rows.is_empty(),
        "hydration must not write observer_outputs rows; got {rows:?}"
    );

    cleanup(&event_store, &observer_id, &event_ids).await;
}

/// Hydration window is bounded: an artifact registered LONG before
/// the runtime starts (older than the observer's window) must NOT
/// be replayed, so live verdicts referencing it drop out of
/// grouping. We force the bound by registering with a configured
/// 1-minute window and writing the registration "now" — the test
/// instead uses a *custom-configured* observer whose hydration
/// window is shorter than the live verdict timing, then asserts
/// the observer's kind index stays empty.
///
/// We can't easily backdate `created_at` (the spine sets it), so
/// the strategy here is the inverse: configure the observer with a
/// 0-second hydration window (effectively no hydration), then
/// write a registration before the runtime starts and assert that
/// post-runtime verdicts do NOT trip the rate (because the kind
/// index never got the registration). This is the "without
/// hydration, restart drops verdicts" failure mode the spec calls
/// out — it's the negative-control for the first test.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hydration_window_zero_means_no_replay_and_verdicts_drop() {
    let Some(db_url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let event_store = EventStore::connect(&db_url).await.unwrap();
    let output_store = ObserverOutputStore::new(event_store.pool().clone());
    let tag = ulid::Ulid::new().to_string();
    let observer_id = format!("obs_hyd_zero_{}", tag);
    let art = ArtifactId::new(format!("art_{}", tag));

    // Pre-"restart" history: just the registration.
    let registered_id = write_registered(&event_store, &art, Kind::Code)
        .await
        .unwrap();

    // Build an observer with a `window` of 1 nanosecond so any
    // historical event falls outside the hydration window. We do
    // NOT change the analyzer's live window (the analyzer's
    // sliding-window prune uses `config.window`, so a 1ns window
    // would also break live grouping). Instead we use a custom
    // wrapper observer for this scenario.
    use async_trait::async_trait;
    use onsager_observers::{EventPattern, Observer, ObserverOutput, SpineEvent};

    /// Wraps a real observer but advertises a zero hydration
    /// window — so the runtime skips replaying through it even
    /// though the inner observer would have benefited.
    struct NoHydrate(GateOverrideObserver);

    #[async_trait]
    impl Observer for NoHydrate {
        fn subscriptions(&self) -> Vec<EventPattern> {
            self.0.subscriptions()
        }
        async fn on_event(&mut self, ev: &SpineEvent) -> Vec<ObserverOutput> {
            self.0.on_event(ev).await
        }
        fn hydration_window(&self) -> Option<Duration> {
            None
        }
    }

    let runtime = ObserverRuntime::new(event_store.clone(), output_store.clone())
        .register(&observer_id, NoHydrate(GateOverrideObserver::default()));
    let (ready_tx, ready_rx) = oneshot::channel();
    let handle = tokio::spawn(async move { runtime.run_with_ready(Some(ready_tx)).await });
    ready_rx.await.unwrap();

    // Live: same 5-verdict burst.
    let mut verdict_ids = Vec::new();
    for v in [
        VerdictSummary::Deny,
        VerdictSummary::Deny,
        VerdictSummary::Deny,
        VerdictSummary::Deny,
        VerdictSummary::Allow,
    ] {
        verdict_ids.push(write_verdict(&event_store, &art, v).await.unwrap());
    }

    // Give the live loop time to drain — bounded so regressions
    // surface quickly.
    tokio::time::sleep(StdDuration::from_millis(500)).await;
    let rows = output_store
        .list_by_observer(&observer_id, 10)
        .await
        .unwrap();
    handle.abort();

    assert!(
        rows.is_empty(),
        "without hydration, post-restart verdicts must drop out of grouping (no kind index); got {rows:?}"
    );

    let mut all_ids = verdict_ids;
    all_ids.push(registered_id);
    cleanup(&event_store, &observer_id, &all_ids).await;
}

/// Live notifications with `id <= cutoff_id` must be skipped: they
/// were already hydrated and re-dispatching them would double-count
/// in the observer's sliding-window buffer. We can't easily race the
/// cutoff capture from a test, but we can pre-stage an
/// `artifact.registered` event before the runtime starts, then
/// assert that after ready the observer's kind index has ONE entry
/// for this artifact, not two.
///
/// This is a weak proxy for "no double-dispatch" — the strong
/// version is structural (the live loop's `if notification.id <=
/// cutoff_id { continue; }` is a hard skip, no dispatch). The unit
/// test in `runtime.rs::tests` exercises the dispatch surface; this
/// E2E asserts the wired-up runtime behaves consistently.
///
/// Arc<AtomicUsize> counts every `on_event` call across both
/// hydration and live paths.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cutoff_skip_avoids_double_dispatch_of_pre_subscribe_events() {
    let Some(db_url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let event_store = EventStore::connect(&db_url).await.unwrap();
    let output_store = ObserverOutputStore::new(event_store.pool().clone());
    let tag = ulid::Ulid::new().to_string();
    let observer_id = format!("obs_hyd_cutoff_{}", tag);
    let art = ArtifactId::new(format!("art_{}", tag));

    use async_trait::async_trait;
    use onsager_observers::{EventPattern, Observer, ObserverOutput, SpineEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts every `on_event` invocation for OUR tagged event ids
    /// (cross-test isolation: other tests' events may also
    /// stream past).
    struct DispatchCounter {
        count: Arc<AtomicUsize>,
        tag: String,
    }

    #[async_trait]
    impl Observer for DispatchCounter {
        fn subscriptions(&self) -> Vec<EventPattern> {
            vec![EventPattern::new("artifact.registered")]
        }
        fn hydration_window(&self) -> Option<Duration> {
            Some(Duration::days(7))
        }
        async fn on_event(&mut self, ev: &SpineEvent) -> Vec<ObserverOutput> {
            // Filter to events that name our tagged artifact.
            if let FactoryEventKind::ArtifactRegistered { artifact_id, .. } = &ev.payload.event
                && artifact_id
                    .as_str()
                    .starts_with(&format!("art_{}", self.tag))
            {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
            Vec::new()
        }
    }

    // Pre-stage: one registration before the runtime starts.
    let registered_id = write_registered(&event_store, &art, Kind::Code)
        .await
        .unwrap();

    let count = Arc::new(AtomicUsize::new(0));
    let runtime = ObserverRuntime::new(event_store.clone(), output_store.clone()).register(
        &observer_id,
        DispatchCounter {
            count: Arc::clone(&count),
            tag: tag.clone(),
        },
    );
    let (ready_tx, ready_rx) = oneshot::channel();
    let handle = tokio::spawn(async move { runtime.run_with_ready(Some(ready_tx)).await });
    ready_rx.await.unwrap();

    // Hydration must have dispatched the pre-staged registration.
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "hydration should have dispatched the pre-staged registration once"
    );

    // Give the live loop time to drain the buffered notification
    // (if it WAS going to be re-dispatched, it would happen here).
    tokio::time::sleep(StdDuration::from_millis(300)).await;
    handle.abort();

    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "live notification for pre-cutoff event id must be skipped, not re-dispatched"
    );

    cleanup(&event_store, &observer_id, &[registered_id]).await;
}
