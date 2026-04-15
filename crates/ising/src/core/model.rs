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

use chrono::{DateTime, Utc};

use onsager_spine::artifact::{ArtifactId, ArtifactState, Kind};
use onsager_spine::factory_event::{FactoryEventKind, ShapingOutcome};

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

            _ => {}
        }
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
    use onsager_spine::artifact::Kind;

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
}
