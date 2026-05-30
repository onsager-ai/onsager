//! Workflow CRUD routes (issue #81).
//!
//! Spec #222 Slice 4 moved this module from stiglab to portal — portal
//! owns the HTTP surface for `/api/workflows*` and the GitHub
//! side-effects of activation (label create / webhook register).
//!
//! All routes are workspace-scoped and auth-gated. Non-members get a 404
//! (matching the `workspaces.rs` pattern — private-resource surface).
//! Repo scope and label creation live in `crate::workflow_activation`;
//! this module just handles the HTTP shape + input validation.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use onsager_github::api::app::{AppConfig, mint_app_jwt, mint_installation_token};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::credential_db;
use crate::state::AppState;
use crate::workflow::Workflow;
use crate::workflow_activation::{
    ActivationError, deregister_webhook, ensure_label_exists, ensure_repo_in_scope,
    ensure_webhook_registered,
};
use crate::workflow_create::{CreateWorkflowBody, validate_create_body};
use crate::workflow_db;
use crate::workflow_install;

/// Spine pool used for workflow CRUD. Portal's `EventStore` is
/// non-optional and already validated at boot, so this just hands back
/// the underlying `PgPool`. The `Result` shape is kept so call sites
/// stay aligned with the stiglab-era helper signature; rewiring every
/// caller for an infallible return is a follow-up cleanup, not Slice 4
/// scope.
#[allow(clippy::result_large_err)]
fn spine_pool(state: &AppState) -> Result<&sqlx::PgPool, Response> {
    Ok(state.spine.pool())
}

/// Authorize an `AuthUser` against a target workspace.
///
/// Two checks, in order:
///
/// 1. **PAT scope (403 on mismatch).** If the principal is a PAT pinned
///    to a workspace, the request must target that exact workspace.
///    Surfaces `pat_workspace_scope_mismatch` so client tooling can
///    react.
/// 2. **Membership (404 on miss).** Otherwise the caller must be a
///    member of the workspace; non-members get a flat 404 to avoid
///    leaking workspace existence.
#[allow(clippy::result_large_err)]
async fn require_workspace_access(
    pool: &sqlx::PgPool,
    auth_user: &AuthUser,
    workspace_id: &str,
) -> Result<(), Response> {
    if let Some(pinned) = auth_user.principal.pinned_workspace_id()
        && pinned != workspace_id
    {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "pat_workspace_scope_mismatch",
                "detail": "PAT is pinned to a different workspace",
            })),
        )
            .into_response());
    }
    match credential_db::is_workspace_member(pool, workspace_id, &auth_user.user_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "workspace not found" })),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("failed to check workspace membership: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

fn bad_request(msg: impl Into<String>) -> Response {
    let msg = msg.into();
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

#[allow(clippy::result_large_err)]
fn require_auth_user(auth_user: &AuthUser) -> Result<&str, Response> {
    // Auth is always-on as of #193; the `AuthUser` extractor 401s
    // unauthenticated requests before they reach this helper.
    Ok(auth_user.user_id.as_str())
}

/// Issue #156 (workspace-scoped post-#164): ensure the workflow's owner
/// has at least one Claude auth credential **in the workflow's
/// workspace** before activating. Returns a 4xx response when missing so
/// the dashboard can surface a "connect Claude credentials" CTA.
///
/// Checks by exact credential name (not "any credential row") because
/// the Claude CLI only honors specific env vars; a user with only
/// custom-named secrets would silently activate into a workflow whose
/// every session fails with "stdout closed without result event".
#[allow(clippy::result_large_err)]
async fn require_owner_credentials(
    state: &AppState,
    workspace_id: &str,
    owner_user_id: &str,
) -> Result<(), Response> {
    match credential_db::user_has_credential_in(
        &state.pool,
        workspace_id,
        owner_user_id,
        CLAUDE_AUTH_CREDENTIAL_NAMES,
    )
    .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "no_credentials_for_workflow",
                "message": format!(
                    "Workflow owner has no Claude credentials in this workspace. Connect one of {:?} in Settings before activating.",
                    CLAUDE_AUTH_CREDENTIAL_NAMES
                ),
                "required_one_of": CLAUDE_AUTH_CREDENTIAL_NAMES,
                "owner_user_id": owner_user_id,
                "workspace_id": workspace_id,
            })),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("credential lookup failed during activation gate: {e}");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "credential check failed" })),
            )
                .into_response())
        }
    }
}

/// Credential names the Claude CLI actually consumes. Either is
/// sufficient: the OAuth token is what the dashboard provisions via
/// the cloud login flow; `ANTHROPIC_API_KEY` is the API-key fallback
/// for direct-API users. New names get added here when the runner
/// learns to honor them.
const CLAUDE_AUTH_CREDENTIAL_NAMES: &[&str] = &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"];

/// POST /api/workflows — create a workflow. If `active=true`, the
/// activation hook runs inline and rejects out-of-scope repos with 400.
///
/// The `HeaderMap` is plumbed into activation so the webhook base URL
/// can be derived from `X-Forwarded-Proto` / `X-Forwarded-Host` when
/// `STIGLAB_TRUST_FORWARDED_HEADERS=1`. Axum extractors ordering:
/// `Json<_>` consumes the body and must come last.
pub async fn create_workflow(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<CreateWorkflowBody>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &body.workspace_id).await {
        return r;
    }
    let spine = match spine_pool(&state) {
        Ok(p) => p,
        Err(r) => return r,
    };

    let (trigger, mut stages) = match validate_create_body(&body) {
        Ok(v) => v,
        Err(e) => return bad_request(e),
    };

    let workflow_id = format!("wf_{}", Uuid::new_v4());
    for s in &mut stages {
        s.workflow_id = workflow_id.clone();
    }
    let now = Utc::now();
    let workflow = Workflow {
        id: workflow_id.clone(),
        workspace_id: body.workspace_id.clone(),
        name: body.name.trim().to_string(),
        trigger,
        // Token-mint cache only (ADR 0024). A non-positive value means
        // "no cache — resolve the install live at activation"; the create
        // gate in `validate_create_body` still requires a real id for
        // `github_issue_webhook` today (gate removal is ADR item 4).
        // Token-mint cache only (ADR 0024); `None`/non-positive means
        // "resolve the install live at activation through the project layer".
        install_id: body.install_id.filter(|id| *id > 0),
        preset_id: body.preset_id.clone(),
        active: false,
        // Mirrors what `insert_workflow_with_stages` persists (ADR 0024):
        // a freshly created workflow carries a trigger binding → `ready`;
        // `autofire` tracks `active`, which is `false` until activation
        // runs below.
        readiness: "ready".to_string(),
        autofire: "off".to_string(),
        created_by: user_id,
        created_at: now,
        updated_at: now,
    };

    if let Err(e) = workflow_db::insert_workflow_with_stages(spine, &workflow, &stages).await {
        tracing::error!("failed to insert workflow: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "failed to create workflow" })),
        )
            .into_response();
    }

    // Activation runs after the row is durable. If it fails the workflow
    // stays `active=false` and the caller sees the activation error.
    if body.active {
        // Issue #156: same activation gate as patch_workflow — refuse to
        // activate if the owner has no credentials in the workflow's
        // workspace (post-#164). Here `created_by` is the caller themselves
        // (just set above), so this in practice surfaces the case where a
        // user creates+activates without ever having connected Claude
        // credentials in this workspace.
        if let Err(r) =
            require_owner_credentials(&state, &workflow.workspace_id, &workflow.created_by).await
        {
            return r;
        }
        if let Err(r) = activate_workflow(&state, &workflow.id, &headers).await {
            return r;
        }
    }

    let activated = body.active;
    let created = Workflow {
        active: activated,
        // `set_workflow_active` flips `autofire` in lockstep with
        // `active` (ADR 0024); reflect that in the returned struct so the
        // create response matches a subsequent read.
        autofire: if activated { "active" } else { "off" }.to_string(),
        ..workflow
    };
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "workflow": created,
            "stages": stages,
        })),
    )
        .into_response()
}

/// GET /api/workflows?workspace_id=...&readiness=...&autofire=... — list
/// workflows for a workspace. The two orthogonal axes (ADR 0024) filter
/// independently: `readiness` (`drafted|ready`) and `autofire`
/// (`off|active|paused`). Both are optional and combine with AND. The
/// response always includes per-axis-value counts for the filter strip
/// (spec #509).
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub workspace_id: String,
    #[serde(default)]
    pub readiness: Option<String>,
    #[serde(default)]
    pub autofire: Option<String>,
}

pub async fn list_workflows(
    State(state): State<AppState>,
    auth_user: AuthUser,
    axum::extract::Query(q): axum::extract::Query<ListQuery>,
) -> Response {
    if let Err(r) = require_auth_user(&auth_user) {
        return r;
    }
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &q.workspace_id).await {
        return r;
    }
    let spine = match spine_pool(&state) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let readiness = match q.readiness.as_deref() {
        None | Some("") | Some("all") => None,
        Some(s) => match workflow_db::Readiness::from_wire(s) {
            Some(v) => Some(v),
            None => return bad_request(format!("unknown readiness filter: {s}")),
        },
    };
    let autofire = match q.autofire.as_deref() {
        None | Some("") | Some("all") => None,
        Some(s) => match workflow_db::Autofire::from_wire(s) {
            Some(v) => Some(v),
            None => return bad_request(format!("unknown autofire filter: {s}")),
        },
    };
    let filter = workflow_db::WorkflowFilter {
        readiness,
        autofire,
    };
    let workflows =
        match workflow_db::list_workflows_for_workspace_filtered(spine, &q.workspace_id, filter)
            .await
        {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("failed to list workflows: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
        };
    let counts = match workflow_db::workflow_axis_counts(spine, &q.workspace_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("failed to compute workflow axis counts: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    Json(serde_json::json!({
        "workflows": workflows,
        "counts": {
            "all": counts.total,
            "readiness": {
                "drafted": counts.drafted,
                "ready": counts.ready,
            },
            "autofire": {
                "off": counts.off,
                "active": counts.active,
                "paused": counts.paused,
            },
        },
    }))
    .into_response()
}

/// GET /api/workflows/:id — single-workflow detail with stages.
pub async fn get_workflow(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workflow_id): Path<String>,
) -> Response {
    if let Err(r) = require_auth_user(&auth_user) {
        return r;
    }
    let spine = match spine_pool(&state) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let workflow = match workflow_db::get_workflow(spine, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workflow.workspace_id).await
    {
        return r;
    }
    let stages = match workflow_db::list_stages_for_workflow(spine, &workflow_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to load stages: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    Json(serde_json::json!({ "workflow": workflow, "stages": stages })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/workflows/:id/runs — list every artifact flowing through a
/// workflow, projected as a "run" (1 artifact == 1 run) with per-stage
/// status derived from `artifacts.current_stage_index` and
/// `workflow_parked_reason`.
///
/// Stage IDs come from spine `workflow_stages` keyed by `(workflow_id,
/// stage_order)`; the per-run stage list aligns 1:1 with that ordering so
/// the dashboard can zip stages by index.
pub async fn list_workflow_runs(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workflow_id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<RunsQuery>,
) -> Response {
    if let Err(r) = require_auth_user(&auth_user) {
        return r;
    }
    // Workflows live on the spine post-Lever D. Portal's `EventStore`
    // is non-optional, so the lookup is unconditional — DB-level
    // failures bubble up as 500 below.
    let spine = state.spine.pool();
    let workflow = match workflow_db::get_workflow(spine, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workflow.workspace_id).await
    {
        return r;
    }
    let stages = match workflow_db::list_stages_for_workflow(spine, &workflow_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to load stages: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let rows = match sqlx::query_as::<_, ArtifactRunRow>(
        "SELECT artifact_id, workflow_id, state, current_stage_index, \
                workflow_parked_reason, created_at, updated_at \
         FROM artifacts \
         WHERE workflow_id = $1 \
         ORDER BY updated_at DESC \
         LIMIT $2",
    )
    .bind(&workflow_id)
    .bind(limit)
    .fetch_all(spine)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("failed to load workflow runs: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load runs" })),
            )
                .into_response();
        }
    };

    let runs: Vec<crate::handlers::runs::WorkflowRun> = rows
        .into_iter()
        .map(|row| project_run(row, &stages))
        .collect();
    Json(serde_json::json!({ "runs": runs })).into_response()
}

#[derive(sqlx::FromRow)]
struct ArtifactRunRow {
    artifact_id: String,
    workflow_id: String,
    state: String,
    current_stage_index: Option<i32>,
    workflow_parked_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// Project a single artifact row + its workflow's stage chain into the
/// dashboard `WorkflowRun` shape.
///
/// Status mapping:
/// - `archived` → run failed; stages before parked index passed,
///   stage at parked index failed, rest pending.
/// - `released` → all stages passed, run passed.
/// - `workflow_parked_reason` set → run blocked; current stage blocked.
/// - otherwise → stages [0..idx] passed, stage[idx] pending, rest pending.
fn project_run(
    row: ArtifactRunRow,
    stages: &[crate::workflow::WorkflowStage],
) -> crate::handlers::runs::WorkflowRun {
    let (status, stage_entries) = crate::handlers::runs::project_run_stages(
        &row.state,
        row.current_stage_index,
        row.workflow_parked_reason.as_deref(),
        row.updated_at,
        stages,
    );
    let artifact_id = row.artifact_id;
    crate::handlers::runs::WorkflowRun {
        id: artifact_id.clone(),
        workflow_id: row.workflow_id,
        artifact_id,
        status,
        stages: stage_entries,
        started_at: row.created_at,
        updated_at: row.updated_at,
    }
}

#[derive(Debug, Deserialize)]
pub struct PatchWorkflowBody {
    pub active: Option<bool>,
}

/// PATCH /api/workflows/:id — toggle `active`. Runs the activation /
/// deactivation hooks as side effects.
///
/// Headers are plumbed into activation for request-derived webhook URL
/// resolution; see `create_workflow`.
pub async fn patch_workflow(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workflow_id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<PatchWorkflowBody>,
) -> Response {
    if let Err(r) = require_auth_user(&auth_user) {
        return r;
    }
    let spine = match spine_pool(&state) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let workflow = match workflow_db::get_workflow(spine, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workflow.workspace_id).await
    {
        return r;
    }

    let Some(desired_active) = body.active else {
        return bad_request("no patchable fields provided");
    };
    if desired_active == workflow.active {
        return Json(serde_json::json!({ "workflow": workflow })).into_response();
    }

    if desired_active {
        // Issue #156 (workspace-scoped post-#164): refuse activation when
        // the workflow's owner has no credentials in the workflow's
        // workspace. Without this guard the workflow would be active
        // but every shaping dispatch would spawn an agent without
        // CLAUDE_CODE_OAUTH_TOKEN and immediately fail with "stdout
        // closed without result event". Failing fast at activation
        // surfaces the broken state in the UX layer (a 4xx the dashboard
        // can render as "connect Claude credentials") instead of in the
        // spine event stream where users won't see it.
        if let Err(r) =
            require_owner_credentials(&state, &workflow.workspace_id, &workflow.created_by).await
        {
            return r;
        }
        if let Err(r) = activate_workflow(&state, &workflow_id, &headers).await {
            return r;
        }
    } else {
        if let Err(r) = deactivate_workflow(&state, &workflow_id).await {
            return r;
        }
    }

    let updated = workflow_db::get_workflow(spine, &workflow_id)
        .await
        .ok()
        .flatten()
        .unwrap_or(workflow);
    Json(serde_json::json!({ "workflow": updated })).into_response()
}

/// DELETE /api/workflows/:id — remove the workflow row + stages. If the
/// workflow is currently active the deactivation hook runs first, which
/// flips `active=false` and drops the repo webhook when no sibling
/// workflow still needs it. After that the row and stage chain go away
/// in a single transaction so a partial delete can't leak.
pub async fn delete_workflow(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workflow_id): Path<String>,
) -> Response {
    if let Err(r) = require_auth_user(&auth_user) {
        return r;
    }
    let spine = match spine_pool(&state) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let workflow = match workflow_db::get_workflow(spine, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workflow.workspace_id).await
    {
        return r;
    }

    if workflow.active
        && let Err(r) = deactivate_workflow(&state, &workflow_id).await
    {
        return r;
    }

    if let Err(e) = workflow_db::delete_workflow(spine, &workflow_id).await {
        tracing::error!("failed to delete workflow: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// Run the activation pipeline against the live GitHub App install. Emits
/// typed failure modes through `Response` so the caller can bubble the HTTP
/// status directly.
///
/// `headers` comes from the originating HTTP request and feeds the
/// webhook-URL resolver (`resolve_webhook_base`) when trust is enabled.
#[allow(clippy::result_large_err)]
async fn activate_workflow(
    state: &AppState,
    workflow_id: &str,
    headers: &axum::http::HeaderMap,
) -> Result<(), Response> {
    let spine = spine_pool(state)?;
    let workflow = match workflow_db::get_workflow(spine, workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return Err(not_found("workflow not found")),
        Err(e) => {
            tracing::error!("load workflow for activation: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };

    let Some(app_cfg) = AppConfig::from_env() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "GitHub App not configured (GITHUB_APP_ID/SLUG/PRIVATE_KEY)"
            })),
        )
            .into_response());
    };
    let app_jwt = match mint_app_jwt(&app_cfg) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("mint app jwt: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };
    // ADR 0024 item 3: resolve the install through the project layer, with
    // the workflow's cached `install_id` as fallback. A GitHub workflow
    // with no resolvable install cannot be activated.
    let Some(install_id) = workflow_install::resolve_workflow_install_id(state, &workflow).await
    else {
        return Err(bad_request(
            "no GitHub App installation for this workflow's repo — \
             register the repo as a project first"
                .to_string(),
        ));
    };
    let token = match mint_installation_token(&app_jwt, install_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("mint installation token: {e}");
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "failed to mint installation token" })),
            )
                .into_response());
        }
    };

    let Some((repo_owner, repo_name)) = workflow.github_repo() else {
        return Err(bad_request(
            "workflow trigger has no GitHub repo to activate".to_string(),
        ));
    };
    let label = workflow
        .github_label()
        .ok_or_else(|| bad_request("workflow trigger has no GitHub label".to_string()))?;
    if let Err(e) = ensure_repo_in_scope(&token, repo_owner, repo_name).await {
        return Err(map_activation_error(e));
    }
    if let Err(e) = ensure_label_exists(&token, repo_owner, repo_name, label).await {
        return Err(map_activation_error(e));
    }

    let secret = workflow_install::resolve_install_webhook_secret(state, install_id).await;
    if let Err(e) =
        ensure_webhook_registered(&token, repo_owner, repo_name, secret.as_deref(), headers).await
    {
        return Err(map_activation_error(e));
    }

    if let Err(e) = workflow_db::set_workflow_active(spine, workflow_id, true).await {
        tracing::error!("mark workflow active: {e}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
async fn deactivate_workflow(state: &AppState, workflow_id: &str) -> Result<(), Response> {
    let spine = spine_pool(state)?;
    let workflow = match workflow_db::get_workflow(spine, workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return Err(not_found("workflow not found")),
        Err(e) => {
            tracing::error!("load workflow for deactivation: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };

    if let Err(e) = workflow_db::set_workflow_active(spine, workflow_id, false).await {
        tracing::error!("mark workflow inactive: {e}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    // Dedup: keep the webhook if any other active workflow on the same repo
    // still needs it.
    let Some((repo_owner, repo_name)) = workflow.github_repo() else {
        // Non-GitHub trigger: nothing to deregister.
        return Ok(());
    };
    let still_needed =
        workflow_db::any_other_active_workflow_on_repo(spine, repo_owner, repo_name, workflow_id)
            .await
            .unwrap_or(true);
    if still_needed {
        return Ok(());
    }

    let Some(app_cfg) = AppConfig::from_env() else {
        // No GitHub App configured — treat as dev env and skip the outbound
        // call. The workflow is already inactive in the DB.
        return Ok(());
    };
    let Ok(app_jwt) = mint_app_jwt(&app_cfg) else {
        return Ok(());
    };
    let Some(install_id) = workflow_install::resolve_workflow_install_id(state, &workflow).await
    else {
        // No resolvable install — nothing to deregister against.
        return Ok(());
    };
    let Ok(token) = mint_installation_token(&app_jwt, install_id).await else {
        return Ok(());
    };
    let _ = deregister_webhook(&token, repo_owner, repo_name).await;
    Ok(())
}

fn map_activation_error(e: ActivationError) -> Response {
    match e {
        ActivationError::RepoOutOfScope { owner, repo } => bad_request(format!(
            "repo {owner}/{repo} is outside the workspace install scope"
        )),
        ActivationError::MissingGithubPermission {
            action,
            permission,
            details,
            upstream,
        } => {
            tracing::warn!(
                action = %action,
                permission = %permission,
                upstream = %upstream,
                "activation missing github permission"
            );
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": details,
                    "code": "github_permission_missing",
                    "permission": permission,
                })),
            )
                .into_response()
        }
        ActivationError::GitHub(msg) => {
            tracing::warn!("activation github error: {msg}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": format!("github api error: {msg}"),
                    "code": "github_api_error",
                })),
            )
                .into_response()
        }
        ActivationError::WebhookUrlNotReachable { url } => {
            tracing::warn!(url = %url, "webhook URL is not publicly reachable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": format!(
                        "Webhook URL {url} is not reachable from GitHub. The stiglab webhook base URL is resolved in order: STIGLAB_WEBHOOK_BASE_URL (or STIGLAB_PUBLIC_BASE_URL), then RAILWAY_PUBLIC_DOMAIN, then X-Forwarded-* headers when STIGLAB_TRUST_FORWARDED_HEADERS=1. Configure one of these to point at a publicly reachable HTTP(S) origin (tunnel like ngrok, Railway public domain, or production host) and retry."
                    ),
                    "code": "webhook_url_not_reachable",
                    "url": url,
                })),
            )
                .into_response()
        }
        ActivationError::WebhookUrlInvalid { url } => {
            tracing::warn!(url = %url, "webhook URL is not a valid absolute URL");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": format!(
                        "Webhook URL {url} is not a valid absolute URL. Check STIGLAB_WEBHOOK_BASE_URL / STIGLAB_PUBLIC_BASE_URL / RAILWAY_PUBLIC_DOMAIN — the first non-empty value must be an absolute HTTP(S) URL like https://stig.example.com (or a bare hostname for RAILWAY_PUBLIC_DOMAIN)."
                    ),
                    "code": "webhook_url_invalid",
                    "url": url,
                })),
            )
                .into_response()
        }
        ActivationError::WebhookUrlUnknown => {
            tracing::warn!("webhook base URL unresolved from env and request");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "Webhook base URL is not configured. Set STIGLAB_WEBHOOK_BASE_URL (or STIGLAB_PUBLIC_BASE_URL) to a public HTTP(S) URL, or set RAILWAY_PUBLIC_DOMAIN, or enable STIGLAB_TRUST_FORWARDED_HEADERS=1 when running behind a trusted proxy that sets X-Forwarded-Proto and X-Forwarded-Host.",
                    "code": "webhook_url_unknown",
                })),
            )
                .into_response()
        }
        ActivationError::Other(e) => {
            tracing::error!("activation unexpected error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{GateKind, WorkflowStage};

    // Create-body validation tests live in `crate::workflow_create`
    // (where the body + validator moved per spec #509). The tests below
    // cover `project_run` — the status table for /api/workflows/:id/runs.

    fn run_row(
        state: &str,
        current_stage_index: Option<i32>,
        parked: Option<&str>,
    ) -> ArtifactRunRow {
        let now = chrono::Utc::now();
        ArtifactRunRow {
            artifact_id: "art_test".into(),
            workflow_id: "wf_test".into(),
            state: state.into(),
            current_stage_index,
            workflow_parked_reason: parked.map(str::to_string),
            created_at: now,
            updated_at: now,
        }
    }

    fn three_stages() -> Vec<WorkflowStage> {
        let now = Utc::now();
        let _ = now; // silence unused — only seq/id matter for projection
        (0..3)
            .map(|i| WorkflowStage {
                id: format!("stage_{i}"),
                workflow_id: "wf_test".into(),
                seq: i,
                gate_kind: GateKind::AgentSession,
                params: serde_json::json!({}),
            })
            .collect()
    }

    fn status_str(v: &serde_json::Value) -> &str {
        v.as_str().expect("status str")
    }

    fn project_run_json(row: ArtifactRunRow, stages: &[WorkflowStage]) -> serde_json::Value {
        serde_json::to_value(project_run(row, stages)).expect("serializable")
    }

    fn stage_status(value: &serde_json::Value, idx: usize) -> &str {
        status_str(&value["stages"][idx]["status"])
    }

    #[test]
    fn project_run_in_progress_partway_through() {
        let row = run_row("in_progress", Some(1), None);
        let stages = three_stages();
        let v = project_run_json(row, &stages);
        assert_eq!(v["status"], "pending");
        assert_eq!(stage_status(&v, 0), "passed");
        assert_eq!(stage_status(&v, 1), "pending");
        assert_eq!(stage_status(&v, 2), "pending");
    }

    #[test]
    fn project_run_released_marks_all_stages_passed() {
        let row = run_row("released", Some(2), None);
        let stages = three_stages();
        let v = project_run_json(row, &stages);
        assert_eq!(v["status"], "passed");
        for i in 0..3 {
            assert_eq!(stage_status(&v, i), "passed", "stage {i}");
        }
    }

    #[test]
    fn project_run_archived_marks_current_stage_failed() {
        let row = run_row("archived", Some(1), None);
        let stages = three_stages();
        let v = project_run_json(row, &stages);
        assert_eq!(v["status"], "failed");
        assert_eq!(stage_status(&v, 0), "passed");
        assert_eq!(stage_status(&v, 1), "failed");
        assert_eq!(stage_status(&v, 2), "pending");
    }

    #[test]
    fn project_run_parked_marks_current_stage_blocked() {
        let row = run_row("under_review", Some(1), Some("ci red"));
        let stages = three_stages();
        let v = project_run_json(row, &stages);
        assert_eq!(v["status"], "blocked");
        assert_eq!(stage_status(&v, 0), "passed");
        assert_eq!(stage_status(&v, 1), "blocked");
        assert_eq!(stage_status(&v, 2), "pending");
    }

    #[test]
    fn project_run_no_current_stage_index_keeps_all_pending() {
        // current_stage_index None happens for trigger-just-fired
        // artifacts before the first tick has set the column.
        let row = run_row("draft", None, None);
        let stages = three_stages();
        let v = project_run_json(row, &stages);
        assert_eq!(v["status"], "pending");
        for i in 0..3 {
            assert_eq!(stage_status(&v, i), "pending", "stage {i}");
        }
    }

    #[test]
    fn project_run_negative_current_stage_index_clamps_to_pending() {
        // i32 -> usize fails for negatives; the projection must treat
        // the index as absent rather than panicking.
        let row = run_row("draft", Some(-1), None);
        let stages = three_stages();
        let v = project_run_json(row, &stages);
        for i in 0..3 {
            assert_eq!(stage_status(&v, i), "pending", "stage {i}");
        }
    }

    #[test]
    fn project_run_current_stage_past_end_keeps_known_passed_intact() {
        // current_stage_index past the last stage (e.g. workflow was
        // edited to remove a stage). Stages with seq < idx still
        // render as passed; nothing renders as the parked/blocked
        // current stage because there is no current stage in range.
        let row = run_row("in_progress", Some(5), None);
        let stages = three_stages();
        let v = project_run_json(row, &stages);
        for i in 0..3 {
            assert_eq!(stage_status(&v, i), "passed", "stage {i}");
        }
    }

    #[test]
    fn project_run_emits_artifact_id_as_run_id() {
        // The dashboard treats artifact_id and run id interchangeably;
        // this test pins that contract.
        let row = run_row("draft", None, None);
        let stages = three_stages();
        let v = project_run_json(row, &stages);
        assert_eq!(v["id"], "art_test");
        assert_eq!(v["artifact_id"], "art_test");
        assert_eq!(v["workflow_id"], "wf_test");
    }
}
