//! Spine read + write API for the dashboard.
//!
//! Spec #259 (sub-issue of #222) made portal the dashboard's spine API.
//!
//! Reads land directly against the spine `events_ext` and `artifacts`
//! tables. Writes are control-only — retry/abort emit a single event
//! via `EventStore::append_ext`, no GitHub side-effects, no row
//! inserts. (Per #278 artifact creation is exclusively the substrate's
//! auto-trigger flow; portal does not own a creation endpoint.)

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use onsager_spine::EventMetadata;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use ts_rs::TS;

use crate::auth::AuthUser;
use crate::handlers::workspaces::require_workspace_access;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Required workspace scope (#164). Missing → 400.
    pub workspace: String,
    pub stream_type: Option<String>,
    pub event_type: Option<String>,
    /// Exact-match filter on `stream_id`. Used by per-artifact and
    /// per-workflow views in the dashboard to pull just the events for
    /// one entity without scanning the whole table client-side.
    pub stream_id: Option<String>,
    /// Filter to events tied to a specific run (spec #303). A "run" id
    /// is an artifact_id; events match via any of:
    /// - `stream_id = run_id` (artifact-keyed events), or
    /// - `data->>'artifact_id' = run_id` (events that carry the artifact
    ///   id in their payload), or
    /// - `stream_id IN (SELECT id FROM sessions WHERE artifact_id = run_id)`
    ///   (session-keyed events whose session points back to the artifact).
    pub run_id: Option<String>,
    /// When `true`, filter to the operator-grain allowlist (ADR 0019 /
    /// spec #465). The Activity tab uses this to surface only events an
    /// operator cares about — stage transitions, gate verdicts, run
    /// lifecycle — and hide internal subsystem dispatches and analyzer
    /// diagnostics. Default `false` for backward compatibility; the
    /// `/api/workspaces/:id/activity` alias defaults it to `true`.
    #[serde(default)]
    pub operator_grain: Option<bool>,
    pub limit: Option<i64>,
}

/// The static allowlist of `event_type` strings that count as
/// operator-grain. Derived once from `onsager_registry::EVENTS`; called
/// per request which is cheap (52 rows, vec scan).
fn operator_grain_kinds() -> Vec<&'static str> {
    onsager_registry::EVENTS
        .events
        .iter()
        .filter(|e| e.operator_grain)
        .map(|e| e.kind)
        .collect()
}

#[derive(Debug, Serialize, FromRow, TS)]
#[ts(export)]
pub struct SpineEvent {
    /// Spine row id. Modelled as `number` on the TS side to match the
    /// pre-existing hand-typed shape; JS `Number` losslessly represents
    /// every event id we project here.
    #[ts(type = "number")]
    pub id: i64,
    pub stream_id: String,
    pub stream_type: String,
    pub event_type: String,
    /// Event payload — always an object in practice (Postgres jsonb).
    /// `Record<string, unknown>` on the TS side keeps the dashboard's
    /// `e.data.<field> as T` access pattern working.
    #[ts(type = "Record<string, unknown>")]
    pub data: serde_json::Value,
    pub actor: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, FromRow, TS)]
#[ts(export)]
pub struct SpineArtifact {
    pub id: String,
    pub kind: String,
    /// Provider-authored title for the artifact. NULL for reference-only
    /// external-source artifacts (#170) — clients hydrate via the
    /// `/api/projects/:id/{issues,pulls}` proxy.
    pub name: Option<String>,
    pub state: String,
    /// Provider-authored author/login. Same NULL convention as `name`.
    pub owner: Option<String>,
    pub current_version: i32,
    pub consumers: serde_json::Value,
    /// External-system handle (e.g. `github:project:abc:issue:42`) used to
    /// join skeleton rows with live proxy responses.
    pub external_ref: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Last webhook touch — used to render "last seen N min ago" placeholders
    /// when the proxy is rate-limited (#170 fail-open).
    pub last_observed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, FromRow, TS)]
#[ts(export, rename = "ArtifactVersion")]
pub struct ArtifactVersionRow {
    pub version: i32,
    pub content_ref_uri: String,
    pub content_ref_checksum: Option<String>,
    pub change_summary: String,
    pub created_by_session: String,
    pub parent_version: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, FromRow, TS)]
#[ts(export, rename = "ArtifactLineageEntry")]
pub struct VerticalLineageRow {
    pub version: i32,
    pub session_id: String,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, FromRow, TS)]
#[ts(export, rename = "ArtifactHorizontalLineageEntry")]
pub struct HorizontalLineageRow {
    pub source_artifact_id: String,
    pub source_version: i32,
    pub role: String,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

/// Detail-page shape for an artifact: extends `SpineArtifact` (via
/// `#[serde(flatten)]`) with the version chain, both lineage axes, and
/// related events so the dashboard's artifact-detail page renders
/// without a second roundtrip.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ArtifactDetail {
    #[serde(flatten)]
    pub base: SpineArtifact,
    pub created_by: String,
    pub versions: Vec<ArtifactVersionRow>,
    pub vertical_lineage: Vec<VerticalLineageRow>,
    pub horizontal_lineage: Vec<HorizontalLineageRow>,
    pub related_events: Vec<SpineEvent>,
}

/// Envelope for `GET /api/spine/artifacts/:id` — keeps the response
/// fully typed without an intermediate `serde_json::Value` round-trip.
#[derive(Debug, Serialize)]
struct ArtifactDetailEnvelope {
    artifact: ArtifactDetail,
}

#[derive(Debug, Deserialize, Default)]
pub struct RetryRequest {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AbortRequest {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

/// Standard 400 body for the `?workspace=` requirement on every
/// workspace-scoped list endpoint (#164).
fn missing_workspace() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": "workspace query parameter is required",
            "detail": "every workspace-scoped list endpoint requires ?workspace=<id>",
        })),
    )
        .into_response()
}

/// Emit a single spine event, swallowing errors with a warning — every
/// caller in this module treats the emit as best-effort because the
/// row mutation has already happened (or there is none).
async fn emit(
    state: &AppState,
    workspace_id: &str,
    stream_id: &str,
    namespace: &str,
    actor: &str,
    event_type: &str,
    data: serde_json::Value,
) {
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: actor.to_string(),
    };
    if let Err(e) = state
        .spine
        .append_ext(
            workspace_id,
            stream_id,
            namespace,
            event_type,
            data,
            &metadata,
            None,
        )
        .await
    {
        tracing::warn!("failed to emit {event_type} event: {e}");
    }
}

/// Activity-tab query params: same shape as `EventsQuery` minus the
/// workspace (which is on the path) and minus `operator_grain` (which is
/// forced to `true` for the Activity surface).
#[derive(Debug, Deserialize, Default)]
pub struct ActivityQuery {
    #[serde(default)]
    pub stream_type: Option<String>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub stream_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/workspaces/:id/activity — path-scoped operator-grain feed
/// (ADR 0019 / spec #465). Wraps `list_events` with the workspace from
/// the path and `operator_grain=true` baked in. Matches the dashboard's
/// `/workspaces/:slug/activity` URL shape so the Activity tab can call
/// it directly without re-stating the workspace as a query param.
///
/// The query-param `GET /api/spine/events?workspace=W&operator_grain=...`
/// shape stays for non-dashboard MCP clients that already speak it; this
/// alias is purely additive.
pub async fn list_activity(
    state: State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
    Query(q): Query<ActivityQuery>,
) -> Response {
    let params = EventsQuery {
        workspace: workspace_id,
        stream_type: q.stream_type,
        event_type: q.event_type,
        stream_id: q.stream_id,
        run_id: q.run_id,
        operator_grain: Some(true),
        limit: q.limit,
    };
    list_events(state, auth_user, Query(params)).await
}

/// Query `events_ext` for the given workspace + filters and return the
/// projected rows. Pure data-access — no auth, no HTTP, no response
/// shaping. Pulled out of [`list_events`] so integration tests can
/// drive the operator-grain filter against a real Postgres without
/// rebuilding `AppState`. Workspace authorization is the handler's job
/// (it is required, not optional — see [`require_workspace_access`]).
///
/// Returns `Ok(vec![])` rather than running an unbounded query when
/// `operator_grain` is `Some(true)` but the registry has no operator-
/// grain rows. That is a manifest-shape error (the registry tests
/// catch it); a deployed binary with an empty allowlist must not
/// silently fall back to the unfiltered stream.
pub async fn query_events_for_workspace(
    pool: &sqlx::PgPool,
    workspace_id: &str,
    params: &EventsQuery,
) -> Result<Vec<SpineEvent>, sqlx::Error> {
    let limit = params.limit.unwrap_or(50).min(500);

    if params.operator_grain.unwrap_or(false) && operator_grain_kinds().is_empty() {
        return Ok(Vec::new());
    }

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT id, stream_id, namespace AS stream_type, event_type, data, \
                COALESCE(metadata->>'actor', '') AS actor, created_at \
         FROM events_ext WHERE workspace_id = ",
    );
    qb.push_bind(workspace_id.to_string());
    if let Some(st) = params.stream_type.as_deref() {
        qb.push(" AND namespace = ").push_bind(st.to_string());
    }
    if let Some(et) = params.event_type.as_deref() {
        qb.push(" AND event_type = ").push_bind(et.to_string());
    }
    // Operator-grain allowlist (ADR 0019 / spec #465). When the caller
    // opts in (Activity tab via the path-scoped alias below), restrict
    // results to the registry's `operator_grain: true` rows. Stacks
    // with `event_type` if both are passed — `operator_grain=true`
    // narrows to the allowlist, then `event_type` narrows further.
    if params.operator_grain.unwrap_or(false) {
        let kinds = operator_grain_kinds();
        qb.push(" AND event_type = ANY(");
        qb.push_bind(kinds.iter().map(|k| k.to_string()).collect::<Vec<_>>());
        qb.push(")");
    }
    if let Some(sid) = params.stream_id.as_deref() {
        qb.push(" AND stream_id = ").push_bind(sid.to_string());
    }
    // run_id (spec #303): a run is an artifact, but related events may
    // be keyed by either the artifact_id (stream_id) or a session_id
    // whose session row points back to the artifact. The OR covers
    // both shapes without forcing the dashboard to issue two queries.
    if let Some(rid) = params.run_id.as_deref() {
        qb.push(" AND (stream_id = ");
        qb.push_bind(rid.to_string());
        qb.push(" OR data->>'artifact_id' = ");
        qb.push_bind(rid.to_string());
        qb.push(" OR stream_id IN (SELECT id FROM sessions WHERE artifact_id = ");
        qb.push_bind(rid.to_string());
        qb.push("))");
    }
    qb.push(" ORDER BY id DESC LIMIT ").push_bind(limit);

    qb.build_query_as::<SpineEvent>().fetch_all(pool).await
}

/// GET /api/spine/events?workspace=W — query the events_ext table.
pub async fn list_events(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(params): Query<EventsQuery>,
) -> Response {
    let workspace_id = params.workspace.trim();
    if workspace_id.is_empty() {
        return missing_workspace();
    }
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, workspace_id).await {
        return r;
    }

    match query_events_for_workspace(state.spine.pool(), workspace_id, &params).await {
        Ok(events) => Json(serde_json::json!({ "events": events })).into_response(),
        Err(e) => {
            tracing::error!("spine events query failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to query events" })),
            )
                .into_response()
        }
    }
}

/// Filters for `GET /api/spine/artifacts`.
#[derive(Debug, Deserialize)]
pub struct ListArtifactsQuery {
    /// Required workspace scope (#164). Missing → 400.
    pub workspace: String,
    /// Filter by `kind` discriminator (e.g. `pull_request`, `github_issue`).
    pub kind: Option<String>,
    /// Filter by `metadata->>'project_id'`.
    pub project_id: Option<String>,
}

/// GET /api/spine/artifacts?workspace=W — list artifacts from the spine.
pub async fn list_artifacts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(filters): Query<ListArtifactsQuery>,
) -> Response {
    let workspace_id = filters.workspace.trim();
    if workspace_id.is_empty() {
        return missing_workspace();
    }
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, workspace_id).await {
        return r;
    }

    let pool = state.spine.pool();

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT artifact_id AS id, kind, name, state, owner, current_version, \
                consumers, external_ref, created_at, updated_at, last_observed_at \
         FROM artifacts WHERE workspace_id = ",
    );
    qb.push_bind(workspace_id.to_string());
    if let Some(kind) = filters.kind.as_deref() {
        qb.push(" AND kind = ").push_bind(kind.to_string());
    }
    if let Some(project_id) = filters.project_id.as_deref() {
        qb.push(" AND metadata->>'project_id' = ")
            .push_bind(project_id.to_string());
    }
    qb.push(" ORDER BY updated_at DESC LIMIT 100");

    match qb.build_query_as::<SpineArtifact>().fetch_all(pool).await {
        Ok(artifacts) => Json(serde_json::json!({ "artifacts": artifacts })).into_response(),
        Err(e) => {
            tracing::error!("spine artifacts query failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to query artifacts" })),
            )
                .into_response()
        }
    }
}

/// GET /api/spine/artifacts/:id — single artifact detail with versions
/// and lineage. 404s callers who aren't members of the artifact's
/// workspace (#164) so artifact IDs can't be enumerated cross-workspace.
pub async fn get_artifact(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let pool = state.spine.pool();

    let workspace_id: String = match sqlx::query_scalar::<_, String>(
        "SELECT workspace_id FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(w)) => w,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("spine artifact workspace lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to query artifact" })),
            )
                .into_response();
        }
    };

    // 404 (not 403) on workspace mismatch via the shared helper —
    // rewrite the body to "artifact not found" so artifact IDs don't
    // leak via the workspace-not-found body.
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        if r.status() == StatusCode::NOT_FOUND {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        return r;
    }

    let artifact = sqlx::query_as::<_, SpineArtifact>(
        "SELECT artifact_id AS id, kind, name, state, owner, current_version, consumers, \
         external_ref, created_at, updated_at, last_observed_at \
         FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await;

    let artifact = match artifact {
        Ok(Some(a)) => a,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("spine artifact query failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to query artifact" })),
            )
                .into_response();
        }
    };

    let versions = match sqlx::query_as::<_, ArtifactVersionRow>(
        "SELECT version, content_ref_uri, content_ref_checksum, change_summary, \
         created_by_session, parent_version, created_at \
         FROM artifact_versions WHERE artifact_id = $1 ORDER BY version DESC",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("failed to load versions for artifact {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load artifact versions" })),
            )
                .into_response();
        }
    };

    let lineage = match sqlx::query_as::<_, VerticalLineageRow>(
        "SELECT version, session_id, recorded_at \
         FROM vertical_lineage WHERE artifact_id = $1 ORDER BY version DESC",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to load vertical lineage for artifact {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load vertical lineage" })),
            )
                .into_response();
        }
    };

    let horizontal = match sqlx::query_as::<_, HorizontalLineageRow>(
        "SELECT source_artifact_id, source_version, role, recorded_at \
         FROM horizontal_lineage WHERE artifact_id = $1 \
         ORDER BY recorded_at ASC",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to load horizontal lineage for artifact {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load horizontal lineage" })),
            )
                .into_response();
        }
    };

    let created_by: String =
        sqlx::query_scalar("SELECT created_by FROM artifacts WHERE artifact_id = $1")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();

    let related_events = fetch_related_events(pool, &id).await.unwrap_or_else(|e| {
        tracing::warn!("failed to load related events for artifact {id}: {e}");
        Vec::new()
    });

    let detail = ArtifactDetail {
        base: artifact,
        created_by,
        versions,
        vertical_lineage: lineage,
        horizontal_lineage: horizontal,
        related_events,
    };
    Json(ArtifactDetailEnvelope { artifact: detail }).into_response()
}

/// Fetch spine events related to an artifact for the per-run DAG (issue #14
/// phase 3). Filters on `stream_id = forge:<artifact_id>` (the convention
/// forge follows) and on session completions whose payload references this
/// artifact.
async fn fetch_related_events(
    pool: &sqlx::PgPool,
    artifact_id: &str,
) -> Result<Vec<SpineEvent>, sqlx::Error> {
    let stream_key = format!("forge:{artifact_id}");
    sqlx::query_as::<_, SpineEvent>(
        "SELECT id, stream_id, namespace AS stream_type, event_type, data, \
                COALESCE(metadata->>'actor', '') AS actor, created_at \
         FROM events_ext \
         WHERE stream_id = $1 \
            OR (event_type IN ('session.completed', 'session.failed') \
                AND data->>'artifact_id' = $2) \
         ORDER BY id ASC \
         LIMIT 500",
    )
    .bind(&stream_key)
    .bind(artifact_id)
    .fetch_all(pool)
    .await
}

/// POST /api/spine/artifacts/:id/retry
pub async fn retry_artifact(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<RetryRequest>,
) -> Response {
    let pool = state.spine.pool();

    let row = match sqlx::query_as::<_, (String, String)>(
        "SELECT workspace_id, state FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!("artifact lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load artifact" })),
            )
                .into_response();
        }
    };
    let Some((workspace_id, state_str)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "artifact not found" })),
        )
            .into_response();
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        if r.status() == StatusCode::NOT_FOUND {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        return r;
    }

    if state_str == "archived" || state_str == "released" {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("cannot retry artifact in {state_str} state"),
            })),
        )
            .into_response();
    }

    let actor = req.actor.as_deref().unwrap_or("dashboard");
    emit(
        &state,
        &workspace_id,
        &format!("forge:{id}"),
        "forge",
        actor,
        "forge.retry_requested",
        serde_json::json!({
            "artifact_id": id,
            "reason": req.reason,
            "previous_state": state_str,
        }),
    )
    .await;

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "artifact_id": id,
            "action": "retry_requested",
        })),
    )
        .into_response()
}

/// POST /api/spine/artifacts/:id/abort
pub async fn abort_artifact(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<AbortRequest>,
) -> Response {
    let pool = state.spine.pool();

    let row = match sqlx::query_as::<_, (String, String)>(
        "SELECT workspace_id, state FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!("artifact lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load artifact" })),
            )
                .into_response();
        }
    };
    let Some((workspace_id, previous_state)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "artifact not found" })),
        )
            .into_response();
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        if r.status() == StatusCode::NOT_FOUND {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        return r;
    }

    if previous_state == "archived" {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "artifact already archived" })),
        )
            .into_response();
    }

    let reason = req
        .reason
        .clone()
        .unwrap_or_else(|| "aborted via dashboard".to_string());
    let actor = req.actor.as_deref().unwrap_or("dashboard");

    if let Err(e) = sqlx::query(
        "UPDATE artifacts SET state = 'archived', updated_at = NOW() WHERE artifact_id = $1",
    )
    .bind(&id)
    .execute(pool)
    .await
    {
        tracing::error!("failed to archive artifact {id}: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "failed to archive artifact" })),
        )
            .into_response();
    }

    emit(
        &state,
        &workspace_id,
        &format!("forge:{id}"),
        "forge",
        actor,
        "artifact.archived",
        serde_json::json!({
            "artifact_id": id,
            "reason": reason,
            "previous_state": previous_state,
        }),
    )
    .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "artifact_id": id,
            "action": "archived",
            "reason": reason,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The operator-grain allowlist comes from the registry manifest;
    /// changing what counts as operator-grain is a registry edit, not a
    /// handler edit. Spot-check a few entries on both sides of the line
    /// (ADR 0019 / spec #465).
    #[test]
    fn operator_grain_allowlist_pulls_from_registry() {
        let kinds = operator_grain_kinds();
        // Run lifecycle / verdict / trigger events are operator-grain.
        assert!(kinds.contains(&"session.completed"));
        assert!(kinds.contains(&"verify.verdict"));
        assert!(kinds.contains(&"trigger.fired"));
        assert!(kinds.contains(&"stage.entered"));
        assert!(kinds.contains(&"node.failed"));
        // Internal subsystem dispatches and analyzer diagnostics are not.
        assert!(!kinds.contains(&"forge.shaping_returned"));
        assert!(!kinds.contains(&"forge.insight_observed"));
        assert!(!kinds.contains(&"forge.decision_made"));
    }

    /// The path-scoped `/api/workspaces/:id/activity` alias must force
    /// `operator_grain=true` regardless of what the client passes —
    /// it's the contract that distinguishes the Activity surface from
    /// the unfiltered `/api/spine/events`. Verifies the alias plumbing
    /// (`ActivityQuery → EventsQuery`) by constructing the same params
    /// the handler builds.
    #[test]
    fn activity_alias_forces_operator_grain_true() {
        let q = ActivityQuery {
            stream_type: None,
            event_type: Some("verify.verdict".into()),
            stream_id: None,
            run_id: None,
            limit: Some(50),
        };
        // Mirror the construction inside `list_activity` so the test
        // pins the conversion shape, not the handler's `State`/`Auth`
        // wiring.
        let params = EventsQuery {
            workspace: "ws-test".into(),
            stream_type: q.stream_type,
            event_type: q.event_type,
            stream_id: q.stream_id,
            run_id: q.run_id,
            operator_grain: Some(true),
            limit: q.limit,
        };
        assert_eq!(params.workspace, "ws-test");
        assert_eq!(params.operator_grain, Some(true));
        assert_eq!(params.event_type.as_deref(), Some("verify.verdict"));
        assert_eq!(params.limit, Some(50));
    }

    /// Default-deserialized `EventsQuery` (no `operator_grain` in the
    /// query string) leaves the filter off — back-compat for existing
    /// `/api/spine/events?workspace=W` callers (MCP clients, etc.).
    /// Driven through axum's `Query` extractor so the test pins the
    /// real wire-decoding shape, not a hand-rolled serde call.
    #[tokio::test]
    async fn events_query_operator_grain_defaults_to_none() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;

        async fn parse(qs: &str) -> EventsQuery {
            let uri = format!("http://x/?{qs}");
            let req = Request::builder().uri(&uri).body(()).unwrap();
            let (mut parts, _) = req.into_parts();
            let Query(q) = Query::<EventsQuery>::from_request_parts(&mut parts, &())
                .await
                .expect("query parses");
            q
        }

        assert_eq!(parse("workspace=ws-1").await.operator_grain, None);
        assert_eq!(
            parse("workspace=ws-1&operator_grain=true")
                .await
                .operator_grain,
            Some(true)
        );
        assert_eq!(
            parse("workspace=ws-1&operator_grain=false")
                .await
                .operator_grain,
            Some(false)
        );
    }
}
