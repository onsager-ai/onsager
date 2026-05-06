//! Project-scoped live-data hydration routes (specs #167, #170, #171).
//!
//! Moved from stiglab → portal as part of spec #222 follow-up 2.
//!
//! Per spec #170, reference-only artifact rows store identity + our derived
//! lifecycle but *not* GitHub-authored fields. The dashboard joins skeleton
//! rows from `/api/spine/artifacts` with the hydrated rows here on
//! `external_ref`. Each endpoint:
//!
//! 1. Authenticates the user (`AuthUser` extractor — auth is always-on).
//! 2. Resolves the project and asserts workspace membership.
//! 3. Mints an installation token via the GitHub App config.
//! 4. Fetches live data from GitHub through a per-process LRU+TTL cache
//!    (`AppState::proxy_cache`). Cache hits skip the GitHub round-trip.
//! 5. Returns a slim, dashboard-shaped JSON payload.
//!
//! Failure modes per #170 fail-open:
//! - Cache miss + GitHub 403 → return `error: "rate_limited"` so the
//!   dashboard renders the skeleton placeholder rather than crash.
//! - GitHub 5xx → 502 Bad Gateway.
//!
//! The backfill endpoint (`POST /api/projects/:id/backfill`) delegates to
//! `crate::backfill::run` which uses the same advisory-lock upsert SQL as
//! the live webhook handler so backfilled and live rows have identical shapes.

use std::sync::OnceLock;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use onsager_spine::webhook_routing::{
    build_trigger_fired_events, spine_namespace, trigger_source, IssueTriggerContext, RoutedEvent,
    WorkflowMatch,
};

use crate::auth::AuthUser;
use crate::backfill::Strategy;
use crate::core::Project;
use crate::handlers::projects::installation_token_for;
use crate::handlers::workspaces::require_workspace_access;
use crate::state::AppState;
use crate::workspace_db;

const HARD_CAP: usize = 200;
const DEFAULT_CAP: usize = 100;

// ── Auth / project helpers ────────────────────────────────────────────────

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

/// Resolve project + assert workspace membership. 404 for both unknown
/// projects and non-members (avoids leaking project existence to outsiders).
async fn require_project_for_user(
    state: &AppState,
    auth_user: &AuthUser,
    project_id: &str,
) -> Result<Project, Response> {
    let project = match workspace_db::get_project(&state.pool, project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Err(not_found("project not found")),
        Err(e) => {
            tracing::error!("failed to load project: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, auth_user, &project.workspace_id).await {
        return Err(rewrite_workspace_404_to_project(r));
    }
    Ok(project)
}

fn rewrite_workspace_404_to_project(resp: Response) -> Response {
    if resp.status() == StatusCode::NOT_FOUND {
        return not_found("project not found");
    }
    resp
}

// ── GitHub client ─────────────────────────────────────────────────────────

fn gh_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("onsager-portal/0.1")
            .timeout(Duration::from_secs(15))
            .build()
            .expect("reqwest build")
    })
}

// ── State/strategy validation ─────────────────────────────────────────────

#[allow(clippy::result_large_err)]
fn normalize_state(raw: Option<&str>) -> Result<&'static str, Response> {
    match raw.unwrap_or("open") {
        "open" => Ok("open"),
        "closed" => Ok("closed"),
        "all" => Ok("all"),
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("invalid state filter: {other}"),
            })),
        )
            .into_response()),
    }
}

// ── GitHub live-data types ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LiveIssue {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    user: Option<LiveUser>,
    labels: Vec<LiveLabel>,
    pull_request: Option<serde_json::Value>,
    comments: u32,
    updated_at: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assignees: Vec<LiveUser>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    closed_at: Option<String>,
    #[serde(default)]
    milestone: Option<LiveMilestone>,
}

#[derive(Debug, Deserialize)]
struct LiveUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct LiveLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct LiveMilestone {
    title: String,
    state: String,
}

#[derive(Debug, Deserialize)]
struct LivePull {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    user: Option<LiveUser>,
    labels: Vec<LiveLabel>,
    draft: bool,
    merged_at: Option<String>,
    updated_at: String,
}

// ── Dashboard-shaped response types ──────────────────────────────────────

#[derive(Debug, Serialize)]
struct LiveIssueRow {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    author: Option<String>,
    labels: Vec<String>,
    comments: u32,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct LiveIssueDetailRow {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    author: Option<String>,
    labels: Vec<String>,
    assignees: Vec<String>,
    comments: u32,
    body: Option<String>,
    milestone: Option<LiveMilestoneRow>,
    created_at: Option<String>,
    updated_at: String,
    closed_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct LiveMilestoneRow {
    title: String,
    state: String,
}

#[derive(Debug, Serialize)]
struct LivePullRow {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    author: Option<String>,
    labels: Vec<String>,
    draft: bool,
    merged: bool,
    updated_at: String,
}

// ── GET /api/projects/:id/issues ──────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct ListLiveQuery {
    pub state: Option<String>,
}

/// GET `/api/projects/:id/issues?state=open|closed|all`.
pub async fn list_project_issues(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(project_id): Path<String>,
    Query(filters): Query<ListLiveQuery>,
) -> Response {
    let project = match require_project_for_user(&state, &auth_user, &project_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };

    let state_q = match normalize_state(filters.state.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let cache_key = format!("issues:{project_id}:{state_q}");
    if let Some(cached) = state.proxy_cache.get(&cache_key) {
        return Json(cached).into_response();
    }

    let token = match installation_token_for(&state.pool, &project.github_app_installation_id).await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "GitHub App not configured" })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("installation_token_for failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "GitHub auth failed" })),
            )
                .into_response();
        }
    };

    let url = format!(
        "https://api.github.com/repos/{owner}/{repo}/issues?state={state_q}&per_page=100",
        owner = project.repo_owner,
        repo = project.repo_name,
    );
    let resp = match gh_client()
        .get(&url)
        .bearer_auth(&token.token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("github issues fetch failed: {e}");
            return Json(serde_json::json!({ "issues": [], "error": "github_unreachable" }))
                .into_response();
        }
    };

    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Json(serde_json::json!({ "issues": [], "error": "rate_limited" })).into_response();
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let snippet = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, "github issues fetch non-2xx: {snippet}");
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "github API error" })),
        )
            .into_response();
    }

    let parsed: Vec<LiveIssue> = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("github issues parse failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "github response parse failed" })),
            )
                .into_response();
        }
    };

    let rows: Vec<LiveIssueRow> = parsed
        .into_iter()
        .filter(|i| i.pull_request.is_none())
        .map(|i| LiveIssueRow {
            number: i.number,
            title: i.title,
            state: i.state,
            html_url: i.html_url,
            author: i.user.map(|u| u.login),
            labels: i.labels.into_iter().map(|l| l.name).collect(),
            comments: i.comments,
            updated_at: i.updated_at,
        })
        .collect();

    let body = serde_json::json!({ "issues": rows });
    state.proxy_cache.put(cache_key, body.clone());
    Json(body).into_response()
}

// ── GET /api/projects/:id/issues/:number ─────────────────────────────────

/// GET `/api/projects/:id/issues/:number` — single hydrated issue.
pub async fn get_project_issue(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((project_id, number)): Path<(String, u64)>,
) -> Response {
    let project = match require_project_for_user(&state, &auth_user, &project_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };

    let cache_key = format!("issue:{project_id}:{number}");
    if let Some(cached) = state.proxy_cache.get(&cache_key) {
        return Json(cached).into_response();
    }

    let token = match installation_token_for(&state.pool, &project.github_app_installation_id).await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "GitHub App not configured" })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("installation_token_for failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "GitHub auth failed" })),
            )
                .into_response();
        }
    };

    let url = format!(
        "https://api.github.com/repos/{owner}/{repo}/issues/{number}",
        owner = project.repo_owner,
        repo = project.repo_name,
    );
    let resp = match gh_client()
        .get(&url)
        .bearer_auth(&token.token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("github single-issue fetch failed: {e}");
            return Json(serde_json::json!({
                "issue": serde_json::Value::Null,
                "error": "github_unreachable",
            }))
            .into_response();
        }
    };

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return not_found("issue not found");
    }
    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Json(serde_json::json!({
            "issue": serde_json::Value::Null,
            "error": "rate_limited",
        }))
        .into_response();
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let snippet = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, "github single-issue fetch non-2xx: {snippet}");
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "github API error" })),
        )
            .into_response();
    }

    let parsed: LiveIssue = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("github single-issue parse failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "github response parse failed" })),
            )
                .into_response();
        }
    };

    if parsed.pull_request.is_some() {
        return not_found("issue not found");
    }

    let row = LiveIssueDetailRow {
        number: parsed.number,
        title: parsed.title,
        state: parsed.state,
        html_url: parsed.html_url,
        author: parsed.user.map(|u| u.login),
        labels: parsed.labels.into_iter().map(|l| l.name).collect(),
        assignees: parsed.assignees.into_iter().map(|u| u.login).collect(),
        comments: parsed.comments,
        body: parsed.body,
        milestone: parsed.milestone.map(|m| LiveMilestoneRow {
            title: m.title,
            state: m.state,
        }),
        created_at: parsed.created_at,
        updated_at: parsed.updated_at,
        closed_at: parsed.closed_at,
    };

    let body = serde_json::json!({ "issue": row });
    state.proxy_cache.put(cache_key, body.clone());
    Json(body).into_response()
}

// ── GET /api/projects/:id/pulls ───────────────────────────────────────────

/// GET `/api/projects/:id/pulls?state=open|closed|all`.
pub async fn list_project_pulls(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(project_id): Path<String>,
    Query(filters): Query<ListLiveQuery>,
) -> Response {
    let project = match require_project_for_user(&state, &auth_user, &project_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };

    let state_q = match normalize_state(filters.state.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let cache_key = format!("pulls:{project_id}:{state_q}");
    if let Some(cached) = state.proxy_cache.get(&cache_key) {
        return Json(cached).into_response();
    }

    let token = match installation_token_for(&state.pool, &project.github_app_installation_id).await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "GitHub App not configured" })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("installation_token_for failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "GitHub auth failed" })),
            )
                .into_response();
        }
    };

    let url = format!(
        "https://api.github.com/repos/{owner}/{repo}/pulls?state={state_q}&per_page=100",
        owner = project.repo_owner,
        repo = project.repo_name,
    );
    let resp = match gh_client()
        .get(&url)
        .bearer_auth(&token.token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("github pulls fetch failed: {e}");
            return Json(serde_json::json!({ "pulls": [], "error": "github_unreachable" }))
                .into_response();
        }
    };

    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Json(serde_json::json!({ "pulls": [], "error": "rate_limited" })).into_response();
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let snippet = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, "github pulls fetch non-2xx: {snippet}");
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "github API error" })),
        )
            .into_response();
    }

    let parsed: Vec<LivePull> = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("github pulls parse failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "github response parse failed" })),
            )
                .into_response();
        }
    };

    let rows: Vec<LivePullRow> = parsed
        .into_iter()
        .map(|p| LivePullRow {
            number: p.number,
            title: p.title,
            state: p.state,
            html_url: p.html_url,
            author: p.user.map(|u| u.login),
            labels: p.labels.into_iter().map(|l| l.name).collect(),
            draft: p.draft,
            merged: p.merged_at.is_some(),
            updated_at: p.updated_at,
        })
        .collect();

    let body = serde_json::json!({ "pulls": rows });
    state.proxy_cache.put(cache_key, body.clone());
    Json(body).into_response()
}

// ── POST /api/projects/:id/backfill ───────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BackfillBody {
    #[serde(default)]
    pub cap: Option<usize>,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

/// POST `/api/projects/:id/backfill` — paginate GitHub issues + PRs and
/// insert reference-only skeleton rows into the spine `artifacts` table.
pub async fn backfill_project(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(project_id): Path<String>,
    Json(body): Json<BackfillBody>,
) -> Response {
    let project = match require_project_for_user(&state, &auth_user, &project_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };

    let cap = body.cap.unwrap_or(DEFAULT_CAP).min(HARD_CAP);
    let _state_filter = match normalize_state(body.state.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let strategy: Strategy = match body.strategy.as_deref().unwrap_or("recent").parse() {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid strategy: {e}") })),
            )
                .into_response()
        }
    };

    let token = match installation_token_for(&state.pool, &project.github_app_installation_id).await
    {
        Ok(Some(t)) => Some(t.token),
        Ok(None) => None,
        Err(e) => {
            tracing::error!("installation_token_for failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "GitHub auth failed" })),
            )
                .into_response();
        }
    };

    match crate::backfill::run(&state.pool, &state.spine, &project_id, strategy, cap, token).await {
        Ok(report) => {
            // Evict cached list responses so the next read repopulates from GitHub.
            state
                .proxy_cache
                .invalidate_prefix(&format!("issues:{project_id}:"));
            state
                .proxy_cache
                .invalidate_prefix(&format!("pulls:{project_id}:"));
            Json(serde_json::json!(report)).into_response()
        }
        Err(e) => {
            tracing::error!("backfill failed for project {project_id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("backfill failed: {e}") })),
            )
                .into_response()
        }
    }
}

// ── POST /api/projects/:id/issues/:number/replay-trigger ─────────────────

#[derive(Debug, Deserialize, Default)]
pub struct ReplayTriggerBody {
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ReplayMatch {
    workflow_id: String,
    workflow_name: String,
    label: String,
}

#[derive(Debug, Serialize)]
struct ReplayResponse {
    project_id: String,
    issue_number: u64,
    dry_run: bool,
    matches: Vec<ReplayMatch>,
    event_ids: Vec<i64>,
}

/// POST `/api/projects/:id/issues/:number/replay-trigger` — debug replay of
/// `workflow.trigger_fired` for an issue's current labels.
pub async fn replay_issue_trigger(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((project_id, issue_number)): Path<(String, u64)>,
    Json(body): Json<ReplayTriggerBody>,
) -> Response {
    let project = match require_project_for_user(&state, &auth_user, &project_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let dry_run = body.dry_run.unwrap_or(true);

    let token = match installation_token_for(&state.pool, &project.github_app_installation_id).await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "GitHub App not configured" })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("installation_token_for failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "GitHub auth failed" })),
            )
                .into_response();
        }
    };

    let issue = match fetch_single_issue(
        &token.token,
        &project.repo_owner,
        &project.repo_name,
        issue_number,
    )
    .await
    {
        Ok(i) => i,
        Err(r) => return r,
    };

    let candidates = match crate::workflow_db::find_active_github_workflows_for_workspace_repo(
        &state.pool,
        &project.workspace_id,
        &project.repo_owner,
        &project.repo_name,
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(
                workspace_id = %project.workspace_id,
                error = %e,
                "failed to look up active workflows for replay-trigger"
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "workflow lookup failed" })),
            )
                .into_response();
        }
    };

    let label_set: std::collections::HashSet<&str> =
        issue.labels.iter().map(|l| l.name.as_str()).collect();
    let mut matches: Vec<(crate::workflow::Workflow, String)> = Vec::new();
    for wf in candidates {
        let Some(label) = wf.github_label() else {
            continue;
        };
        if label_set.contains(label) {
            let label_owned = label.to_string();
            matches.push((wf, label_owned));
        }
    }

    let preview: Vec<ReplayMatch> = matches
        .iter()
        .map(|(w, label)| ReplayMatch {
            workflow_id: w.id.clone(),
            workflow_name: w.name.clone(),
            label: label.clone(),
        })
        .collect();

    if dry_run || matches.is_empty() {
        return Json(serde_json::json!(ReplayResponse {
            project_id: project.id,
            issue_number,
            dry_run,
            matches: preview,
            event_ids: Vec::new(),
        }))
        .into_response();
    }

    let metadata = onsager_spine::EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: "portal".to_string(),
    };

    let mut event_ids = Vec::new();
    let mut by_label: std::collections::BTreeMap<String, Vec<WorkflowMatch>> =
        std::collections::BTreeMap::new();
    for (wf, label) in matches {
        by_label.entry(label).or_default().push(WorkflowMatch {
            id: wf.id,
            workspace_id: wf.workspace_id,
            trigger_kind_tag: wf.trigger.kind_tag().to_string(),
        });
    }
    for (label, wfs) in by_label {
        let events: Vec<RoutedEvent> = build_trigger_fired_events(
            &IssueTriggerContext {
                repo_owner: &project.repo_owner,
                repo_name: &project.repo_name,
                issue_number,
                title: &issue.title,
                label: &label,
                source: trigger_source::MANUAL_REPLAY,
                replayed_by: Some(&auth_user.user_id),
            },
            &wfs,
        );
        for ev in events {
            let mut data = match serde_json::to_value(&ev.kind) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("failed to serialize replay-trigger event: {e}");
                    continue;
                }
            };
            if let Some(obj) = data.as_object_mut() {
                obj.entry("workspace_id".to_string())
                    .or_insert(serde_json::Value::String(project.workspace_id.clone()));
            }
            match state
                .spine
                .append_ext(
                    &project.workspace_id,
                    &ev.kind.stream_id(),
                    spine_namespace(&ev.kind),
                    ev.kind.event_type(),
                    data,
                    &metadata,
                    None,
                )
                .await
            {
                Ok(id) => event_ids.push(id),
                Err(e) => {
                    tracing::error!("failed to emit replay-trigger event: {e}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": "failed to emit event" })),
                    )
                        .into_response();
                }
            }
        }
    }

    Json(serde_json::json!(ReplayResponse {
        project_id: project.id,
        issue_number,
        dry_run: false,
        matches: preview,
        event_ids,
    }))
    .into_response()
}

// ── Helper: fetch a single issue from GitHub ──────────────────────────────

#[allow(clippy::result_large_err)]
async fn fetch_single_issue(
    token: &str,
    repo_owner: &str,
    repo_name: &str,
    issue_number: u64,
) -> Result<LiveIssue, Response> {
    let url =
        format!("https://api.github.com/repos/{repo_owner}/{repo_name}/issues/{issue_number}");
    let resp = match gh_client()
        .get(&url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("github issue fetch failed: {e}");
            return Err(Json(serde_json::json!({
                "issue": serde_json::Value::Null,
                "error": "github_unreachable",
            }))
            .into_response());
        }
    };
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(not_found("issue not found"));
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let snippet = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, "github issue fetch non-2xx: {snippet}");
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "github API error" })),
        )
            .into_response());
    }
    resp.json::<LiveIssue>().await.map_err(|e| {
        tracing::warn!("github issue parse failed: {e}");
        (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "github response parse failed" })),
        )
            .into_response()
    })
}
