//! MCP tool registry — single source of truth for what tools the
//! portal MCP server exposes.
//!
//! Two surfaces consume this:
//!
//! 1. The MCP server itself (`tools/list`, `tools/call` dispatch).
//! 2. `xtask check-tools-and-skills` (and `check-hitl-coverage`, when
//!    the dashboard side lands), which cross-references this registry
//!    against the public skills bundle and the dashboard's HitlCard
//!    slot map.
//!
//! Tool implementations live under `super::tools` (per-group files).
//! This module just wires names, descriptions, schemas, categories, and
//! invocation pointers together.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use schemars::schema::RootSchema;
use serde_json::Value;

use crate::auth::AuthUser;
use crate::state::AppState;

use super::ToolResult;
use super::tools;

/// HITL slot category — tells the dashboard which `HitlCard` variant
/// to render when this tool is invoked from chat. Mirrors the three
/// shapes from spec #288's HITL design (constructive / diff /
/// destructive) plus the read-only escape hatch.
///
/// Read-only tools render as plain info blocks in chat (no HITL card).
/// Mutation tools must declare a non-`ReadOnly` category;
/// `check-hitl-coverage` will hard-fail any mutation tool missing a
/// slot assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// New thing being created. Card renders editable fields and a
    /// tool-defined commit button.
    Constructive,
    /// Existing thing being mutated. Card renders before/after diff
    /// with `+`/`-`/`~` rows.
    Diff,
    /// Take-down / one-shot. Card renders side-effects + reversibility
    /// copy; irreversibles require type-to-confirm.
    Destructive,
    /// No mutation — read / query / lookup. No HITL card.
    ReadOnly,
}

/// Boxed async handler signature shared by every tool.
pub type ToolInvoke = for<'a> fn(
    &'a AppState,
    &'a AuthUser,
    Value,
) -> Pin<Box<dyn Future<Output = ToolResult> + Send + 'a>>;

/// One registered tool. The `input_schema` is the `schemars`-derived
/// JSON Schema for the tool's argument shape; clients consume it via
/// `tools/list`.
pub struct ToolDescriptor {
    pub name: &'static str,
    pub description: &'static str,
    pub category: ToolCategory,
    pub input_schema: RootSchema,
    pub invoke: ToolInvoke,
}

impl std::fmt::Debug for ToolDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolDescriptor")
            .field("name", &self.name)
            .field("category", &self.category)
            .finish_non_exhaustive()
    }
}

/// Lazily-initialized global registry. Adding a tool means a single
/// entry here plus its implementation under `tools::<group>`.
pub fn registry() -> &'static [ToolDescriptor] {
    static REG: OnceLock<Vec<ToolDescriptor>> = OnceLock::new();
    REG.get_or_init(build_registry).as_slice()
}

fn build_registry() -> Vec<ToolDescriptor> {
    vec![
        // --- Action tools (workflows) ---
        ToolDescriptor {
            name: "propose_workflow",
            description: "Create a new workflow blueprint (trigger + ordered stages). The workflow is always created inactive — the activation pipeline (GitHub label + webhook register) needs request headers the MCP entry point doesn't yet plumb through. Activate via the REST PATCH endpoint after creation; passing `active: true` here returns an InvalidParams error.",
            category: ToolCategory::Constructive,
            input_schema: super::input_schema::<tools::workflows::ProposeWorkflowArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::workflows::propose_workflow(state, user, args))
            },
        },
        ToolDescriptor {
            name: "propose_workflow_draft",
            description: "Propose a workflow *draft* (no workspace, no install, no repo). Validates the `{name, trigger, stages}` shape and echoes it back so the dashboard can route through the FTUE HitlCard and into client-side draft storage. Drafts live in localStorage until the binding flow (#402) promotes them into a real spine workflow via `propose_workflow`. Use this tool from the workspace-less `/chat` entry; for workspace-scoped design use `propose_workflow` directly.",
            category: ToolCategory::Constructive,
            input_schema: super::input_schema::<tools::workflows::ProposeWorkflowDraftArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::workflows::propose_workflow_draft(state, user, args))
            },
        },
        ToolDescriptor {
            name: "run_workflow",
            description: "Fire a workflow's `manual` trigger to start a new run. Workflow must be active and declare a matching manual trigger name.",
            category: ToolCategory::Destructive,
            input_schema: super::input_schema::<tools::workflows::RunWorkflowArgs>(),
            invoke: |state, user, args| Box::pin(tools::workflows::run_workflow(state, user, args)),
        },
        ToolDescriptor {
            name: "edit_workflow",
            description: "Deactivate an existing workflow (`active: false`). Re-activation is not supported via MCP today — the activation pipeline runs GitHub side-effects (label + webhook register) that need request headers the MCP entry point doesn't yet plumb through; use the REST PATCH endpoint to re-activate. Rename and stage-chain replacement are also REST-only for now.",
            category: ToolCategory::Diff,
            input_schema: super::input_schema::<tools::workflows::EditWorkflowArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::workflows::edit_workflow(state, user, args))
            },
        },
        ToolDescriptor {
            name: "schedule_workflow",
            description: "Set or update the workflow's trigger (typically `cron` / `interval` / `delay`, but any registered trigger kind is accepted). Replaces any current trigger. Validates the kind against the registry manifest and rejects the self-amplifying `spine_event { event_kind: \"trigger.fired\" }` case — same guards `propose_workflow` runs.",
            category: ToolCategory::Diff,
            input_schema: super::input_schema::<tools::workflows::ScheduleWorkflowArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::workflows::schedule_workflow(state, user, args))
            },
        },
        ToolDescriptor {
            name: "list_workflows",
            description: "List workflows in a workspace.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::workflows::ListWorkflowsArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::workflows::list_workflows(state, user, args))
            },
        },
        // --- Action tools (runs) ---
        ToolDescriptor {
            name: "list_runs",
            description: "List recent runs (one per artifact) for a given workflow.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::runs::ListRunsArgs>(),
            invoke: |state, user, args| Box::pin(tools::runs::list_runs(state, user, args)),
        },
        ToolDescriptor {
            name: "cancel_run",
            description: "Abort an in-flight run: archives the artifact (sets `state = 'archived'`) and emits `artifact.archived` on the `forge:<artifact_id>` stream. Mirrors REST `POST /api/spine/artifacts/:id/abort`. Irreversible at the artifact level — the row is archived synchronously; downstream consumers see the same event shape as the dashboard abort path.",
            category: ToolCategory::Destructive,
            input_schema: super::input_schema::<tools::runs::CancelRunArgs>(),
            invoke: |state, user, args| Box::pin(tools::runs::cancel_run(state, user, args)),
        },
        // --- Diagnostic tools ---
        ToolDescriptor {
            name: "inspect_run",
            description: "Return a structured summary of a run: artifact metadata, current stage, parked reason if any, and recent spine events.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::diagnostics::InspectRunArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::diagnostics::inspect_run(state, user, args))
            },
        },
        ToolDescriptor {
            name: "get_stage_logs",
            description: "Fetch log chunks for an agent-session stage by session_id. Returns ordered chunks plus current session state.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::diagnostics::GetStageLogsArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::diagnostics::get_stage_logs(state, user, args))
            },
        },
        ToolDescriptor {
            name: "get_artifact",
            description: "Return a single spine artifact's metadata and current state.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::artifacts::GetArtifactArgs>(),
            invoke: |state, user, args| Box::pin(tools::artifacts::get_artifact(state, user, args)),
        },
        ToolDescriptor {
            name: "propose_remediation",
            description: "Server-side AI analysis of a failed run. Reads the artifact's state, recent spine events, and trailing session logs, then asks Claude for `proposed_actions` (registered tool names + concrete arguments) the operator can review via HitlCard. Requires an `ANTHROPIC_API_KEY` workspace credential; falls back to the v1 stub envelope (state + log pointers, no AI call) when the credential is missing, the per-workspace monthly budget is exhausted, or the model call errors. Pass `model: \"opus\"` for hard cases (cost: ~5x sonnet); defaults to Sonnet.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::diagnostics::ProposeRemediationArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::diagnostics::propose_remediation(state, user, args))
            },
        },
        // --- 0.2 substrate authoring (spec #395) ---
        ToolDescriptor {
            name: "submit_spec_plan",
            description: "Persist a new substrate `SpecPlan` (ADR 0015) under `(workspace_id, spec_plan_id)`. The `spec_plan` argument carries the full `{ specs, deps }` JSON shape the substrate consumes; structural checks (`SpecPlan::validate`) run before insert so cycles and dangling deps surface as `InvalidParams`. First-write only — duplicate `spec_plan_id` is rejected.",
            category: ToolCategory::Constructive,
            input_schema: super::input_schema::<tools::substrate_specs::SubmitSpecPlanArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_specs::submit_spec_plan(state, user, args))
            },
        },
        ToolDescriptor {
            name: "update_spec",
            description: "Replace a single `SpecRef` inside an existing `SpecPlan`. Identity is matched on `spec.id`; renaming is not supported (submit a fresh plan via `submit_spec_plan` instead). The plan is re-validated after the swap, so a malformed update (dangling dep, dupe id with a sibling) is rejected on the call that introduced it.",
            category: ToolCategory::Diff,
            input_schema: super::input_schema::<tools::substrate_specs::UpdateSpecArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_specs::update_spec(state, user, args))
            },
        },
        ToolDescriptor {
            name: "list_spec_plans",
            description: "List every persisted `SpecPlan` in a workspace, newest first.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::substrate_specs::ListSpecPlansArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_specs::list_spec_plans(state, user, args))
            },
        },
        ToolDescriptor {
            name: "get_spec_plan",
            description: "Read a single `SpecPlan` by `(workspace_id, spec_plan_id)`.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::substrate_specs::GetSpecPlanArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_specs::get_spec_plan(state, user, args))
            },
        },
        ToolDescriptor {
            name: "compile_dry_run",
            description: "Run the Plan Compiler (ADR 0017) against a candidate `SpecPlan` over the workspace's current Workflow Library snapshot. Returns the resulting `ExecutionPlan` shape (node/edge counts + per-spec entry/exit edges + the library versions that were resolved) on success, or the full `CompileError` payload on failure — including the batched `Invariant(Vec)` set from ADR 0018. No persistence; this is the linter for authors.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::substrate_specs::CompileDryRunArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_specs::compile_dry_run(state, user, args))
            },
        },
        ToolDescriptor {
            name: "get_execution_plan",
            description: "Load a persisted `SpecPlan` by id and recompile it on read against the latest Workflow Library snapshot. Same response shape as `compile_dry_run`. v1 recompiles on every call — caching is a later spec.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::substrate_specs::GetExecutionPlanArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_specs::get_execution_plan(
                    state, user, args,
                ))
            },
        },
        ToolDescriptor {
            name: "submit_workflow",
            description: "Register a new `Workflow` (ADR 0016) for `spec_kind` in the substrate Workflow Library. Inserts a fresh monotonic version (per-kind `MAX(version) + 1`); the new row becomes the active version the Plan Compiler resolves. Workflow payload round-trips via the substrate's `Workflow` serde derive (executors flow through `typetag`'s `kind` discriminator).",
            category: ToolCategory::Constructive,
            input_schema: super::input_schema::<tools::substrate_workflows::SubmitWorkflowArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_workflows::submit_workflow(
                    state, user, args,
                ))
            },
        },
        ToolDescriptor {
            name: "update_workflow",
            description: "Append a new version of a `Workflow` for `spec_kind` (ADR 0016 — one active version per kind, latest wins). Semantically identical to `submit_workflow`; the separate tool exists so the dashboard HitlCard can render a diff against the current active version. Argument shape is unchanged.",
            category: ToolCategory::Diff,
            input_schema: super::input_schema::<tools::substrate_workflows::UpdateWorkflowArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_workflows::update_workflow(
                    state, user, args,
                ))
            },
        },
        ToolDescriptor {
            name: "retire_workflow",
            description: "Mark the currently-active `Workflow` version for `spec_kind` inactive via the `retired_at` column. Compile passes stop resolving the kind on the next call; previous versions are preserved as audit history. Irreversible at the row level — a fresh `submit_workflow` is needed to re-establish an active workflow.",
            category: ToolCategory::Destructive,
            input_schema: super::input_schema::<tools::substrate_workflows::RetireWorkflowArgs>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_workflows::retire_workflow(
                    state, user, args,
                ))
            },
        },
        ToolDescriptor {
            name: "list_workflows_v2",
            description: "List one card per `spec_kind` in the substrate Workflow Library — the latest version plus the retire status. Workflow bodies are omitted from this view; use `get_workflow_v2` for the full graph.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::substrate_workflows::ListWorkflowsV2Args>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_workflows::list_workflows_v2(
                    state, user, args,
                ))
            },
        },
        ToolDescriptor {
            name: "get_workflow_v2",
            description: "Read a substrate `Workflow` by `(spec_kind, version)`. `version` omitted resolves the latest **active** version; an explicit retired version still returns so authors can inspect what they shipped before retire.",
            category: ToolCategory::ReadOnly,
            input_schema: super::input_schema::<tools::substrate_workflows::GetWorkflowV2Args>(),
            invoke: |state, user, args| {
                Box::pin(tools::substrate_workflows::get_workflow_v2(
                    state, user, args,
                ))
            },
        },
    ]
}
