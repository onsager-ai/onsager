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
///
/// `Substrate` is the 0.2 production-line role (`onsager-substrate` +
/// `onsager-nodes`) that replaces the deprecated 0.1 `Forge` crate
/// (spec #363). The substrate scheduler dispatches executors and drives
/// artifacts through their lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Subsystem {
    Stiglab,
    Synodic,
    Ising,
    Portal,
    /// The 0.2 substrate — the scheduler (`onsager-nodes::scheduler`) and
    /// executors (`onsager-nodes::{script,agent,verify,...}`) that emit
    /// runtime lifecycle events (`node.*`, `agent.session_*`,
    /// `synodic.verdict`) onto the spine. Replaces the Forge tick loop
    /// per [ADR 0009](../../../docs/adr/0009-three-layer-pipeline.md).
    /// Not in `SCANNED` because substrate source lives in
    /// `onsager-substrate` / `onsager-nodes`, outside the
    /// `crates/{stiglab,synodic,ising}/` scope the emit / listener
    /// scan walks.
    Substrate,
}

impl Subsystem {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stiglab => "stiglab",
            Self::Synodic => "synodic",
            Self::Ising => "ising",
            Self::Portal => "portal",
            Self::Substrate => "substrate",
        }
    }

    /// Subsystems that own source under `crates/<name>/` and whose emit /
    /// listener call sites the CI check scans. `Portal` is excluded: its
    /// emitters live outside the workspace's lint surface. `Substrate`
    /// is excluded for the same reason — its emitters live in
    /// `onsager-substrate` / `onsager-nodes`, not the scanned source
    /// trees.
    pub const SCANNED: &'static [Subsystem] = &[Self::Stiglab, Self::Synodic, Self::Ising];
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
    /// Open follow-up issue tracking the path from diagnostic-only to a
    /// real consumer. Surfaces in `xtask check-events` so dangling
    /// skeletons can be re-examined: an event that has been
    /// diagnostic-only for an extended period either gets a real
    /// consumer or the row gets removed. `None` is allowed today (warn-
    /// mode landing per spec #275); ratchet to required is a follow-up.
    /// Has no meaning for real rows (`diagnostic_only = false`).
    pub tracking_issue: Option<u32>,
    /// Whether this event surfaces in the Activity tab's operator-grain
    /// stream (ADR 0019 / spec #465). `true` for events an operator
    /// cares about — stage transitions, gate verdicts, run lifecycle,
    /// trigger fires, escalations, agent/node lifecycle. `false` for
    /// internal subsystem-to-subsystem dispatches and analyzer
    /// diagnostics that would only add noise to the feed.
    ///
    /// Coverage is enforced by the field being required at construction
    /// time — adding a `FactoryEventKind` variant means adding a row
    /// here, and adding a row means picking a value. No default.
    pub operator_grain: bool,
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
            producers: &[Subsystem::Substrate],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "New artifact accepted and ID assigned.",
        },
        EventDefinition {
            kind: "artifact.state_changed",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "Artifact transitioned between lifecycle states.",
        },
        EventDefinition {
            kind: "artifact.archived",
            schema_version: 1,
            producers: &[Subsystem::Portal, Subsystem::Substrate],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
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
            tracking_issue: None,
            operator_grain: true,
            description: "A pull request was opened for an artifact.",
        },
        EventDefinition {
            kind: "git.ci_completed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "A CI check finished for a PR.",
        },
        EventDefinition {
            kind: "git.pr_merged",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Substrate, Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "A PR was merged.",
        },
        EventDefinition {
            kind: "git.pr_closed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "A PR was closed without merging.",
        },
        // -- Forge process events -------------------------------------------
        EventDefinition {
            kind: "forge.shaping_dispatched",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[Subsystem::Stiglab],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: false,
            description: "ShapingRequest sent to Stiglab via the spine (replaces POST /api/shaping).",
        },
        EventDefinition {
            kind: "forge.shaping_returned",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: false,
            description: "ShapingResult received from Stiglab and recorded by Forge.",
        },
        EventDefinition {
            kind: "forge.gate_requested",
            schema_version: 1,
            producers: &[Subsystem::Portal, Subsystem::Substrate],
            consumers: &[Subsystem::Synodic],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: false,
            description: "GateRequest sent to Synodic via the spine (replaces POST /api/gate). Portal emits it for the GatePoint::RepoWrite gate (#548) before minting a write token for a bound-but-unpinned repo.",
        },
        EventDefinition {
            kind: "forge.gate_verdict",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[Subsystem::Ising],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "GateVerdict observed by Forge after Synodic responded.",
        },
        EventDefinition {
            kind: "forge.insight_observed",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: false,
            description: "Insight forwarded to the scheduling kernel.",
        },
        EventDefinition {
            kind: "forge.decision_made",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("diagnostic trace of forge scheduling kernel; no downstream action"),
            tracking_issue: None,
            operator_grain: false,
            description: "Scheduling kernel produced a ShapingDecision.",
        },
        // -- Stiglab events -------------------------------------------------
        EventDefinition {
            kind: "stiglab.session_completed",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "A session finished successfully; carries optional artifact_id, token usage, branch, and PR number.",
        },
        EventDefinition {
            kind: "stiglab.session_result_ready",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: false,
            description: "Full session ShapingResult ready for Forge to act on (replaces POST /api/shaping response). Renamed from stiglab.shaping_result_ready per spec #285.",
        },
        EventDefinition {
            kind: "stiglab.session_failed",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "A session terminated with an error.",
        },
        EventDefinition {
            kind: "stiglab.session_aborted",
            schema_version: 1,
            producers: &[Subsystem::Stiglab],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("dashboard event-timeline troubleshooting for node/deadline failures"),
            tracking_issue: None,
            operator_grain: true,
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
            tracking_issue: None,
            operator_grain: true,
            description: "Dashboard task request from portal; stiglab dispatches the session to an agent node.",
        },
        EventDefinition {
            kind: "portal.session_cancel_requested",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Stiglab],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "Dashboard cancel-session request from portal (spec #303); stiglab forwards a CancelSession to the session's agent node.",
        },
        EventDefinition {
            kind: "portal.pr_open_failed",
            schema_version: 1,
            producers: &[Subsystem::Portal],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "observable in dashboard event timeline for operator troubleshooting when portal cannot open a PR for a completed session",
            ),
            tracking_issue: None,
            operator_grain: true,
            description: "Portal failed to open a GitHub PR for a completed session (empty branch, GitHub API error, or missing App config).",
        },
        // -- Synodic events -------------------------------------------------
        // Per spec #285: the redundant `synodic.gate_evaluated` /
        // `gate_denied` / `gate_modified` summary variants were dropped;
        // the dashboard renders gate timelines from `synodic.gate_verdict`
        // directly (verdict-shape filtering is a query-side concern).
        EventDefinition {
            kind: "synodic.gate_verdict",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[Subsystem::Portal, Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "Full GateVerdict in response to forge.gate_requested (replaces POST /api/gate response). Portal consumes the RepoWrite verdict (#548) to mint-or-deny a per-repo write token.",
        },
        EventDefinition {
            kind: "synodic.escalation_started",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("audit trail for escalation context"),
            tracking_issue: None,
            operator_grain: true,
            description: "An escalation was initiated.",
        },
        EventDefinition {
            kind: "synodic.escalation_resolved",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            tracking_issue: None,
            operator_grain: true,
            description: "An escalation was resolved (human, delegate, or timeout).",
        },
        EventDefinition {
            kind: "synodic.escalation_timed_out",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            tracking_issue: None,
            operator_grain: true,
            description: "An escalation timed out and the default verdict was applied.",
        },
        EventDefinition {
            kind: "synodic.gate_resolution_proposed",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            tracking_issue: None,
            operator_grain: true,
            description: "A delegate proposed a resolution for an active escalation.",
        },
        EventDefinition {
            kind: "synodic.rule_proposed",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            tracking_issue: None,
            operator_grain: false,
            description: "A crystallization candidate rule was created.",
        },
        EventDefinition {
            kind: "synodic.rule_approved",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            tracking_issue: None,
            operator_grain: false,
            description: "A proposed rule was approved and entered the active set.",
        },
        EventDefinition {
            kind: "synodic.rule_disabled",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            tracking_issue: None,
            operator_grain: false,
            description: "A rule was disabled.",
        },
        EventDefinition {
            kind: "synodic.rule_version_created",
            schema_version: 1,
            producers: &[Subsystem::Synodic],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in synodic governance UI; audit trail"),
            tracking_issue: None,
            operator_grain: false,
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
            tracking_issue: None,
            operator_grain: false,
            description: "An insight passed validation and was recorded on the spine.",
        },
        EventDefinition {
            kind: "ising.insight_emitted",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: false,
            description: "Machine-readable signal emitted on the spine for other subsystems to consume.",
        },
        EventDefinition {
            kind: "ising.insight_suppressed",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in dashboard ising views"),
            tracking_issue: None,
            operator_grain: false,
            description: "An insight was deduplicated or fell below confidence threshold.",
        },
        EventDefinition {
            kind: "ising.rule_proposed",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[Subsystem::Synodic],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: false,
            description: "An insight was packaged as a rule proposal for Synodic.",
        },
        EventDefinition {
            kind: "ising.analyzer_error",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("operator troubleshooting in dashboard"),
            tracking_issue: None,
            operator_grain: false,
            description: "An analyzer encountered an error during its run.",
        },
        EventDefinition {
            kind: "ising.catchup_completed",
            schema_version: 1,
            producers: &[Subsystem::Ising],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("ising health monitoring in dashboard"),
            tracking_issue: None,
            operator_grain: false,
            description: "Ising finished catching up from a lag position.",
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
            producers: &[Subsystem::Stiglab, Subsystem::Substrate, Subsystem::Portal],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
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
            tracking_issue: None,
            operator_grain: true,
            description: "Audit record for a manual / CLI / replay trigger fire (actor + workflow).",
        },
        EventDefinition {
            kind: "stage.entered",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in workflow run timeline"),
            tracking_issue: None,
            operator_grain: true,
            description: "A workflow-tagged artifact entered a new stage.",
        },
        // Per spec #285: per-gate `stage.gate_passed` / `stage.gate_failed`
        // events were dropped; the run timeline reconstructs gate outcomes
        // from `synodic.gate_verdict` plus the stage advancement signal.
        EventDefinition {
            kind: "stage.advanced",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some("rendered in workflow run timeline"),
            tracking_issue: None,
            operator_grain: true,
            description: "All gates on a stage resolved and the artifact advanced.",
        },
        EventDefinition {
            kind: "plan.run_requested",
            schema_version: 1,
            // Portal's `run_spec_plan` MCP tool emits this; the
            // substrate scheduler host consumes it, loads the stored
            // SpecPlan, compiles it, and runs it (#501, ADR 0023).
            // Carries an additive optional `encrypted_session_token`
            // (#536) the scheduler decrypts to inject ONSAGER_SESSION_TOKEN
            // into agent nodes — additive, so no schema_version bump.
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "A persisted multi-spec SpecPlan was requested to run.",
        },
        EventDefinition {
            kind: "plan.run_completed",
            schema_version: 1,
            // Substrate scheduler emits on plan-run success; portal
            // consumes it to revoke the plan-scoped session token (#536).
            producers: &[Subsystem::Substrate],
            consumers: &[Subsystem::Portal],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "A plan run finished successfully.",
        },
        EventDefinition {
            kind: "plan.run_failed",
            schema_version: 1,
            // Substrate scheduler emits on plan-run failure; portal
            // consumes it to revoke the plan-scoped session token (#536).
            producers: &[Subsystem::Substrate],
            consumers: &[Subsystem::Portal],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
            description: "A plan run failed terminally.",
        },
        // -- Registry events removed by spec #285 ---------------------------
        // The nine `registry.*` mutation events had no in-tree consumer
        // or dashboard reader. The registry tables remain the source of
        // truth; mutations no longer publish a spine event. Add a row
        // back here when there is a real consumer.

        // -- Substrate runtime (RUN-02, #360) -------------------------------
        // Lifecycle events emitted by the substrate scheduler and the
        // executor catalog. Producer is `Substrate` (the
        // `onsager-nodes::scheduler` + executors under
        // `onsager-nodes/src/{script,agent,verify,...}`); consumers will
        // arrive with OBS-01 (#361) once the Observer runtime is wired
        // up. Until then they're diagnostic-only so the dashboard run
        // timeline can render them without falsely claiming a
        // subsystem listener exists.
        EventDefinition {
            kind: "node.started",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard run timeline; Observer consumer arrives with OBS-01 (#361)",
            ),
            tracking_issue: Some(361),
            operator_grain: true,
            description: "Substrate scheduler dispatched a node — execution began.",
        },
        EventDefinition {
            kind: "node.completed",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard run timeline; Observer consumer arrives with OBS-01 (#361)",
            ),
            tracking_issue: Some(361),
            operator_grain: true,
            description: "A node finished successfully; outputs persisted under the edge's ArtifactId.",
        },
        EventDefinition {
            kind: "node.failed",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard run timeline; Observer consumer arrives with OBS-01 (#361)",
            ),
            tracking_issue: Some(361),
            operator_grain: true,
            description: "A node's executor returned Err; the plan is aborted (v1 — no retries).",
        },
        EventDefinition {
            kind: "node.awaiting_human",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard HITL inbox; Human executor approval round-trips via portal (#357)",
            ),
            tracking_issue: Some(357),
            operator_grain: true,
            description: "A Human executor is waiting on an out-of-band approval decision.",
        },
        EventDefinition {
            kind: "node.human_approved",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard HITL audit; the substrate's own Human executor consumes the approval inline",
            ),
            tracking_issue: None,
            operator_grain: true,
            description: "A pending Human executor node received an approval decision.",
        },
        EventDefinition {
            kind: "node.human_rejected",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard HITL audit; the substrate's own Human executor consumes the rejection inline",
            ),
            tracking_issue: None,
            operator_grain: true,
            description: "A pending Human executor node received a rejection decision.",
        },
        EventDefinition {
            kind: "synodic.verdict",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard run timeline as gate outcome; Observer / dashboard read it directly — distinct from the legacy `synodic.gate_verdict` emitted by the 0.1 Synodic subsystem (retired by MIG-03)",
            ),
            tracking_issue: None,
            operator_grain: true,
            description: "Verify executor produced a verdict — pass / fail outcome with per-check details.",
        },
        EventDefinition {
            kind: "agent.session_started",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard agent timeline; supersedes the 0.1 `stiglab.session_*` events once MIG-01 retires stiglab",
            ),
            tracking_issue: None,
            operator_grain: true,
            description: "Agent executor opened an LLM session.",
        },
        EventDefinition {
            kind: "agent.session_completed",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard agent timeline; carries optional token usage for budget consumers",
            ),
            tracking_issue: None,
            operator_grain: true,
            description: "Agent executor's LLM session finished successfully.",
        },
        EventDefinition {
            kind: "agent.session_failed",
            schema_version: 1,
            producers: &[Subsystem::Substrate],
            consumers: &[],
            diagnostic_only: true,
            reason: Some(
                "rendered in dashboard agent timeline; the executor wraps this into ExecutorError::Failed for the scheduler",
            ),
            tracking_issue: None,
            operator_grain: true,
            description: "Agent executor's LLM session terminated with an error.",
        },
        // -- Gate adapters (GitHub webhooks) --------------------------------
        EventDefinition {
            kind: "gate.check_updated",
            schema_version: 1,
            // Portal owns the GitHub webhook ingress as of #222 Slice 1.
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: false,
            description: "A GitHub check_suite/check_run/status arrived for a tracked PR.",
        },
        EventDefinition {
            kind: "gate.manual_approval_signal",
            schema_version: 1,
            // Portal owns the GitHub webhook ingress as of #222 Slice 1.
            producers: &[Subsystem::Portal],
            consumers: &[Subsystem::Substrate],
            diagnostic_only: false,
            reason: None,
            tracking_issue: None,
            operator_grain: true,
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
        assert!(first.get("tracking_issue").is_some());
        assert!(
            first
                .get("operator_grain")
                .and_then(|v| v.as_bool())
                .is_some(),
            "operator_grain must serialize as a bool"
        );
    }

    /// Operator-grain coverage (spec #465): at least one row in each
    /// state must be present so the Activity feed has signal and the
    /// allowlist isn't accidentally collapsed to "everything" or
    /// "nothing".
    #[test]
    fn manifest_has_both_operator_grain_states() {
        let grain_true = EVENTS.events.iter().filter(|e| e.operator_grain).count();
        let grain_false = EVENTS.events.iter().filter(|e| !e.operator_grain).count();
        assert!(
            grain_true > 0,
            "manifest must include at least one operator-grain event"
        );
        assert!(
            grain_false > 0,
            "manifest must include at least one non-operator-grain event"
        );
    }

    /// Spot-check the operator-grain partition against ADR 0019's
    /// definition: stage transitions / gate verdicts / run lifecycle
    /// surface; internal subsystem dispatches do not.
    #[test]
    fn operator_grain_partition_matches_adr_0019() {
        let surfaced = [
            "stage.entered",
            "stage.advanced",
            "synodic.gate_verdict",
            "synodic.verdict",
            "trigger.fired",
            "artifact.registered",
            "artifact.state_changed",
            "stiglab.session_completed",
            "node.failed",
        ];
        for kind in surfaced {
            let def = EVENTS.lookup(kind).expect(kind);
            assert!(
                def.operator_grain,
                "`{kind}` should be operator-grain per ADR 0019"
            );
        }
        let hidden = [
            "forge.shaping_dispatched",
            "forge.shaping_returned",
            "forge.gate_requested",
            "stiglab.session_result_ready",
            "ising.insight_emitted",
            "synodic.rule_proposed",
            "gate.check_updated",
        ];
        for kind in hidden {
            let def = EVENTS.lookup(kind).expect(kind);
            assert!(
                !def.operator_grain,
                "`{kind}` should NOT be operator-grain (internal dispatch/diagnostic)"
            );
        }
    }

    #[test]
    fn lookup_finds_known_kind_and_misses_unknown() {
        let def = EVENTS
            .lookup("forge.shaping_dispatched")
            .expect("known kind");
        assert!(def.producers.contains(&Subsystem::Substrate));
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
