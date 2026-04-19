//! Cached `InterceptEngine` keyed by [`RulesRevision`] (issue #32).
//!
//! Forge invokes Synodic's `/gate` endpoint twice per pipeline tick. The
//! original handler reloaded every active rule from storage and constructed
//! a fresh engine on each call — an O(N) cost in the active rule count, with
//! no caching and no invalidation protocol.
//!
//! [`EngineCache`] caches a compiled engine behind the rule set's revision
//! token. Steady state cost per call is one cheap `SELECT COUNT/MAX(updated_at)`
//! roundtrip plus a clone of an `Arc`. On a rule mutation, the next call
//! observes the new revision and rebuilds.
//!
//! ## Concurrency
//!
//! The fast path takes a read lock and clones the inner `Arc<InterceptEngine>`.
//! On miss, it upgrades to a write lock and double-checks the revision before
//! rebuilding so two concurrent misses don't both fetch + compile.
//!
//! ## Future work
//!
//! Per #32's "better" suggestion, a `synodic.rules_updated` pg_notify event
//! would let the cache invalidate on mutation rather than on the next call's
//! revision check. That requires the broader event-bus migration in #27 and
//! is out of scope for this PR. The current revision check is portable
//! across SQLite + Postgres and incurs ~ms per call — the rebuild cost it
//! avoids is on the order of 100ms to 1s for a moderately sized rule set.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::core::intercept::{InterceptEngine, InterceptRule};
use crate::core::storage::{RulesRevision, Storage};

/// A compiled engine plus the revision token it was built from.
///
/// The engine is wrapped in `Arc` so handlers can clone the snapshot
/// without holding any cache lock across the (synchronous) `evaluate`
/// call.
struct CachedEngine {
    revision: RulesRevision,
    engine: Arc<InterceptEngine>,
}

/// Read-through cache for the compiled `InterceptEngine`.
///
/// Construct one per process and share via `Arc`. The cache is `active_only`
/// because the gate path only consults enabled rules; an `enabled = FALSE`
/// row that flips to `enabled = TRUE` will bump the revision via its
/// `updated_at` change and force a rebuild.
pub struct EngineCache {
    cell: RwLock<Option<CachedEngine>>,
}

impl Default for EngineCache {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineCache {
    pub fn new() -> Self {
        Self {
            cell: RwLock::new(None),
        }
    }

    /// Return the engine matching the current rule set, fetching and
    /// compiling only if the cached revision is stale (or empty).
    pub async fn get_or_refresh(&self, store: &dyn Storage) -> Result<Arc<InterceptEngine>> {
        let current = store.get_rules_revision(true).await?;

        // Fast path: cached engine matches the current revision.
        {
            let guard = self.cell.read().await;
            if let Some(ref entry) = *guard {
                if entry.revision == current {
                    return Ok(Arc::clone(&entry.engine));
                }
            }
        }

        // Slow path: rebuild under the write lock. Re-check inside the lock
        // so two racing misses produce only one fetch.
        let mut guard = self.cell.write().await;
        if let Some(ref entry) = *guard {
            if entry.revision == current {
                return Ok(Arc::clone(&entry.engine));
            }
        }
        let storage_rules = store.get_rules(true).await?;
        let rules: Vec<InterceptRule> = storage_rules.iter().map(InterceptRule::from).collect();
        let engine = Arc::new(InterceptEngine::new(rules));
        *guard = Some(CachedEngine {
            revision: current,
            engine: Arc::clone(&engine),
        });
        Ok(engine)
    }

    /// Drop the cached engine, forcing the next `get_or_refresh` to rebuild.
    /// Useful for tests; production callers rely on revision-based
    /// invalidation via `get_or_refresh`.
    #[cfg(test)]
    pub async fn invalidate(&self) {
        *self.cell.write().await = None;
    }

    /// Whether the cache currently holds a compiled engine.
    #[cfg(test)]
    pub async fn is_warm(&self) -> bool {
        self.cell.read().await.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::storage::{
        CreateRule, FeedbackEvent, FeedbackFilters, GovernanceEvent, GovernanceEventFilters,
        GovernanceScores, Lifecycle, PipelineRun, ProbeResult, Rule, RulesRevision, ThreatCategory,
        UpdateRule,
    };
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use std::sync::Mutex;

    #[derive(Default)]
    struct TestStore {
        revision: Mutex<RulesRevision>,
        rules: Mutex<Vec<Rule>>,
        get_rules_calls: Mutex<usize>,
        get_revision_calls: Mutex<usize>,
    }

    impl TestStore {
        fn bump(&self, label: &str) {
            let mut r = self.revision.lock().unwrap();
            r.count += 1;
            r.max_updated_at = label.into();
        }

        fn get_rules_count(&self) -> usize {
            *self.get_rules_calls.lock().unwrap()
        }

        fn get_revision_count(&self) -> usize {
            *self.get_revision_calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl Storage for TestStore {
        async fn migrate(&self) -> Result<()> {
            Ok(())
        }
        async fn get_rules(&self, _active_only: bool) -> Result<Vec<Rule>> {
            *self.get_rules_calls.lock().unwrap() += 1;
            Ok(self.rules.lock().unwrap().clone())
        }
        async fn get_rules_revision(&self, _active_only: bool) -> Result<RulesRevision> {
            *self.get_revision_calls.lock().unwrap() += 1;
            Ok(self.revision.lock().unwrap().clone())
        }
        async fn get_rule(&self, _id: &str) -> Result<Option<Rule>> {
            Ok(None)
        }
        async fn create_rule(&self, _rule: CreateRule) -> Result<Rule> {
            unimplemented!()
        }
        async fn update_rule(&self, _id: &str, _update: UpdateRule) -> Result<()> {
            Ok(())
        }
        async fn delete_rule(&self, _id: &str) -> Result<()> {
            Ok(())
        }
        async fn get_threat_categories(&self) -> Result<Vec<ThreatCategory>> {
            Ok(vec![])
        }
        async fn get_threat_category(&self, _id: &str) -> Result<Option<ThreatCategory>> {
            Ok(None)
        }
        async fn record_feedback(&self, _event: FeedbackEvent) -> Result<()> {
            Ok(())
        }
        async fn get_feedback(&self, _filters: FeedbackFilters) -> Result<Vec<FeedbackEvent>> {
            Ok(vec![])
        }
        async fn record_scores(&self, _scores: GovernanceScores) -> Result<()> {
            Ok(())
        }
        async fn get_scores(
            &self,
            _project_id: Option<&str>,
            _since: DateTime<Utc>,
        ) -> Result<Vec<GovernanceScores>> {
            Ok(vec![])
        }
        async fn record_pipeline_run(&self, _run: PipelineRun) -> Result<()> {
            Ok(())
        }
        async fn get_pipeline_runs(
            &self,
            _project_id: Option<&str>,
            _limit: Option<i64>,
        ) -> Result<Vec<PipelineRun>> {
            Ok(vec![])
        }
        async fn record_probe(&self, _result: ProbeResult) -> Result<()> {
            Ok(())
        }
        async fn get_probes(&self, _rule_id: &str) -> Result<Vec<ProbeResult>> {
            Ok(vec![])
        }
        async fn get_governance_events(
            &self,
            _filters: GovernanceEventFilters,
        ) -> Result<Vec<GovernanceEvent>> {
            Ok(vec![])
        }
        async fn get_governance_event(&self, _id: &str) -> Result<Option<GovernanceEvent>> {
            Ok(None)
        }
        async fn create_governance_event(
            &self,
            _ev: crate::core::storage::CreateGovernanceEvent,
        ) -> Result<GovernanceEvent> {
            unimplemented!()
        }
        async fn resolve_governance_event(&self, _id: &str, _notes: Option<String>) -> Result<()> {
            Ok(())
        }
    }

    fn sample_rule(id: &str) -> Rule {
        let now = Utc::now();
        Rule {
            id: id.into(),
            description: format!("rule {id}"),
            category_id: "general".into(),
            tools: vec![],
            condition_type: "regex".into(),
            condition_value: "(?i)test".into(),
            lifecycle: Lifecycle::Active,
            alpha: 1,
            beta: 1,
            prior_alpha: 1,
            prior_beta: 1,
            enabled: true,
            project_id: None,
            created_at: now,
            updated_at: now,
            crystallized_at: None,
            cross_project_validated: false,
        }
    }

    #[tokio::test]
    async fn first_call_warms_cache() {
        let store = TestStore::default();
        let cache = EngineCache::new();
        assert!(!cache.is_warm().await);

        let _e = cache.get_or_refresh(&store).await.unwrap();
        assert!(cache.is_warm().await);
        assert_eq!(store.get_revision_count(), 1);
        assert_eq!(store.get_rules_count(), 1);
    }

    #[tokio::test]
    async fn second_call_with_unchanged_revision_skips_get_rules() {
        let store = TestStore::default();
        let cache = EngineCache::new();

        let e1 = cache.get_or_refresh(&store).await.unwrap();
        let e2 = cache.get_or_refresh(&store).await.unwrap();
        assert!(Arc::ptr_eq(&e1, &e2), "cache should hand back the same Arc");
        assert_eq!(store.get_revision_count(), 2);
        assert_eq!(store.get_rules_count(), 1, "no second fetch on hit");
    }

    #[tokio::test]
    async fn revision_change_triggers_rebuild() {
        let store = TestStore::default();
        store.rules.lock().unwrap().push(sample_rule("r1"));
        let cache = EngineCache::new();

        let e1 = cache.get_or_refresh(&store).await.unwrap();
        store.rules.lock().unwrap().push(sample_rule("r2"));
        store.bump("after-r2");
        let e2 = cache.get_or_refresh(&store).await.unwrap();

        assert!(
            !Arc::ptr_eq(&e1, &e2),
            "engine must be rebuilt on revision change"
        );
        assert_eq!(store.get_rules_count(), 2);
    }

    #[tokio::test]
    async fn manual_invalidate_forces_rebuild() {
        let store = TestStore::default();
        let cache = EngineCache::new();

        cache.get_or_refresh(&store).await.unwrap();
        cache.invalidate().await;
        cache.get_or_refresh(&store).await.unwrap();

        assert_eq!(
            store.get_rules_count(),
            2,
            "invalidate should drop the cached engine"
        );
    }

    #[tokio::test]
    async fn concurrent_misses_only_fetch_once() {
        // Three concurrent first calls into an empty cache should produce
        // exactly one rules fetch — the double-check inside the write lock
        // is what guards this.
        let store = Arc::new(TestStore::default());
        let cache = Arc::new(EngineCache::new());

        let mut handles = Vec::new();
        for _ in 0..3 {
            let store = Arc::clone(&store);
            let cache = Arc::clone(&cache);
            handles.push(tokio::spawn(async move {
                cache.get_or_refresh(&*store).await.unwrap()
            }));
        }
        let mut results: Vec<Arc<InterceptEngine>> = Vec::with_capacity(handles.len());
        for h in handles {
            results.push(h.await.unwrap());
        }

        // All three handlers got the same compiled engine.
        for w in results.windows(2) {
            assert!(Arc::ptr_eq(&w[0], &w[1]));
        }
        assert_eq!(store.get_rules_count(), 1);
    }
}
