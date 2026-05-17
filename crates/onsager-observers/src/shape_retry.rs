//! Shape retry-spike observer — ported from the Ising
//! `ShapeRetrySpikeAnalyzer` (issue #36 follow-up) to the substrate
//! Observer trait.
//!
//! Looks at the average shaping attempts per artifact within a
//! sliding window, grouped by artifact `Kind`. A kind whose artifacts
//! systematically need many reshape cycles is one whose decomposer or
//! shaping rules are underspecified — the right downstream response
//! is a Synodic rule that caps rework or tightens the spec.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use onsager_artifact::{ArtifactId, Kind};
use onsager_spine::factory_event::FactoryEventKind;

use crate::observer::{Observer, SpineEvent};
use crate::output::{Insight, ObserverOutput};
use crate::pattern::EventPattern;

/// Stable insight tag emitted on the spine. Matches the legacy Ising
/// `signal_kind`.
pub const TAG: &str = "shape_retry_spike";

/// Tunable knobs for [`ShapeRetryObserver`].
#[derive(Debug, Clone)]
pub struct ShapeRetryConfig {
    /// Lookback window over which shaping records are considered.
    pub window: Duration,
    /// Minimum distinct artifacts of a kind required before a rate
    /// is computed.
    pub min_artifacts: usize,
    /// Average shaping attempts per artifact above which the kind is
    /// flagged.
    pub min_avg_shapings: f64,
    /// Minimum time between re-emissions for the same kind.
    pub emit_cooldown: Duration,
}

impl Default for ShapeRetryConfig {
    fn default() -> Self {
        Self {
            window: Duration::days(7),
            min_artifacts: 3,
            // 4 reshapes/artifact on average is generous — Forge's
            // default budget is small, so anything north of this is
            // real friction.
            min_avg_shapings: 4.0,
            emit_cooldown: Duration::minutes(5),
        }
    }
}

#[derive(Debug, Clone)]
struct ShapingRecord {
    event_id: i64,
    artifact_id: ArtifactId,
    recorded_at: DateTime<Utc>,
}

/// Observer port of `ising::analyzers::ShapeRetrySpikeAnalyzer`.
pub struct ShapeRetryObserver {
    config: ShapeRetryConfig,
    artifacts: HashMap<String, Kind>,
    shapings: Vec<ShapingRecord>,
    last_emitted: HashMap<Kind, DateTime<Utc>>,
}

impl ShapeRetryObserver {
    pub fn new(config: ShapeRetryConfig) -> Self {
        Self {
            config,
            artifacts: HashMap::new(),
            shapings: Vec::new(),
            last_emitted: HashMap::new(),
        }
    }
}

impl Default for ShapeRetryObserver {
    fn default() -> Self {
        Self::new(ShapeRetryConfig::default())
    }
}

#[async_trait]
impl Observer for ShapeRetryObserver {
    fn subscriptions(&self) -> Vec<EventPattern> {
        vec![
            EventPattern::new("artifact.registered"),
            EventPattern::new("artifact.archived"),
            EventPattern::new("forge.shaping_returned"),
        ]
    }

    async fn on_event(&mut self, event: &SpineEvent) -> Vec<ObserverOutput> {
        match &event.payload.event {
            FactoryEventKind::ArtifactRegistered {
                artifact_id, kind, ..
            } => {
                self.artifacts
                    .insert(artifact_id.as_str().to_owned(), kind.clone());
                Vec::new()
            }
            FactoryEventKind::ArtifactArchived { artifact_id, .. } => {
                self.artifacts.remove(artifact_id.as_str());
                Vec::new()
            }
            FactoryEventKind::ForgeShapingReturned {
                artifact_id,
                outcome: _,
                ..
            } => {
                // All outcomes (Completed/Failed/Partial/Aborted)
                // count toward the per-artifact attempt total — the
                // signal is "had to reshape", regardless of result.
                self.shapings.push(ShapingRecord {
                    event_id: event.event_id,
                    artifact_id: artifact_id.clone(),
                    recorded_at: event.created_at,
                });
                self.prune_old();
                self.evaluate()
            }
            _ => Vec::new(),
        }
    }
}

impl ShapeRetryObserver {
    fn prune_old(&mut self) {
        let cutoff = Utc::now() - self.config.window;
        self.shapings.retain(|r| r.recorded_at >= cutoff);
    }

    fn evaluate(&mut self) -> Vec<ObserverOutput> {
        let now = Utc::now();
        let cutoff = now - self.config.window;
        // kind → artifact_id → records
        let mut buckets: HashMap<Kind, HashMap<String, Vec<&ShapingRecord>>> = HashMap::new();
        for r in &self.shapings {
            if r.recorded_at < cutoff {
                continue;
            }
            let Some(kind) = self.artifacts.get(r.artifact_id.as_str()) else {
                continue;
            };
            buckets
                .entry(kind.clone())
                .or_default()
                .entry(r.artifact_id.as_str().to_owned())
                .or_default()
                .push(r);
        }

        let mut outputs = Vec::new();
        for (kind, by_artifact) in buckets {
            let artifact_count = by_artifact.len();
            if artifact_count < self.config.min_artifacts {
                continue;
            }
            let total_shapings: usize = by_artifact.values().map(|v| v.len()).sum();
            let avg = total_shapings as f64 / artifact_count as f64;
            if avg < self.config.min_avg_shapings {
                continue;
            }
            if let Some(last) = self.last_emitted.get(&kind)
                && now - *last < self.config.emit_cooldown
            {
                continue;
            }

            let mut evidence: Vec<i64> = by_artifact
                .values()
                .flat_map(|recs| recs.iter().map(|r| r.event_id))
                .collect();
            evidence.sort_unstable_by(|a, b| b.cmp(a));
            evidence.truncate(5);

            let denom = self.config.min_avg_shapings.max(1.0);
            let excess = ((avg - self.config.min_avg_shapings) / denom).max(0.0);
            let confidence = (0.6 + excess * 0.3).min(0.95);

            let kind_label = kind.to_string();
            self.last_emitted.insert(kind.clone(), now);

            outputs.push(ObserverOutput::Insight(Insight {
                observation: format!(
                    "{} artifacts: avg {:.1} shaping attempts per artifact across {} \
                     artifacts in the last {} day(s) — decomposition or shaping rules \
                     for this kind may be underspecified",
                    kind_label,
                    avg,
                    artifact_count,
                    self.config.window.num_days(),
                ),
                confidence,
                evidence_event_ids: evidence,
                about_artifact_id: None,
                tag: Some(TAG.to_string()),
            }));
        }
        outputs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use onsager_artifact::{ArtifactId, Kind};
    use onsager_spine::{FactoryEvent, factory_event::ShapingOutcome};

    fn registered(event_id: i64, id: &ArtifactId, kind: Kind) -> SpineEvent {
        SpineEvent {
            event_id,
            event_type: "artifact.registered".into(),
            payload: FactoryEvent {
                event: FactoryEventKind::ArtifactRegistered {
                    artifact_id: id.clone(),
                    kind,
                    name: "t".into(),
                    owner: "marvin".into(),
                },
                correlation_id: None,
                causation_id: None,
                actor: "test".into(),
                timestamp: Utc::now(),
            },
            created_at: Utc::now(),
        }
    }

    fn shaping(event_id: i64, id: &ArtifactId) -> SpineEvent {
        SpineEvent {
            event_id,
            event_type: "forge.shaping_returned".into(),
            payload: FactoryEvent {
                event: FactoryEventKind::ForgeShapingReturned {
                    request_id: format!("req_{event_id}"),
                    artifact_id: id.clone(),
                    outcome: ShapingOutcome::Partial,
                },
                correlation_id: None,
                causation_id: None,
                actor: "test".into(),
                timestamp: Utc::now(),
            },
            created_at: Utc::now(),
        }
    }

    async fn feed(obs: &mut ShapeRetryObserver, events: &[SpineEvent]) -> Vec<ObserverOutput> {
        let mut all = Vec::new();
        for ev in events {
            all.extend(obs.on_event(ev).await);
        }
        all
    }

    #[tokio::test]
    async fn emits_when_avg_shapings_exceeds_threshold() {
        let mut obs = ShapeRetryObserver::default();
        let mut events = Vec::new();
        let mut seq = 1i64;
        for n in 0..3 {
            let id = ArtifactId::new(format!("art_code_{n}"));
            events.push(registered(seq, &id, Kind::Code));
            seq += 1;
            for _ in 0..5 {
                events.push(shaping(seq, &id));
                seq += 1;
            }
        }

        let outputs = feed(&mut obs, &events).await;
        // Insights accumulate as each new shaping event re-evaluates;
        // we only need at least one to confirm detection.
        assert!(!outputs.is_empty(), "expected at least one insight");
        let last = outputs.last().unwrap();
        match last {
            ObserverOutput::Insight(i) => {
                assert_eq!(i.tag.as_deref(), Some(TAG));
                assert!(i.observation.contains("shaping attempts"));
                assert!(!i.evidence_event_ids.is_empty());
                assert!(i.evidence_event_ids.len() <= 5);
            }
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn no_emission_below_avg_threshold() {
        let mut obs = ShapeRetryObserver::default();
        let mut events = Vec::new();
        let mut seq = 1i64;
        for n in 0..3 {
            let id = ArtifactId::new(format!("art_code_{n}"));
            events.push(registered(seq, &id, Kind::Code));
            seq += 1;
            for _ in 0..2 {
                events.push(shaping(seq, &id));
                seq += 1;
            }
        }
        let outputs = feed(&mut obs, &events).await;
        assert!(outputs.is_empty(), "got {:?}", outputs);
    }

    #[tokio::test]
    async fn no_emission_below_min_artifacts() {
        let mut obs = ShapeRetryObserver::default();
        let id = ArtifactId::new("art_code_0");
        let mut events = vec![registered(1, &id, Kind::Code)];
        for i in 0..20 {
            events.push(shaping(i + 2, &id));
        }
        let outputs = feed(&mut obs, &events).await;
        assert!(outputs.is_empty(), "one artifact must not trip a kind");
    }

    #[tokio::test]
    async fn cooldown_throttles_repeat_emissions() {
        let mut obs = ShapeRetryObserver::new(ShapeRetryConfig {
            emit_cooldown: Duration::hours(1),
            ..ShapeRetryConfig::default()
        });
        let mut events = Vec::new();
        let mut seq = 1i64;
        for n in 0..3 {
            let id = ArtifactId::new(format!("art_code_{n}"));
            events.push(registered(seq, &id, Kind::Code));
            seq += 1;
            for _ in 0..5 {
                events.push(shaping(seq, &id));
                seq += 1;
            }
        }
        let outputs = feed(&mut obs, &events).await;
        // Within a one-hour cooldown only the first crossing emits;
        // subsequent shapings of the same kind are suppressed.
        assert_eq!(outputs.len(), 1, "got {:?}", outputs);
    }
}
