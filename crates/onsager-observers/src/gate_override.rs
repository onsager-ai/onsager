//! Gate override-rate observer — ported from the Ising
//! `GateOverrideAnalyzer` (issue #36) to the substrate Observer trait.
//!
//! Flags artifact kinds whose gate verdicts cross a deny+escalate
//! ratio over a sliding window. These are the kinds where policy
//! friction is loudest — prime candidates for rule review, rewording,
//! or retirement.
//!
//! The original analyzer ran on a tick-based pass over a rebuilt
//! `FactoryModel`. The observer port keeps the same detection logic
//! but accumulates state event-by-event: an artifact-id → kind index
//! and a window-bounded verdict buffer. Emission is throttled per kind
//! via [`GateOverrideConfig::emit_cooldown`] so a burst of verdicts
//! does not flood `observer_outputs`.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use onsager_artifact::{ArtifactId, Kind};
use onsager_spine::factory_event::{FactoryEventKind, VerdictSummary};

use crate::observer::{Observer, SpineEvent};
use crate::output::{Insight, ObserverOutput};
use crate::pattern::EventPattern;

/// Stable insight tag emitted on the spine for this observer's output.
/// Matches the legacy Ising `signal_kind` so existing dashboard
/// filters keep working.
pub const TAG: &str = "repeated_gate_override";

/// Tunable knobs for [`GateOverrideObserver`].
#[derive(Debug, Clone)]
pub struct GateOverrideConfig {
    /// How far back to look for verdicts when computing the rate.
    pub window: Duration,
    /// Minimum verdicts per kind before a rate is computed.
    pub min_samples: usize,
    /// Override ratio that must be crossed before emitting.
    pub threshold: f64,
    /// Minimum time between re-emissions for the same kind. Without
    /// this, every additional verdict after the threshold is crossed
    /// would emit a fresh insight.
    pub emit_cooldown: Duration,
}

impl Default for GateOverrideConfig {
    fn default() -> Self {
        Self {
            window: Duration::days(7),
            min_samples: 5,
            threshold: 0.5,
            emit_cooldown: Duration::minutes(5),
        }
    }
}

#[derive(Debug, Clone)]
struct VerdictRecord {
    event_id: i64,
    artifact_id: ArtifactId,
    verdict: VerdictSummary,
    recorded_at: DateTime<Utc>,
}

/// Observer port of `ising::analyzers::GateOverrideAnalyzer`.
pub struct GateOverrideObserver {
    config: GateOverrideConfig,
    /// Live artifact-id → kind index. Archival removes entries to
    /// match the original `FactoryModel.artifacts` behavior; verdicts
    /// whose artifact has since been archived drop out of grouping.
    artifacts: HashMap<String, Kind>,
    /// Verdict records, pruned to the configured window on every
    /// verdict event.
    verdicts: Vec<VerdictRecord>,
    /// Last emit time per kind; throttle re-emission inside the
    /// cooldown window.
    last_emitted: HashMap<Kind, DateTime<Utc>>,
}

impl GateOverrideObserver {
    pub fn new(config: GateOverrideConfig) -> Self {
        Self {
            config,
            artifacts: HashMap::new(),
            verdicts: Vec::new(),
            last_emitted: HashMap::new(),
        }
    }
}

impl Default for GateOverrideObserver {
    fn default() -> Self {
        Self::new(GateOverrideConfig::default())
    }
}

#[async_trait]
impl Observer for GateOverrideObserver {
    fn subscriptions(&self) -> Vec<EventPattern> {
        vec![
            EventPattern::new("artifact.registered"),
            EventPattern::new("artifact.archived"),
            EventPattern::new("forge.gate_verdict"),
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
            FactoryEventKind::ForgeGateVerdict {
                artifact_id,
                verdict,
                ..
            } => {
                self.verdicts.push(VerdictRecord {
                    event_id: event.event_id,
                    artifact_id: artifact_id.clone(),
                    verdict: verdict.clone(),
                    recorded_at: event.created_at,
                });
                self.prune_old();
                self.evaluate()
            }
            _ => Vec::new(),
        }
    }
}

impl GateOverrideObserver {
    fn prune_old(&mut self) {
        let cutoff = Utc::now() - self.config.window;
        self.verdicts.retain(|r| r.recorded_at >= cutoff);
    }

    fn evaluate(&mut self) -> Vec<ObserverOutput> {
        let now = Utc::now();
        let cutoff = now - self.config.window;
        let mut buckets: HashMap<Kind, Vec<&VerdictRecord>> = HashMap::new();
        for record in &self.verdicts {
            if record.recorded_at < cutoff {
                continue;
            }
            let Some(kind) = self.artifacts.get(record.artifact_id.as_str()) else {
                continue;
            };
            buckets.entry(kind.clone()).or_default().push(record);
        }

        let mut outputs = Vec::new();
        for (kind, records) in buckets {
            if records.len() < self.config.min_samples {
                continue;
            }
            let total = records.len();
            let overrides = records
                .iter()
                .filter(|r| matches!(r.verdict, VerdictSummary::Deny | VerdictSummary::Escalate))
                .count();
            let rate = overrides as f64 / total as f64;
            if rate < self.config.threshold {
                continue;
            }
            if let Some(last) = self.last_emitted.get(&kind)
                && now - *last < self.config.emit_cooldown
            {
                continue;
            }

            let kind_label = kind.to_string();
            let mut evidence_ids: Vec<i64> = records
                .iter()
                .filter(|r| matches!(r.verdict, VerdictSummary::Deny | VerdictSummary::Escalate))
                .map(|r| r.event_id)
                .collect();
            evidence_ids.sort_unstable_by(|a, b| b.cmp(a));
            evidence_ids.truncate(5);

            // Confidence: 0.5 at the threshold, climbing toward 0.95 as
            // the override ratio approaches 1.0. Mirrors the Ising
            // analyzer's shape so dashboard sorting stays comparable.
            let range = (1.0 - self.config.threshold).max(f64::EPSILON);
            let excess = (rate - self.config.threshold).max(0.0) / range;
            let confidence = (0.5 + excess * 0.4).min(0.95);

            self.last_emitted.insert(kind.clone(), now);

            outputs.push(ObserverOutput::Insight(Insight {
                observation: format!(
                    "{} artifacts: {:.0}% gate-override rate over {} verdicts in the \
                     last {} day(s) — rules governing this kind may need review",
                    kind_label,
                    rate * 100.0,
                    total,
                    self.config.window.num_days(),
                ),
                confidence,
                evidence_event_ids: evidence_ids,
                about_artifact_id: None,
                tag: Some(TAG.to_string()),
            }));
        }
        outputs
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use onsager_artifact::{ArtifactId, ArtifactState, Kind};
    use onsager_spine::{FactoryEvent, factory_event::GatePoint};

    fn registered_event(event_id: i64, id: &ArtifactId, kind: Kind) -> SpineEvent {
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

    fn verdict_event(event_id: i64, id: &ArtifactId, v: VerdictSummary) -> SpineEvent {
        SpineEvent {
            event_id,
            event_type: "forge.gate_verdict".into(),
            payload: FactoryEvent {
                event: FactoryEventKind::ForgeGateVerdict {
                    artifact_id: id.clone(),
                    gate_point: GatePoint::PreDispatch,
                    verdict: v,
                },
                correlation_id: None,
                causation_id: None,
                actor: "test".into(),
                timestamp: Utc::now(),
            },
            created_at: Utc::now(),
        }
    }

    fn archived_event(event_id: i64, id: &ArtifactId) -> SpineEvent {
        SpineEvent {
            event_id,
            event_type: "artifact.archived".into(),
            payload: FactoryEvent {
                event: FactoryEventKind::ArtifactArchived {
                    artifact_id: id.clone(),
                    reason: "test".into(),
                },
                correlation_id: None,
                causation_id: None,
                actor: "test".into(),
                timestamp: Utc::now(),
            },
            created_at: Utc::now(),
        }
    }

    async fn feed(obs: &mut GateOverrideObserver, events: &[SpineEvent]) -> Vec<ObserverOutput> {
        let mut all = Vec::new();
        for ev in events {
            all.extend(obs.on_event(ev).await);
        }
        all
    }

    #[tokio::test]
    async fn emits_when_override_rate_exceeds_threshold() {
        let mut obs = GateOverrideObserver::default();
        let id = ArtifactId::new("art_c");
        let mut events = vec![registered_event(1, &id, Kind::Code)];
        for (i, v) in [
            VerdictSummary::Deny,
            VerdictSummary::Deny,
            VerdictSummary::Deny,
            VerdictSummary::Deny,
            VerdictSummary::Allow,
        ]
        .iter()
        .enumerate()
        {
            events.push(verdict_event(10 + i as i64, &id, v.clone()));
        }

        let outputs = feed(&mut obs, &events).await;
        // Only one Insight should emerge — only the last verdict (the
        // fifth in the window) crosses min_samples=5 and the rate is
        // 4/5 = 80% > 50%.
        assert_eq!(outputs.len(), 1, "got {:?}", outputs);
        match &outputs[0] {
            ObserverOutput::Insight(i) => {
                assert_eq!(i.tag.as_deref(), Some(TAG));
                assert!(i.confidence >= 0.5 && i.confidence <= 0.95);
                assert!(i.observation.contains("gate-override"));
                assert!(!i.evidence_event_ids.is_empty());
            }
            _ => panic!("expected Insight, got {:?}", outputs[0]),
        }
    }

    #[tokio::test]
    async fn no_emission_below_threshold() {
        let mut obs = GateOverrideObserver::default();
        let id = ArtifactId::new("art_c");
        let mut events = vec![registered_event(1, &id, Kind::Code)];
        // 1 deny + 4 allow = 20% < 50% threshold.
        for (i, v) in [
            VerdictSummary::Deny,
            VerdictSummary::Allow,
            VerdictSummary::Allow,
            VerdictSummary::Allow,
            VerdictSummary::Allow,
        ]
        .iter()
        .enumerate()
        {
            events.push(verdict_event(10 + i as i64, &id, v.clone()));
        }

        let outputs = feed(&mut obs, &events).await;
        assert!(outputs.is_empty(), "got {:?}", outputs);
    }

    #[tokio::test]
    async fn no_emission_below_min_samples() {
        let mut obs = GateOverrideObserver::default();
        let id = ArtifactId::new("art_c");
        let events = vec![
            registered_event(1, &id, Kind::Code),
            verdict_event(10, &id, VerdictSummary::Deny),
            verdict_event(11, &id, VerdictSummary::Deny),
        ];

        let outputs = feed(&mut obs, &events).await;
        assert!(outputs.is_empty(), "got {:?}", outputs);
    }

    #[tokio::test]
    async fn cooldown_throttles_repeat_emissions() {
        let mut obs = GateOverrideObserver::new(GateOverrideConfig {
            emit_cooldown: Duration::hours(1),
            ..GateOverrideConfig::default()
        });
        let id = ArtifactId::new("art_c");
        let mut events = vec![registered_event(1, &id, Kind::Code)];
        for i in 0..5 {
            events.push(verdict_event(10 + i, &id, VerdictSummary::Deny));
        }
        let first = feed(&mut obs, &events).await;
        assert_eq!(first.len(), 1, "first burst emits once");

        // Six more denies in the same hot window should not re-emit.
        let more: Vec<SpineEvent> = (0..6)
            .map(|i| verdict_event(20 + i, &id, VerdictSummary::Deny))
            .collect();
        let second = feed(&mut obs, &more).await;
        assert!(second.is_empty(), "cooldown should suppress repeats");
    }

    #[tokio::test]
    async fn archived_artifact_drops_from_kind_grouping() {
        // Mirrors the original FactoryModel behavior: once an artifact
        // is archived, its verdicts no longer contribute to its kind's
        // rate. The cooldown is wound back so the first emission is
        // not blocked.
        let mut obs = GateOverrideObserver::default();
        let id = ArtifactId::new("art_c");
        let mut events = vec![registered_event(1, &id, Kind::Code)];
        for i in 0..4 {
            events.push(verdict_event(10 + i, &id, VerdictSummary::Deny));
        }
        events.push(archived_event(100, &id));
        // One more deny after archival — should not emit because the
        // kind lookup now fails for the entire history.
        events.push(verdict_event(101, &id, VerdictSummary::Deny));
        let outputs = feed(&mut obs, &events).await;
        assert!(outputs.is_empty(), "got {:?}", outputs);
    }

    #[tokio::test]
    async fn ignores_unrelated_payload_variants() {
        // The runtime fan-out's pattern match is permissive — the
        // observer should silently drop any unrelated event types it
        // happens to receive (e.g. through `*` wildcards on a sibling
        // observer that shares this one's mailbox).
        let mut obs = GateOverrideObserver::default();
        let id = ArtifactId::new("art_c");
        let unrelated = SpineEvent {
            event_id: 5,
            event_type: "artifact.state_changed".into(),
            payload: FactoryEvent {
                event: FactoryEventKind::ArtifactStateChanged {
                    artifact_id: id.clone(),
                    from_state: ArtifactState::Draft,
                    to_state: ArtifactState::InProgress,
                },
                correlation_id: None,
                causation_id: None,
                actor: "test".into(),
                timestamp: Utc::now(),
            },
            created_at: Utc::now(),
        };
        let outputs = obs.on_event(&unrelated).await;
        assert!(outputs.is_empty());
    }
}
