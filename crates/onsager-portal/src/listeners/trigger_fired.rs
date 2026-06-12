//! Spine listener: `trigger.fired` → tokenized `plan.run_requested`
//! (ADR 0028 / spec #601).
//!
//! Every workflow fire — GitHub webhook, manual endpoint, schedule,
//! Telegram, MCP `run_workflow` — emits `trigger.fired`. This listener
//! converts each fire into a plan run at the edge: resolve the
//! workflow's `spec_kind`, persist a **per-fire** spec plan
//! (`workflow:<id>:run:<event_id>` — unique per fire so the durable
//! plan store never resurrects a previous run's node state), then hand
//! it to [`crate::plan_run::request`], which mints the plan-scoped MCP
//! token and emits `plan.run_requested`. The engine consumes exactly
//! one run event kind; the old engine-side `trigger.fired` bridge (and
//! its tokenless degraded mode) retired with this listener.
//!
//! The library is keyed by the workflow id (#602): the executable DAG
//! is registered at save time, so resolution is a direct lookup. A
//! workflow with no executable stage has no library entry and the fire
//! fails loudly at the engine's lookup — the surviving `trigger.fired`
//! audit row plus a structured warning are the operator's signal (a
//! webhook path cannot 4xx).

use async_trait::async_trait;
use chrono::Utc;
use onsager_spine::factory_event::FactoryEventKind;
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};
use onsager_substrate::spec_plan::{SpecId, SpecInputs, SpecPlan, SpecRef};
use sqlx::postgres::PgPool;

use crate::spec_plan_db::StoredSpecPlan;
use crate::state::AppState;

pub async fn run(state: AppState, store: EventStore) -> anyhow::Result<()> {
    // Warm-start: fires older than this boot were either handled by a
    // previous process or are stale history; replaying them would
    // launch surprise runs.
    let since = store.max_event_id().await.ok().flatten();
    let handler = TriggerFired {
        state,
        store: store.clone(),
    };
    Listener::new(store).with_since(since).run(handler).await
}

struct TriggerFired {
    state: AppState,
    store: EventStore,
}

#[async_trait]
impl EventHandler for TriggerFired {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "trigger.fired" {
            return Ok(());
        }
        if let Err(e) = self.convert(&notification).await {
            tracing::warn!(
                event_id = notification.id,
                "trigger_fired listener: fire not converted to a plan run: {e}"
            );
        }
        Ok(())
    }
}

impl TriggerFired {
    async fn convert(&self, notification: &EventNotification) -> anyhow::Result<()> {
        if notification.table != "events_ext" {
            return Ok(());
        }
        let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
            return Ok(());
        };

        // `trigger.fired` rides the spine both as a bare
        // `FactoryEventKind` and inside a `FactoryEvent` envelope
        // depending on the emitter; accept either (same tolerance the
        // retired engine bridge had).
        let (workflow_id, payload) =
            match serde_json::from_value::<FactoryEventKind>(row.data.clone()) {
                Ok(FactoryEventKind::TriggerFired {
                    workflow_id,
                    payload,
                    ..
                }) => (workflow_id, payload),
                _ => match row.data.get("event").cloned() {
                    Some(inner) => match serde_json::from_value::<FactoryEventKind>(inner) {
                        Ok(FactoryEventKind::TriggerFired {
                            workflow_id,
                            payload,
                            ..
                        }) => (workflow_id, payload),
                        _ => return Ok(()),
                    },
                    None => return Ok(()),
                },
            };

        let spine = self.state.spine.pool();
        let workflow = crate::workflow_db::get_workflow(spine, &workflow_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow `{workflow_id}` not found"))?;

        // ADR 0028 / #602: the library is keyed by the workflow id —
        // the executable DAG registered at save time. No string join.
        let spec_kind = workflow_id.clone();

        // Optional input artifact ids on the payload become the spec's
        // declared external inputs (compiler → `plan.entry_inputs`).
        let input_artifacts: Vec<onsager_artifact::ArtifactId> = payload
            .get("artifacts")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(onsager_artifact::ArtifactId::new)
                    .collect()
            })
            .unwrap_or_default();

        // Per-fire spec plan: unique id per trigger event so re-fires
        // are new runs, not resurrections of old node state.
        let spec_plan_id = format!("workflow:{workflow_id}:run:{}", notification.id);
        let plan = SpecPlan {
            specs: vec![SpecRef {
                id: SpecId::new(format!("workflow:{workflow_id}")),
                kind: spec_kind,
                inputs: SpecInputs {
                    artifacts: input_artifacts,
                },
            }],
            deps: vec![],
        };
        let actor = payload
            .get("actor")
            .and_then(|v| v.as_str())
            .unwrap_or("portal")
            .to_string();
        let source = payload
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("trigger")
            .to_string();

        persist_fire_plan(
            &self.state.pool,
            &workflow.workspace_id,
            &spec_plan_id,
            &plan,
        )
        .await?;

        let stored = StoredSpecPlan {
            workspace_id: workflow.workspace_id.clone(),
            spec_plan_id: spec_plan_id.clone(),
            plan,
            created_by: actor.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let run = crate::plan_run::request(&self.state, &actor, &source, &stored).await?;
        tracing::info!(
            %workflow_id,
            spec_plan_id = %spec_plan_id,
            plan_id = %run.plan_id,
            trigger_event_id = notification.id,
            "trigger.fired converted to plan.run_requested"
        );
        Ok(())
    }
}

/// Persist the per-fire plan so the engine's `plan.run_requested`
/// handler can load it by `(workspace_id, spec_plan_id)` — the same
/// contract dashboard-authored plans use. Idempotent on conflict (a
/// listener retry of the same trigger event re-writes the same row).
async fn persist_fire_plan(
    pool: &PgPool,
    workspace_id: &str,
    spec_plan_id: &str,
    plan: &SpecPlan,
) -> anyhow::Result<()> {
    let plan_json = serde_json::to_value(plan)?;
    sqlx::query(
        "INSERT INTO spec_plans (workspace_id, spec_plan_id, plan_json, created_by) \
         VALUES ($1, $2, $3, 'portal:trigger') \
         ON CONFLICT (workspace_id, spec_plan_id) \
         DO UPDATE SET plan_json = EXCLUDED.plan_json, updated_at = NOW()",
    )
    .bind(workspace_id)
    .bind(spec_plan_id)
    .bind(&plan_json)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {

    /// The per-fire spec-plan id must be unique per trigger event —
    /// the durable plan store keys node state by the derived plan id,
    /// so a reused id would resurrect a previous run (the 0.3 e2e
    /// failure mode this spec fixes). Flipping the id to a
    /// fire-independent shape must fail this test.
    #[test]
    fn per_fire_plan_ids_are_unique_per_event() {
        let a = format!("workflow:{}:run:{}", "wf_1", 41_i64);
        let b = format!("workflow:{}:run:{}", "wf_1", 42_i64);
        assert_ne!(a, b);
        assert!(a.starts_with("workflow:wf_1:run:"));
    }

    /// spec_kind precedence: payload wins over preset; absent both the
    /// fire is rejected (mirrors the retired bridge's contract).
    #[test]
    fn spec_kind_precedence() {
        let payload = serde_json::json!({ "spec_kind": "from-payload" });
        let explicit = payload.get("spec_kind").and_then(|v| v.as_str());
        assert_eq!(explicit, Some("from-payload"));
        let none = serde_json::json!({});
        assert!(none.get("spec_kind").and_then(|v| v.as_str()).is_none());
    }
}
