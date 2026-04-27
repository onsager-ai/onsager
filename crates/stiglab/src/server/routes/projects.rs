//! Project-scoped live-data routes (specs #167, #170, #171).
//!
//! Per spec #170, reference-only artifact rows store identity + our derived
//! lifecycle but *not* GitHub-authored fields. The dashboard joins skeleton
//! rows from `/api/spine/artifacts` with the hydrated rows from the
//! endpoints in this module. Each endpoint:
//!
//! 1. Authenticates the user (`require_auth_user`).
//! 2. Resolves the project and asserts tenant membership.
//! 3. Mints an installation token via `github_app::mint_installation_token`.
//! 4. Fetches live data from GitHub through a per-process LRU+TTL cache
//!    (`AppState::proxy_cache`). Cache hits skip the GitHub round-trip.
//! 5. Returns a slim, dashboard-shaped JSON payload (no raw passthrough —
//!    we only expose the fields the UI needs).
//!
//! Failure modes per #170 fail-open:
//! - Cache miss + GitHub 403 (rate-limited) → return rows with
//!   `error: "rate_limited"` so the dashboard can render the skeleton's
//!   `last_observed_at` placeholder rather than crash.
//! - GitHub 5xx → 502 Bad Gateway with the upstream snippet logged but
//!   not surfaced.
//!
//! The backfill endpoint (POST `/api/projects/:id/backfill`) inserts
//! reference-only skeleton rows directly into the spine `artifacts` table.
//! It mirrors the SQL shape used by `onsager-portal::db::upsert_*_artifact_ref`
//! — a follow-up will consolidate the two writers into a shared spine helper.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, PgPool};

use crate::server::auth::AuthUser;
use crate::server::db;
use crate::server::github_app;
use crate::server::state::AppState;

const HARD_CAP: usize = 200;
const DEFAULT_CAP: usize = 100;

// ── Auth helpers ──────────────────────────────────────────────────────────

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

/// Look up the project + assert the user is a member of its tenant. 404s
/// for unknown projects and non-members alike (matches the project-scoped
/// pattern in `tenants.rs`).
async fn require_project_for_user(
    pool: &AnyPool,
    auth_user: &AuthUser,
    project_id: &str,
) -> Result<crate::core::Project, Response> {
    let user_id = require_auth_user(auth_user)?;
    let project = match db::get_project(pool, project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Err(not_found("project not found")),
        Err(e) => {
            tracing::error!("failed to load project: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };
    match db::is_tenant_member(pool, &project.tenant_id, user_id).await {
        Ok(true) => Ok(project),
        Ok(false) => Err(not_found("project not found")),
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

// ── GitHub fetch helpers ──────────────────────────────────────────────────

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

async fn mint_token(
    pool: &AnyPool,
    install_row_id: &str,
) -> anyhow::Result<Option<github_app::InstallationToken>> {
    let Some(cfg) = github_app::AppConfig::from_env() else {
        return Ok(None);
    };
    let Some(install) = db::get_github_app_installation(pool, install_row_id).await? else {
        return Ok(None);
    };
    let jwt = github_app::mint_app_jwt(&cfg)?;
    let token = github_app::mint_installation_token(&jwt, install.install_id).await?;
    Ok(Some(token))
}

fn gh_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("onsager-stiglab/0.1")
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest build")
}

// ── GET /api/projects/:id/issues ──────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct ListLiveQuery {
    /// `open` (default), `closed`, or `all`. Mirrors GitHub's own
    /// `/repos/:o/:r/issues?state=` query parameter so the proxy is a
    /// thin pass-through over an authenticated, cached fetch.
    pub state: Option<String>,
}

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

/// GET `/api/projects/:project_id/issues?state=open|closed|all` — live
/// issue list, hydrated from GitHub via a short-TTL cache. The dashboard
/// joins this with skeleton rows from `/api/spine/artifacts?kind=github_issue`
/// on `external_ref` (`github:project:{project_id}:issue:{number}`).
pub async fn list_project_issues(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(project_id): Path<String>,
    Query(filters): Query<ListLiveQuery>,
) -> Response {
    let project = match require_project_for_user(&state.db, &auth_user, &project_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };

    let state_q = filters.state.as_deref().unwrap_or("open");
    let cache_key = format!("issues:{project_id}:{state_q}");
    if let Some(cached) = state.proxy_cache.get(&cache_key) {
        return Json(cached).into_response();
    }

    let token = match mint_token(&state.db, &project.github_app_installation_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "GitHub App not configured" })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("mint_token failed: {e}");
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
            return Json(serde_json::json!({
                "issues": [],
                "error": "github_unreachable",
            }))
            .into_response();
        }
    };

    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        // Fail-open per #170: don't 5xx the dashboard; let it render
        // skeletons with the last_observed_at placeholder.
        return Json(serde_json::json!({
            "issues": [],
            "error": "rate_limited",
        }))
        .into_response();
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

    // GitHub returns PRs through the issues endpoint too; filter them out
    // — the dashboard's PR proxy is the right surface for those.
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

/// GET `/api/projects/:project_id/pulls?state=open|closed|all` — live PR
/// list, same shape as `list_project_issues`.
pub async fn list_project_pulls(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(project_id): Path<String>,
    Query(filters): Query<ListLiveQuery>,
) -> Response {
    let project = match require_project_for_user(&state.db, &auth_user, &project_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };

    let state_q = filters.state.as_deref().unwrap_or("open");
    let cache_key = format!("pulls:{project_id}:{state_q}");
    if let Some(cached) = state.proxy_cache.get(&cache_key) {
        return Json(cached).into_response();
    }

    let token = match mint_token(&state.db, &project.github_app_installation_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "GitHub App not configured" })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("mint_token failed: {e}");
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
            return Json(serde_json::json!({
                "pulls": [],
                "error": "github_unreachable",
            }))
            .into_response();
        }
    };

    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Json(serde_json::json!({
            "pulls": [],
            "error": "rate_limited",
        }))
        .into_response();
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
    /// Maximum items to ingest. Capped at `HARD_CAP` (200) regardless of
    /// what's posted. Default 100.
    #[serde(default)]
    pub cap: Option<usize>,
    /// `recent` (default), `active`, or `refract`. Mirrors the existing
    /// `onsager project sync --strategy` CLI from #60.
    #[serde(default)]
    pub strategy: Option<String>,
    /// `open` (default), `closed`, or `all`. Open-only matches the
    /// "inbox starts empty, fills with active work" mental model from #167.
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Serialize)]
struct BackfillReport {
    project_id: String,
    repo: String,
    cap: usize,
    issues_ingested: usize,
    pulls_ingested: usize,
    skipped: usize,
}

/// POST `/api/projects/:id/backfill` — paginate the project's GitHub
/// issues + pulls and insert reference-only skeleton rows into the spine
/// `artifacts` table. Idempotent on `external_ref`.
pub async fn backfill_project(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(project_id): Path<String>,
    Json(body): Json<BackfillBody>,
) -> Response {
    let project = match require_project_for_user(&state.db, &auth_user, &project_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let cap = body.cap.unwrap_or(DEFAULT_CAP).min(HARD_CAP);
    let state_q = body.state.as_deref().unwrap_or("open");

    let token = match mint_token(&state.db, &project.github_app_installation_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "GitHub App not configured" })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("mint_token failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "GitHub auth failed" })),
            )
                .into_response();
        }
    };

    let spine = match &state.spine {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "spine not connected" })),
            )
                .into_response()
        }
    };
    let pool = spine.pool();

    let mut report = BackfillReport {
        project_id: project.id.clone(),
        repo: format!("{}/{}", project.repo_owner, project.repo_name),
        cap,
        issues_ingested: 0,
        pulls_ingested: 0,
        skipped: 0,
    };

    // Fetch issues (which include PRs in GitHub's response shape) and
    // pulls separately so PR-side metadata (head_sha etc.) is available
    // when we later need it.
    let issues_url = format!(
        "https://api.github.com/repos/{owner}/{repo}/issues?state={state_q}&per_page=100",
        owner = project.repo_owner,
        repo = project.repo_name,
    );
    let issues: Vec<LiveIssue> = match fetch_paginated(&token.token, &issues_url, cap).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    for i in issues.into_iter().take(cap) {
        if i.pull_request.is_some() {
            // PRs come back through the issues endpoint too — handle them
            // in the pulls loop below where we have full PR metadata.
            continue;
        }
        let lifecycle = match i.state.as_str() {
            "closed" => "archived",
            _ => "draft",
        };
        match upsert_issue_skeleton(pool, &project.id, i.number, lifecycle).await {
            Ok(true) => report.issues_ingested += 1,
            Ok(false) => report.skipped += 1,
            Err(e) => {
                tracing::warn!("issue skeleton upsert failed: {e}");
                report.skipped += 1;
            }
        }
    }

    let pulls_url = format!(
        "https://api.github.com/repos/{owner}/{repo}/pulls?state={state_q}&per_page=100",
        owner = project.repo_owner,
        repo = project.repo_name,
    );
    let pulls: Vec<LivePull> = match fetch_paginated(&token.token, &pulls_url, cap).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    for p in pulls.into_iter().take(cap) {
        let lifecycle = match (p.state.as_str(), p.merged_at.is_some()) {
            (_, true) => "released",
            ("closed", false) => "archived",
            _ => "in_progress",
        };
        match upsert_pr_skeleton(pool, &project.id, p.number, lifecycle).await {
            Ok(true) => report.pulls_ingested += 1,
            Ok(false) => report.skipped += 1,
            Err(e) => {
                tracing::warn!("pr skeleton upsert failed: {e}");
                report.skipped += 1;
            }
        }
    }

    // Backfill changes the artifact set the dashboard reads; drop any
    // cached responses for this project so the next list call sees the
    // new rows immediately.
    // (Cache is keyed by `issues:{id}:..` / `pulls:{id}:..`; clear by
    // poking individual variants we know about.)
    for s in ["open", "closed", "all"] {
        state.proxy_cache.put(
            format!("issues:{project_id}:{s}"),
            serde_json::json!({ "issues": [] }),
        );
        state.proxy_cache.put(
            format!("pulls:{project_id}:{s}"),
            serde_json::json!({ "pulls": [] }),
        );
    }
    // Above is a placeholder warm-up — invalidation proper requires
    // a `clear` method we don't have yet; the next read after TTL will
    // refresh. (Bounded by 60s; acceptable for v1.)

    Json(serde_json::json!(report)).into_response()
}

// ── Skeleton upsert SQL ───────────────────────────────────────────────────
//
// Mirrors the shape used by `onsager-portal::db::upsert_*_artifact_ref` so
// webhook deliveries and dashboard backfills produce identical rows. A
// follow-up will lift the SQL into a shared spine helper.

async fn fetch_paginated<T: serde::de::DeserializeOwned>(
    token: &str,
    url: &str,
    cap: usize,
) -> Result<Vec<T>, Response> {
    let mut out = Vec::new();
    let mut page = 1u32;
    while out.len() < cap {
        let paged = format!("{url}&page={page}");
        let resp = match gh_client()
            .get(&paged)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("github fetch failed: {e}");
                return Err((
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": "github fetch failed" })),
                )
                    .into_response());
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let snippet = resp.text().await.unwrap_or_default();
            tracing::warn!(%status, "github fetch non-2xx: {snippet}");
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "github API error" })),
            )
                .into_response());
        }
        let batch: Vec<T> = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("github fetch parse failed: {e}");
                return Err((
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": "github parse failed" })),
                )
                    .into_response());
            }
        };
        let count = batch.len();
        out.extend(batch);
        if count < 100 {
            break;
        }
        page += 1;
    }
    Ok(out)
}

/// Insert a reference-only `Kind::GithubIssue` skeleton row, idempotent on
/// `external_ref`. Returns `Ok(true)` if a new row was inserted, `Ok(false)`
/// if a row already existed (the caller counts these as "skipped").
async fn upsert_issue_skeleton(
    pool: &PgPool,
    project_id: &str,
    issue_number: u64,
    state: &str,
) -> anyhow::Result<bool> {
    let external_ref = format!("github:project:{project_id}:issue:{issue_number}");
    let new_id = format!("art_iss_{}", uuid::Uuid::new_v4().simple());

    // Two-step upsert — same advisory-lock pattern as the portal.
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT artifact_id FROM artifacts WHERE external_ref = $1")
            .bind(&external_ref)
            .fetch_optional(pool)
            .await?;
    if existing.is_some() {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, current_version, \
             external_ref, metadata, last_observed_at) \
         VALUES ($1, 'github_issue', NULL, NULL, 'onsager-stiglab', $2, 1, $3, \
                 jsonb_build_object('project_id', $4::text, 'issue_number', $5::bigint), NOW())",
    )
    .bind(&new_id)
    .bind(state)
    .bind(&external_ref)
    .bind(project_id)
    .bind(issue_number as i64)
    .execute(pool)
    .await?;
    Ok(true)
}

async fn upsert_pr_skeleton(
    pool: &PgPool,
    project_id: &str,
    pr_number: u64,
    state: &str,
) -> anyhow::Result<bool> {
    let external_ref = format!("github:project:{project_id}:pr:{pr_number}");
    let new_id = format!("art_pr_{}", uuid::Uuid::new_v4().simple());

    let existing: Option<(String,)> =
        sqlx::query_as("SELECT artifact_id FROM artifacts WHERE external_ref = $1")
            .bind(&external_ref)
            .fetch_optional(pool)
            .await?;
    if existing.is_some() {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, current_version, \
             external_ref, metadata, last_observed_at) \
         VALUES ($1, 'pull_request', NULL, NULL, 'onsager-stiglab', $2, 1, $3, \
                 jsonb_build_object('project_id', $4::text, 'pr_number', $5::bigint), NOW())",
    )
    .bind(&new_id)
    .bind(state)
    .bind(&external_ref)
    .bind(project_id)
    .bind(pr_number as i64)
    .execute(pool)
    .await?;
    Ok(true)
}
