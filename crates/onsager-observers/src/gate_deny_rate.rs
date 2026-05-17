//! Gate deny-rate observer — ported from the Ising
//! `GateDenyRateAnalyzer` (issue #62) to the substrate Observer trait.
//!
//! Sibling of [`gate_override`](crate::gate_override): same input
//! stream, but only counts `Deny` verdicts (not `Escalate`). The
//! distinction matters for downstream rule action — a high deny rate
//! suggests the gate should be *relaxed* (Rewrite), while a high
//! override rate suggests the gate should be *reviewed* (Retire).
//!
//! Like its sibling, this observer keeps an artifact-id → kind index
//! and a window-bounded verdict buffer, throttles emissions per kind
//! via [`GateDenyRateConfig::emit_cooldown`], and reuses the same
//! evidence shape.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use onsager_artifact::{ArtifactId, Kind};
use onsager_spine::factory_event::{FactoryEventKind, VerdictSummary};

use crate::observer::{Observer, SpineEvent};
use crate::output::{Insight, ObserverOutput};
use crate::pattern::EventPattern;

/// Stable insight tag emitted on the spine. Matches the legacy Ising
/// `signal_kind`.
pub const TAG: &str = "gate_deny_rate";

/// Tunable knobs for [`GateDenyRateObserver`].
#[derive(Debug, Clone)]
pub struct GateDenyRateConfig {
    pub window: Duration,
    /// Minimum verdicts in the window before a rate is computed.
    pub min_samples: usize,
    /// Deny rate at or above which the kind is flagged. Default 0.40
    /// mirrors the spec's worked example (`>40% over last 20 PRs`).
    pub min_deny_rate: f64,
    /// Minimum time between re-emissions for the same kind.
    pub emit_cooldown: Duration,
}

impl Default for GateDenyRateConfig {
    fn default() -> Self {
        Self {
            window: Duration::days(7),
            min_samples: 20,
            min_deny_rate: 0.40,
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

/// Observer port of `ising::analyzers::GateDenyRateAnalyzer`.
pub struct GateDenyRateObserver {
    config: GateDenyRateConfig,
    artifacts: HashMap<String, Kind>,
    verdicts: Vec<VerdictRecord>,
    last_emitted: HashMap<Kind, DateTime<Utc>>,
}

impl GateDenyRateObserver {
    pub fn new(config: GateDenyRateConfig) -> Self {
        Self {
            config,
            artifacts: HashMap::new(),
            verdicts: Vec::new(),
            last_emitted: HashMap::new(),
        }
    }
}

impl Default for GateDenyRateObserver {
    fn default() -> Self {
        Self::new(GateDenyRateConfig::default())
    }
}

#[async_trait]
impl Observer for GateDenyRateObserver {
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

impl GateDenyRateObserver {
    fn prune_old(&mut self) {
        let cutoff = Utc::now() - self.config.window;
        self.verdicts.retain(|r| r.recorded_at >= cutoff);
    }

    fn evaluate(&mut self) -> Vec<ObserverOutput> {
        let now = Utc::now();
        let cutoff = now - self.config.window;
        let mut buckets: HashMap<Kind, Vec<&VerdictRecord>> = HashMap::new();
        for r in &self.verdicts {
            if r.recorded_at < cutoff {
                continue;
            }
            let Some(kind) = self.artifacts.get(r.artifact_id.as_str()) else {
                continue;
            };
            buckets.entry(kind.clone()).or_default().push(r);
        }

        let mut outputs = Vec::new();
        for (kind, records) in buckets {
            let total = records.len();
            if total < self.config.min_samples {
                continue;
            }
            let denies = records
                .iter()
                .filter(|r| matches!(r.verdict, VerdictSummary::Deny))
                .count();
            let rate = denies as f64 / total as f64;
            if rate < self.config.min_deny_rate {
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
                .filter(|r| matches!(r.verdict, VerdictSummary::Deny))
                .map(|r| r.event_id)
                .collect();
            evidence_ids.sort_unstable_by(|a, b| b.cmp(a));
            evidence_ids.truncate(5);

            // Confidence: 0.6 at the threshold, climbing toward 0.95
            // as the deny rate doubles past the threshold.
            let denom = self.config.min_deny_rate.max(0.01);
            let excess = ((rate - self.config.min_deny_rate) / denom).max(0.0);
            let confidence = (0.6 + excess * 0.3).min(0.95);

            self.last_emitted.insert(kind.clone(), now);

            outputs.push(ObserverOutput::Insight(Insight {
                observation: format!(
                    "{kind_label}: {denies}/{total} ({:.0}%) verdicts denied over the last \
                     {} day(s) — the gate may be too strict; review the rule before it \
                     becomes friction overhead",
                    rate * 100.0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use onsager_artifact::{ArtifactId, Kind};
    use onsager_spine::{FactoryEvent, factory_event::GatePoint};

    fn registered(event_id: i64, id: &ArtifactId, kind: Kind) -> SpineEvent {
        SpineEvent {
            event_id,
            event_type: "artifact.registered".into(),
            payload: FactoryEvent {
                event: FactoryEventKind::ArtifactRegistered {
                    artifact_id: id.clone(),
                    kind,
                    name: "x".into(),
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

    fn verdict(event_id: i64, id: &ArtifactId, v: VerdictSummary) -> SpineEvent {
        SpineEvent {
            event_id,
            event_type: "forge.gate_verdict".into(),
            payload: FactoryEvent {
                event: FactoryEventKind::ForgeGateVerdict {
                    artifact_id: id.clone(),
                    gate_point: GatePoint::StateTransition,
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

    async fn feed(obs: &mut GateDenyRateObserver, events: &[SpineEvent]) -> Vec<ObserverOutput> {
        let mut all = Vec::new();
        for ev in events {
            all.extend(obs.on_event(ev).await);
        }
        all
    }

    #[tokio::test]
    async fn fires_when_deny_rate_exceeds_threshold() {
        let mut obs = GateDenyRateObserver::default();
        let id = ArtifactId::new("art_code");
        let mut events = vec![registered(1, &id, Kind::Code)];
        for i in 0..10 {
            events.push(verdict(i + 2, &id, VerdictSummary::Deny));
        }
        for i in 0..10 {
            events.push(verdict(i + 12, &id, VerdictSummary::Allow));
        }
        // 10/20 = 50% > 40% threshold, exactly 20 samples = at threshold.
        let outputs = feed(&mut obs, &events).await;
        assert_eq!(outputs.len(), 1, "got {:?}", outputs);
        match &outputs[0] {
            ObserverOutput::Insight(i) => {
                assert_eq!(i.tag.as_deref(), Some(TAG));
                assert_eq!(i.evidence_event_ids.len(), 5);
            }
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn does_not_fire_below_threshold() {
        let mut obs = GateDenyRateObserver::default();
        let id = ArtifactId::new("art_code");
        let mut events = vec![registered(1, &id, Kind::Code)];
        for i in 0..4 {
            events.push(verdict(i + 2, &id, VerdictSummary::Deny));
        }
        for i in 0..16 {
            events.push(verdict(i + 6, &id, VerdictSummary::Allow));
        }
        let outputs = feed(&mut obs, &events).await;
        assert!(outputs.is_empty(), "got {:?}", outputs);
    }

    #[tokio::test]
    async fn does_not_fire_below_min_samples() {
        let mut obs = GateDenyRateObserver::default();
        let id = ArtifactId::new("art_code");
        let mut events = vec![registered(1, &id, Kind::Code)];
        for i in 0..5 {
            events.push(verdict(i + 2, &id, VerdictSummary::Deny));
        }
        let outputs = feed(&mut obs, &events).await;
        assert!(outputs.is_empty(), "got {:?}", outputs);
    }

    #[tokio::test]
    async fn escalate_verdicts_do_not_count() {
        // Sibling test to `gate_override`: this observer counts only
        // Deny, so a stream of Escalates must NOT trip the rate.
        let mut obs = GateDenyRateObserver::default();
        let id = ArtifactId::new("art_code");
        let mut events = vec![registered(1, &id, Kind::Code)];
        for i in 0..20 {
            events.push(verdict(i + 2, &id, VerdictSummary::Escalate));
        }
        let outputs = feed(&mut obs, &events).await;
        assert!(outputs.is_empty(), "escalate is not deny");
    }
}
