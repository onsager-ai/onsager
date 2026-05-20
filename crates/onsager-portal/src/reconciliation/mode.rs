//! Per-project ingestion-mode selector + interval policy.
//!
//! Three modes (spec #121 § Design / "Ingestion-mode selector"):
//!
//! * [`IngestionMode::WebhookReconciler`] — default. Webhooks for
//!   low latency, reconciler poll as a backstop at low frequency.
//! * [`IngestionMode::PollingOnly`] — local-dev / webhook-less
//!   installs. Full-rate poll, no public URL or HMAC secret
//!   required. Useful for `just dev` smoke and for projects whose
//!   App webhook can't reach the portal.
//! * [`IngestionMode::WebhookOnly`] — opt out of the reconciler.
//!   Not recommended (silent drops become permanent); included for
//!   parity with the spec.
//!
//! Interval policy answers the "Human decides" gate in spec #121:
//! 300 s reconciler, 60 s polling-only, 30 s floor. The floor is
//! the runaway-config guard — the same project config that picks a
//! mode can't drive the loop into a tight loop against GitHub.
//!
//! Currently the interval is mode-derived, not per-project tunable.
//! A future spec can widen the column shape to carry per-project
//! overrides; the floor stays in code.

use std::time::Duration;

/// Reconciler-mode tick interval. 5 minutes — high enough to be
/// effectively free on the GitHub authenticated rate limit (5000/h)
/// across a typical workspace, low enough that a dropped webhook
/// surfaces within one tick.
pub const RECONCILER_INTERVAL: Duration = Duration::from_secs(300);

/// Polling-only tick interval. 60 s — the local-dev case wants
/// "labelled issue appears in spine quickly", and there's no
/// webhook to race with.
pub const POLLING_ONLY_INTERVAL: Duration = Duration::from_secs(60);

/// Minimum tick interval. 30 s. Guards against a runaway config
/// from hammering the upstream API; any computed interval below
/// the floor is bumped up to it.
pub const MIN_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// The three ingestion modes from spec #121.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IngestionMode {
    #[default]
    WebhookReconciler,
    PollingOnly,
    WebhookOnly,
}

impl IngestionMode {
    /// Canonical wire / database form of this mode. Matches the
    /// `projects.ingestion_mode` CHECK constraint (spine migration
    /// 033).
    pub fn as_str(self) -> &'static str {
        match self {
            IngestionMode::WebhookReconciler => "webhook+reconciler",
            IngestionMode::PollingOnly => "polling-only",
            IngestionMode::WebhookOnly => "webhook-only",
        }
    }

    /// Parse the column value. Unknown values fall back to the
    /// default mode so a forward-rolled project row from a future
    /// migration doesn't crash the scheduler — the unknown mode
    /// surfaces in logs (callers should warn) and the project polls
    /// at the conservative reconciler interval.
    pub fn parse(s: &str) -> Self {
        match s {
            "webhook+reconciler" => IngestionMode::WebhookReconciler,
            "polling-only" => IngestionMode::PollingOnly,
            "webhook-only" => IngestionMode::WebhookOnly,
            _ => IngestionMode::WebhookReconciler,
        }
    }

    /// Should the scheduler spawn a poll loop for a project in this
    /// mode? `webhook-only` returns false; the other two return true.
    pub fn polls(self) -> bool {
        !matches!(self, IngestionMode::WebhookOnly)
    }

    /// Tick interval for this mode after floor enforcement.
    pub fn poll_interval(self) -> Duration {
        let raw = match self {
            IngestionMode::WebhookReconciler => RECONCILER_INTERVAL,
            IngestionMode::PollingOnly => POLLING_ONLY_INTERVAL,
            IngestionMode::WebhookOnly => RECONCILER_INTERVAL,
        };
        raw.max(MIN_POLL_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trip_for_known_modes() {
        for mode in [
            IngestionMode::WebhookReconciler,
            IngestionMode::PollingOnly,
            IngestionMode::WebhookOnly,
        ] {
            assert_eq!(IngestionMode::parse(mode.as_str()), mode);
        }
    }

    #[test]
    fn unknown_mode_falls_back_to_default() {
        // Forward-rolled rows or typo'd configs land in the safest
        // available mode (the default) rather than crashing the
        // scheduler.
        assert_eq!(
            IngestionMode::parse("never-heard-of-it"),
            IngestionMode::WebhookReconciler
        );
    }

    #[test]
    fn webhook_only_does_not_poll() {
        assert!(!IngestionMode::WebhookOnly.polls());
        assert!(IngestionMode::WebhookReconciler.polls());
        assert!(IngestionMode::PollingOnly.polls());
    }

    #[test]
    fn polling_only_interval_is_shorter_than_reconciler() {
        assert!(
            IngestionMode::PollingOnly.poll_interval()
                < IngestionMode::WebhookReconciler.poll_interval()
        );
    }

    #[test]
    fn poll_interval_respects_floor() {
        // The floor is the runaway-config guard. Mode constants
        // already sit comfortably above it, but a future override
        // can't drop below the floor without changing this function.
        assert!(IngestionMode::PollingOnly.poll_interval() >= MIN_POLL_INTERVAL);
    }
}
