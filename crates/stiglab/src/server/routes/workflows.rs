//! Workflow CRUD routes (issue #81).
//!
//! All routes are tenant-scoped and auth-gated. Non-members get a 404
//! (matching the `tenants.rs` pattern — private-resource surface). Repo
//! scope and label creation live in `server::workflow_activation`; this
//! module just handles the HTTP shape + input validation.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use sqlx::AnyPool;
use uuid::Uuid;

use crate::core::preset::{resolve_preset, PRESET_IDS};
use crate::core::workflow::{GateKind, TriggerKind, Workflow, WorkflowStage};
use crate::server::auth::{decrypt_credential, AuthUser};
use crate::server::db;
use crate::server::github_app::{mint_app_jwt, mint_installation_token, AppConfig};
use crate::server::state::AppState;
use crate::server::workflow_activation::{
    deregister_webhook, ensure_label_exists, ensure_repo_in_scope, ensure_webhook_registered,
    ActivationError,
};
use crate::server::workflow_db;
use crate::server::workflow_spine_mirror;

/// Push the current `tenant_workflows` row + its stages into the spine
/// `workflows` schema forge reads. Best-effort: a failure here means the
/// workflow won't fire until the next successful sync (via another CRUD or
/// startup backfill), but we don't abort the request — the stiglab-side
/// write already committed.
async fn mirror_to_spine(state: &AppState, workflow_id: &str) {
    let Some(spine) = state.spine.as_ref() else {
        return;
    };
    let workflow = match workflow_db::get_workflow(&state.db, workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(workflow_id, "spine mirror: load workflow failed: {e}");
            return;
        }
    };
    let stages = match workflow_db::list_stages_for_workflow(&state.db, workflow_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(workflow_id, "spine mirror: load stages failed: {e}");
            return;
        }
    };
    if let Err(e) = workflow_spine_mirror::upsert(spine.pool(), &workflow, &stages).await {
        tracing::warn!(workflow_id, "spine mirror: upsert failed: {e}");
    }
}

#[allow(clippy::result_large_err)]
async fn require_tenant_member(
    pool: &AnyPool,
    user_id: &str,
    tenant_id: &str,
) -> Result<(), Response> {
    match db::is_tenant_member(pool, tenant_id, user_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(not_found("tenant not found")),
        Err(e) => {
            tracing::error!("failed to check tenant membership: {e}");
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
    if auth_user.user_id == "anonymous" {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "authentication required" })),
        )
            .into_response())
    } else {
        Ok(auth_user.user_id.as_str())
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkflowBody {
    pub tenant_id: String,
    pub name: String,
    pub trigger_kind: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub trigger_label: String,
    pub install_id: i64,
    #[serde(default)]
    pub preset_id: Option<String>,
    /// Optional explicit stage chain. Required when `preset_id` is not set.
    #[serde(default)]
    pub stages: Option<Vec<CreateStageBody>>,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateStageBody {
    pub gate_kind: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// Validation helper kept public for unit tests.
pub fn validate_create_body(
    body: &CreateWorkflowBody,
) -> Result<(TriggerKind, Vec<WorkflowStage>), String> {
    if body.name.trim().is_empty() {
        return Err("name is required".into());
    }

    let trigger_kind = body
        .trigger_kind
        .parse::<TriggerKind>()
        .map_err(|e| e.to_string())?;

    match trigger_kind {
        TriggerKind::GithubIssueWebhook => {
            if body.trigger_label.trim().is_empty() {
                return Err("trigger_label is required for github-issue-webhook".into());
            }
            if body.repo_owner.trim().is_empty() || body.repo_name.trim().is_empty() {
                return Err("repo_owner and repo_name are required".into());
            }
            if body.install_id <= 0 {
                return Err("install_id is required".into());
            }
        }
    }

    // Reject requests that ship both a preset and explicit stages — the
    // two are mutually exclusive and silently preferring one is surprising.
    if body.preset_id.is_some() && body.stages.as_ref().is_some_and(|s| !s.is_empty()) {
        return Err("provide either preset_id or stages, not both".into());
    }

    let stages = if let Some(preset_id) = body.preset_id.as_deref() {
        if !PRESET_IDS.contains(&preset_id) {
            return Err(format!("unknown preset_id: {preset_id}"));
        }
        let expansion =
            resolve_preset(preset_id).ok_or_else(|| format!("unknown preset_id: {preset_id}"))?;
        expansion
            .stages
            .into_iter()
            .enumerate()
            .map(|(i, s)| WorkflowStage {
                id: Uuid::new_v4().to_string(),
                workflow_id: String::new(),
                seq: i as i32,
                gate_kind: s.gate_kind,
                params: s.params,
            })
            .collect()
    } else {
        let explicit = body
            .stages
            .as_ref()
            .ok_or_else(|| "stages or preset_id required".to_string())?;
        if explicit.is_empty() {
            return Err("at least one stage is required".into());
        }
        let mut out = Vec::with_capacity(explicit.len());
        for (i, s) in explicit.iter().enumerate() {
            let gate_kind = s.gate_kind.parse::<GateKind>().map_err(|e| e.to_string())?;
            out.push(WorkflowStage {
                id: Uuid::new_v4().to_string(),
                workflow_id: String::new(),
                seq: i as i32,
                gate_kind,
                params: s.params.clone().unwrap_or_else(|| serde_json::json!({})),
            });
        }
        out
    };

    Ok((trigger_kind, stages))
}

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
    if let Err(r) = require_tenant_member(&state.db, &user_id, &body.tenant_id).await {
        return r;
    }

    let (trigger_kind, mut stages) = match validate_create_body(&body) {
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
        tenant_id: body.tenant_id.clone(),
        name: body.name.trim().to_string(),
        trigger_kind,
        repo_owner: body.repo_owner.trim().to_string(),
        repo_name: body.repo_name.trim().to_string(),
        trigger_label: body.trigger_label.trim().to_string(),
        install_id: body.install_id,
        preset_id: body.preset_id.clone(),
        active: false,
        created_by: user_id,
        created_at: now,
        updated_at: now,
    };

    if let Err(e) = workflow_db::insert_workflow_with_stages(&state.db, &workflow, &stages).await {
        tracing::error!("failed to insert workflow: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "failed to create workflow" })),
        )
            .into_response();
    }
    mirror_to_spine(&state, &workflow.id).await;

    // Activation runs after the row is durable. If it fails the workflow
    // stays `active=false` and the caller sees the activation error.
    if body.active {
        if let Err(r) = activate_workflow(&state, &workflow.id, &headers).await {
            return r;
        }
    }

    let activated = body.active;
    let created = Workflow {
        active: activated,
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

/// GET /api/workflows?tenant_id=... — list workflows for a tenant.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub tenant_id: String,
}

pub async fn list_workflows(
    State(state): State<AppState>,
    auth_user: AuthUser,
    axum::extract::Query(q): axum::extract::Query<ListQuery>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    if let Err(r) = require_tenant_member(&state.db, &user_id, &q.tenant_id).await {
        return r;
    }
    match workflow_db::list_workflows_for_tenant(&state.db, &q.tenant_id).await {
        Ok(workflows) => Json(serde_json::json!({ "workflows": workflows })).into_response(),
        Err(e) => {
            tracing::error!("failed to list workflows: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// GET /api/workflows/:id — single-workflow detail with stages.
pub async fn get_workflow(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workflow_id): Path<String>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    let workflow = match workflow_db::get_workflow(&state.db, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_tenant_member(&state.db, &user_id, &workflow.tenant_id).await {
        return r;
    }
    let stages = match workflow_db::list_stages_for_workflow(&state.db, &workflow_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to load stages: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    Json(serde_json::json!({ "workflow": workflow, "stages": stages })).into_response()
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
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    let workflow = match workflow_db::get_workflow(&state.db, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_tenant_member(&state.db, &user_id, &workflow.tenant_id).await {
        return r;
    }

    let Some(desired_active) = body.active else {
        return bad_request("no patchable fields provided");
    };
    if desired_active == workflow.active {
        return Json(serde_json::json!({ "workflow": workflow })).into_response();
    }

    if desired_active {
        if let Err(r) = activate_workflow(&state, &workflow_id, &headers).await {
            return r;
        }
    } else if let Err(r) = deactivate_workflow(&state, &workflow_id).await {
        return r;
    }

    let updated = workflow_db::get_workflow(&state.db, &workflow_id)
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
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    let workflow = match workflow_db::get_workflow(&state.db, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_tenant_member(&state.db, &user_id, &workflow.tenant_id).await {
        return r;
    }

    if workflow.active {
        if let Err(r) = deactivate_workflow(&state, &workflow_id).await {
            return r;
        }
    }

    if let Err(e) = workflow_db::delete_workflow(&state.db, &workflow_id).await {
        tracing::error!("failed to delete workflow: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    if let Some(spine) = state.spine.as_ref() {
        if let Err(e) = workflow_spine_mirror::delete(spine.pool(), &workflow_id).await {
            tracing::warn!(workflow_id = %workflow_id, "spine mirror: delete failed: {e}");
        }
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
    let workflow = match workflow_db::get_workflow(&state.db, workflow_id).await {
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
    let token = match mint_installation_token(&app_jwt, workflow.install_id).await {
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

    if let Err(e) = ensure_repo_in_scope(&token, &workflow.repo_owner, &workflow.repo_name).await {
        return Err(map_activation_error(e));
    }
    if let Err(e) = ensure_label_exists(
        &token,
        &workflow.repo_owner,
        &workflow.repo_name,
        &workflow.trigger_label,
    )
    .await
    {
        return Err(map_activation_error(e));
    }

    let secret = resolve_install_webhook_secret(state, workflow.install_id).await;
    if let Err(e) = ensure_webhook_registered(
        &token,
        &workflow.repo_owner,
        &workflow.repo_name,
        secret.as_deref(),
        headers,
    )
    .await
    {
        return Err(map_activation_error(e));
    }

    if let Err(e) = workflow_db::set_workflow_active(&state.db, workflow_id, true).await {
        tracing::error!("mark workflow active: {e}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    mirror_to_spine(state, workflow_id).await;
    Ok(())
}

#[allow(clippy::result_large_err)]
async fn deactivate_workflow(state: &AppState, workflow_id: &str) -> Result<(), Response> {
    let workflow = match workflow_db::get_workflow(&state.db, workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return Err(not_found("workflow not found")),
        Err(e) => {
            tracing::error!("load workflow for deactivation: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };

    if let Err(e) = workflow_db::set_workflow_active(&state.db, workflow_id, false).await {
        tracing::error!("mark workflow inactive: {e}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    mirror_to_spine(state, workflow_id).await;

    // Dedup: keep the webhook if any other active workflow on the same repo
    // still needs it.
    let still_needed = workflow_db::any_other_active_workflow_on_repo(
        &state.db,
        &workflow.repo_owner,
        &workflow.repo_name,
        workflow_id,
    )
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
    let Ok(token) = mint_installation_token(&app_jwt, workflow.install_id).await else {
        return Ok(());
    };
    let _ = deregister_webhook(&token, &workflow.repo_owner, &workflow.repo_name).await;
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

async fn resolve_install_webhook_secret(state: &AppState, install_id: i64) -> Option<String> {
    let cipher = workflow_db::get_install_webhook_secret_cipher(&state.db, install_id)
        .await
        .ok()
        .flatten()
        .flatten()?;
    let key = state.config.credential_key.as_ref()?;
    decrypt_credential(key, &cipher).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_body() -> CreateWorkflowBody {
        CreateWorkflowBody {
            tenant_id: "t1".into(),
            name: "sdd".into(),
            trigger_kind: "github-issue-webhook".into(),
            repo_owner: "acme".into(),
            repo_name: "widgets".into(),
            trigger_label: "spec".into(),
            install_id: 42,
            preset_id: Some("github-issue-to-pr".into()),
            stages: None,
            active: false,
        }
    }

    #[test]
    fn validate_preset_expands_to_stage_chain() {
        let (trigger, stages) = validate_create_body(&base_body()).unwrap();
        assert_eq!(trigger, TriggerKind::GithubIssueWebhook);
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].gate_kind, GateKind::AgentSession);
        assert_eq!(stages[0].seq, 0);
    }

    #[test]
    fn validate_rejects_bad_trigger_kind() {
        let mut body = base_body();
        body.trigger_kind = "polling".into();
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.to_lowercase().contains("invalid trigger kind"));
    }

    #[test]
    fn validate_rejects_missing_label() {
        let mut body = base_body();
        body.trigger_label = "".into();
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.contains("trigger_label"));
    }

    #[test]
    fn validate_rejects_unknown_preset() {
        let mut body = base_body();
        body.preset_id = Some("nope".into());
        body.stages = None;
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.contains("unknown preset_id"));
    }

    #[test]
    fn validate_accepts_explicit_stages_when_no_preset() {
        let mut body = base_body();
        body.preset_id = None;
        body.stages = Some(vec![
            CreateStageBody {
                gate_kind: "agent-session".into(),
                params: Some(serde_json::json!({"profile": "implementer"})),
            },
            CreateStageBody {
                gate_kind: "external-check".into(),
                params: None,
            },
            CreateStageBody {
                gate_kind: "manual-approval".into(),
                params: None,
            },
        ]);
        let (_t, stages) = validate_create_body(&body).unwrap();
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].gate_kind, GateKind::AgentSession);
        assert_eq!(stages[1].gate_kind, GateKind::ExternalCheck);
        assert_eq!(stages[2].gate_kind, GateKind::ManualApproval);
    }

    #[test]
    fn validate_rejects_empty_stage_chain_without_preset() {
        let mut body = base_body();
        body.preset_id = None;
        body.stages = Some(vec![]);
        assert!(validate_create_body(&body).is_err());
    }

    #[test]
    fn validate_rejects_missing_install_id() {
        let mut body = base_body();
        body.install_id = 0;
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.contains("install_id"));
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut body = base_body();
        body.name = "  ".into();
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.contains("name"));
    }

    #[test]
    fn validate_rejects_both_preset_and_stages() {
        let mut body = base_body();
        body.preset_id = Some("github-issue-to-pr".into());
        body.stages = Some(vec![CreateStageBody {
            gate_kind: "agent-session".into(),
            params: None,
        }]);
        let err = validate_create_body(&body).unwrap_err();
        assert!(err.contains("preset_id or stages"));
    }
}
