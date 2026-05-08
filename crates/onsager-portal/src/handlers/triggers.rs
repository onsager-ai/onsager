//! Manual / replay trigger HTTP endpoints (#241 — Category 4 of the
//! trigger taxonomy v2 umbrella #236).
//!
//! Two routes:
//!
//!   POST /api/workflows/{id}/triggers/manual/{name}
//!   POST /api/workflows/{id}/triggers/replay/{source_event_id}
//!
//! Both are authenticated and workspace-scoped (membership required).
//! Both emit two spine events:
//!
//!   1. `workflow.trigger_fired` — same shape forge's
//!      `trigger_subscriber` already consumes, so a manual fire flows
//!      through the workflow runtime exactly like a webhook fire would.
//!   2. `workflow.manual_triggered` — audit-only, namespace `audit`,
//!      capturing actor + workflow + trigger source for the audit log.
//!
//! Replay re-emits the payload of a past `TriggerFired` carrying a
//! `replay_of` marker (and an accumulated `replay_chain`) so downstream
//! consumers can distinguish original vs. replay fires. Replay-of-
//! replay is allowed (#241 resolution).
//!
//! Authorization is "any authenticated workspace member" in v1, per
//! the #241 resolution. Per-tenant role gating (admin-only,
//! workflow-owner-only) is a follow-up.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventMetadata, TriggerKind};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::AuthUser;
use crate::credential_db;
use crate::state::AppState;
use crate::workflow_db;

#[derive(Debug, Deserialize, Default)]
pub struct ManualFireBody {
    /// Optional JSON object merged into the emitted payload. Top-level
    /// keys collide with the canonical fields (`workflow_id`,
    /// `workspace_id`, `name`, `actor`, `source`, `fired_at`,
    /// `trigger_kind`); colliding keys lose to the canonical values.
    #[serde(default)]
    pub payload: Option<Value>,
}

/// `POST /api/workflows/:id/triggers/manual/:name` — fire a manual
/// trigger.
pub async fn fire_manual(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workflow_id, name)): Path<(String, String)>,
    body: Option<Json<ManualFireBody>>,
) -> Response {
    let workflow = match load_workflow(&state, &workflow_id).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    if let Err(r) = require_membership(&state, &auth_user, &workflow.workspace_id).await {
        return r;
    }

    // Trigger kind on the workflow row must be `Manual { name }` and the
    // path-supplied `name` must match. v1 supports exactly one trigger
    // per workflow (current schema), so a workflow that doesn't declare
    // this manual trigger gets a 409 — the fire would emit a
    // `trigger.fired` that no consumer would pick up.
    match &workflow.trigger {
        TriggerKind::Manual { name: declared } if declared == &name => {}
        _ => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "manual_trigger_not_declared",
                    "detail": format!(
                        "workflow `{}` does not declare a manual trigger named `{}` \
                         (its trigger kind is `{}`)",
                        workflow.id,
                        name,
                        workflow.trigger.kind_tag(),
                    ),
                })),
            )
                .into_response();
        }
    }
    if !workflow.active {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "workflow_inactive",
                "detail": "activate the workflow before firing a manual trigger",
            })),
        )
            .into_response();
    }

    let extra = body.map(|Json(b)| b.payload).unwrap_or(None);
    let now = Utc::now();
    let mut payload = serde_json::json!({
        "trigger_kind": "manual",
        "workflow_id": workflow.id,
        "workspace_id": workflow.workspace_id,
        "name": name,
        "fired_at": now,
        "actor": auth_user.user_id,
        "source": "ui",
    });
    merge_extra(&mut payload, extra);

    let trigger_event_id = match emit_trigger_fired(
        &state,
        &workflow,
        "manual",
        payload,
        &auth_user.user_id,
        now,
    )
    .await
    {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = emit_audit(
        &state,
        &workflow,
        "ui_fire",
        serde_json::json!({
            "manual_name": name,
            "trigger_event_id": trigger_event_id,
        }),
        &auth_user.user_id,
        now,
    )
    .await
    {
        return r;
    }

    Json(serde_json::json!({
        "workflow_id": workflow.id,
        "trigger_kind": "manual",
        "name": name,
        "trigger_event_id": trigger_event_id,
        "actor": auth_user.user_id,
    }))
    .into_response()
}

/// `POST /api/workflows/:id/triggers/replay/:source_event_id` — re-emit
/// a past `TriggerFired` event under a workflow.
pub async fn replay(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workflow_id, source_event_id)): Path<(String, i64)>,
) -> Response {
    let workflow = match load_workflow(&state, &workflow_id).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    if let Err(r) = require_membership(&state, &auth_user, &workflow.workspace_id).await {
        return r;
    }

    let source = match load_trigger_fired(&state, source_event_id).await {
        Ok(s) => s,
        Err(r) => return r,
    };

    let now = Utc::now();
    let payload =
        build_replay_payload(&source, &workflow, source_event_id, &auth_user.user_id, now);

    let trigger_event_id = match emit_trigger_fired(
        &state,
        &workflow,
        "replay",
        payload,
        &auth_user.user_id,
        now,
    )
    .await
    {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = emit_audit(
        &state,
        &workflow,
        "ui_replay",
        serde_json::json!({
            "source_event_id": source_event_id,
            "source_trigger_kind": source.trigger_kind,
            "trigger_event_id": trigger_event_id,
        }),
        &auth_user.user_id,
        now,
    )
    .await
    {
        return r;
    }

    Json(serde_json::json!({
        "workflow_id": workflow.id,
        "trigger_kind": "replay",
        "source_event_id": source_event_id,
        "trigger_event_id": trigger_event_id,
        "actor": auth_user.user_id,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct WorkflowRow {
    id: String,
    workspace_id: String,
    trigger: TriggerKind,
    active: bool,
}

async fn load_workflow(state: &AppState, workflow_id: &str) -> Result<WorkflowRow, Response> {
    let workflow = match workflow_db::get_workflow(state.spine.pool(), workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "workflow not found" })),
            )
                .into_response());
        }
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response());
        }
    };
    Ok(WorkflowRow {
        id: workflow.id,
        workspace_id: workflow.workspace_id,
        trigger: workflow.trigger,
        active: workflow.active,
    })
}

async fn require_membership(
    state: &AppState,
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
    match credential_db::is_workspace_member(&state.pool, workspace_id, &auth_user.user_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "workflow not found" })),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("failed to check workspace membership: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

struct PastTriggerFired {
    trigger_kind: String,
    payload: Value,
}

async fn load_trigger_fired(state: &AppState, event_id: i64) -> Result<PastTriggerFired, Response> {
    let row = match state.spine.get_ext_event_by_id(event_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "source_event_not_found",
                    "detail": format!("no event with id {event_id}"),
                })),
            )
                .into_response());
        }
        Err(e) => {
            tracing::error!("failed to fetch source event: {e}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response());
        }
    };

    // Accept either the FactoryEvent envelope or the bare FactoryEventKind
    // shape — both have appeared in events_ext historically.
    let kind = if let Ok(env) = serde_json::from_value::<FactoryEvent>(row.data.clone()) {
        env.event
    } else {
        match serde_json::from_value::<FactoryEventKind>(row.data) {
            Ok(k) => k,
            Err(_) => {
                return Err((
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "source_event_not_factory_event",
                        "detail": "event payload is not a FactoryEvent",
                    })),
                )
                    .into_response());
            }
        }
    };
    match kind {
        FactoryEventKind::TriggerFired {
            trigger_kind,
            payload,
            ..
        } => Ok(PastTriggerFired {
            trigger_kind,
            payload,
        }),
        _ => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "source_event_not_trigger_fired",
                "detail": "source event is not a TriggerFired event",
            })),
        )
            .into_response()),
    }
}

fn build_replay_payload(
    source: &PastTriggerFired,
    workflow: &WorkflowRow,
    source_event_id: i64,
    actor: &str,
    now: chrono::DateTime<Utc>,
) -> Value {
    let mut payload = match source.payload.clone() {
        Value::Object(map) => Value::Object(map),
        other => serde_json::json!({ "original_payload": other }),
    };
    if let Value::Object(map) = &mut payload {
        let mut chain: Vec<i64> = map
            .get("replay_chain")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_i64()).collect())
            .unwrap_or_default();
        chain.push(source_event_id);
        map.insert("replay_of".into(), Value::from(source_event_id));
        map.insert(
            "replay_chain".into(),
            Value::Array(chain.into_iter().map(Value::from).collect()),
        );
        map.insert("replay_actor".into(), Value::String(actor.to_string()));
        map.insert("replay_fired_at".into(), Value::String(now.to_rfc3339()));
        map.insert("workflow_id".into(), Value::String(workflow.id.clone()));
        map.insert(
            "workspace_id".into(),
            Value::String(workflow.workspace_id.clone()),
        );
        map.insert("trigger_kind".into(), Value::String("replay".into()));
        map.insert("source".into(), Value::String("ui_replay".into()));
    }
    payload
}

fn merge_extra(into: &mut Value, extra: Option<Value>) {
    let Some(extra) = extra else { return };
    if let (Value::Object(target), Value::Object(extra)) = (into, extra) {
        for (k, v) in extra {
            // Canonical fields win — never overwrite the route-level
            // identity of the fire.
            target.entry(k).or_insert(v);
        }
    }
}

async fn emit_trigger_fired(
    state: &AppState,
    workflow: &WorkflowRow,
    trigger_kind: &str,
    payload: Value,
    actor: &str,
    now: chrono::DateTime<Utc>,
) -> Result<i64, Response> {
    let envelope = FactoryEvent {
        event: FactoryEventKind::TriggerFired {
            workflow_id: workflow.id.clone(),
            trigger_kind: trigger_kind.to_string(),
            payload,
        },
        correlation_id: None,
        causation_id: None,
        actor: actor.to_string(),
        timestamp: now,
    };
    let data = match serde_json::to_value(&envelope) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("failed to serialize trigger.fired: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: actor.to_string(),
    };
    state
        .spine
        .append_ext(
            &workflow.workspace_id,
            &format!("workflow:{}", workflow.id),
            "workflow",
            "trigger.fired",
            data,
            &metadata,
            None,
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to emit trigger.fired: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
}

async fn emit_audit(
    state: &AppState,
    workflow: &WorkflowRow,
    event_subtype: &str,
    detail: Value,
    actor: &str,
    now: chrono::DateTime<Utc>,
) -> Result<(), Response> {
    let payload = serde_json::json!({
        "workflow_id": workflow.id,
        "workspace_id": workflow.workspace_id,
        "actor": actor,
        "event_subtype": event_subtype,
        "fired_at": now,
        "detail": detail,
    });
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: actor.to_string(),
    };
    state
        .spine
        .append_ext(
            &workflow.workspace_id,
            &format!("audit:workflow:{}", workflow.id),
            "audit",
            "workflow.manual_triggered",
            payload,
            &metadata,
            None,
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to emit workflow.manual_triggered: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
    Ok(())
}
