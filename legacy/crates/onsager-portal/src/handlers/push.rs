//! Web Push subscription + test-dispatch endpoints (ADR 0020 § Invocation
//! channels, spec #472).
//!
//! - `GET    /api/push/config`    — VAPID public key + enabled flag
//! - `POST   /api/push/subscribe` — upsert a subscription
//! - `DELETE /api/push/subscribe` — drop a subscription (by endpoint)
//! - `POST   /api/push/test`      — fan a synthetic payload to the caller
//!   (dev convenience — gated on `dev_login_enabled` so prod can't be
//!   spammed)
//!
//! All routes are workspace-scope-gated via `require_workspace_access`.
//! The dashboard uses these from `apps/dashboard/src/lib/push.ts`.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use ts_rs::TS;

use crate::auth::AuthUser;
use crate::credential_db;
use crate::push::{HitlKind, PushPayload, dispatch_to_user};
use crate::push_db;
use crate::state::AppState;

async fn require_workspace_access(
    pool: &PgPool,
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

/// `GET /api/push/config` — VAPID public key + enabled flag. The
/// dashboard reads this once on first paint to decide whether to show
/// the push-notifications toggle in settings.
#[derive(Debug, Serialize, TS)]
#[ts(export, rename = "PushConfigResponse")]
pub struct PushConfigResponse {
    pub enabled: bool,
    /// VAPID public key, base64-url (no padding). `None` when push is
    /// disabled (the dashboard then hides the subscription UI).
    pub vapid_public_key: Option<String>,
}

pub async fn get_config(State(state): State<AppState>) -> Json<PushConfigResponse> {
    let vapid = state.push.vapid.as_ref();
    Json(PushConfigResponse {
        enabled: vapid.is_some(),
        vapid_public_key: vapid.map(|v| v.public_key.clone()),
    })
}

/// `POST /api/push/subscribe` body. Mirrors the browser's
/// `PushSubscription.toJSON()` shape (endpoint + keys.p256dh +
/// keys.auth) plus the workspace the caller is scoping to.
#[derive(Debug, Deserialize)]
pub struct SubscribeBody {
    pub workspace_id: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    #[serde(default)]
    pub user_agent: Option<String>,
}

#[derive(Debug, Serialize, TS)]
#[ts(export, rename = "PushSubscribeResponse")]
pub struct SubscribeResponse {
    pub id: String,
}

pub async fn subscribe(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<SubscribeBody>,
) -> Response {
    if state.push.vapid.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "push_disabled" })),
        )
            .into_response();
    }
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &body.workspace_id).await {
        return r;
    }
    if body.endpoint.is_empty() || body.p256dh.is_empty() || body.auth.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing_fields",
                "detail": "endpoint, p256dh and auth are required",
            })),
        )
            .into_response();
    }
    match push_db::upsert(
        &state.pool,
        &auth_user.user_id,
        &body.workspace_id,
        &body.endpoint,
        &body.p256dh,
        &body.auth,
        body.user_agent.as_deref(),
    )
    .await
    {
        Ok(row) => Json(SubscribeResponse { id: row.id }).into_response(),
        Err(e) => {
            tracing::error!("push upsert failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `DELETE /api/push/subscribe` body — same `endpoint` the browser
/// provided originally.
#[derive(Debug, Deserialize)]
pub struct UnsubscribeBody {
    pub endpoint: String,
}

pub async fn unsubscribe(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UnsubscribeBody>,
) -> Response {
    match push_db::delete_by_endpoint(&state.pool, &auth_user.user_id, &body.endpoint).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("push delete failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `POST /api/push/test` body. Lets an operator fire a synthetic push
/// at their own subscriptions during setup — useful for confirming
/// VAPID keys + service worker + browser permissions all line up
/// before the real HITL stream lands.
#[derive(Debug, Deserialize)]
pub struct TestBody {
    pub workspace_id: String,
    pub click_path: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub scope_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestResponse {
    pub delivered: usize,
}

pub async fn test_dispatch(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<TestBody>,
) -> Response {
    if !state.config.dev_login_enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "test_dispatch_disabled" })),
        )
            .into_response();
    }
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &body.workspace_id).await {
        return r;
    }

    let payload = PushPayload {
        title: body.title.unwrap_or_else(|| "Onsager test push".into()),
        body: "If you see this, push notifications are working.".into(),
        hitl_id: "test".into(),
        scope_key: body.scope_key.unwrap_or_else(|| "workspace".into()),
        click_path: body.click_path,
        kind: HitlKind::Approval,
    };
    match dispatch_to_user(
        &state.pool,
        state.push.vapid.as_deref(),
        &auth_user.user_id,
        &payload,
    )
    .await
    {
        Ok(delivered) => Json(TestResponse { delivered }).into_response(),
        Err(crate::push::DispatchError::NotConfigured) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "push_disabled" })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("push test dispatch failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "dispatch_failed" })),
            )
                .into_response()
        }
    }
}
