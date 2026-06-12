//! MCP tools for the substrate `WorkflowLibrary` (spec #395).
//!
//! Five tools live here:
//!
//! - `submit_workflow` (constructive) — wrap
//!   [`onsager_substrate::WorkflowLibrary::register`]. Inserts a new
//!   monotonic version for the given `spec_kind`.
//! - `update_workflow` (diff) — append a new monotonic version per
//!   ADR 0016. Semantically the same call as `submit_workflow`; the
//!   separate tool exists so the dashboard HitlCard can render a
//!   diff view against the current active version.
//! - `retire_workflow` (destructive) — mark the currently-active
//!   version for `spec_kind` inactive via the `retired_at` column
//!   (migration 029).
//! - `list_workflows_v2` (read-only) — one card per `spec_kind` in
//!   the library, current version + retire status.
//! - `get_workflow_v2` (read-only) — full workflow body for a
//!   specific `(spec_kind, version)` pair. `version = None` →
//!   latest active.

use onsager_substrate::{Workflow, WorkflowLibrary};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::AuthUser;
use crate::state::AppState;
use crate::substrate_library_db;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

fn internal_serialize(e: serde_json::Error) -> ToolError {
    ToolError::Internal(format!("response serialization failed: {e}"))
}

fn parse_workflow(raw: Value) -> Result<Workflow, ToolError> {
    serde_json::from_value(raw)
        .map_err(|e| ToolError::InvalidParams(format!("invalid workflow payload: {e}")))
}

// =============================================================================
// submit_workflow
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubmitWorkflowArgs {
    /// Workspace scope for the authoring caller. The substrate
    /// `workflow_library` table is workspace-agnostic (ADR 0016 — one
    /// flat catalog), but every MCP tool runs through
    /// `require_workspace_access` to keep the auth surface uniform
    /// with the rest of the portal MCP server. Workspace-scoped
    /// libraries are a future spec.
    pub workspace_id: String,
    /// Spec kind this workflow registers under. Looked up by the
    /// compiler when a `SpecRef::kind` references it.
    pub spec_kind: String,
    /// `Workflow` payload — the full
    /// `{ nodes, edges, entry_specs, output_specs }` JSON. Round-trips
    /// via [`Workflow`]'s serde derive (executors flow through
    /// `typetag`'s `kind` discriminator).
    pub workflow: Value,
}

#[derive(Debug, Serialize)]
struct WorkflowRegisterResponse {
    spec_kind: String,
    version: i32,
    workflow: Workflow,
}

pub async fn submit_workflow(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: SubmitWorkflowArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid submit_workflow args: {e}")))?;

    if args.spec_kind.trim().is_empty() {
        return Err(ToolError::InvalidParams("spec_kind is required".into()));
    }
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let workflow = parse_workflow(args.workflow)?;
    let library = WorkflowLibrary::new(state.spine.pool().clone());
    let version = library
        .register(args.spec_kind.trim(), &workflow)
        .await
        .map_err(|e| {
            tracing::error!("mcp submit_workflow register failed: {e}");
            ToolError::Internal(format!("failed to register workflow: {e}"))
        })?;

    serde_json::to_value(WorkflowRegisterResponse {
        spec_kind: args.spec_kind.trim().to_string(),
        version,
        workflow,
    })
    .map_err(internal_serialize)
}

// =============================================================================
// update_workflow
// =============================================================================

/// `update_workflow` shares its argument shape with `submit_workflow`.
/// The two tools differ in HITL category — the dashboard renders one
/// as a constructive card and the other as a diff card against the
/// current active version — and in the description authors see.
pub type UpdateWorkflowArgs = SubmitWorkflowArgs;

pub async fn update_workflow(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    // Identical persistence call to `submit_workflow`: per ADR 0016
    // an "update" is a fresh monotonic version that becomes the new
    // active row, not a mutation of an existing row.
    submit_workflow(state, auth_user, args).await
}

// =============================================================================
// retire_workflow
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RetireWorkflowArgs {
    pub workspace_id: String,
    pub spec_kind: String,
}

pub async fn retire_workflow(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: RetireWorkflowArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid retire_workflow args: {e}")))?;
    if args.spec_kind.trim().is_empty() {
        return Err(ToolError::InvalidParams("spec_kind is required".into()));
    }
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let spec_kind = args.spec_kind.trim();
    let row = substrate_library_db::retire_latest(state.spine.pool(), spec_kind)
        .await
        .map_err(|e| match e {
            substrate_library_db::LibraryDbError::NotFound => ToolError::NotFound(format!(
                "no active workflow registered for spec kind `{spec_kind}`"
            )),
            substrate_library_db::LibraryDbError::AlreadyRetired => ToolError::InvalidParams(
                format!("workflow for spec kind `{spec_kind}` is already retired"),
            ),
            other => {
                tracing::error!("mcp retire_workflow failed: {other}");
                ToolError::Internal(format!("failed to retire workflow: {other}"))
            }
        })?;

    // Same envelope key as `get_workflow_v2` so MCP clients can
    // route the response through one renderer; `retired_at` on the
    // row carries the destructive signal.
    Ok(serde_json::json!({ "workflow": row }))
}

// =============================================================================
// list_workflows_v2
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWorkflowsV2Args {
    pub workspace_id: String,
}

pub async fn list_workflows_v2(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: ListWorkflowsV2Args = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid list_workflows_v2 args: {e}")))?;
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let cards = substrate_library_db::list_cards(state.spine.pool())
        .await
        .map_err(|e| {
            tracing::error!("mcp list_workflows_v2 failed: {e}");
            ToolError::Internal(format!("failed to list workflows: {e}"))
        })?;
    Ok(serde_json::json!({ "workflows": cards }))
}

// =============================================================================
// get_workflow_v2
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetWorkflowV2Args {
    pub workspace_id: String,
    pub spec_kind: String,
    /// Specific version to read. Omitted → latest **active** version.
    /// Reads of a retired-but-explicitly-named version return the
    /// row unchanged so authors can inspect what they shipped before
    /// the retire.
    #[serde(default)]
    pub version: Option<i32>,
}

pub async fn get_workflow_v2(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: GetWorkflowV2Args = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid get_workflow_v2 args: {e}")))?;
    if args.spec_kind.trim().is_empty() {
        return Err(ToolError::InvalidParams("spec_kind is required".into()));
    }
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let spec_kind = args.spec_kind.trim();
    let row = substrate_library_db::get_by_kind(state.spine.pool(), spec_kind, args.version)
        .await
        .map_err(|e| {
            tracing::error!("mcp get_workflow_v2 failed: {e}");
            ToolError::Internal(format!("failed to read workflow: {e}"))
        })?
        .ok_or_else(|| {
            let qualifier = match args.version {
                Some(v) => format!("`{spec_kind}` at version {v}"),
                None => format!("`{spec_kind}`"),
            };
            ToolError::NotFound(format!("workflow {qualifier} not found"))
        })?;
    Ok(serde_json::json!({ "workflow": row }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn submit_workflow_args_round_trip() {
        let raw = json!({
            "workspace_id": "ws1",
            "spec_kind": "feature",
            "workflow": { "nodes": [], "edges": [] }
        });
        let parsed: SubmitWorkflowArgs = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.spec_kind, "feature");
        let w: Workflow = serde_json::from_value(parsed.workflow).unwrap();
        assert!(w.nodes.is_empty());
    }

    #[test]
    fn get_workflow_v2_version_defaults_to_none() {
        let parsed: GetWorkflowV2Args = serde_json::from_value(json!({
            "workspace_id": "ws1",
            "spec_kind": "feature"
        }))
        .unwrap();
        assert!(parsed.version.is_none());
    }

    #[test]
    fn get_workflow_v2_accepts_explicit_version() {
        let parsed: GetWorkflowV2Args = serde_json::from_value(json!({
            "workspace_id": "ws1",
            "spec_kind": "feature",
            "version": 3
        }))
        .unwrap();
        assert_eq!(parsed.version, Some(3));
    }

    #[test]
    fn retire_workflow_args_round_trip() {
        let parsed: RetireWorkflowArgs = serde_json::from_value(json!({
            "workspace_id": "ws1",
            "spec_kind": "feature"
        }))
        .unwrap();
        assert_eq!(parsed.spec_kind, "feature");
    }

    #[test]
    fn rejects_empty_spec_kind_in_submit_workflow() {
        let parsed: SubmitWorkflowArgs = serde_json::from_value(json!({
            "workspace_id": "ws1",
            "spec_kind": "   ",
            "workflow": { "nodes": [], "edges": [] }
        }))
        .unwrap();
        assert!(parsed.spec_kind.trim().is_empty());
    }
}
