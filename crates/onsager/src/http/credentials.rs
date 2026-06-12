//! Workspace credential CRUD. Values are AES-256-GCM sealed with
//! `ONSAGER_CREDENTIAL_KEY` and never leave the server; the list route
//! returns names + timestamps only. The M1 session runner decrypts
//! values at launch and hands them to agents as env vars.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::{AuthUser, require_workspace_access};
use crate::state::AppState;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Credential {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn list_credentials(
    State(state): State<AppState>,
    user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &user, &workspace_id).await {
        return r;
    }
    let rows: Result<Vec<Credential>, _> = sqlx::query_as(
        "SELECT name, created_at, updated_at FROM credentials \
         WHERE workspace_id = $1 ORDER BY name",
    )
    .bind(&workspace_id)
    .fetch_all(&state.pool)
    .await;
    match rows {
        Ok(credentials) => Json(serde_json::json!({ "credentials": credentials })).into_response(),
        Err(e) => {
            tracing::error!("list credentials failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SetCredentialBody {
    pub value: String,
}

pub async fn set_credential(
    State(state): State<AppState>,
    user: AuthUser,
    Path((workspace_id, name)): Path<(String, String)>,
    Json(body): Json<SetCredentialBody>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &user, &workspace_id).await {
        return r;
    }
    let Some(key) = state.config.credential_key.as_deref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "credential storage not configured (set ONSAGER_CREDENTIAL_KEY)"
            })),
        )
            .into_response();
    };
    if name.trim().is_empty() || body.value.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "name and value are required" })),
        )
            .into_response();
    }
    let cipher = match crate::crypto::encrypt(key, &body.value) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("credential encrypt failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let result = sqlx::query(
        "INSERT INTO credentials (workspace_id, name, value_cipher) VALUES ($1, $2, $3) \
         ON CONFLICT (workspace_id, name) \
             DO UPDATE SET value_cipher = $3, updated_at = NOW()",
    )
    .bind(&workspace_id)
    .bind(&name)
    .bind(&cipher)
    .execute(&state.pool)
    .await;
    match result {
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => {
            tracing::error!("set credential failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn delete_credential(
    State(state): State<AppState>,
    user: AuthUser,
    Path((workspace_id, name)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &user, &workspace_id).await {
        return r;
    }
    match sqlx::query("DELETE FROM credentials WHERE workspace_id = $1 AND name = $2")
        .bind(&workspace_id)
        .bind(&name)
        .execute(&state.pool)
        .await
    {
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => {
            tracing::error!("delete credential failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
