//! `/api/activation` and `/api/admin/activation-funnel` route handlers
//! (spec #404).
//!
//! Four FTUE activation events — `ftue.inspected`, `ftue.drafted`,
//! `ftue.bound`, `ftue.activated` — share one append-only sink. Three
//! are fired from the dashboard (Inspected on first DAG/YAML toggle,
//! Drafted on the first `WorkflowDraft.updated_at` write, Bound on the
//! `BindDraftDialog` success path) and arrive via `POST /api/activation`;
//! the fourth is emitted server-side by the `workflow_activated`
//! spine listener (see `crates/onsager-portal/src/listeners/`).
//!
//! Fire-once is enforced server-side. The handler computes a `dedup_key`
//! per the spec table — `event|user_id-or-anonymous_id|primary-context-id`
//! — and inserts with `ON CONFLICT(dedup_key) DO NOTHING`. Client repeats
//! drop silently.

use std::convert::Infallible;

use axum::Json;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{AuthUser, SessionKind};
use crate::state::AppState;

/// Newtype around `Option<AuthUser>` that does not 4xx on missing /
/// invalid auth. The activation endpoint is the only handler that
/// accepts both anonymous (pre-auth `ftue.inspected`) and authenticated
/// traffic; everywhere else, `AuthUser` is required and the standard
/// rejecting extractor is the right shape.
pub struct OptionalAuthUser(pub Option<AuthUser>);

impl FromRequestParts<AppState> for OptionalAuthUser {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(OptionalAuthUser(
            AuthUser::from_request_parts(parts, state).await.ok(),
        ))
    }
}

/// Closed enum of activation event names. The wire format pins these
/// four — anything else is rejected at the handler with a 400.
const VALID_EVENTS: &[&str] = &[
    "ftue.inspected",
    "ftue.drafted",
    "ftue.bound",
    "ftue.activated",
];

const VALID_SURFACES: &[&str] = &["landing", "chat", "dialog", "spine"];
const VALID_PATHS: &[&str] = &["cloud", "oss"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationEventBody {
    pub event: String,
    pub occurred_at: DateTime<Utc>,
    pub anonymous_id: String,
    pub surface: String,
    pub path: String,
    #[serde(default)]
    pub context: ActivationContext,
}

/// Closed-shape context payload. Matches the spec's interface verbatim;
/// `deny_unknown_fields` rejects anything else at the wire boundary so
/// the funnel contract is mechanically falsifiable.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_status: Option<String>,
}

/// POST /api/activation — record one rung crossing.
///
/// Auth is optional: `ftue.inspected` from cloud.onsager.ai (axis 2 /
/// spec #399) fires before sign-in and carries a null `user_id`. We
/// extract the auth user when present via the same cookie / PAT
/// extractor, but a missing/invalid auth header is **not** an error
/// here — the row is simply written with `user_id = NULL`.
///
/// `ftue.activated` from the dashboard path is rejected: that rung is
/// authoritatively emitted by the spine listener, never by the client.
/// The substrate already knows when a run terminates; coupling
/// activation measurement to whether the user happens to view the page
/// would re-introduce rejected-alternative #4.
pub async fn record_activation(
    State(state): State<AppState>,
    OptionalAuthUser(auth_user): OptionalAuthUser,
    Json(body): Json<ActivationEventBody>,
) -> Response {
    if !VALID_EVENTS.contains(&body.event.as_str()) {
        return reject("unknown event name");
    }
    if !VALID_SURFACES.contains(&body.surface.as_str()) {
        return reject("unknown surface");
    }
    if !VALID_PATHS.contains(&body.path.as_str()) {
        return reject("unknown path");
    }
    if body.anonymous_id.trim().is_empty() {
        return reject("anonymous_id required");
    }
    if body.event == "ftue.activated" {
        return reject("ftue.activated is emitted server-side only");
    }

    let user_id = auth_user.as_ref().map(|u| u.user_id.clone());
    let dedup_key = match build_dedup_key(
        &body.event,
        user_id.as_deref(),
        &body.anonymous_id,
        &body.context,
    ) {
        Ok(k) => k,
        Err(msg) => return reject(msg),
    };
    let context_json = match serde_json::to_value(&body.context) {
        Ok(v) => v,
        Err(_) => return reject("context serialization failed"),
    };

    let id = Uuid::new_v4().to_string();
    let res = sqlx::query(
        "INSERT INTO activation_events \
             (id, event, occurred_at, user_id, anonymous_id, surface, path, context, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (dedup_key) DO NOTHING",
    )
    .bind(&id)
    .bind(&body.event)
    .bind(body.occurred_at)
    .bind(user_id.as_deref())
    .bind(&body.anonymous_id)
    .bind(&body.surface)
    .bind(&body.path)
    .bind(&context_json)
    .bind(&dedup_key)
    .execute(&state.pool)
    .await;

    match res {
        Ok(result) => Json(serde_json::json!({
            "recorded": result.rows_affected() > 0,
        }))
        .into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "activation_events insert failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "insert failed" })),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct FunnelQuery {
    /// Inclusive lower bound (ISO timestamp). Default: 30 days ago.
    pub from: Option<DateTime<Utc>>,
    /// Exclusive upper bound (ISO timestamp). Default: now.
    pub to: Option<DateTime<Utc>>,
}

/// GET /api/admin/activation-funnel — distinct-actor counts per rung.
///
/// Admin-gated. The spec calls for "existing role checks" but no such
/// table exists today; pre-launch we gate on (a) `SessionKind::Dev`
/// (always allowed — dev seed user is the only principal in local
/// dev) or (b) the caller's `github_login` is listed in
/// `ONSAGER_ADMIN_LOGINS`. Cloud deploys configure that env var; OSS
/// self-hosters either set it or run under dev-login. A real
/// workspace-membership-role surface is a follow-up.
pub async fn get_funnel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<FunnelQuery>,
) -> Response {
    if !is_admin(&state, &auth_user) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "admin only" })),
        )
            .into_response();
    }

    let to = q.to.unwrap_or_else(Utc::now);
    let from = q.from.unwrap_or_else(|| to - chrono::Duration::days(30));

    let rows: Result<Vec<(String, i64)>, sqlx::Error> = sqlx::query_as(
        "SELECT event, COUNT(DISTINCT COALESCE(user_id, anonymous_id))::BIGINT AS n \
           FROM activation_events \
          WHERE occurred_at >= $1 AND occurred_at < $2 \
          GROUP BY event",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&state.pool)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "activation funnel query failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "query failed" })),
            )
                .into_response();
        }
    };

    let mut counts = serde_json::Map::new();
    for event in VALID_EVENTS {
        counts.insert((*event).to_string(), serde_json::json!(0));
    }
    for (event, n) in rows {
        counts.insert(event, serde_json::json!(n));
    }

    Json(serde_json::json!({
        "from": from,
        "to": to,
        "counts": counts,
    }))
    .into_response()
}

fn is_admin(state: &AppState, auth_user: &AuthUser) -> bool {
    if auth_user.session_kind == SessionKind::Dev {
        return true;
    }
    let login = auth_user.github_login.to_ascii_lowercase();
    state
        .config
        .admin_github_logins
        .iter()
        .any(|l| l.to_ascii_lowercase() == login)
}

fn build_dedup_key(
    event: &str,
    user_id: Option<&str>,
    anonymous_id: &str,
    context: &ActivationContext,
) -> Result<String, &'static str> {
    let actor = user_id.unwrap_or(anonymous_id);
    match event {
        "ftue.inspected" => Ok(format!("ftue.inspected|{actor}")),
        "ftue.drafted" => {
            let draft_id = context
                .draft_id
                .as_deref()
                .ok_or("ftue.drafted requires context.draft_id")?;
            Ok(format!("ftue.drafted|{actor}|{draft_id}"))
        }
        "ftue.bound" => {
            let user_id = user_id.ok_or("ftue.bound requires authenticated user_id")?;
            let draft_id = context
                .draft_id
                .as_deref()
                .ok_or("ftue.bound requires context.draft_id")?;
            Ok(format!("ftue.bound|{user_id}|{draft_id}"))
        }
        // `ftue.activated` is rejected earlier — listed for completeness
        // so the listener can reuse this helper.
        "ftue.activated" => {
            let user_id = user_id.ok_or("ftue.activated requires user_id")?;
            let workflow_id = context
                .workflow_id
                .as_deref()
                .ok_or("ftue.activated requires context.workflow_id")?;
            Ok(format!("ftue.activated|{user_id}|{workflow_id}"))
        }
        _ => Err("unknown event"),
    }
}

fn reject(msg: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

/// Build a dedup key for a server-emitted `ftue.activated` row. Exposed
/// to the spine listener so the activation table's UNIQUE constraint
/// catches concurrent emitters (multiple `stage.advanced` events for
/// the same workflow during a deploy bounce, etc.).
pub(crate) fn activated_dedup_key(user_id: &str, workflow_id: &str) -> String {
    format!("ftue.activated|{user_id}|{workflow_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_draft(draft_id: &str) -> ActivationContext {
        ActivationContext {
            draft_id: Some(draft_id.to_string()),
            ..ActivationContext::default()
        }
    }

    #[test]
    fn dedup_keys_match_spec_table() {
        let empty = ActivationContext::default();
        assert_eq!(
            build_dedup_key("ftue.inspected", None, "anon-uuid", &empty).unwrap(),
            "ftue.inspected|anon-uuid"
        );
        assert_eq!(
            build_dedup_key("ftue.inspected", Some("user-1"), "anon-uuid", &empty).unwrap(),
            "ftue.inspected|user-1"
        );
        assert_eq!(
            build_dedup_key(
                "ftue.drafted",
                Some("user-1"),
                "anon",
                &ctx_with_draft("d_1"),
            )
            .unwrap(),
            "ftue.drafted|user-1|d_1"
        );
        assert_eq!(
            build_dedup_key("ftue.bound", Some("user-1"), "anon", &ctx_with_draft("d_1"),).unwrap(),
            "ftue.bound|user-1|d_1"
        );
        assert_eq!(
            activated_dedup_key("user-1", "wf_1"),
            "ftue.activated|user-1|wf_1"
        );
    }

    #[test]
    fn dedup_key_rejects_missing_draft_id() {
        let err = build_dedup_key(
            "ftue.drafted",
            Some("user-1"),
            "anon",
            &ActivationContext::default(),
        )
        .unwrap_err();
        assert!(err.contains("draft_id"));
    }

    #[test]
    fn ftue_bound_requires_user_id() {
        let err = build_dedup_key("ftue.bound", None, "anon", &ctx_with_draft("d_1")).unwrap_err();
        assert!(err.contains("user_id"));
    }

    #[test]
    fn unknown_top_level_fields_are_rejected() {
        let json = serde_json::json!({
            "event": "ftue.drafted",
            "occurred_at": "2026-05-19T00:00:00Z",
            "anonymous_id": "a",
            "surface": "chat",
            "path": "cloud",
            "context": { "draft_id": "d_1" },
            "extra_field": "nope",
        });
        let err = serde_json::from_value::<ActivationEventBody>(json).unwrap_err();
        assert!(err.to_string().contains("extra_field"), "{err}");
    }

    #[test]
    fn unknown_context_keys_are_rejected() {
        let json = serde_json::json!({
            "event": "ftue.drafted",
            "occurred_at": "2026-05-19T00:00:00Z",
            "anonymous_id": "a",
            "surface": "chat",
            "path": "cloud",
            "context": { "draft_id": "d_1", "secret_email": "me@x" },
        });
        let err = serde_json::from_value::<ActivationEventBody>(json).unwrap_err();
        assert!(err.to_string().contains("secret_email"), "{err}");
    }
}
