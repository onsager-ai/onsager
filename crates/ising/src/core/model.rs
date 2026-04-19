//! Factory model — accumulated internal representation of factory behavior
//! (ising-v0.1 §4.2).
//!
//! This is not a raw event log — it tracks:
//! - Artifact state transition histories
//! - Shaping request/result pairs with timing
//! - Gate verdict patterns per artifact kind
//! - Session duration and outcome distributions
//! - Insight history (own previous outputs)

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};

use onsager_artifact::{ArtifactId, ArtifactState, Kind};
use onsager_spine::factory_event::{FactoryEventKind, GatePoint, ShapingOutcome, VerdictSummary};

/// A tracked shaping attempt.
#[derive(Debug, Clone)]
pub struct ShapingRecord {
    /// The spine event ID that produced this record (for traceable evidence).
    pub event_id: i64,
    pub request_id: String,
    pub artifact_id: ArtifactId,
    pub outcome: ShapingOutcome,
    pub duration_ms: Option<u64>,
    pub recorded_at: DateTime<Utc>,
}

/// A tracked gate verdict emitted by Forge after consulting Synodic.
///
/// The verdict variant itself is the override signal: `Deny` / `Escalate`
/// are the "override-equivalent" outcomes that flag rule friction for the
/// gate-override-rate insight. The event doesn't carry a rule_id today, so
/// grouping happens by artifact kind (resolved via `FactoryModel.artifacts`).
#[derive(Debug, Clone)]
pub struct GateVerdictRecord {
    pub event_id: i64,
    pub artifact_id: ArtifactId,
    pub gate_point: GatePoint,
    pub verdict: VerdictSummary,
    pub recorded_at: DateTime<Utc>,
}

/// A tracked artifact state.
#[derive(Debug, Clone)]
pub struct TrackedArtifact {
    pub artifact_id: ArtifactId,
    pub kind: Kind,
    pub current_state: ArtifactState,
    pub version: u32,
    pub shaping_count: u32,
    pub first_seen: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
}

/// Accumulated factory model — the internal state Ising maintains.
#[derive(Debug)]
pub struct FactoryModel {
    /// Tracked artifacts by ID.
    pub artifacts: HashMap<String, TrackedArtifact>,
    /// Recent shaping records (bounded by retention window).
    pub shaping_records: Vec<ShapingRecord>,
    /// Recent gate verdict records (bounded by retention window).
    pub gate_verdict_records: Vec<GateVerdictRecord>,
    /// Total events processed.
    pub events_processed: u64,
    /// Last processed event ID (for catch-up).
    pub last_event_id: Option<i64>,
}

impl FactoryModel {
    pub fn new() -> Self {
        Self {
            artifacts: HashMap::new(),
            shaping_records: Vec::new(),
            gate_verdict_records: Vec::new(),
            events_processed: 0,
            last_event_id: None,
        }
    }

    /// Ingest a factory event into the model.
    pub fn ingest(&mut self, event_id: i64, event: &FactoryEventKind) {
        self.events_processed += 1;
        self.last_event_id = Some(event_id);

        match event {
            FactoryEventKind::ArtifactRegistered {
                artifact_id,
                kind,
                name: _,
                owner: _,
            } => {
                self.artifacts.insert(
                    artifact_id.as_str().to_owned(),
                    TrackedArtifact {
                        artifact_id: artifact_id.clone(),
                        kind: kind.clone(),
                        current_state: ArtifactState::Draft,
                        version: 0,
                        shaping_count: 0,
                        first_seen: Utc::now(),
                        last_updated: Utc::now(),
                    },
                );
            }

            FactoryEventKind::ArtifactStateChanged {
                artifact_id,
                to_state,
                ..
            } => {
                if let Some(tracked) = self.artifacts.get_mut(artifact_id.as_str()) {
                    tracked.current_state = *to_state;
                    tracked.last_updated = Utc::now();
                }
            }

            FactoryEventKind::ArtifactVersionCreated {
                artifact_id,
                version,
                ..
            } => {
                if let Some(tracked) = self.artifacts.get_mut(artifact_id.as_str()) {
                    tracked.version = *version;
                    tracked.last_updated = Utc::now();
                }
            }

            FactoryEventKind::ForgeShapingReturned {
                request_id,
                artifact_id,
                outcome,
            } => {
                if let Some(tracked) = self.artifacts.get_mut(artifact_id.as_str()) {
                    tracked.shaping_count += 1;
                    tracked.last_updated = Utc::now();
                }

                self.shaping_records.push(ShapingRecord {
                    event_id,
                    request_id: request_id.clone(),
                    artifact_id: artifact_id.clone(),
                    outcome: *outcome,
                    duration_ms: None,
                    recorded_at: Utc::now(),
                });
            }

            FactoryEventKind::ForgeGateVerdict {
                artifact_id,
                gate_point,
                verdict,
            } => {
                self.gate_verdict_records.push(GateVerdictRecord {
                    event_id,
                    artifact_id: artifact_id.clone(),
                    gate_point: *gate_point,
                    verdict: verdict.clone(),
                    recorded_at: Utc::now(),
                });
            }

            _ => {}
        }
    }

    /// Gate verdict records no older than `cutoff`.
    pub fn gate_verdicts_since(&self, cutoff: DateTime<Utc>) -> Vec<&GateVerdictRecord> {
        self.gate_verdict_records
            .iter()
            .filter(|r| r.recorded_at >= cutoff)
            .collect()
    }

    /// Deny+Escalate rate per artifact `Kind` over the given window, along
    /// with the count of verdicts observed. Only kinds with at least
    /// `min_samples` verdicts in the window appear in the output, so rates
    /// aren't computed from noise.
    ///
    /// `Deny` and `Escalate` are both counted as "overrides" — the signal is
    /// "rules rejecting proposed actions often enough that the policy is
    /// worth revisiting," which both verdicts evidence.
    pub fn override_rate_by_kind(
        &self,
        window: Duration,
        min_samples: usize,
    ) -> HashMap<Kind, (f64, usize, Vec<i64>)> {
        let cutoff = Utc::now() - window;
        let mut buckets: HashMap<Kind, Vec<&GateVerdictRecord>> = HashMap::new();
        for record in self.gate_verdicts_since(cutoff) {
            let Some(artifact) = self.artifacts.get(record.artifact_id.as_str()) else {
                continue;
            };
            buckets
                .entry(artifact.kind.clone())
                .or_default()
                .push(record);
        }

        buckets
            .into_iter()
            .filter_map(|(kind, records)| {
                if records.len() < min_samples {
                    return None;
                }
                let total = records.len();
                let overrides = records
                    .iter()
                    .filter(|r| {
                        matches!(r.verdict, VerdictSummary::Deny | VerdictSummary::Escalate)
                    })
                    .count();
                // Evidence event-ids: the most recent override verdicts. Ordering
                // by event_id descending matches "most recent first" (spine ids
                // are monotonic) without needing to resort by timestamp.
                let mut override_ids: Vec<i64> = records
                    .iter()
                    .filter(|r| {
                        matches!(r.verdict, VerdictSummary::Deny | VerdictSummary::Escalate)
                    })
                    .map(|r| r.event_id)
                    .collect();
                override_ids.sort_unstable_by(|a, b| b.cmp(a));
                override_ids.truncate(5);
                let rate = overrides as f64 / total as f64;
                Some((kind, (rate, total, override_ids)))
            })
            .collect()
    }

    /// Get shaping records for a specific artifact.
    pub fn shaping_history(&self, artifact_id: &ArtifactId) -> Vec<&ShapingRecord> {
        self.shaping_records
            .iter()
            .filter(|r| r.artifact_id.as_str() == artifact_id.as_str())
            .collect()
    }

    /// Get shaping records for a specific artifact kind.
    pub fn shaping_history_by_kind(
        &self,
        kind: &Kind,
    ) -> Vec<(&TrackedArtifact, Vec<&ShapingRecord>)> {
        self.artifacts
            .values()
            .filter(|a| &a.kind == kind)
            .map(|a| {
                let records = self.shaping_history(&a.artifact_id);
                (a, records)
            })
            .collect()
    }

    /// Count failure rate for a given artifact over last N shaping attempts.
    pub fn failure_rate(&self, artifact_id: &ArtifactId, last_n: usize) -> f64 {
        let history = self.shaping_history(artifact_id);
        let recent: Vec<_> = history.iter().rev().take(last_n).collect();
        if recent.is_empty() {
            return 0.0;
        }
        let failures = recent
            .iter()
            .filter(|r| matches!(r.outcome, ShapingOutcome::Failed | ShapingOutcome::Aborted))
            .count();
        failures as f64 / recent.len() as f64
    }
}

impl Default for FactoryModel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::Kind;

    #[test]
    fn ingest_artifact_registered() {
        let mut model = FactoryModel::new();
        model.ingest(
            1,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: ArtifactId::new("art_test1"),
                kind: Kind::Code,
                name: "test".into(),
                owner: "marvin".into(),
            },
        );
        assert_eq!(model.artifacts.len(), 1);
        assert_eq!(model.events_processed, 1);
    }

    #[test]
    fn ingest_state_change() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_test1");
        model.ingest(
            1,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: id.clone(),
                kind: Kind::Code,
                name: "test".into(),
                owner: "marvin".into(),
            },
        );
        model.ingest(
            2,
            &FactoryEventKind::ArtifactStateChanged {
                artifact_id: id.clone(),
                from_state: ArtifactState::Draft,
                to_state: ArtifactState::InProgress,
            },
        );
        assert_eq!(
            model.artifacts["art_test1"].current_state,
            ArtifactState::InProgress
        );
    }

    #[test]
    fn shaping_history_tracking() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_test1");
        model.ingest(
            1,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: id.clone(),
                kind: Kind::Code,
                name: "test".into(),
                owner: "marvin".into(),
            },
        );
        model.ingest(
            2,
            &FactoryEventKind::ForgeShapingReturned {
                request_id: "req_1".into(),
                artifact_id: id.clone(),
                outcome: ShapingOutcome::Completed,
            },
        );
        model.ingest(
            3,
            &FactoryEventKind::ForgeShapingReturned {
                request_id: "req_2".into(),
                artifact_id: id.clone(),
                outcome: ShapingOutcome::Failed,
            },
        );

        let history = model.shaping_history(&id);
        assert_eq!(history.len(), 2);
        assert_eq!(model.artifacts["art_test1"].shaping_count, 2);
    }

    #[test]
    fn failure_rate_calculation() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_test1");
        model.ingest(
            1,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: id.clone(),
                kind: Kind::Code,
                name: "test".into(),
                owner: "marvin".into(),
            },
        );

        // 3 completed, 2 failed = 40% failure rate
        for i in 0..3 {
            model.ingest(
                i + 2,
                &FactoryEventKind::ForgeShapingReturned {
                    request_id: format!("req_{i}"),
                    artifact_id: id.clone(),
                    outcome: ShapingOutcome::Completed,
                },
            );
        }
        for i in 0..2 {
            model.ingest(
                i + 5,
                &FactoryEventKind::ForgeShapingReturned {
                    request_id: format!("req_fail_{i}"),
                    artifact_id: id.clone(),
                    outcome: ShapingOutcome::Failed,
                },
            );
        }

        let rate = model.failure_rate(&id, 5);
        assert!((rate - 0.4).abs() < f64::EPSILON);

        // Last 2 are both failures
        let rate_last2 = model.failure_rate(&id, 2);
        assert!((rate_last2 - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn failure_rate_empty() {
        let model = FactoryModel::new();
        let id = ArtifactId::new("art_nonexistent");
        assert!((model.failure_rate(&id, 5)).abs() < f64::EPSILON);
    }

    #[test]
    fn ingests_gate_verdict_records() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_gv1");

        model.ingest(
            1,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: id.clone(),
                kind: Kind::Code,
                name: "svc".into(),
                owner: "marvin".into(),
            },
        );
        model.ingest(
            2,
            &FactoryEventKind::ForgeGateVerdict {
                artifact_id: id.clone(),
                gate_point: GatePoint::PreDispatch,
                verdict: VerdictSummary::Deny,
            },
        );

        assert_eq!(model.gate_verdict_records.len(), 1);
        assert_eq!(model.gate_verdict_records[0].event_id, 2);
        assert_eq!(model.gate_verdict_records[0].verdict, VerdictSummary::Deny);
    }

    #[test]
    fn override_rate_groups_by_kind_and_respects_min_samples() {
        let mut model = FactoryModel::new();

        // Kind::Code: 4 deny + 1 allow = 80% override (meets min_samples=3).
        let code_id = ArtifactId::new("art_code");
        model.ingest(
            1,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: code_id.clone(),
                kind: Kind::Code,
                name: "svc".into(),
                owner: "marvin".into(),
            },
        );
        for (i, verdict) in [
            VerdictSummary::Deny,
            VerdictSummary::Deny,
            VerdictSummary::Deny,
            VerdictSummary::Allow,
            VerdictSummary::Deny,
        ]
        .iter()
        .enumerate()
        {
            model.ingest(
                i as i64 + 100,
                &FactoryEventKind::ForgeGateVerdict {
                    artifact_id: code_id.clone(),
                    gate_point: GatePoint::PreDispatch,
                    verdict: verdict.clone(),
                },
            );
        }

        // Kind::Document: 1 deny only (below default min_samples=3 — filtered).
        let doc_id = ArtifactId::new("art_doc");
        model.ingest(
            50,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: doc_id.clone(),
                kind: Kind::Document,
                name: "readme".into(),
                owner: "marvin".into(),
            },
        );
        model.ingest(
            51,
            &FactoryEventKind::ForgeGateVerdict {
                artifact_id: doc_id,
                gate_point: GatePoint::StateTransition,
                verdict: VerdictSummary::Deny,
            },
        );

        let rates = model.override_rate_by_kind(Duration::days(7), 3);
        assert!(rates.contains_key(&Kind::Code), "code must be present");
        assert!(
            !rates.contains_key(&Kind::Document),
            "document under min_samples must be dropped"
        );
        let (rate, total, evidence) = &rates[&Kind::Code];
        assert_eq!(*total, 5);
        assert!((rate - 0.8).abs() < 1e-9);
        assert!(!evidence.is_empty());
        assert!(evidence.windows(2).all(|w| w[0] >= w[1]), "evidence sorted");
    }
}
