//! MCP tools that mutate or list workflows.
//!
//! Each tool delegates to the same DB helpers (`workflow_db`) that the
//! REST handlers in `crate::handlers::workflows` use; activation
//! side-effects (label creation, webhook registration) run inline via
//! `crate::workflow_activation` exactly as they do in the REST path.

use chrono::Utc;
use onsager_registry::TRIGGERS;
use onsager_spine::TriggerKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::state::AppState;
use crate::workflow::{GateKind, Workflow, WorkflowStage};
use crate::workflow_db;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

// =============================================================================
// propose_workflow
// =============================================================================

/// Arguments for `propose_workflow`. The trigger config is supplied as
/// a fully-formed [`TriggerKind`] so the MCP client can declare any
/// supported trigger shape without having to know the flat-field form
/// the REST `POST /api/workflows` endpoint accepts.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeWorkflowArgs {
    pub workspace_id: String,
    pub name: String,
    pub trigger: TriggerKind,
    /// GitHub App installation id this workflow fires under. Required
    /// for `github_*` triggers and webhook activation side-effects;
    /// schedule/manual triggers may pass `0` when there is no install
    /// to bind.
    #[serde(default)]
    pub install_id: i64,
    /// Ordered stage chain. At least one stage is required.
    pub stages: Vec<StageInput>,
    /// Activate the workflow inline (runs label create + webhook
    /// register for GitHub triggers).
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StageInput {
    pub gate_kind: GateKind,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct WorkflowEnvelope<'a> {
    workflow: &'a Workflow,
    stages: &'a [WorkflowStage],
}

pub async fn propose_workflow(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: ProposeWorkflowArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid propose_workflow args: {e}")))?;

    if args.name.trim().is_empty() {
        return Err(ToolError::InvalidParams("name is required".into()));
    }
    if args.stages.is_empty() {
        return Err(ToolError::InvalidParams(
            "at least one stage is required".into(),
        ));
    }
    if args.active {
        // Activation runs the GitHub side-effects pipeline (label
        // create + webhook register) which needs the request `HeaderMap`
        // for forwarded-host resolution — context the MCP entry point
        // doesn't plumb today. Setting `active=true` without that
        // pipeline would leave GitHub-trigger workflows in an
        // inconsistent state (active in spine, no webhook in repo).
        // Reject explicitly; clients activate via the REST PATCH or
        // (once the headers plumb-through follow-up lands) a future
        // version of this tool.
        return Err(ToolError::InvalidParams(
            "MCP propose_workflow cannot activate inline — omit `active` or pass `false`, \
             then activate via the REST PATCH endpoint (the activation pipeline needs \
             request headers the MCP entry point doesn't yet plumb through)"
                .into(),
        ));
    }

    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;
    let spine = state.spine.pool();

    let workflow_id = format!("wf_{}", Uuid::new_v4());
    let stages: Vec<WorkflowStage> = args
        .stages
        .iter()
        .enumerate()
        .map(|(i, s)| WorkflowStage {
            id: Uuid::new_v4().to_string(),
            workflow_id: workflow_id.clone(),
            seq: i as i32,
            gate_kind: s.gate_kind,
            params: s.params.clone().unwrap_or_else(|| serde_json::json!({})),
        })
        .collect();

    let now = Utc::now();
    let workflow = Workflow {
        id: workflow_id.clone(),
        workspace_id: args.workspace_id.clone(),
        name: args.name.trim().to_string(),
        trigger: args.trigger,
        install_id: args.install_id,
        preset_id: None,
        active: false,
        created_by: auth_user.user_id.clone(),
        created_at: now,
        updated_at: now,
    };

    workflow_db::insert_workflow_with_stages(spine, &workflow, &stages)
        .await
        .map_err(|e| {
            tracing::error!("mcp propose_workflow insert failed: {e}");
            ToolError::Internal(format!("failed to insert workflow: {e}"))
        })?;

    let envelope = WorkflowEnvelope {
        workflow: &workflow,
        stages: &stages,
    };
    serde_json::to_value(envelope).map_err(internal_serialize)
}

// =============================================================================
// propose_workflow_draft
// =============================================================================

/// Arguments for `propose_workflow_draft` (spec #413, paired with the
/// `onsager-ftue-chat` skill grant). The shape mirrors what the dashboard
/// authors client-side as a `WorkflowDocument` — same `{name, trigger,
/// stages}` triple as `propose_workflow`, but with no `workspace_id` and
/// no `install_id` required (binding fills those in later, per #402).
///
/// Trigger is delivered as a flat object (the FTUE preamble steers the
/// agent to a github-label shape today; non-label triggers are accepted
/// as opaque key/values and round-trip into the binding flow).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeWorkflowDraftArgs {
    pub name: String,
    pub trigger: DraftTriggerInput,
    pub stages: Vec<DraftStageInput>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DraftTriggerInput {
    /// Pre-bind: empty string. The binding step (#402) writes the
    /// GitHub-App install record id.
    #[serde(default)]
    pub install_id: String,
    /// Pre-bind: empty string. Binding writes the repo owner.
    #[serde(default)]
    pub repo_owner: String,
    /// Pre-bind: empty string. Binding writes the repo name.
    #[serde(default)]
    pub repo_name: String,
    /// GitHub label name (for label-style triggers) or a cron expression
    /// (for schedule-style triggers). Templates round-trip this field
    /// verbatim so cron strings survive draft → bind.
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DraftStageInput {
    /// Stable string id. The dashboard canonicalises ids when the
    /// document is committed; pre-commit the agent can pass any
    /// non-empty string (`"stage-0"`, etc.).
    pub id: String,
    pub name: String,
    pub gate_kind: GateKind,
    /// Workspace-defined artifact-kind discriminator. The FTUE default
    /// is `"Issue"`; non-FTUE flows may declare workspace-scoped kinds.
    pub artifact_kind: String,
    /// Free-form gate config (`{"prompt": "…"}` for `agent-session`,
    /// etc.). Validated by the binding step when the draft is promoted.
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Serialize)]
struct WorkflowDraftEnvelope<'a> {
    draft: ProposedDraft<'a>,
}

#[derive(Debug, Serialize)]
struct ProposedDraft<'a> {
    name: &'a str,
    trigger: &'a DraftTriggerInput,
    stages: &'a [DraftStageInput],
}

/// Echo a validated workflow draft. No DB writes — drafts live
/// client-side per spec #401 until the binding flow (#402) promotes them
/// into a real spine workflow via `propose_workflow`. The MCP boundary
/// gives the chat agent a structured way to *propose* the draft to the
/// dashboard, which routes the tool call through a `HitlCard` and into
/// `useWorkflowDraft.setWorkflow`.
pub async fn propose_workflow_draft(
    _state: &AppState,
    _auth_user: &AuthUser,
    args: Value,
) -> ToolResult {
    let args: ProposeWorkflowDraftArgs = serde_json::from_value(args).map_err(|e| {
        ToolError::InvalidParams(format!("invalid propose_workflow_draft args: {e}"))
    })?;

    if args.name.trim().is_empty() {
        return Err(ToolError::InvalidParams("name is required".into()));
    }
    if args.stages.is_empty() {
        return Err(ToolError::InvalidParams(
            "at least one stage is required".into(),
        ));
    }
    for (i, stage) in args.stages.iter().enumerate() {
        if stage.id.trim().is_empty() {
            return Err(ToolError::InvalidParams(format!(
                "stage[{i}].id is required"
            )));
        }
        if stage.name.trim().is_empty() {
            return Err(ToolError::InvalidParams(format!(
                "stage[{i}].name is required"
            )));
        }
        if stage.artifact_kind.trim().is_empty() {
            return Err(ToolError::InvalidParams(format!(
                "stage[{i}].artifact_kind is required"
            )));
        }
    }

    let envelope = WorkflowDraftEnvelope {
        draft: ProposedDraft {
            name: args.name.trim(),
            trigger: &args.trigger,
            stages: &args.stages,
        },
    };
    serde_json::to_value(envelope).map_err(internal_serialize)
}

// =============================================================================
// run_workflow
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunWorkflowArgs {
    pub workflow_id: String,
    /// Manual trigger name to fire. Must match the workflow's declared
    /// `Manual { name }` trigger.
    pub trigger_name: String,
    /// Optional JSON payload merged into the emitted `trigger.fired`
    /// event. Canonical fields (`workflow_id`, `workspace_id`, `name`,
    /// `actor`, `source`, `fired_at`, `trigger_kind`) override any
    /// colliding keys.
    #[serde(default)]
    pub payload: Option<Value>,
}

pub async fn run_workflow(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: RunWorkflowArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid run_workflow args: {e}")))?;

    let workflow = load_workflow(state, &args.workflow_id).await?;
    require_workspace_access(&state.pool, auth_user, &workflow.workspace_id).await?;

    match &workflow.trigger {
        TriggerKind::Manual { name } if name == &args.trigger_name => {}
        _ => {
            return Err(ToolError::InvalidParams(format!(
                "workflow {} does not declare manual trigger `{}` (its trigger kind is `{}`)",
                workflow.id,
                args.trigger_name,
                workflow.trigger.kind_tag()
            )));
        }
    }
    if !workflow.active {
        return Err(ToolError::InvalidParams(
            "workflow is inactive — activate before firing".into(),
        ));
    }

    let now = Utc::now();
    let mut payload = serde_json::json!({
        "trigger_kind": "manual",
        "workflow_id": workflow.id,
        "workspace_id": workflow.workspace_id,
        "name": args.trigger_name,
        "fired_at": now,
        "actor": auth_user.user_id,
        "source": "mcp",
    });
    if let Some(Value::Object(extra)) = args.payload
        && let Value::Object(target) = &mut payload
    {
        for (k, v) in extra {
            target.entry(k).or_insert(v);
        }
    }

    let metadata = onsager_spine::EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: auth_user.user_id.clone(),
    };
    let event_id = state
        .spine
        .append_ext(
            &workflow.workspace_id,
            &workflow.id,
            "workflow",
            "trigger.fired",
            payload,
            &metadata,
            None,
        )
        .await
        .map_err(|e| {
            tracing::error!("mcp run_workflow emit failed: {e}");
            ToolError::Internal(format!("failed to emit trigger.fired: {e}"))
        })?;

    Ok(serde_json::json!({
        "workflow_id": workflow.id,
        "trigger_event_id": event_id,
        "fired_at": now,
    }))
}

// =============================================================================
// edit_workflow
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditWorkflowArgs {
    pub workflow_id: String,
    /// Deactivate the workflow (`false`). Re-activation is **not**
    /// supported via MCP today — the activation pipeline runs GitHub
    /// side-effects (label create + webhook register) that need the
    /// request `HeaderMap`, which the MCP entry point doesn't plumb
    /// through. Pass `true` and the tool returns `InvalidParams`;
    /// use the REST PATCH endpoint to (re-)activate.
    #[serde(default)]
    pub active: Option<bool>,
}

pub async fn edit_workflow(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: EditWorkflowArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid edit_workflow args: {e}")))?;
    let workflow = load_workflow(state, &args.workflow_id).await?;
    require_workspace_access(&state.pool, auth_user, &workflow.workspace_id).await?;
    let spine = state.spine.pool();

    if let Some(desired) = args.active
        && desired != workflow.active
    {
        if desired {
            return Err(ToolError::InvalidParams(
                "MCP edit_workflow cannot activate inline — use the REST PATCH endpoint \
                 (the activation pipeline needs request headers the MCP entry point \
                 doesn't yet plumb through). Deactivation is supported."
                    .into(),
            ));
        }
        workflow_db::set_workflow_active(spine, &workflow.id, desired)
            .await
            .map_err(|e| ToolError::Internal(format!("failed to set workflow active: {e}")))?;
    }

    let updated = workflow_db::get_workflow(spine, &workflow.id)
        .await
        .map_err(|e| ToolError::Internal(format!("workflow reload failed: {e}")))?
        .ok_or_else(|| ToolError::Internal("workflow vanished after edit".into()))?;
    Ok(serde_json::json!({ "workflow": updated }))
}

// =============================================================================
// schedule_workflow
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleWorkflowArgs {
    pub workflow_id: String,
    /// New trigger kind. Typically `Cron`, `Interval`, or `Delay` — but
    /// any [`TriggerKind`] variant is accepted; the registry manifest
    /// gates which ones the persistence layer permits.
    pub trigger: TriggerKind,
}

pub async fn schedule_workflow(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: ScheduleWorkflowArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid schedule_workflow args: {e}")))?;

    let workflow = load_workflow(state, &args.workflow_id).await?;
    require_workspace_access(&state.pool, auth_user, &workflow.workspace_id).await?;

    let (kind_tag, config) = args.trigger.to_storage();

    // Same validations `workflow_db::insert_workflow_with_stages`
    // performs on create — apply them on every trigger swap so a
    // schedule update can't sneak a kind past the registry manifest
    // or land a self-amplifying `spine_event { event_kind: "trigger.fired" }`
    // workflow. Forge's trigger loader would otherwise reject (or
    // worst-case loop on) the row.
    if TRIGGERS.lookup(kind_tag).is_none() {
        return Err(ToolError::InvalidParams(format!(
            "trigger kind `{kind_tag}` is not in the registry manifest"
        )));
    }
    if let TriggerKind::SpineEvent { event_kind, .. } = &args.trigger
        && event_kind == "trigger.fired"
    {
        return Err(ToolError::InvalidParams(
            "spine_event workflow cannot listen for `trigger.fired` (would self-amplify)".into(),
        ));
    }

    let spine = state.spine.pool();
    sqlx::query(
        "UPDATE workflows SET trigger_kind = $1, trigger_config = $2 WHERE workflow_id = $3",
    )
    .bind(kind_tag)
    .bind(&config)
    .bind(&workflow.id)
    .execute(spine)
    .await
    .map_err(|e| {
        tracing::error!("mcp schedule_workflow update failed: {e}");
        ToolError::Internal(format!("failed to update workflow trigger: {e}"))
    })?;

    let updated = workflow_db::get_workflow(spine, &workflow.id)
        .await
        .map_err(|e| ToolError::Internal(format!("workflow reload failed: {e}")))?
        .ok_or_else(|| ToolError::Internal("workflow vanished after schedule".into()))?;
    Ok(serde_json::json!({ "workflow": updated }))
}

// =============================================================================
// list_workflows
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWorkflowsArgs {
    pub workspace_id: String,
}

pub async fn list_workflows(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: ListWorkflowsArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid list_workflows args: {e}")))?;
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;
    let workflows =
        workflow_db::list_workflows_for_workspace(state.spine.pool(), &args.workspace_id)
            .await
            .map_err(|e| {
                tracing::error!("mcp list_workflows failed: {e}");
                ToolError::Internal(format!("failed to list workflows: {e}"))
            })?;
    Ok(serde_json::json!({ "workflows": workflows }))
}

// =============================================================================
// helpers
// =============================================================================

pub(super) async fn load_workflow(
    state: &AppState,
    workflow_id: &str,
) -> Result<Workflow, ToolError> {
    workflow_db::get_workflow(state.spine.pool(), workflow_id)
        .await
        .map_err(|e| {
            tracing::error!("mcp workflow lookup failed: {e}");
            ToolError::Internal(format!("workflow lookup failed: {e}"))
        })?
        .ok_or_else(|| ToolError::NotFound(format!("workflow `{workflow_id}` not found")))
}

fn internal_serialize(e: serde_json::Error) -> ToolError {
    ToolError::Internal(format!("response serialization failed: {e}"))
}

#[cfg(test)]
mod propose_workflow_draft_tests {
    use super::*;
    use serde_json::json;

    fn draft_args() -> Value {
        json!({
            "name": "Triage every labeled issue",
            "trigger": {
                "install_id": "",
                "repo_owner": "",
                "repo_name": "",
                "label": "needs-triage"
            },
            "stages": [
                {
                    "id": "stage-0",
                    "name": "Triage agent",
                    "gate_kind": "agent-session",
                    "artifact_kind": "Issue",
                    "config": { "prompt": "Classify this issue." }
                }
            ]
        })
    }

    #[test]
    fn parses_canonical_draft_shape() {
        let parsed: ProposeWorkflowDraftArgs = serde_json::from_value(draft_args()).unwrap();
        assert_eq!(parsed.name, "Triage every labeled issue");
        assert_eq!(parsed.trigger.label, "needs-triage");
        assert!(parsed.trigger.install_id.is_empty());
        assert_eq!(parsed.stages.len(), 1);
        assert_eq!(parsed.stages[0].gate_kind, GateKind::AgentSession);
        assert_eq!(parsed.stages[0].artifact_kind, "Issue");
    }

    #[test]
    fn empty_trigger_fields_default_to_blank_strings() {
        let parsed: ProposeWorkflowDraftArgs = serde_json::from_value(json!({
            "name": "Schedule-only draft",
            "trigger": { "label": "0 9 * * 1" },
            "stages": [
                { "id": "s0", "name": "Generate", "gate_kind": "agent-session", "artifact_kind": "PR" }
            ]
        }))
        .unwrap();
        assert_eq!(parsed.trigger.install_id, "");
        assert_eq!(parsed.trigger.repo_owner, "");
        assert_eq!(parsed.trigger.repo_name, "");
        assert_eq!(parsed.trigger.label, "0 9 * * 1");
    }

    #[test]
    fn rejects_unknown_gate_kind() {
        let bad = json!({
            "name": "x",
            "trigger": { "label": "l" },
            "stages": [
                { "id": "s0", "name": "x", "gate_kind": "not-a-gate", "artifact_kind": "Issue" }
            ]
        });
        assert!(serde_json::from_value::<ProposeWorkflowDraftArgs>(bad).is_err());
    }
}
