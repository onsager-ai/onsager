//! MCP tools for substrate Spec Plans + the compiler validation
//! surface (spec #395).
//!
//! Six tools live here:
//!
//! - `submit_spec_plan` (constructive) — first-write a `SpecPlan` row
//!   under `(workspace_id, spec_plan_id)`.
//! - `update_spec` (diff) — replace a single `SpecRef` inside an
//!   existing plan.
//! - `list_spec_plans` (read-only) — every plan in a workspace.
//! - `get_spec_plan` (read-only) — one plan by id.
//! - `compile_dry_run` (read-only) — run
//!   [`onsager_substrate::compile`] over a candidate `SpecPlan` against
//!   the workspace's current Workflow Library snapshot. Returns the
//!   resulting `ExecutionPlan` or the full `CompileError` (including
//!   the batched `Invariant(Vec)` payload per ADR 0018). No
//!   persistence.
//! - `get_execution_plan` (read-only) — same compile pass, but loads
//!   a persisted `SpecPlan` first. Recompile-on-read per the spec's
//!   v1 default (caching is a later spec).
//!
//! The heavyweight payloads — `SpecPlan`, `SpecRef`, and the resulting
//! `ExecutionPlan` — flow as raw `serde_json::Value` in the argument
//! structs. The schemars-derived JSON Schema represents them as
//! `true` (any); validation happens inline by deserializing into the
//! concrete substrate types. This matches the existing
//! `propose_workflow_draft` pattern of accepting opaque inner shapes
//! while keeping the outer envelope strongly typed.

use onsager_substrate::{CompileError, SpecPlan, SpecRef, compile};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::AuthUser;
use crate::spec_plan_db;
use crate::state::AppState;
use crate::substrate_library_db;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

// =============================================================================
// helpers
// =============================================================================

fn internal_serialize(e: serde_json::Error) -> ToolError {
    ToolError::Internal(format!("response serialization failed: {e}"))
}

fn parse_spec_plan(raw: Value) -> Result<SpecPlan, ToolError> {
    serde_json::from_value(raw)
        .map_err(|e| ToolError::InvalidParams(format!("invalid spec_plan payload: {e}")))
}

fn parse_spec_ref(raw: Value) -> Result<SpecRef, ToolError> {
    serde_json::from_value(raw)
        .map_err(|e| ToolError::InvalidParams(format!("invalid spec payload: {e}")))
}

/// Render a `CompileError` as the structured payload the MCP response
/// returns to the client. Mirrors the variant shape so authors get
/// the same names they see in substrate's Rust error type.
fn compile_error_to_value(err: &CompileError) -> Value {
    match err {
        CompileError::SpecPlan(e) => serde_json::json!({
            "kind": "spec_plan",
            "message": e.to_string(),
        }),
        CompileError::MissingKind { spec_id, kind } => serde_json::json!({
            "kind": "missing_kind",
            "message": err.to_string(),
            "spec_id": spec_id.as_str(),
            "spec_kind": kind,
        }),
        CompileError::NoExit {
            from,
            to,
            from_kind,
        } => serde_json::json!({
            "kind": "no_exit",
            "message": err.to_string(),
            "from": from.as_str(),
            "to": to.as_str(),
            "from_kind": from_kind,
        }),
        CompileError::NoEntry { from, to, to_kind } => serde_json::json!({
            "kind": "no_entry",
            "message": err.to_string(),
            "from": from.as_str(),
            "to": to.as_str(),
            "to_kind": to_kind,
        }),
        CompileError::MultipleIncomingDeps { to, from } => serde_json::json!({
            "kind": "multiple_incoming_deps",
            "message": err.to_string(),
            "to": to.as_str(),
            "from": from.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        }),
        CompileError::Invariant(violations) => serde_json::json!({
            "kind": "invariant",
            "message": err.to_string(),
            "violations": violations
                .iter()
                .map(|v| serde_json::json!({ "message": v.to_string() }))
                .collect::<Vec<_>>(),
        }),
    }
}

// =============================================================================
// submit_spec_plan
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubmitSpecPlanArgs {
    pub workspace_id: String,
    /// External plan identity. Reuse a GitHub issue number, an
    /// `mcp:<uuid>`, or any stable stringable.
    pub spec_plan_id: String,
    /// `SpecPlan` payload — the full `{ specs: [...], deps: [...] }`
    /// JSON the substrate consumes. Validated by deserializing into
    /// `onsager_substrate::SpecPlan` and running structural checks
    /// (`SpecPlan::validate`) before insert.
    pub spec_plan: Value,
}

pub async fn submit_spec_plan(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: SubmitSpecPlanArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid submit_spec_plan args: {e}")))?;

    if args.spec_plan_id.trim().is_empty() {
        return Err(ToolError::InvalidParams("spec_plan_id is required".into()));
    }

    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let plan = parse_spec_plan(args.spec_plan)?;
    plan.validate().map_err(|e| {
        ToolError::InvalidParams(format!("spec plan failed structural validation: {e}"))
    })?;

    let stored = spec_plan_db::insert(
        &state.pool,
        &args.workspace_id,
        args.spec_plan_id.trim(),
        &plan,
        &auth_user.user_id,
    )
    .await
    .map_err(|e| match e {
        spec_plan_db::SpecPlanStoreError::Duplicate(id) => {
            ToolError::InvalidParams(format!("spec plan `{id}` already exists"))
        }
        other => {
            tracing::error!("mcp submit_spec_plan insert failed: {other}");
            ToolError::Internal(format!("failed to insert spec plan: {other}"))
        }
    })?;

    serde_json::to_value(SubmitSpecPlanResponse { spec_plan: stored }).map_err(internal_serialize)
}

#[derive(Debug, Serialize)]
struct SubmitSpecPlanResponse {
    spec_plan: spec_plan_db::StoredSpecPlan,
}

// =============================================================================
// update_spec
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateSpecArgs {
    pub workspace_id: String,
    pub spec_plan_id: String,
    /// New `SpecRef` payload. Identity is matched on `id` — the
    /// existing `SpecRef` with the same `id` is replaced. Renaming
    /// is not supported; submit a fresh plan instead.
    pub spec: Value,
}

pub async fn update_spec(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: UpdateSpecArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid update_spec args: {e}")))?;

    if args.spec_plan_id.trim().is_empty() {
        return Err(ToolError::InvalidParams("spec_plan_id is required".into()));
    }
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;
    let new_spec = parse_spec_ref(args.spec)?;

    // `replace_spec` validates the post-swap plan *before* the DB
    // write, so a malformed update (dupe id from a sibling spec
    // rename, dangling dep) surfaces as `InvalidParams` without
    // corrupting the stored row.
    let stored = spec_plan_db::replace_spec(
        &state.pool,
        &args.workspace_id,
        args.spec_plan_id.trim(),
        new_spec,
        true,
    )
    .await
    .map_err(|e| match e {
        spec_plan_db::SpecPlanStoreError::PlanNotFound(id) => {
            ToolError::NotFound(format!("spec plan `{id}` not found"))
        }
        spec_plan_db::SpecPlanStoreError::SpecNotFound(id) => {
            ToolError::InvalidParams(format!("spec `{id}` not found in plan"))
        }
        spec_plan_db::SpecPlanStoreError::Validation(e) => {
            ToolError::InvalidParams(format!("updated spec plan failed validation: {e}"))
        }
        other => {
            tracing::error!("mcp update_spec failed: {other}");
            ToolError::Internal(format!("failed to update spec: {other}"))
        }
    })?;

    serde_json::to_value(SubmitSpecPlanResponse { spec_plan: stored }).map_err(internal_serialize)
}

// =============================================================================
// list_spec_plans
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSpecPlansArgs {
    pub workspace_id: String,
}

pub async fn list_spec_plans(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: ListSpecPlansArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid list_spec_plans args: {e}")))?;
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let plans = spec_plan_db::list(&state.pool, &args.workspace_id)
        .await
        .map_err(|e| {
            tracing::error!("mcp list_spec_plans failed: {e}");
            ToolError::Internal(format!("failed to list spec plans: {e}"))
        })?;
    Ok(serde_json::json!({ "spec_plans": plans }))
}

// =============================================================================
// get_spec_plan
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSpecPlanArgs {
    pub workspace_id: String,
    pub spec_plan_id: String,
}

pub async fn get_spec_plan(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: GetSpecPlanArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid get_spec_plan args: {e}")))?;
    if args.spec_plan_id.trim().is_empty() {
        return Err(ToolError::InvalidParams("spec_plan_id is required".into()));
    }
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let spec_plan_id = args.spec_plan_id.trim();
    let plan = spec_plan_db::get(&state.pool, &args.workspace_id, spec_plan_id)
        .await
        .map_err(|e| {
            tracing::error!("mcp get_spec_plan failed: {e}");
            ToolError::Internal(format!("failed to get spec plan: {e}"))
        })?
        .ok_or_else(|| ToolError::NotFound(format!("spec plan `{spec_plan_id}` not found")))?;
    serde_json::to_value(SubmitSpecPlanResponse { spec_plan: plan }).map_err(internal_serialize)
}

// =============================================================================
// compile_dry_run
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompileDryRunArgs {
    pub workspace_id: String,
    /// `SpecPlan` payload to compile. Same JSON shape as
    /// `submit_spec_plan.spec_plan`. The plan is **not** persisted —
    /// this is the linter for authors.
    pub spec_plan: Value,
}

pub async fn compile_dry_run(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: CompileDryRunArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid compile_dry_run args: {e}")))?;
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let plan = parse_spec_plan(args.spec_plan)?;
    let snapshot = substrate_library_db::snapshot_active(state.spine.pool())
        .await
        .map_err(|e| {
            tracing::error!("mcp compile_dry_run snapshot failed: {e}");
            ToolError::Internal(format!("failed to snapshot workflow library: {e}"))
        })?;

    serde_json::to_value(compile_result_value(&plan, &snapshot)).map_err(internal_serialize)
}

// =============================================================================
// get_execution_plan
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetExecutionPlanArgs {
    pub workspace_id: String,
    pub spec_plan_id: String,
}

pub async fn get_execution_plan(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: GetExecutionPlanArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid get_execution_plan args: {e}")))?;
    if args.spec_plan_id.trim().is_empty() {
        return Err(ToolError::InvalidParams("spec_plan_id is required".into()));
    }
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let spec_plan_id = args.spec_plan_id.trim();
    let stored = spec_plan_db::get(&state.pool, &args.workspace_id, spec_plan_id)
        .await
        .map_err(|e| {
            tracing::error!("mcp get_execution_plan get failed: {e}");
            ToolError::Internal(format!("failed to load spec plan: {e}"))
        })?
        .ok_or_else(|| ToolError::NotFound(format!("spec plan `{spec_plan_id}` not found")))?;

    let snapshot = substrate_library_db::snapshot_active(state.spine.pool())
        .await
        .map_err(|e| {
            tracing::error!("mcp get_execution_plan snapshot failed: {e}");
            ToolError::Internal(format!("failed to snapshot workflow library: {e}"))
        })?;

    serde_json::to_value(compile_result_value(&stored.plan, &snapshot)).map_err(internal_serialize)
}

// =============================================================================
// shared compile-response formatting
// =============================================================================

fn compile_result_value(
    plan: &SpecPlan,
    snapshot: &substrate_library_db::LibrarySnapshot,
) -> Value {
    match compile(plan, snapshot) {
        Ok(execution_plan) => {
            let spec_index: serde_json::Map<String, Value> = execution_plan
                .spec_index
                .iter()
                .map(|(id, slot)| {
                    (
                        id.as_str().to_string(),
                        serde_json::json!({
                            "entry_edges": slot.entry_edges,
                            "exit_edges": slot
                                .exit_edges
                                .iter()
                                .map(|o| serde_json::json!({
                                    "edge_id": o.edge_id,
                                    "provenance": o.provenance,
                                }))
                                .collect::<Vec<_>>(),
                        }),
                    )
                })
                .collect();
            serde_json::json!({
                "ok": true,
                "execution_plan": {
                    "node_count": execution_plan.nodes.len(),
                    "edge_count": execution_plan.edges.len(),
                    "spec_index": spec_index,
                    "library_versions": snapshot
                        .versions
                        .iter()
                        .map(|(k, (id, v))| (k.clone(), serde_json::json!({ "id": id, "version": v })))
                        .collect::<serde_json::Map<_, _>>(),
                },
            })
        }
        Err(err) => serde_json::json!({
            "ok": false,
            "error": compile_error_to_value(&err),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn submit_spec_plan_args_round_trip() {
        let raw = json!({
            "workspace_id": "ws1",
            "spec_plan_id": "github:42",
            "spec_plan": {
                "specs": [
                    { "id": "github:42", "kind": "feature" }
                ],
                "deps": []
            }
        });
        let parsed: SubmitSpecPlanArgs = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.spec_plan_id, "github:42");
        let plan: SpecPlan = serde_json::from_value(parsed.spec_plan).unwrap();
        assert_eq!(plan.specs.len(), 1);
    }

    #[test]
    fn update_spec_args_round_trip() {
        let raw = json!({
            "workspace_id": "ws1",
            "spec_plan_id": "github:42",
            "spec": { "id": "github:42", "kind": "feature" }
        });
        let parsed: UpdateSpecArgs = serde_json::from_value(raw).unwrap();
        let spec: SpecRef = serde_json::from_value(parsed.spec).unwrap();
        assert_eq!(spec.kind, "feature");
    }

    #[test]
    fn compile_dry_run_args_round_trip() {
        let raw = json!({
            "workspace_id": "ws1",
            "spec_plan": { "specs": [], "deps": [] }
        });
        let parsed: CompileDryRunArgs = serde_json::from_value(raw).unwrap();
        let plan: SpecPlan = serde_json::from_value(parsed.spec_plan).unwrap();
        assert!(plan.specs.is_empty());
    }

    #[test]
    fn compile_error_to_value_classifies_missing_kind() {
        let plan = SpecPlan {
            specs: vec![onsager_substrate::SpecRef {
                id: "s1".into(),
                kind: "no-such-kind".to_string(),
                inputs: Default::default(),
            }],
            deps: vec![],
        };
        let snapshot = substrate_library_db::LibrarySnapshot::default();
        let result = compile_result_value(&plan, &snapshot);
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["kind"], "missing_kind");
        assert_eq!(result["error"]["spec_id"], "s1");
        assert_eq!(result["error"]["spec_kind"], "no-such-kind");
    }

    #[test]
    fn compile_error_to_value_classifies_spec_plan_failure() {
        let plan = SpecPlan {
            specs: vec![
                onsager_substrate::SpecRef {
                    id: "a".into(),
                    kind: "k".into(),
                    inputs: Default::default(),
                },
                onsager_substrate::SpecRef {
                    id: "a".into(),
                    kind: "k".into(),
                    inputs: Default::default(),
                },
            ],
            deps: vec![],
        };
        let snapshot = substrate_library_db::LibrarySnapshot::default();
        let result = compile_result_value(&plan, &snapshot);
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["kind"], "spec_plan");
    }

    #[test]
    fn empty_plan_compiles_ok() {
        let plan = SpecPlan::default();
        let snapshot = substrate_library_db::LibrarySnapshot::default();
        let result = compile_result_value(&plan, &snapshot);
        assert_eq!(result["ok"], true);
        assert_eq!(result["execution_plan"]["node_count"], 0);
        assert_eq!(result["execution_plan"]["edge_count"], 0);
    }

    #[test]
    fn rejects_empty_spec_plan_id() {
        // Sanity: the trim().is_empty() guard in submit_spec_plan
        // applies whether the caller passes "" or "   ".
        let bad = json!({
            "workspace_id": "ws1",
            "spec_plan_id": "   ",
            "spec_plan": { "specs": [], "deps": [] }
        });
        let parsed: SubmitSpecPlanArgs = serde_json::from_value(bad).unwrap();
        assert!(parsed.spec_plan_id.trim().is_empty());
    }
}
