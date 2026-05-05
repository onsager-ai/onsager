//! Event-type registry manifest — Lever E of spec #131 / ADR 0004.
//!
//! This module is the **single source of truth** for which subsystem
//! produces and consumes each `FactoryEventKind` variant. It is a static,
//! human-reviewed manifest. Subsystems do **not** self-register at runtime;
//! the manifest is reviewed when changes land and CI (`cargo xtask
//! check-events`) enforces the contract.
//!
//! See spec issue #150 and the registry CLAUDE.md for the update process.

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
    /// reads do not count — set [`audit_only`] for those.
    ///
    /// [`audit_only`]: EventDefinition::audit_only
    pub consumers: &'static [Subsystem],
    /// `true` when no subsystem consumer is expected — the event exists for
    /// the dashboard / audit log only. Lets the "every event has a consumer"
    /// check stay strict for events that should have one, while honestly
    /// labelling events that shouldn't.
    pub audit_only: bool,
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
        // -- Artifact lifecycle (forge writes "artifact.state_changed"
        //    via `emit_pipeline_event`; the others are written by the
        //    portal / ingestion side). -------------------------------
        EventDefinition {
            kind: "artifact.registered",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Ising],
            audit_only: false,
            description: "New artifact accepted and ID assigned.",
        },
        EventDefinition {
            kind: "artifact.state_changed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Ising],
            audit_only: false,
            description: "Artifact transitioned between lifecycle states.",
        },
        EventDefinition {
            kind: "artifact.version_created",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Ising],
            audit_only: false,
            description: "New version committed for an artifact.",
        },
        EventDefinition {
            kind: "artifact.lineage_extended",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "New vertical or horizontal lineage entry recorded.",
        },
        EventDefinition {
            kind: "artifact.quality_recorded",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "New quality signal appended.",
        },
        EventDefinition {
            kind: "artifact.routed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "Released artifact dispatched to a consumer sink.",
        },
        EventDefinition {
            kind: "artifact.archived",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "Artifact reached terminal state (archived).",
        },
        // -- Warehouse / delivery / deliverable -------------------------
        EventDefinition {
            kind: "warehouse.bundle_sealed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "A new bundle was sealed for an artifact.",
        },
        EventDefinition {
            kind: "delivery.succeeded",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "A delivery attempt succeeded.",
        },
        EventDefinition {
            kind: "delivery.failed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "A delivery attempt failed; carries retry/abandoned flag.",
        },
        EventDefinition {
            kind: "deliverable.created",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "A workflow run produced its first artifact reference.",
        },
        EventDefinition {
            kind: "deliverable.updated",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "A workflow run added an artifact reference to its deliverable.",
        },
        // -- Git lifecycle (onsager-portal webhooks) --------------------
        EventDefinition {
            kind: "git.branch_created",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "A working branch was pushed for an artifact.",
        },
        EventDefinition {
            kind: "git.commit_pushed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "A commit was pushed to an artifact's branch.",
        },
        EventDefinition {
            kind: "git.pr_opened",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Ising],
            audit_only: false,
            description: "A pull request was opened for an artifact.",
        },
        EventDefinition {
            kind: "git.pr_review_received",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "A reviewer left a verdict on a PR.",
        },
        EventDefinition {
            kind: "git.ci_completed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "A CI check finished for a PR.",
        },
        EventDefinition {
            kind: "git.pr_merged",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge, Subsystem::Ising],
            audit_only: false,
            description: "A PR was merged.",
        },
        EventDefinition {
            kind: "git.pr_closed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "A PR was closed without merging.",
        },
        // -- Forge process events ---------------------------------------
        EventDefinition {
            kind: "forge.shaping_dispatched",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Stiglab],
            audit_only: false,
            description: "ShapingRequest sent to Stiglab via the spine (replaces POST /api/shaping).",
        },
        EventDefinition {
            kind: "forge.shaping_returned",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Ising],
            audit_only: false,
            description: "ShapingResult received from Stiglab and recorded by Forge.",
        },
        EventDefinition {
            kind: "forge.gate_requested",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Synodic],
            audit_only: false,
            description: "GateRequest sent to Synodic via the spine (replaces POST /api/gate).",
        },
        EventDefinition {
            kind: "forge.gate_verdict",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Ising],
            audit_only: false,
            description: "GateVerdict observed by Forge after Synodic responded.",
        },
        EventDefinition {
            kind: "forge.insight_observed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "Insight forwarded to the scheduling kernel.",
        },
        EventDefinition {
            kind: "forge.decision_made",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "Scheduling kernel produced a ShapingDecision.",
        },
        EventDefinition {
            kind: "forge.idle_tick",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "Scheduling kernel returned None (idle, reduced frequency).",
        },
        EventDefinition {
            kind: "forge.state_changed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "Forge process state machine transitioned.",
        },
        // -- Stiglab events --------------------------------------------
        EventDefinition {
            kind: "stiglab.session_created",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            audit_only: true,
            description: "A new session was allocated for a shaping request.",
        },
        EventDefinition {
            kind: "stiglab.session_dispatched",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            audit_only: true,
            description: "A session was dispatched to a Stiglab node.",
        },
        EventDefinition {
            kind: "stiglab.session_running",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            audit_only: true,
            description: "A session began active execution.",
        },
        EventDefinition {
            kind: "stiglab.session_completed",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "A session finished successfully; carries optional artifact_id, token usage, branch, and PR number.",
        },
        EventDefinition {
            kind: "stiglab.shaping_result_ready",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "Full ShapingResult ready for Forge to act on (replaces POST /api/shaping response).",
        },
        EventDefinition {
            kind: "stiglab.session_failed",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "A session terminated with an error.",
        },
        EventDefinition {
            kind: "stiglab.session_aborted",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            audit_only: true,
            description: "A session was aborted (node lost, deadline exceeded).",
        },
        EventDefinition {
            kind: "stiglab.event_upgraded",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            audit_only: true,
            description: "A session-internal event was promoted to a factory event.",
        },
        EventDefinition {
            kind: "stiglab.node_registered",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            audit_only: true,
            description: "A new Stiglab node joined the pool.",
        },
        EventDefinition {
            kind: "stiglab.node_deregistered",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            audit_only: true,
            description: "A Stiglab node left the pool.",
        },
        EventDefinition {
            kind: "stiglab.node_heartbeat_missed",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            audit_only: true,
            description: "A node missed its expected heartbeat.",
        },
        // -- Synodic events --------------------------------------------
        EventDefinition {
            kind: "synodic.gate_evaluated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "Gate request evaluated and a verdict issued (summary; full payload on synodic.gate_verdict).",
        },
        EventDefinition {
            kind: "synodic.gate_denied",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "Gate request denied (subset of gate_evaluated, for filtering).",
        },
        EventDefinition {
            kind: "synodic.gate_modified",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "Gate request resolved with verdict Modify (subset of gate_evaluated).",
        },
        EventDefinition {
            kind: "synodic.gate_verdict",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "Full GateVerdict in response to forge.gate_requested (replaces POST /api/gate response).",
        },
        EventDefinition {
            kind: "synodic.escalation_started",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "An escalation was initiated.",
        },
        EventDefinition {
            kind: "synodic.escalation_resolved",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "An escalation was resolved (human, delegate, or timeout).",
        },
        EventDefinition {
            kind: "synodic.escalation_timed_out",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "An escalation timed out and the default verdict was applied.",
        },
        EventDefinition {
            kind: "synodic.gate_resolution_proposed",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A delegate proposed a resolution for an active escalation.",
        },
        EventDefinition {
            kind: "synodic.rule_proposed",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A crystallization candidate rule was created.",
        },
        EventDefinition {
            kind: "synodic.rule_approved",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A proposed rule was approved and entered the active set.",
        },
        EventDefinition {
            kind: "synodic.rule_disabled",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A rule was disabled.",
        },
        EventDefinition {
            kind: "synodic.rule_version_created",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A rule was modified, producing a new version.",
        },
        // -- Ising events ----------------------------------------------
        EventDefinition {
            kind: "ising.insight_detected",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            audit_only: true,
            description: "An insight passed validation and was recorded on the spine.",
        },
        EventDefinition {
            kind: "ising.insight_emitted",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "Machine-readable signal emitted on the spine for other subsystems to consume.",
        },
        EventDefinition {
            kind: "ising.insight_suppressed",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            audit_only: true,
            description: "An insight was deduplicated or fell below confidence threshold.",
        },
        EventDefinition {
            kind: "ising.rule_proposed",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[Subsystem::Synodic],
            audit_only: false,
            description: "An insight was packaged as a rule proposal for Synodic.",
        },
        EventDefinition {
            kind: "ising.analyzer_error",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            audit_only: true,
            description: "An analyzer encountered an error during its run.",
        },
        EventDefinition {
            kind: "ising.catchup_completed",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            audit_only: true,
            description: "Ising finished catching up from a lag position.",
        },
        // -- Refract (intent decomposition) ----------------------------
        EventDefinition {
            kind: "refract.intent_submitted",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "A new intent was submitted for decomposition.",
        },
        EventDefinition {
            kind: "refract.decomposed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "A decomposer produced an artifact tree for an intent.",
        },
        EventDefinition {
            kind: "refract.failed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            audit_only: true,
            description: "Decomposition failed — no decomposer matched, or the matched decomposer errored out.",
        },
        // -- Workflow runtime (issue #80 / #81) ------------------------
        EventDefinition {
            kind: "trigger.fired",
            schema_version: 1,
            // Producers (all four trigger categories from #236):
            // - Stiglab manual-replay route (`/api/projects/:id/issues/:n/replay-trigger`).
            // - Forge scheduler (#238 — cron / delay / interval).
            // - Forge event-trigger listeners (#239 — spine_event /
            //   pg_notify / outbox_row).
            // - Portal — live GitHub `issues.labeled` webhook receiver
            //   (#222 Slice 1) plus the future `onsager trigger fire` CLI
            //   / "Run now" UI button (#241).
            producers: &[Subsystem::Stiglab, Subsystem::Forge, Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "A trigger fired (webhook / schedule / event / manual).",
        },
        EventDefinition {
            kind: "stage.entered",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "A workflow-tagged artifact entered a new stage.",
        },
        EventDefinition {
            kind: "stage.gate_passed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "A gate on the current stage resolved successfully.",
        },
        EventDefinition {
            kind: "stage.gate_failed",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "A gate on the current stage failed; the artifact is parked.",
        },
        EventDefinition {
            kind: "stage.advanced",
            schema_version: 1,
            producers: &[Subsystem::Forge],
            consumers: &[],
            audit_only: true,
            description: "All gates on a stage resolved and the artifact advanced.",
        },
        // -- Registry events (issue #14) -------------------------------
        EventDefinition {
            kind: "registry.type_proposed",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A new artifact type was proposed (not yet active).",
        },
        EventDefinition {
            kind: "registry.type_approved",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A proposed type was approved and entered the active catalog.",
        },
        EventDefinition {
            kind: "registry.type_deprecated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A type was deprecated (retained for audit).",
        },
        EventDefinition {
            kind: "registry.adapter_registered",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "An adapter implementation was registered in the catalog.",
        },
        EventDefinition {
            kind: "registry.adapter_deprecated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "An adapter was deprecated.",
        },
        EventDefinition {
            kind: "registry.gate_registered",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A gate evaluator was registered.",
        },
        EventDefinition {
            kind: "registry.gate_deprecated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "A gate evaluator was deprecated.",
        },
        EventDefinition {
            kind: "registry.profile_registered",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "An agent profile was registered.",
        },
        EventDefinition {
            kind: "registry.profile_deprecated",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            audit_only: true,
            description: "An agent profile was deprecated.",
        },
        // -- Gate adapters (GitHub webhooks) ---------------------------
        EventDefinition {
            kind: "gate.check_updated",
            schema_version: 1,
            // Portal owns the GitHub webhook ingress as of #222 Slice 1.
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "A GitHub check_suite/check_run/status arrived for a tracked PR.",
        },
        EventDefinition {
            kind: "gate.manual_approval_signal",
            schema_version: 1,
            // Portal owns the GitHub webhook ingress as of #222 Slice 1.
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Forge],
            audit_only: false,
            description: "A manual-approval gate received a signal (e.g. PR merged).",
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Manifest invariant: every `kind` is unique. A duplicate would let two
    /// rows disagree on producers/consumers and silently win the "first
    /// match" race in `lookup`.
    #[test]
    fn manifest_kinds_are_unique() {
        let mut seen: HashSet<&str> = HashSet::new();
        for e in EVENTS.events {
            assert!(seen.insert(e.kind), "duplicate manifest kind: {}", e.kind);
        }
    }

    /// Strict version of the Lever E "every event has a producer and a
    /// consumer" check: an event must declare a producer, and must either
    /// declare a consumer or be `audit_only`.
    #[test]
    fn manifest_every_event_has_producer_and_consumer_or_audit_only() {
        for e in EVENTS.events {
            assert!(
                !e.producers.is_empty(),
                "event `{}` has no producer",
                e.kind
            );
            assert!(
                !e.consumers.is_empty() || e.audit_only,
                "event `{}` has no consumer and is not audit_only",
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
}
