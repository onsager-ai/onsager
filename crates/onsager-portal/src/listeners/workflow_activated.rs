//! Spine listener for `stage.advanced` → `ftue.activated` activation row
//! (spec #404).
//!
//! The fourth rung of the FTUE activation ladder (Inspected → Drafted →
//! Bound → Activated) is the moment a bound workflow's first run reaches
//! a terminal stage status. The substrate is the truth: when an
//! artifact moves past the final stage of its workflow, the spine emits
//! `stage.advanced` with `to_stage_index: None`. This listener consumes
//! that signal, resolves the workflow's `created_by` user, and writes
//! the `ftue.activated` row into the portal-owned `activation_events`
//! table — fire-once per (user, workflow) via the table's
//! `dedup_key` UNIQUE constraint.
//!
//! Why not have the dashboard fire this? See rejected alternative #4 in
//! the spec: coupling activation measurement to whether the user
//! happens to view the run-detail page would mean a closed tab silently
//! drops the rung. The substrate already knows.
//!
//! Failure / cancellation: today's spine vocabulary has no
//! workflow-run-level terminal signal for failure (`node.failed` is per
//! executor, not per run). v1 lights up the "completed" path only; the
//! `terminal_status` field on the row is set to `completed`. When the
//! substrate gains an explicit run-terminal event, extend the match
//! below.

use std::sync::Arc;

use async_trait::async_trait;
use onsager_spine::factory_event::FactoryEventKind;
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::handlers::activation::activated_dedup_key;

pub async fn run(pool: PgPool, store: EventStore, is_oss: bool) -> anyhow::Result<()> {
    let handler = WorkflowActivated {
        pool: Arc::new(pool),
        store: store.clone(),
        is_oss,
    };
    // Warm-start cursor — `ftue.activated` is a forward-looking signal;
    // backfilling years of historical `stage.advanced` events on every
    // process boot would log misleading "first activation" rows and
    // run a DB lookup per event. `dedup_key` would still drop the
    // duplicate inserts, but the work itself is wasted.
    let since = store.max_event_id().await.ok().flatten();
    Listener::new(store).with_since(since).run(handler).await
}

struct WorkflowActivated {
    pool: Arc<PgPool>,
    store: EventStore,
    is_oss: bool,
}

impl WorkflowActivated {
    async fn handle_stage_advanced(&self, notification: &EventNotification) -> anyhow::Result<()> {
        let ext_row = match notification.table.as_str() {
            "events_ext" => {
                let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
                    return Ok(());
                };
                row
            }
            _ => return Ok(()),
        };
        let kind = serde_json::from_value::<FactoryEventKind>(ext_row.data.clone())?;

        let FactoryEventKind::StageAdvanced {
            workflow_id,
            to_stage_index,
            ..
        } = kind
        else {
            return Ok(());
        };

        // Only the artifact-just-completed-the-final-stage transition is
        // a workflow-run terminal status today. Mid-workflow advances
        // are not activation moments.
        if to_stage_index.is_some() {
            return Ok(());
        }

        // Resolve the workflow's owner. The spine `workflows` table is
        // keyed by `workflow_id` (see migration 006). Without a
        // `created_by` we cannot attribute the activation rung to a
        // user — skip silently.
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT created_by FROM workflows WHERE workflow_id = $1")
                .bind(&workflow_id)
                .fetch_optional(&*self.pool)
                .await?;
        let Some((Some(user_id),)) = row else {
            tracing::debug!(
                %workflow_id,
                "ftue.activated: workflow has no created_by, skipping"
            );
            return Ok(());
        };

        let dedup_key = activated_dedup_key(&user_id, &workflow_id);
        let id = Uuid::new_v4().to_string();
        let anonymous_id = format!("server:{user_id}");
        let context = serde_json::json!({
            "workflow_id": workflow_id,
            "terminal_status": "completed",
        });
        // Use the spine row's timestamp so reporting reflects when the
        // run actually reached terminal state, not when the listener
        // happened to process it.
        let occurred_at = ext_row.created_at;
        let path = if self.is_oss { "oss" } else { "cloud" };

        let res = sqlx::query(
            "INSERT INTO activation_events \
                 (id, event, occurred_at, user_id, anonymous_id, surface, path, context, dedup_key) \
             VALUES ($1, 'ftue.activated', $2, $3, $4, 'spine', $5, $6, $7) \
             ON CONFLICT (dedup_key) DO NOTHING",
        )
        .bind(&id)
        .bind(occurred_at)
        .bind(&user_id)
        .bind(&anonymous_id)
        .bind(path)
        .bind(&context)
        .bind(&dedup_key)
        .execute(&*self.pool)
        .await?;

        if res.rows_affected() > 0 {
            tracing::info!(
                %workflow_id,
                %user_id,
                "ftue.activated: recorded"
            );
        }
        Ok(())
    }
}

#[async_trait]
impl EventHandler for WorkflowActivated {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "stage.advanced" {
            return Ok(());
        }
        if let Err(e) = self.handle_stage_advanced(&notification).await {
            tracing::warn!(
                error = %e,
                event_id = notification.id,
                "workflow_activated listener: handler error"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    fn _type_check_run_signature(pool: PgPool, store: EventStore) {
        std::mem::drop(run(pool, store, false));
    }
}
