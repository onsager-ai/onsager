//! Shared plan-run request path (ADR 0028 / spec #601).
//!
//! One function turns a stored `SpecPlan` into a tokenized
//! `plan.run_requested` emission: derive the run `plan_id`, mint the
//! plan-scoped session token (#536) when a credential key is
//! configured, attach the workspace's bound repo set (#555), and emit.
//! Both run entry points share it:
//!
//! - the `run_spec_plan` MCP tool (the dashboard Plans → Run button),
//! - the `trigger_fired` listener (every workflow fire — webhook,
//!   manual, schedule, MCP — converted at the edge per ADR 0028).
//!
//! Portal is the only minter; the engine consumes exactly one run
//! event kind.

use onsager_spine::EventMetadata;

use crate::session_token;
use crate::spec_plan_db::StoredSpecPlan;
use crate::state::AppState;

/// Outcome of [`request`]: the derived run id and the emitted
/// `plan.run_requested` event id.
pub struct PlanRunRequested {
    pub plan_id: String,
    pub event_id: i64,
}

/// Emit a tokenized `plan.run_requested` for `stored`. `actor` is the
/// principal the run is attributed to (a user id on interactive paths,
/// `"portal"` on webhook-driven fires); `source` lands in the payload
/// for the dashboard timeline.
pub async fn request(
    state: &AppState,
    actor: &str,
    source: &str,
    stored: &StoredSpecPlan,
) -> anyhow::Result<PlanRunRequested> {
    let workspace_id = &stored.workspace_id;
    let plan_id = onsager_substrate::derive_plan_id(workspace_id, &stored.spec_plan_id);

    // Plan-scoped session token (spec #536): one token per plan, reused
    // across the plan's agent nodes, revoked on the plan-terminal
    // event. Absent a credential key the plan runs without a token —
    // the authoritative gate still applies (mirrors the chat path's
    // graceful degradation).
    let encrypted_session_token = match state.config.credential_key.as_deref() {
        Some(key) => {
            match session_token::mint_for_plan(&state.pool, key, actor, workspace_id, &plan_id)
                .await
            {
                Ok(token) => Some(token),
                Err(e) => {
                    tracing::error!(%plan_id, "failed to mint plan session token: {e}");
                    None
                }
            }
        }
        None => None,
    };

    // Bound repo set with read-scoped encrypted tokens (#555). Empty
    // when the workspace binds no repos; omitted from the wire then.
    let repos = crate::repo_access::build_workspace_repo_access(state, workspace_id).await;

    let mut payload = serde_json::json!({
        "spec_plan_id": stored.spec_plan_id,
        "workspace_id": workspace_id,
        "plan_id": plan_id,
        "actor": actor,
        "source": source,
    });
    if let Some(token) = &encrypted_session_token {
        payload["encrypted_session_token"] = serde_json::Value::String(token.clone());
    }
    if !repos.is_empty() {
        payload["repos"] = serde_json::to_value(&repos).unwrap_or_default();
    }
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: actor.to_string(),
    };
    let stream_id = format!("plan:{}:{}", workspace_id, stored.spec_plan_id);
    let event_id = state
        .spine
        .append_ext(
            workspace_id,
            &stream_id,
            "plan",
            "plan.run_requested",
            payload,
            &metadata,
            None,
        )
        .await?;

    Ok(PlanRunRequested { plan_id, event_id })
}
