//! Typed observer outputs — the three substrate-recognized shapes.
//!
//! Per ADR 0013 ("Output is typed"), every observer emits a closed
//! set of variants so the dashboard can render them with one
//! component family. The three are:
//!
//! - [`QualitySignal`](onsager_artifact::QualitySignal) — append-only
//!   record about artifact quality (already defined in
//!   `onsager-artifact`; observers reuse it so signals raised by
//!   substrate-side audits and ones raised by lifecycle hooks land
//!   in the same shape).
//! - [`Insight`] — a structured *observation* about the system: a
//!   recurring pattern, a correlation, a hypothesis. Carries
//!   confidence and pointers to the spine rows that support it.
//! - [`Alert`] — an *action-worthy* signal: a deadline crossed, a
//!   verdict pattern that needs human attention, an analyzer crash.
//!   Severity-banded so triage can sort.
//!
//! The three exist on one [`ObserverOutput`] enum so [`Observer`]
//! impls can return a heterogeneous batch from a single event.
//!
//! [`Observer`]: crate::Observer

use onsager_artifact::QualitySignal;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ObserverOutput
// ---------------------------------------------------------------------------

/// One typed output from an observer's `on_event` call.
///
/// `f64` confidence on [`Insight`] blocks `Eq`; only `PartialEq` is
/// derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObserverOutput {
    /// A [`QualitySignal`] about an artifact — same shape as
    /// `onsager-artifact::QualitySignal`.
    QualitySignal(QualitySignal),
    /// A structured observation. See [`Insight`].
    Insight(Insight),
    /// An action-worthy signal. See [`Alert`].
    Alert(Alert),
}

impl ObserverOutput {
    /// The output's kind tag, matching the serde `kind` discriminator
    /// and the `kind` column of `observer_outputs`.
    pub fn kind(&self) -> ObserverOutputKind {
        match self {
            Self::QualitySignal(_) => ObserverOutputKind::QualitySignal,
            Self::Insight(_) => ObserverOutputKind::Insight,
            Self::Alert(_) => ObserverOutputKind::Alert,
        }
    }
}

/// The three valid `kind` values for the `observer_outputs` table.
///
/// Kept as a closed enum so the SQL column and the in-Rust shape do
/// not drift — new variants would need both a Rust change and a
/// schema migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverOutputKind {
    QualitySignal,
    Insight,
    Alert,
}

impl ObserverOutputKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QualitySignal => "quality_signal",
            Self::Insight => "insight",
            Self::Alert => "alert",
        }
    }
}

// ---------------------------------------------------------------------------
// Insight
// ---------------------------------------------------------------------------

/// A structured observation produced by an observer.
///
/// Distinct from the legacy `onsager-spine::protocol::Insight` (which
/// is the Ising-to-Forge advisory shape from forge-v0.1) — this is
/// the substrate-native shape ADR 0013 names. The two coexist while
/// MIG-03 retires Ising; once the migration is complete the legacy
/// one is removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Insight {
    /// Free-form observation text. Rendered as the headline in the
    /// dashboard.
    pub observation: String,
    /// `0.0..=1.0` — observer's self-reported confidence. Consumers
    /// may filter by threshold.
    pub confidence: f64,
    /// `events.id` rows that support this insight. Observers should
    /// include at least the triggering event id so the dashboard can
    /// link the insight back to its evidence.
    #[serde(default)]
    pub evidence_event_ids: Vec<i64>,
    /// Optional artifact this insight is about (when the observer's
    /// scope is narrow enough to name one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_artifact_id: Option<String>,
    /// Optional free-form tag (`"velocity"`, `"flaky_test"`, ...) so
    /// the dashboard can group similar insights.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

// ---------------------------------------------------------------------------
// Alert
// ---------------------------------------------------------------------------

/// An action-worthy signal an observer raised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    /// Short title — rendered as the dashboard inbox row.
    pub title: String,
    /// Free-form detail body (Markdown allowed; renderer is
    /// dashboard-side).
    pub detail: String,
    pub severity: AlertSeverity,
    /// `events.id` rows that support / triggered this alert.
    #[serde(default)]
    pub evidence_event_ids: Vec<i64>,
    /// Optional artifact this alert is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_artifact_id: Option<String>,
}

/// Severity band on an [`Alert`].
///
/// Ordered as written — `Info < Warning < Error < Critical` — and
/// `Ord` is derived so consumers can compare against a threshold
/// (`alert.severity >= AlertSeverity::Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

impl AlertSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl Insight {
    /// Convenience: bare-minimum insight from an observation and
    /// confidence. Evidence and tagging stay empty.
    pub fn new(observation: impl Into<String>, confidence: f64) -> Self {
        Self {
            observation: observation.into(),
            confidence,
            evidence_event_ids: Vec::new(),
            about_artifact_id: None,
            tag: None,
        }
    }
}

impl Alert {
    pub fn new(
        title: impl Into<String>,
        detail: impl Into<String>,
        severity: AlertSeverity,
    ) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
            severity,
            evidence_event_ids: Vec::new(),
            about_artifact_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Construction shortcuts on ObserverOutput
// ---------------------------------------------------------------------------

impl ObserverOutput {
    pub fn insight(insight: Insight) -> Self {
        Self::Insight(insight)
    }

    pub fn alert(alert: Alert) -> Self {
        Self::Alert(alert)
    }

    pub fn quality_signal(signal: QualitySignal) -> Self {
        Self::QualitySignal(signal)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::{QualitySource, QualityValue};

    #[test]
    fn output_kind_round_trips() {
        let ev = ObserverOutput::Insight(Insight::new("recurring flake", 0.8));
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "insight");
        let back: ObserverOutput = serde_json::from_value(json).unwrap();
        assert_eq!(back, ev);
        assert_eq!(back.kind(), ObserverOutputKind::Insight);
        assert_eq!(back.kind().as_str(), "insight");
    }

    #[test]
    fn alert_severity_is_ordered() {
        assert!(AlertSeverity::Info < AlertSeverity::Warning);
        assert!(AlertSeverity::Warning < AlertSeverity::Error);
        assert!(AlertSeverity::Error < AlertSeverity::Critical);
    }

    #[test]
    fn alert_round_trips() {
        let ev = ObserverOutput::Alert(Alert::new(
            "Stage SLA crossed",
            "Stage `build` exceeded p95 by 4×.",
            AlertSeverity::Warning,
        ));
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "alert");
        assert_eq!(json["severity"], "warning");
        let back: ObserverOutput = serde_json::from_value(json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn quality_signal_round_trips() {
        let ev = ObserverOutput::QualitySignal(QualitySignal {
            source: QualitySource::IsingInference,
            dimension: "completeness".into(),
            value: QualityValue::Score(0.42),
            recorded_at: chrono::Utc::now(),
            recorded_by: "obs:demo".into(),
        });
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "quality_signal");
        let back: ObserverOutput = serde_json::from_value(json).unwrap();
        assert_eq!(back.kind(), ObserverOutputKind::QualitySignal);
        if let ObserverOutput::QualitySignal(s) = back {
            assert_eq!(s.dimension, "completeness");
        } else {
            panic!("expected QualitySignal variant");
        }
    }

    #[test]
    fn insight_skips_none_optional_fields() {
        let ev = ObserverOutput::Insight(Insight::new("seed", 0.5));
        let json = serde_json::to_value(&ev).unwrap();
        assert!(json.get("about_artifact_id").is_none());
        assert!(json.get("tag").is_none());
    }
}
