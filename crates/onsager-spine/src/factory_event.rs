// budget-allow: factory event enum (~40 variants after spec #285 prune); macro/derive compression deferred to follow-up spec per #261
//! Factory events — the authoritative event types emitted to the factory event
//! spine by each subsystem.
//!
//! See `specs/forge-v0.1.md §9` for the Forge event contract. Additional event
//! types from the other subsystems are included here so that the spine
//! library provides a single typed vocabulary.

use chrono::{DateTime, Utc};
use onsager_artifact::{ArtifactId, ArtifactState, Kind, NodeId};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Factory event envelope
// ---------------------------------------------------------------------------

/// A factory event as written to the event spine.
///
/// All subsystems write events through this envelope. The `event` field carries
/// the typed payload; the wrapper carries tracing metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryEvent {
    /// The typed event payload.
    pub event: FactoryEventKind,
    /// Correlation ID for tracing a causal chain of events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// ID of the event that caused this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<i64>,
    /// The subsystem or actor that produced this event.
    pub actor: String,
    /// Timestamp (usually DB-assigned, but included for serialization).
    pub timestamp: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Typed event payloads
// ---------------------------------------------------------------------------

/// All factory event types across all subsystems.
///
/// ## Forge events (authoritative per forge-v0.1 §9)
///
/// ## Session events (chat-session lifecycle — portal's in-process runner)
///
/// `f64` fields (confidence) block `Eq`; only `PartialEq` is derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FactoryEventKind {
    // -- Artifact lifecycle (Forge) -----------------------------------------
    /// New artifact accepted and ID assigned.
    ArtifactRegistered {
        artifact_id: ArtifactId,
        kind: Kind,
        name: String,
        owner: String,
    },

    /// Artifact transitioned between lifecycle states.
    ArtifactStateChanged {
        artifact_id: ArtifactId,
        from_state: ArtifactState,
        to_state: ArtifactState,
    },

    /// Artifact reached terminal state (archived).
    ArtifactArchived {
        artifact_id: ArtifactId,
        reason: String,
    },

    // -- Git lifecycle events -------------------------------------------------
    GitPrOpened {
        artifact_id: ArtifactId,
        repo: String,
        pr_number: u64,
        url: String,
    },
    GitCiCompleted {
        artifact_id: ArtifactId,
        pr_number: u64,
        check_name: String,
        conclusion: String,
    },
    GitPrMerged {
        artifact_id: ArtifactId,
        pr_number: u64,
        merge_sha: String,
    },
    GitPrClosed {
        artifact_id: ArtifactId,
        pr_number: u64,
    },

    // -- Forge process events -----------------------------------------------
    // -- Chat-session lifecycle (portal's in-process runner, ADR 0027) ------
    /// A chat agent session finished successfully. Emitted by portal's
    /// in-process session runner (spec #583; formerly
    /// `stiglab.session_completed`).
    SessionCompleted {
        session_id: String,
        duration_ms: u64,
        /// Artifact this session was shaping (issue #14 phase 2). Optional so
        /// non-shaping sessions (e.g. direct task POSTs) don't emit a
        /// meaningless id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<String>,
        /// LLM token usage for this session (issue #39). Optional so
        /// pre-accounting sessions and mock dispatchers don't fabricate a
        /// zero bill — `None` means "not reported", not "cost nothing".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_usage: Option<TokenUsage>,
        /// Working-tree branch the agent pushed at session completion (issue
        /// #60). Used by `onsager-portal` to attach `vertical_lineage` when
        /// the matching PR webhook arrives. Optional — sessions that don't
        /// touch a git working dir leave it `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// PR number the agent opened, when known at completion time.
        /// Optional for the same reasons as `branch`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pr_number: Option<u64>,
    },

    /// A chat agent session terminated with an error. Emitted by portal's
    /// in-process session runner (spec #583; formerly
    /// `stiglab.session_failed`).
    SessionFailed {
        session_id: String,
        error: String,
        /// Artifact this session was shaping (issue #156). Optional so
        /// non-shaping sessions (direct task POSTs) don't emit a
        /// meaningless id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<String>,
    },

    // -- Portal intents (dashboard → session runner) -------------------------
    /// Dashboard task request from portal. Diagnostic timeline record —
    /// dispatch is in-process post-ADR 0027 (spec #583); portal's
    /// `SessionRunner` spawns the session directly.
    PortalSessionRequested { session_id: String },

    /// Portal failed to open a GitHub PR for a completed session (spec #273).
    /// Emitted as a diagnostic signal when the GitHub API call fails, the
    /// session pushed an empty branch, or the GitHub App is not configured.
    /// No retry is attempted inside the listener — the operator investigates
    /// via the dashboard event timeline.
    PortalPrOpenFailed {
        /// Issue artifact the session was shaping, if known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<ArtifactId>,
        /// Branch the session pushed to, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// Workspace the session belongs to.
        workspace_id: String,
        /// Short reason code: `"empty_branch"`, `"github_api_error"`,
        /// `"no_installation"`, `"app_not_configured"`, etc.
        reason: String,
    },

    /// Dashboard cancel-session request from portal (spec #303). Diagnostic
    /// timeline record — the cancel itself is applied in-process by
    /// portal's `SessionRunner` (spec #583). Best-effort: a terminal
    /// session makes the cancel a no-op.
    PortalSessionCancelRequested {
        session_id: String,
        /// Actor identifier captured for audit / debugging.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
    },

    // -- Workflow runtime events (issue #80) --------------------------------
    /// A trigger (e.g. a GitHub issue webhook) fired and produced a payload
    /// the trigger subscriber will translate into an artifact registration.
    /// Emitted by the portal webhook receiver; consumed by the scheduler.
    TriggerFired {
        /// Workflow whose trigger fired.
        workflow_id: String,
        /// Trigger classification (matches the `workflows.trigger_kind`
        /// column). v1 always `"github_issue_webhook"`.
        trigger_kind: String,
        /// Free-form payload the subscriber needs to translate the trigger
        /// into an artifact (e.g. issue number, title, body, repo).
        payload: serde_json::Value,
    },

    /// Audit record for a manual / CLI / replay trigger fire (#241).
    /// Emitted alongside the underlying `TriggerFired` so audit views
    /// can attribute fires to a user. `event_subtype` distinguishes
    /// surfaces (`"ui_fire"`, `"ui_replay"`, `"cli_fire"`,
    /// `"cli_replay"`); `detail` carries source-specific fields
    /// (manual name, source event id).
    WorkflowManualTriggered {
        workflow_id: String,
        workspace_id: String,
        actor: String,
        event_subtype: String,
        detail: serde_json::Value,
    },

    /// All gates on a stage resolved and the artifact advanced to the next
    /// stage (or reached terminal state when this was the last stage).
    ///
    /// Per spec #285 the redundant per-gate `stage.gate_passed` /
    /// `stage.gate_failed` events are gone; the run timeline reconstructs
    /// gate outcomes from `verify.verdict` plus this advancement
    /// signal.
    StageAdvanced {
        artifact_id: ArtifactId,
        workflow_id: String,
        from_stage_index: u32,
        /// `None` when the artifact has just completed the final stage.
        to_stage_index: Option<u32>,
    },

    /// A persisted multi-spec `SpecPlan` was requested to run (#501,
    /// ADR 0023). Emitted by portal's `run_spec_plan` MCP tool;
    /// consumed by the substrate scheduler host, which loads the stored
    /// plan from `spec_plans`, compiles it, and drives it to
    /// completion. Portal does not run the plan itself — the seam rule
    /// (portal is the edge, the scheduler host is the runtime).
    SpecPlanRunRequested {
        /// `(workspace_id, spec_plan_id)` keys the `spec_plans` row the
        /// scheduler loads and compiles.
        spec_plan_id: String,
        workspace_id: String,
    },

    /// A plan run reached a terminal state (#536). Emitted by the
    /// substrate scheduler when [`SpecPlanRunRequested`] finishes;
    /// portal consumes it to revoke the plan-scoped session token. The
    /// `plan_id` is the derived run id (`derive_plan_id`), not the
    /// spec_plan_id.
    PlanRunCompleted {
        plan_id: String,
        workspace_id: String,
        spec_plan_id: String,
    },

    /// A plan run failed terminally (#536). Same producer/consumer as
    /// [`PlanRunCompleted`] — portal revokes the plan-scoped token on
    /// either terminal signal.
    PlanRunFailed {
        plan_id: String,
        workspace_id: String,
        spec_plan_id: String,
    },

    // -- Registry events removed by spec #285 -------------------------------
    // The nine `registry.*` mutation events (`type_proposed`, `type_approved`,
    // `type_deprecated`, `adapter_registered`, `adapter_deprecated`,
    // `gate_registered`, `gate_deprecated`, `profile_registered`,
    // `profile_deprecated`) had no in-tree consumer or dashboard reader;
    // the registry tables are still the source of truth, but mutations no
    // longer publish a spine event. Add a variant back here when there is
    // a real consumer.

    // -- Workflow events (issue #81 — workflow CRUD + webhook) --------------
    // `TriggerFired` is defined above in the issue #80 block; the
    // webhook router emits the same variant so forge's trigger subscriber can
    // consume it with no translation.
    /// A GitHub `check_suite`, `check_run`, or `status` event arrived for a
    /// PR we care about. Forge's external-check gate consumes this to advance
    /// or block artifacts whose current stage is `external-check`.
    GateCheckUpdated {
        repo_owner: String,
        repo_name: String,
        pr_number: u64,
        check_name: String,
        conclusion: String,
    },

    /// A manual-approval gate received a signal (e.g. the PR was merged).
    /// Forge's manual-approval gate advances when this arrives.
    GateManualApprovalSignal {
        repo_owner: String,
        repo_name: String,
        pr_number: u64,
        source: String,
    },

    // -- Substrate runtime (RUN-02, #360) -----------------------------------
    // Lifecycle events the 0.2 substrate scheduler and the executor
    // catalog emit. The on-wire shapes mirror the typed payload structs
    // in `onsager-substrate::events`; the structs there are the
    // authoring surface for substrate-side code, this enum is the
    // single wire vocabulary the spine read API hands to the dashboard
    // and (post OBS-01 / #361) to Observers.
    //
    // `plan_id` is the externally-assigned identifier for one
    // Execution Plan run (`onsager-nodes::scheduler::PlanId`) carried
    // as a `String` here so the spine crate doesn't need to depend on
    // the runtime; downstream consumers parse it back into `PlanId`
    // via `PlanId::new(...)`.
    /// Substrate scheduler dispatched a node — execution began.
    NodeStarted {
        plan_id: String,
        node_id: NodeId,
        /// Executor catalog key (`"script"`, `"agent"`, `"verify"`,
        /// `"noop"`; the Human / SubWorkflow impls retired with #585).
        executor_kind: String,
    },

    /// A node finished successfully; the scheduler persisted each output
    /// artifact under its declared edge `ArtifactId`.
    NodeCompleted {
        plan_id: String,
        node_id: NodeId,
        /// `ArtifactId`s the executor materialized — order matches the
        /// node's declared output edges.
        output_artifact_ids: Vec<ArtifactId>,
        /// `Some(true)` when the executor declared it produced nothing
        /// on purpose (an intentional empty delivery), distinguishing
        /// it from "delivered nothing" (`output_artifact_ids` empty,
        /// field absent). Populated by the authoritative gate (§4c,
        /// #520); `None` until then so behavior is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        declared_empty: Option<bool>,
    },

    /// A node's executor returned Err. The scheduler aborts the plan
    /// (v1; no retries).
    NodeFailed {
        plan_id: String,
        node_id: NodeId,
        /// Free-text error message from the executor.
        error: String,
    },

    /// A node is parked waiting on an out-of-band human decision.
    /// Emitted by the Agent executor's authoritative gate (#520 §4c)
    /// when a session delivers no output; surfaces in the operator
    /// activity feed. (The Human executor that resolved these inline
    /// retired with #585.)
    NodeAwaitingHuman {
        plan_id: String,
        node_id: NodeId,
        /// Free-text prompt shown to the human reviewer.
        prompt: String,
    },

    /// Verify executor produced a verdict — pass / fail outcome with
    /// per-check details. Per spec #360 this is the substrate-native
    /// verdict event (renamed when the governance subsystem retired,
    /// ADR 0027 / spec #582).
    VerifyVerdict {
        plan_id: String,
        node_id: NodeId,
        /// True when every check the executor ran passed.
        passed: bool,
        /// Per-check (name, passed) results so the dashboard can
        /// render the failure surface without a follow-up read.
        check_results: Vec<VerifyCheckResult>,
    },

    /// Agent executor opened an LLM session.
    AgentSessionStarted {
        plan_id: String,
        node_id: NodeId,
        /// Session identifier minted by the agent runner.
        session_id: String,
        /// Model name (`"claude-sonnet-4-6"`, etc.).
        model: String,
    },

    /// Agent executor's LLM session finished successfully.
    AgentSessionCompleted {
        plan_id: String,
        node_id: NodeId,
        session_id: String,
        /// Token usage for this session — `None` when the runner does
        /// not report it (stub / mock runners).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_usage: Option<TokenUsage>,
    },

    /// Agent executor's LLM session terminated with an error.
    AgentSessionFailed {
        plan_id: String,
        node_id: NodeId,
        session_id: String,
        /// Free-text error from the runner.
        error: String,
    },
}

impl FactoryEventKind {
    /// Returns the dot-separated event type string (e.g., "artifact.registered").
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ArtifactRegistered { .. } => "artifact.registered",
            Self::ArtifactStateChanged { .. } => "artifact.state_changed",
            Self::ArtifactArchived { .. } => "artifact.archived",
            Self::GitPrOpened { .. } => "git.pr_opened",
            Self::GitCiCompleted { .. } => "git.ci_completed",
            Self::GitPrMerged { .. } => "git.pr_merged",
            Self::GitPrClosed { .. } => "git.pr_closed",
            Self::SessionCompleted { .. } => "session.completed",
            Self::SessionFailed { .. } => "session.failed",
            Self::PortalSessionRequested { .. } => "portal.session_requested",
            Self::PortalSessionCancelRequested { .. } => "portal.session_cancel_requested",
            Self::PortalPrOpenFailed { .. } => "portal.pr_open_failed",
            Self::TriggerFired { .. } => "trigger.fired",
            Self::WorkflowManualTriggered { .. } => "workflow.manual_triggered",
            Self::StageAdvanced { .. } => "stage.advanced",
            Self::SpecPlanRunRequested { .. } => "plan.run_requested",
            Self::PlanRunCompleted { .. } => "plan.run_completed",
            Self::PlanRunFailed { .. } => "plan.run_failed",
            Self::GateCheckUpdated { .. } => "gate.check_updated",
            Self::GateManualApprovalSignal { .. } => "gate.manual_approval_signal",
            Self::NodeStarted { .. } => "node.started",
            Self::NodeCompleted { .. } => "node.completed",
            Self::NodeFailed { .. } => "node.failed",
            Self::NodeAwaitingHuman { .. } => "node.awaiting_human",
            Self::VerifyVerdict { .. } => "verify.verdict",
            Self::AgentSessionStarted { .. } => "agent.session_started",
            Self::AgentSessionCompleted { .. } => "agent.session_completed",
            Self::AgentSessionFailed { .. } => "agent.session_failed",
        }
    }

    /// Returns the stream_type for this event.
    pub fn stream_type(&self) -> &'static str {
        match self {
            Self::ArtifactRegistered { .. }
            | Self::ArtifactStateChanged { .. }
            | Self::ArtifactArchived { .. } => "artifact",
            Self::GitPrOpened { .. }
            | Self::GitCiCompleted { .. }
            | Self::GitPrMerged { .. }
            | Self::GitPrClosed { .. } => "git",
            Self::SessionCompleted { .. } | Self::SessionFailed { .. } => "session",
            Self::PortalSessionRequested { .. }
            | Self::PortalSessionCancelRequested { .. }
            | Self::PortalPrOpenFailed { .. } => "portal",
            Self::TriggerFired { .. } | Self::StageAdvanced { .. } => "workflow",
            // Plan-run intent (portal → scheduler host) and the
            // plan-terminal signals (scheduler → portal, #536). Their
            // own namespace so the dashboard can scope plan-run
            // lifecycle without mixing it with per-node substrate
            // lifecycle.
            Self::SpecPlanRunRequested { .. }
            | Self::PlanRunCompleted { .. }
            | Self::PlanRunFailed { .. } => "plan",
            // Audit-only fan-out for manual fires (#241). Lives in the
            // `audit` namespace so audit views can filter without
            // mixing with first-class workflow runtime events.
            Self::WorkflowManualTriggered { .. } => "audit",
            Self::GateCheckUpdated { .. } | Self::GateManualApprovalSignal { .. } => "gate",
            // Substrate runtime — node lifecycle, Verify verdicts, and
            // agent-session signals share the `substrate` stream so the
            // dashboard / Observer (#361) can subscribe with one prefix.
            Self::NodeStarted { .. }
            | Self::NodeCompleted { .. }
            | Self::NodeFailed { .. }
            | Self::NodeAwaitingHuman { .. }
            | Self::VerifyVerdict { .. }
            | Self::AgentSessionStarted { .. }
            | Self::AgentSessionCompleted { .. }
            | Self::AgentSessionFailed { .. } => "substrate",
        }
    }

    /// Returns the primary entity ID this event relates to.
    pub fn stream_id(&self) -> String {
        match self {
            Self::ArtifactRegistered { artifact_id, .. }
            | Self::ArtifactStateChanged { artifact_id, .. }
            | Self::ArtifactArchived { artifact_id, .. } => artifact_id.to_string(),
            Self::GitPrOpened { artifact_id, .. }
            | Self::GitCiCompleted { artifact_id, .. }
            | Self::GitPrMerged { artifact_id, .. }
            | Self::GitPrClosed { artifact_id, .. } => artifact_id.to_string(),
            Self::SessionCompleted { session_id, .. } | Self::SessionFailed { session_id, .. } => {
                session_id.clone()
            }
            Self::PortalSessionRequested { session_id, .. }
            | Self::PortalSessionCancelRequested { session_id, .. } => session_id.clone(),
            Self::PortalPrOpenFailed {
                workspace_id,
                branch,
                ..
            } => branch
                .as_deref()
                .map(|b| format!("portal:pr_open_failed:{b}"))
                .unwrap_or_else(|| format!("portal:pr_open_failed:{workspace_id}")),
            Self::TriggerFired { workflow_id, .. } => format!("workflow:{workflow_id}"),
            Self::WorkflowManualTriggered { workflow_id, .. } => {
                format!("audit:workflow:{workflow_id}")
            }
            Self::StageAdvanced { artifact_id, .. } => {
                format!("workflow:{artifact_id}")
            }
            Self::SpecPlanRunRequested {
                workspace_id,
                spec_plan_id,
            } => format!("plan:{workspace_id}:{spec_plan_id}"),
            // Keyed by the derived run id so portal's revoke listener
            // reads `plan_id` straight off the stream (#536).
            Self::PlanRunCompleted { plan_id, .. } | Self::PlanRunFailed { plan_id, .. } => {
                format!("plan:{plan_id}")
            }
            Self::GateCheckUpdated {
                repo_owner,
                repo_name,
                pr_number,
                ..
            }
            | Self::GateManualApprovalSignal {
                repo_owner,
                repo_name,
                pr_number,
                ..
            } => format!("{repo_owner}/{repo_name}#{pr_number}"),
            // Substrate runtime — `(plan_id, node_id)` is the natural
            // composite key so the dashboard run timeline can scope
            // by plan and node without parsing the payload.
            Self::NodeStarted {
                plan_id, node_id, ..
            }
            | Self::NodeCompleted {
                plan_id, node_id, ..
            }
            | Self::NodeFailed {
                plan_id, node_id, ..
            }
            | Self::NodeAwaitingHuman {
                plan_id, node_id, ..
            }
            | Self::VerifyVerdict {
                plan_id, node_id, ..
            }
            | Self::AgentSessionStarted {
                plan_id, node_id, ..
            }
            | Self::AgentSessionCompleted {
                plan_id, node_id, ..
            }
            | Self::AgentSessionFailed {
                plan_id, node_id, ..
            } => format!("plan:{plan_id}:{node_id}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Supporting enums
// ---------------------------------------------------------------------------

/// Gate points in the legacy 0.1 gate protocol. Kept for historical
/// spine rows; the governance subsystem retired with ADR 0027.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatePoint {
    PreDispatch,
    StateTransition,
    ConsumerRouting,
    ToolLevel,
    /// A run wants to *write* (push + open PR) to a bound-but-unpinned
    /// repo it selected mid-run (spec #548, Part of #544). Binding a repo
    /// to a workspace grants candidacy, not blanket write consent — this
    /// gate is the consent point. The write-scoped token is minted only
    /// after the verdict is `Allow`. The pinned repo (named in the
    /// trigger) is pre-approved and never reaches this point.
    RepoWrite,
}

/// LLM token usage carried on [`FactoryEventKind::SessionCompleted`]
/// (issue #39). Accounting primitives only — USD cost is resolved downstream
/// by the budget consumer, not on the event, so we don't have to version the
/// pricing table every time a model changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export)]
pub struct TokenUsage {
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    /// Cache-read input tokens (Anthropic-style prompt caching). Zero for
    /// providers without a cache concept.
    #[serde(default)]
    #[ts(type = "number")]
    pub cache_read_tokens: u64,
    /// Cache-creation input tokens.
    #[serde(default)]
    #[ts(type = "number")]
    pub cache_write_tokens: u64,
    /// Model identifier (`"claude-sonnet-4-6"`, etc.) so the downstream
    /// pricing table can resolve cost without guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// One check's outcome carried on [`FactoryEventKind::VerifyVerdict`].
/// The Verify executor's [`Check`](../../../crates/onsager-nodes/src/verify.rs)
/// list maps 1:1 onto this struct list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyCheckResult {
    /// Check name (e.g. `"cargo_test"`, `"clippy"`, `"schema_lint"`).
    pub name: String,
    /// `true` when this check passed.
    pub passed: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[path = "factory_event_tests.rs"]
#[cfg(test)]
mod tests;
