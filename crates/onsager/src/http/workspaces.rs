//! Workspace listing. Creation beyond the dev seed arrives with a
//! consumer (multi-workspace UI is not a v2 goal yet).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Workspace {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_workspaces(State(state): State<AppState>, user: AuthUser) -> Response {
    let rows: Result<Vec<Workspace>, _> = sqlx::query_as(
        "SELECT w.id, w.slug, w.name, w.created_by, w.created_at \
         FROM workspaces w \
         JOIN workspace_members m ON m.workspace_id = w.id \
         WHERE m.user_id = $1 \
         ORDER BY w.created_at",
    )
    .bind(&user.user_id)
    .fetch_all(&state.pool)
    .await;
    match rows {
        Ok(workspaces) => Json(serde_json::json!({ "workspaces": workspaces })).into_response(),
        Err(e) => {
            tracing::error!("list workspaces failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
