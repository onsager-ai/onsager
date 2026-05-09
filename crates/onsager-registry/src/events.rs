//! Event-type registry manifest — Lever E of spec #131 / ADR 0004.
//!
//! This module is the **single source of truth** for which subsystem
//! produces and consumes each `FactoryEventKind` variant. It is a static,
//! human-reviewed manifest. Subsystems do **not** self-register at runtime;
//! the manifest is reviewed when changes land and CI (`cargo xtask
//! check-events`) enforces the contract.
//!
//! Schema (per spec #272): every row is in one of two states.
//!
//! 1. **Real** — `consumers` is non-empty. The event has at least one
//!    in-tree subsystem listener.
//! 2. **Diagnostic-only** — `diagnostic_only: true` plus a non-empty
//!    `reason` string. The event is emitted today and read by something
//!    concrete (dashboard timeline, audit trail) but no subsystem listens
//!    for it.
//!
//! See spec issue #150 for the original Lever E plan and #272 for the
//! schema simplification.

use serde::Serialize;

/// Subsystem that produces or consumes events on the spine.
///
/// `Portal` is the umbrella for `onsager-portal` (GitHub webhooks) and any
/// other external boundary that writes to the spine without being one of
/// the four core subsystems. It exists so events whose only producer is a
/// webhook receiver still satisfy the "every event has a producer" check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Subsystem {
    Forge,
    Stiglab,
    Synodic,
    Ising,
    Portal,
}

impl Subsystem {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Forge => "forge",
            Self::Stiglab => "stiglab",
            Self::Synodic => "synodic",
            Self::Ising => "ising",
            Self::Portal => "portal",
        }
    }

    /// Subsystems that own source under `crates/<name>/` and whose emit /
    /// listener call sites the CI check scans. `Portal` is excluded: its
    /// emitters live outside the workspace's lint surface.
    pub const SCANNED: &'static [Subsystem] =
        &[Self::Forge, Self::Stiglab, Self::Synodic, Self::Ising];
}

/// One row of the event-type registry manifest.
///
/// Every row is either **real** (`consumers` non-empty) or
/// **diagnostic-only** (`diagnostic_only: true` plus a non-empty
/// [`reason`]). `xtask check-events` rejects rows that are neither.
///
/// [`reason`]: EventDefinition::reason
#[derive(Debug, Clone, Serialize)]
pub struct EventDefinition {
    /// Wire `event_type` string, matching `FactoryEventKind::event_type()`
    /// (e.g. `"forge.shaping_dispatched"`).
    pub kind: &'static str,
    /// Schema version for this event's payload. Bumped on backwards-
    /// incompatible payload changes; additive `Option<T>` fields with
    /// `#[serde(default)]` do **not** bump.
    pub schema_version: u32,
    /// Subsystems that emit this event onto the spine.
    pub producers: &'static [Subsystem],
    /// Subsystems that listen for this event and act on it (i.e. dispatch
    /// off the `event_type` string and parse the payload). Dashboard-only
    /// reads do not count — set [`diagnostic_only`] + [`reason`] for those.
    ///
    /// [`diagnostic_only`]: EventDefinition::diagnostic_only
    /// [`reason`]: EventDefinition::reason
    pub consumers: &'static [Subsystem],
    /// `true` when no subsystem consumer is expected — the event is read
    /// by a non-subsystem concern (dashboard timeline, audit trail). Must
    /// be paired with a non-empty [`reason`].
    ///
    /// [`reason`]: EventDefinition::reason
    pub diagnostic_only: bool,
    /// Why this row is diagnostic-only — what reads it today (e.g.
    /// `"rendered in dashboard event timeline"`). Required when
    /// `diagnostic_only` is `true`; `None` for real rows. Free-form by
    /// design.
    pub reason: Option<&'static str>,
    /// One-line description for the manifest read API and dashboard.
    pub description: &'static str,
}

/// The full registry of factory event types.
#[derive(Debug, Clone, Serialize)]
pub struct EventManifest {
    pub events: &'static [EventDefinition],
}

impl EventManifest {
    pub fn lookup(&self, kind: &str) -> Option<&EventDefinition> {
        self.events.iter().find(|e| e.kind == kind)
    }
}

/// The canonical event manifest. Every variant in `FactoryEventKind` must
/// have a row here; CI's `cargo xtask check-events` enforces it.
pub const EVENTS: EventManifest = EventManifest {
    events: &[
        // -- Artifact lifecycle ---------------------------------------------
        EventDefinition {
            kind: "artifact.registered",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            description: "New artifact accepted and ID assigned.",
        },
        EventDefinition {
            kind: "artifact.state_changed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            description: "Artifact transitioned between lifecycle states.",
        },
        EventDefinition {
            kind: "artifact.archived",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "forge consumer in flight per spec #273; remains diagnostic-only until that PR lands",
            ),
            description: "Artifact reached terminal state (archived).",
        },
        // -- Git lifecycle (onsager-portal webhooks) ------------------------
        EventDefinition {
            kind: "git.pr_opened",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            description: "A pull request was opened for an artifact.",
        },
        EventDefinition {
            kind: "git.ci_completed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "A CI check finished for a PR.",
        },
        EventDefinition {
            kind: "git.pr_merged",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge, Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            description: "A PR was merged.",
        },
        EventDefinition {
            kind: "git.pr_closed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "A PR was closed without merging.",
        },
        // -- Forge process events -------------------------------------------
        EventDefinition {
            kind: "forge.shaping_dispatched",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Stiglab],
            diagnostic_only: false,
            reason: None,
            description: "ShapingRequest sent to Stiglab via the spine (replaces POST /api/shaping).",
        },
        EventDefinition {
            kind: "forge.shaping_returned",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            description: "ShapingResult received from Stiglab and recorded by Forge.",
        },
        EventDefinition {
            kind: "forge.gate_requested",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Synodic],
            diagnostic_only: false,
            reason: None,
            description: "GateRequest sent to Synodic via the spine (replaces POST /api/gate).",
        },
        EventDefinition {
            kind: "forge.gate_verdict",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            description: "GateVerdict observed by Forge after Synodic responded.",
        },
        EventDefinition {
            kind: "forge.insight_observed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "Insight forwarded to the scheduling kernel.",
        },
        EventDefinition {
            kind: "forge.decision_made",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("diagnostic trace of forge scheduling kernel; no downstream action"),
            description: "Scheduling kernel produced a ShapingDecision.",
        },
        // -- Stiglab events -------------------------------------------------
        EventDefinition {
            kind: "stiglab.session_completed",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "A session finished successfully; carries optional artifact_id, token usage, branch, and PR number.",
        },
        EventDefinition {
            kind: "stiglab.shaping_result_ready",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "Full ShapingResult ready for Forge to act on (replaces POST /api/shaping response).",
        },
        EventDefinition {
            kind: "stiglab.session_failed",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "A session terminated with an error.",
        },
        EventDefinition {
            kind: "stiglab.session_aborted",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("dashboard event-timeline troubleshooting for node/deadline failures"),
            description: "A session was aborted (node lost, deadline exceeded).",
        },
        // -- Portal intents (dashboard → agent dispatch) --------------------
        EventDefinition {
            kind: "portal.session_requested",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Stiglab],
            diagnostic_only: false,
            reason: None,
            description: "Dashboard task request from portal; stiglab dispatches the session to an agent node.",
        },
        // -- Synodic events -------------------------------------------------
        EventDefinition {
            kind: "synodic.gate_evaluated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("dashboard filtering & audit trail; gate_verdict is the consumer event"),
            description: "Gate request evaluated and a verdict issued (summary; full payload on synodic.gate_verdict).",
        },
        EventDefinition {
            kind: "synodic.gate_denied",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("dashboard deny-verdict filtering"),
            description: "Gate request denied (subset of gate_evaluated, for filtering).",
        },
        EventDefinition {
            kind: "synodic.gate_modified",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("dashboard modify-verdict filtering"),
            description: "Gate request resolved with verdict Modify (subset of gate_evaluated).",
        },
        EventDefinition {
            kind: "synodic.gate_verdict",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "Full GateVerdict in response to forge.gate_requested (replaces POST /api/gate response).",
        },
        EventDefinition {
            kind: "synodic.escalation_started",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for escalation context"),
            description: "An escalation was initiated.",
        },
        EventDefinition {
            kind: "synodic.escalation_resolved",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            description: "An escalation was resolved (human, delegate, or timeout).",
        },
        EventDefinition {
            kind: "synodic.escalation_timed_out",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            description: "An escalation timed out and the default verdict was applied.",
        },
        EventDefinition {
            kind: "synodic.gate_resolution_proposed",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            description: "A delegate proposed a resolution for an active escalation.",
        },
        EventDefinition {
            kind: "synodic.rule_proposed",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            description: "A crystallization candidate rule was created.",
        },
        EventDefinition {
            kind: "synodic.rule_approved",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            description: "A proposed rule was approved and entered the active set.",
        },
        EventDefinition {
            kind: "synodic.rule_disabled",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            description: "A rule was disabled.",
        },
        EventDefinition {
            kind: "synodic.rule_version_created",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            description: "A rule was modified, producing a new version.",
        },
        // -- Ising events ---------------------------------------------------
        EventDefinition {
            kind: "ising.insight_detected",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in dashboard ising views"),
            description: "An insight passed validation and was recorded on the spine.",
        },
        EventDefinition {
            kind: "ising.insight_emitted",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "Machine-readable signal emitted on the spine for other subsystems to consume.",
        },
        EventDefinition {
            kind: "ising.insight_suppressed",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in dashboard ising views"),
            description: "An insight was deduplicated or fell below confidence threshold.",
        },
        EventDefinition {
            kind: "ising.rule_proposed",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[Subsystem::Synodic],
            diagnostic_only: false,
            reason: None,
            description: "An insight was packaged as a rule proposal for Synodic.",
        },
        EventDefinition {
            kind: "ising.analyzer_error",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("operator troubleshooting in dashboard"),
            description: "An analyzer encountered an error during its run.",
        },
        EventDefinition {
            kind: "ising.catchup_completed",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("ising health monitoring in dashboard"),
            description: "Ising finished catching up from a lag position.",
        },
        // -- Refract (intent decomposition) ---------------------------------
        EventDefinition {
            kind: "refract.intent_submitted",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in refract intent timeline"),
            description: "A new intent was submitted for decomposition.",
        },
        EventDefinition {
            kind: "refract.decomposed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in refract intent timeline"),
            description: "A decomposer produced an artifact tree for an intent.",
        },
        EventDefinition {
            kind: "refract.failed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in refract intent timeline"),
            description: "Decomposition failed — no decomposer matched, or the matched decomposer errored out.",
        },
        // -- Workflow runtime (issue #80 / #81) -----------------------------
        EventDefinition {
            kind: "trigger.fired",
            schema_version: 1,
            // Producers (all four trigger categories from #236):
            // - Stiglab manual-replay route (`/api/projects/:id/issues/:n/replay-trigger`).
            // - Forge scheduler (#238 — cron / delay / interval).
            // - Forge event-trigger listeners (#239 — spine_event /
            //   pg_notify / outbox_row).
            // - Portal — live GitHub `issues.labeled` webhook receiver
            //   (#222 Slice 1), GitHub `pull_request.closed` /
            //   `workflow_run.completed` / Telegram receivers (#240),
            //   the `onsager trigger fire` CLI and the dashboard
            //   "Run now" / replay endpoints (#241).
            producers: &[Subsystem::Stiglab, Subsystem::Forge, Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "A trigger fired (webhook / schedule / event / manual).",
        },
        EventDefinition {
            kind: "workflow.manual_triggered",
            schema_version: 1,
            // Producers: portal (UI button + replay endpoint) and the
            // `onsager-trigger` CLI which writes events directly to the
            // spine — both surfaces emit alongside the underlying
            // `trigger.fired` event so audit views can attribute manual /
            // replay fires to a user (#241).
            producers: &[Subsystem::Portal],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for manual / CLI / replay fires"),
            description: "Audit record for a manual / CLI / replay trigger fire (actor + workflow).",
        },
        EventDefinition {
            kind: "stage.entered",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in workflow run timeline"),
            description: "A workflow-tagged artifact entered a new stage.",
        },
        EventDefinition {
            kind: "stage.gate_passed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in workflow run timeline"),
            description: "A gate on the current stage resolved successfully.",
        },
        EventDefinition {
            kind: "stage.gate_failed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in workflow run timeline"),
            description: "A gate on the current stage failed; the artifact is parked.",
        },
        EventDefinition {
            kind: "stage.advanced",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in workflow run timeline"),
            description: "All gates on a stage resolved and the artifact advanced.",
        },
        // -- Registry events (issue #14) ------------------------------------
        EventDefinition {
            kind: "registry.type_proposed",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for registry catalog mutations"),
            description: "A new artifact type was proposed (not yet active).",
        },
        EventDefinition {
            kind: "registry.type_approved",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for registry catalog mutations"),
            description: "A proposed type was approved and entered the active catalog.",
        },
        EventDefinition {
            kind: "registry.type_deprecated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for registry catalog mutations"),
            description: "A type was deprecated (retained for audit).",
        },
        EventDefinition {
            kind: "registry.adapter_registered",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for registry catalog mutations"),
            description: "An adapter implementation was registered in the catalog.",
        },
        EventDefinition {
            kind: "registry.adapter_deprecated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for registry catalog mutations"),
            description: "An adapter was deprecated.",
        },
        EventDefinition {
            kind: "registry.gate_registered",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for registry catalog mutations"),
            description: "A gate evaluator was registered.",
        },
        EventDefinition {
            kind: "registry.gate_deprecated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for registry catalog mutations"),
            description: "A gate evaluator was deprecated.",
        },
        EventDefinition {
            kind: "registry.profile_registered",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for registry catalog mutations"),
            description: "An agent profile was registered.",
        },
        EventDefinition {
            kind: "registry.profile_deprecated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for registry catalog mutations"),
            description: "An agent profile was deprecated.",
        },
        // -- Gate adapters (GitHub webhooks) --------------------------------
        EventDefinition {
            kind: "gate.check_updated",
            schema_version: 1,
            // Portal owns the GitHub webhook ingress as of #222 Slice 1.
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "A GitHub check_suite/check_run/status arrived for a tracked PR.",
        },
        EventDefinition {
            kind: "gate.manual_approval_signal",
            schema_version: 1,
            // Portal owns the GitHub webhook ingress as of #222 Slice 1.
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            diagnostic_only: false,
            reason: None,
            description: "A manual-approval gate received a signal (e.g. PR merged).",
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Manifest invariant: every `kind` is unique.
    #[test]
    fn manifest_kinds_are_unique() {
        let mut seen: HashSet<&str> = HashSet::new();
        for e in EVENTS.events {
            assert!(seen.insert(e.kind), "duplicate manifest kind: {}", e.kind);
        }
    }

    /// Strict version of the schema invariant: every row has a producer,
    /// and is either real (non-empty consumers) or diagnostic-only with a
    /// non-empty `reason` string.
    #[test]
    fn manifest_every_event_has_producer_and_is_real_or_diagnostic() {
        for e in EVENTS.events {
            assert!(
                !e.producers.is_empty(),
                "event `{}` has no producer",
                e.kind
            );
            let real = !e.consumers.is_empty();
            let diagnostic = e.diagnostic_only && e.reason.is_some_and(|r| !r.is_empty());
            assert!(
                real || diagnostic,
                "event `{}` is neither real (non-empty consumers) nor \
                 diagnostic-only (diagnostic_only=true with non-empty reason)",
                e.kind
            );
            assert!(
                !e.diagnostic_only || e.consumers.is_empty(),
                "event `{}` is both real and diagnostic-only",
                e.kind
            );
            assert!(
                e.reason.is_none() || e.diagnostic_only,
                "event `{}` has a reason but is not diagnostic-only",
                e.kind
            );
        }
    }

    /// Round-trips through `serde_json` so the dashboard read API gets the
    /// shape the frontend expects.
    #[test]
    fn manifest_serializes_to_json() {
        let json = serde_json::to_value(&EVENTS).expect("manifest serializes");
        let events = json
            .get("events")
            .and_then(|v| v.as_array())
            .expect("events array present");
        assert_eq!(events.len(), EVENTS.events.len());
        let first = &events[0];
        assert!(first.get("kind").is_some());
        assert!(first.get("producers").is_some());
        assert!(first.get("consumers").is_some());
        assert!(first.get("diagnostic_only").is_some());
        assert!(first.get("reason").is_some());
    }

    #[test]
    fn lookup_finds_known_kind_and_misses_unknown() {
        let def = EVENTS
            .lookup("forge.shaping_dispatched")
            .expect("known kind");
        assert!(def.producers.contains(&Subsystem::Forge));
        assert!(def.consumers.contains(&Subsystem::Stiglab));
        assert!(EVENTS.lookup("does.not_exist").is_none());
    }

    /// Diagnostic-only rows must carry a non-empty reason string.
    #[test]
    fn diagnostic_only_rows_have_reason() {
        for e in EVENTS.events {
            if e.diagnostic_only {
                let r = e.reason.expect("diagnostic-only row missing reason");
                assert!(!r.is_empty(), "event `{}` has empty reason", e.kind);
            }
        }
    }
}
