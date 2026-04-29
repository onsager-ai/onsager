//! Project-scoped live-data routes (specs #167, #170, #171).
//!
//! Per spec #170, reference-only artifact rows store identity + our derived
//! lifecycle but *not* GitHub-authored fields. The dashboard joins skeleton
//! rows from `/api/spine/artifacts` with the hydrated rows from the
//! endpoints in this module. Each endpoint:
//!
//! 1. Authenticates the user (`require_auth_user`).
//! 2. Resolves the project and asserts workspace membership.
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

use std::sync::OnceLock;
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
use crate::server::webhook_router::{
    build_trigger_fired_events, trigger_source, IssueTriggerContext, RoutedEvent,
};
use crate::server::workflow_db;
use onsager_spine::factory_event::FactoryEventKind;

const HARD_CAP: usize = 200;
const DEFAULT_CAP: usize = 100;

// ── Auth helpers ──────────────────────────────────────────────────────────

#[allow(clippy::result_large_err)]
fn require_auth_user(auth_user: &AuthUser) -> Result<&str, Response> {
    // Auth is always-on as of #193; the `AuthUser` extractor 401s
    // unauthenticated requests before they reach this helper.
    Ok(auth_user.user_id.as_str())
}

/// Look up the project + assert the user is a member of its workspace. 404s
/// for unknown projects and non-members alike (matches the project-scoped
/// pattern in `workspaces.rs`).
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
    match db::is_workspace_member(pool, &project.workspace_id, user_id).await {
        Ok(true) => Ok(project),
        Ok(false) => Err(not_found("project not found")),
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

/// Shared `reqwest::Client` for proxy + backfill calls. Building a client is
/// relatively expensive (TLS + DNS + connection pool init) and only needs to
/// happen once per process.
fn gh_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("onsager-stiglab/0.1")
            .timeout(Duration::from_secs(15))
            .build()
            .expect("reqwest build")
    })
}

/// Validate the `state` query parameter against a fixed allowlist. The value
/// flows directly into both the cache key (cardinality bound) and the
/// upstream GitHub URL (no parameter injection); rejecting unknown values
/// here is the whole defense.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Strategy {
    /// Newest-first; cheapest. Default.
    Recent,
    /// Drop stale items (closed-but-unmerged PRs, closed issues) before
    /// applying the cap.
    Active,
    /// Re-rank candidates (open before closed, more labels first) before
    /// the cap. Same heuristic as `onsager-portal::backfill::Strategy::Refract`.
    Refract,
}

#[allow(clippy::result_large_err)]
fn normalize_strategy(raw: Option<&str>) -> Result<Strategy, Response> {
    match raw.unwrap_or("recent") {
        "recent" => Ok(Strategy::Recent),
        "active" => Ok(Strategy::Active),
        "refract" => Ok(Strategy::Refract),
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("invalid strategy: {other}"),
            })),
        )
            .into_response()),
    }
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

    let state_q = match normalize_state(filters.state.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
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

    let state_q = match normalize_state(filters.state.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
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
    let state_q = match normalize_state(body.state.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let strategy = match normalize_strategy(body.strategy.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };

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

    // Fetch over-cap so the strategy filter has room to drop stale items
    // before the shared budget is applied. `HARD_CAP` is the upper bound
    // on what we'll actually write; `cap * 2` is the upper bound on what
    // we *consider* (with a 200-item ceiling per fetch).
    let fetch_cap = (cap * 2).min(HARD_CAP);
    let issues_url = format!(
        "https://api.github.com/repos/{owner}/{repo}/issues?state={state_q}&per_page=100",
        owner = project.repo_owner,
        repo = project.repo_name,
    );
    let issues: Vec<LiveIssue> = match fetch_paginated(&token.token, &issues_url, fetch_cap).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let pulls_url = format!(
        "https://api.github.com/repos/{owner}/{repo}/pulls?state={state_q}&per_page=100",
        owner = project.repo_owner,
        repo = project.repo_name,
    );
    let pulls: Vec<LivePull> = match fetch_paginated(&token.token, &pulls_url, fetch_cap).await {
        Ok(v) => v,
        Err(e) => return e,
    };

    let (issues, pulls) = apply_strategy(strategy, issues, pulls);

    // Single budget across both kinds, so a `cap=100` request really tops
    // out at 100 total writes (was 200 in the original). Issues consume
    // first, then PRs.
    let mut budget = cap;

    for i in issues {
        if budget == 0 {
            break;
        }
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
            Ok(true) => {
                report.issues_ingested += 1;
                budget -= 1;
            }
            Ok(false) => report.skipped += 1,
            Err(e) => {
                tracing::warn!("issue skeleton upsert failed: {e}");
                report.skipped += 1;
            }
        }
    }

    for p in pulls {
        if budget == 0 {
            break;
        }
        let lifecycle = match (p.state.as_str(), p.merged_at.is_some()) {
            (_, true) => "released",
            ("closed", false) => "archived",
            _ => "in_progress",
        };
        match upsert_pr_skeleton(pool, &project.id, p.number, lifecycle).await {
            Ok(true) => {
                report.pulls_ingested += 1;
                budget -= 1;
            }
            Ok(false) => report.skipped += 1,
            Err(e) => {
                tracing::warn!("pr skeleton upsert failed: {e}");
                report.skipped += 1;
            }
        }
    }

    // Real eviction (#10): drop every cached list response keyed under
    // this project so the next read repopulates from GitHub. Previous
    // attempt overwrote with empty arrays, which caused subsequent reads
    // to return empty results until TTL elapsed — bug.
    state
        .proxy_cache
        .invalidate_prefix(&format!("issues:{project_id}:"));
    state
        .proxy_cache
        .invalidate_prefix(&format!("pulls:{project_id}:"));

    Json(serde_json::json!(report)).into_response()
}

/// Apply the chosen ranking to the candidate sets before the shared cap.
/// Mirrors the heuristics in `crates/onsager-portal/src/backfill/mod.rs` —
/// kept in sync by hand for now; a follow-up should lift this into the
/// onsager-spine helper that retires the duplicated upsert SQL.
fn apply_strategy(
    strategy: Strategy,
    issues: Vec<LiveIssue>,
    pulls: Vec<LivePull>,
) -> (Vec<LiveIssue>, Vec<LivePull>) {
    match strategy {
        Strategy::Recent => (issues, pulls),
        Strategy::Active => (
            issues.into_iter().filter(|i| i.state == "open").collect(),
            pulls
                .into_iter()
                .filter(|p| p.state == "open" || p.merged_at.is_some())
                .collect(),
        ),
        Strategy::Refract => (refract_rank_issues(issues), refract_rank_pulls(pulls)),
    }
}

fn refract_rank_issues(mut issues: Vec<LiveIssue>) -> Vec<LiveIssue> {
    issues.sort_by(|a, b| {
        let key = |i: &LiveIssue| (i.state == "open", i.labels.len());
        key(b).cmp(&key(a))
    });
    issues
}

fn refract_rank_pulls(mut pulls: Vec<LivePull>) -> Vec<LivePull> {
    pulls.sort_by(|a, b| {
        let key = |p: &LivePull| (p.state == "open", p.merged_at.is_some());
        key(b).cmp(&key(a))
    });
    pulls
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
///
/// Concurrency: the spine schema does not enforce `UNIQUE(external_ref)`
/// (other writers may legitimately share the column), so a naive
/// "check then insert" races. We serialize concurrent upserts on the same
/// `external_ref` via a transaction-scoped advisory lock — same pattern as
/// `onsager-portal::db::upsert_pr_artifact_ref`.
async fn upsert_issue_skeleton(
    pool: &PgPool,
    project_id: &str,
    issue_number: u64,
    state: &str,
) -> anyhow::Result<bool> {
    let external_ref = format!("github:project:{project_id}:issue:{issue_number}");
    let new_id = format!("art_iss_{}", uuid::Uuid::new_v4().simple());

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&external_ref)
        .execute(&mut *tx)
        .await?;

    let existing: Option<(String,)> =
        sqlx::query_as("SELECT artifact_id FROM artifacts WHERE external_ref = $1")
            .bind(&external_ref)
            .fetch_optional(&mut *tx)
            .await?;
    if existing.is_some() {
        tx.commit().await?;
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
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Insert a reference-only `Kind::PullRequest` skeleton row. Same idempotency
/// guarantee as `upsert_issue_skeleton`.
async fn upsert_pr_skeleton(
    pool: &PgPool,
    project_id: &str,
    pr_number: u64,
    state: &str,
) -> anyhow::Result<bool> {
    let external_ref = format!("github:project:{project_id}:pr:{pr_number}");
    let new_id = format!("art_pr_{}", uuid::Uuid::new_v4().simple());

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&external_ref)
        .execute(&mut *tx)
        .await?;

    let existing: Option<(String,)> =
        sqlx::query_as("SELECT artifact_id FROM artifacts WHERE external_ref = $1")
            .bind(&external_ref)
            .fetch_optional(&mut *tx)
            .await?;
    if existing.is_some() {
        tx.commit().await?;
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
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

// ── POST /api/projects/:id/issues/:number/replay-trigger ─────────────────
//
// Active-mode debug counterpart to the passive `issues.labeled` webhook
// path. Reads the issue's *current* labels live from GitHub, finds active
// workflows in the project's workspace whose `(repo, label)` matches, and
// either previews (`dry_run=true`, default) or emits one
// `workflow.trigger_fired` per match identical to the webhook path —
// distinguishable only by `payload.source = "manual_replay"` plus
// `payload.replayed_by = <user_id>`.

#[derive(Debug, Deserialize, Default)]
pub struct ReplayTriggerBody {
    /// `true` (default) returns the matched-workflow list without
    /// emitting; `false` emits one `workflow.trigger_fired` per match.
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
    /// Spine event IDs for the emitted `workflow.trigger_fired` events.
    /// Empty in dry-run mode and when there were zero matches.
    event_ids: Vec<i64>,
}

pub async fn replay_issue_trigger(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((project_id, issue_number)): Path<(String, u64)>,
    Json(body): Json<ReplayTriggerBody>,
) -> Response {
    let project = match require_project_for_user(&state.db, &auth_user, &project_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let dry_run = body.dry_run.unwrap_or(true);

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

    let mut matches: Vec<(crate::core::workflow::Workflow, String)> = Vec::new();
    for label in &issue.labels {
        let hits = match workflow_db::find_active_github_workflows_for_label_in_workspace(
            &state.db,
            &project.workspace_id,
            &project.repo_owner,
            &project.repo_name,
            &label.name,
        )
        .await
        {
            Ok(h) => h,
            Err(e) => {
                tracing::error!(
                    workspace_id = %project.workspace_id,
                    label = %label.name,
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
        for wf in hits {
            matches.push((wf, label.name.clone()));
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
            dry_run: true,
            matches: preview,
            event_ids: Vec::new(),
        }))
        .into_response();
    }

    let Some(spine) = state.spine.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "spine not connected" })),
        )
            .into_response();
    };

    // Group matches by label so each `build_trigger_fired_events` call
    // produces one event per workflow for that label — matching the
    // webhook path which fires once per `(label, workflow)` pair.
    let mut event_ids = Vec::new();
    let mut by_label: std::collections::BTreeMap<String, Vec<crate::core::workflow::Workflow>> =
        std::collections::BTreeMap::new();
    for (wf, label) in matches {
        by_label.entry(label).or_default().push(wf);
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
            // Match the webhook path's workspace_id stamping. `TriggerFired`
            // carries the workflow's workspace inside `payload`, so the
            // top-level field is informational redundancy for the
            // workspace-scoped event listings.
            if let Some(obj) = data.as_object_mut() {
                obj.entry("workspace_id".to_string())
                    .or_insert(serde_json::Value::String(project.workspace_id.clone()));
            }
            match spine
                .emit_raw(
                    &ev.kind.stream_id(),
                    spine_namespace_for(&ev.kind),
                    "stiglab",
                    ev.kind.event_type(),
                    &data,
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

fn spine_namespace_for(kind: &FactoryEventKind) -> &'static str {
    // Keep aligned with `webhooks::spine_namespace` — replays must land
    // in the same partition as live webhook events so listeners see one
    // unified stream.
    match kind {
        FactoryEventKind::TriggerFired { .. } => "workflow",
        _ => "stiglab",
    }
}

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
            tracing::warn!("github single-issue fetch failed: {e}");
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "github fetch failed" })),
            )
                .into_response());
        }
    };
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(not_found("issue not found"));
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let snippet = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, "github single-issue fetch non-2xx: {snippet}");
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "github API error" })),
        )
            .into_response());
    }
    let parsed: LiveIssue = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("github single-issue parse failed: {e}");
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "github response parse failed" })),
            )
                .into_response());
        }
    };
    Ok(parsed)
}
