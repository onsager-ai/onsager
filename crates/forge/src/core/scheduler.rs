//! Schedule-trigger producer (#238).
//!
//! Ticks every 5s, finds active workflows whose trigger kind is `cron` /
//! `delay` / `interval`, computes the next fire-at, and emits
//! `FactoryEventKind::TriggerFired` to the spine when due. The
//! [`crate::core::trigger_subscriber`] consumes those events, registers
//! an artifact, and enters stage 0 — same path as webhook-driven fires.
//!
//! ## Reliability
//!
//! The scheduler shares forge's process; a panic must not take down forge.
//! [`run`] wraps the tick loop in a supervisor that catches panics and
//! restarts with exponential backoff.
//!
//! ## Catch-up policy
//!
//! If forge is down for N minutes, missed firings are **skipped**: the
//! scheduler fires once and stamps `last_fired_at = now()`. Replay-of-
//! missed-firings is a per-workflow follow-up; v1 keeps the failure surface
//! small.
//!
//! ## Idempotency
//!
//! Each `TriggerFired` carries `payload.last_fired_at` and
//! `payload.scheduled_for`. Forge's trigger subscriber dedupes on the
//! resulting external_ref `forge:trigger:{workflow_id}:{kind}:{ts}`.
//!
//! ## State persistence
//!
//! `last_fired_at` lives in the spine sidecar table
//! `workflow_trigger_state` (migration 018) — keeps the `workflows` row
//! kind-agnostic.

use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use sqlx::PgPool;

use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{DelayAnchor, EventMetadata, EventStore, TriggerKind};

/// Fixed tick interval (per #238 resolution: 5s, not per-deploy
/// configurable in v1 — keeps the failure surface small).
pub const TICK_INTERVAL: Duration = Duration::from_secs(5);

/// Run the schedule-trigger producer. Loops forever; on a panic in the
/// inner tick body, restarts with exponential backoff (capped at 30s).
pub async fn run(store: EventStore) -> anyhow::Result<()> {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        let store_clone = store.clone();
        let result = tokio::spawn(async move { run_tick_loop(store_clone).await }).await;

        match result {
            Ok(Ok(())) => {
                tracing::info!("forge scheduler: tick loop exited cleanly");
                return Ok(());
            }
            Ok(Err(e)) => {
                tracing::error!("forge scheduler: tick loop returned error: {e:#}");
            }
            Err(join_err) if join_err.is_panic() => {
                tracing::error!("forge scheduler: tick loop panicked: {join_err}");
            }
            Err(join_err) => {
                tracing::error!("forge scheduler: tick loop join error: {join_err}");
                return Err(anyhow::anyhow!("scheduler join error: {join_err}"));
            }
        }

        tracing::warn!("forge scheduler: restarting tick loop after {:?}", backoff);
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

async fn run_tick_loop(store: EventStore) -> anyhow::Result<()> {
    let mut interval = tokio::time::interval(TICK_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(e) = run_one_tick(&store).await {
            tracing::warn!("forge scheduler: tick failed: {e:#}");
        }
    }
}

/// Run one scheduling tick. Public for tests; production callers use
/// [`run`].
pub async fn run_one_tick(store: &EventStore) -> anyhow::Result<()> {
    let now = Utc::now();
    let pool = store.pool();
    let candidates = load_schedule_candidates(pool).await?;
    for candidate in candidates {
        if let Err(e) = process_candidate(store, &candidate, now).await {
            tracing::warn!(
                workflow_id = %candidate.workflow_id,
                trigger_kind = %candidate.trigger.kind_tag(),
                "forge scheduler: candidate fire failed: {e:#}"
            );
        }
    }
    Ok(())
}

/// One schedule-triggered workflow, hydrated from the spine.
#[derive(Debug, Clone)]
pub struct ScheduleCandidate {
    pub workflow_id: String,
    pub workspace_id: String,
    pub workflow_created_at: DateTime<Utc>,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub trigger: TriggerKind,
}

async fn load_schedule_candidates(pool: &PgPool) -> anyhow::Result<Vec<ScheduleCandidate>> {
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT w.workflow_id, w.workspace_id, w.created_at, w.trigger_kind, \
                w.trigger_config, s.last_fired_at \
           FROM workflows w \
           LEFT JOIN workflow_trigger_state s USING (workflow_id) \
          WHERE w.active = TRUE \
            AND w.trigger_kind IN ('cron', 'delay', 'interval')",
    )
    .fetch_all(pool)
    .await
    .context("loading schedule candidates")?;

    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        let workflow_id: String = row.try_get("workflow_id")?;
        let workspace_id: String = row.try_get("workspace_id")?;
        let workflow_created_at: DateTime<Utc> = row.try_get("created_at")?;
        let kind_tag: String = row.try_get("trigger_kind")?;
        let cfg: serde_json::Value = row.try_get("trigger_config")?;
        // Use a typed `Option<DateTime<Utc>>` decode and `?` so a real DB
        // schema/type error surfaces instead of silently being treated as
        // "never fired" (which would re-fire workflows that did fire).
        let last_fired_at: Option<DateTime<Utc>> = row.try_get("last_fired_at")?;
        let trigger = match TriggerKind::from_storage(&kind_tag, &cfg) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    workflow_id = %workflow_id,
                    "forge scheduler: skipping unparseable trigger: {e}"
                );
                continue;
            }
        };
        candidates.push(ScheduleCandidate {
            workflow_id,
            workspace_id,
            workflow_created_at,
            last_fired_at,
            trigger,
        });
    }
    Ok(candidates)
}

async fn process_candidate(
    store: &EventStore,
    candidate: &ScheduleCandidate,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let Some(scheduled_for) = next_fire_at(
        &candidate.trigger,
        candidate.workflow_created_at,
        candidate.last_fired_at,
        now,
    )?
    else {
        // One-shot delay that has already fired; nothing to do.
        return Ok(());
    };

    if scheduled_for > now {
        // Not yet due.
        return Ok(());
    }

    // Persist `last_fired_at` *before* emitting `trigger.fired` so a crash
    // between the two steps drops this fire (skip-missed semantics already
    // permits losing one fire) instead of duplicating it on restart. The
    // alternative ordering (emit then persist) lets a crash mid-step
    // re-fire on the next tick because `last_fired_at` is still stale —
    // the failure mode Copilot flagged on PR #247.
    upsert_state(store.pool(), &candidate.workflow_id, now, scheduled_for).await?;
    emit_trigger_fired(store, candidate, scheduled_for, now).await?;
    Ok(())
}

/// Compute the next fire-at instant for a schedule trigger.
///
/// - `Cron` — next time the cron schedule fires after `last_fired_at`
///   (or after `now` if never fired). Catch-up policy: if multiple
///   firings have been missed, return the latest missed firing — caller
///   stamps `last_fired_at = now` to skip the rest.
/// - `Delay` — `workflow_created_at + seconds`, but only when
///   `last_fired_at` is `None` (one-shot). Returns `Ok(None)` after
///   the one fire.
/// - `Interval` — `last_fired_at + period` (or `now` if never fired).
///   Same skip-missed semantics as cron.
pub fn next_fire_at(
    trigger: &TriggerKind,
    workflow_created_at: DateTime<Utc>,
    last_fired_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<DateTime<Utc>>> {
    match trigger {
        TriggerKind::Cron {
            expression,
            timezone,
        } => {
            let schedule = Schedule::from_str(expression)
                .with_context(|| format!("invalid cron expression `{expression}`"))?;
            let after = last_fired_at.unwrap_or(workflow_created_at);
            let next = match timezone.as_deref() {
                Some(tz_name) => {
                    let tz: Tz = tz_name
                        .parse()
                        .with_context(|| format!("invalid timezone `{tz_name}`"))?;
                    let after_tz = after.with_timezone(&tz);
                    schedule
                        .after(&after_tz)
                        .next()
                        .map(|dt| dt.with_timezone(&Utc))
                }
                None => schedule.after(&after).next(),
            };
            Ok(next)
        }
        TriggerKind::Delay { seconds, anchor } => {
            // Anchor is `WorkflowActivatedAt` for v1 — measured from the
            // workflow's creation timestamp. After one fire, never again.
            if last_fired_at.is_some() {
                return Ok(None);
            }
            let DelayAnchor::WorkflowActivatedAt = anchor;
            let due = workflow_created_at
                + chrono::Duration::seconds(i64_from_u64(*seconds, "delay seconds")?);
            Ok(Some(due))
        }
        TriggerKind::Interval { period_seconds } => {
            let period =
                chrono::Duration::seconds(i64_from_u64(*period_seconds, "interval period")?);
            let baseline = last_fired_at.unwrap_or(workflow_created_at);
            let mut next = baseline + period;
            // Skip-missed: advance until next > now - period, so we fire
            // once at the latest scheduled instant <= now.
            while next + period <= now {
                next += period;
            }
            Ok(Some(next))
        }
        other => Err(anyhow::anyhow!(
            "next_fire_at called on non-schedule trigger: {}",
            other.kind_tag()
        )),
    }
}

fn i64_from_u64(v: u64, what: &str) -> anyhow::Result<i64> {
    i64::try_from(v).map_err(|_| anyhow::anyhow!("{what} {v} overflows i64 seconds"))
}

async fn emit_trigger_fired(
    store: &EventStore,
    candidate: &ScheduleCandidate,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "trigger_kind": candidate.trigger.kind_tag(),
        "workflow_id": candidate.workflow_id,
        "workspace_id": candidate.workspace_id,
        "scheduled_for": scheduled_for,
        "fired_at": now,
        "last_fired_at": candidate.last_fired_at,
        "source": "forge_scheduler",
    });

    let envelope = FactoryEvent {
        event: FactoryEventKind::TriggerFired {
            workflow_id: candidate.workflow_id.clone(),
            trigger_kind: candidate.trigger.kind_tag().to_string(),
            payload,
        },
        correlation_id: None,
        causation_id: None,
        actor: "forge_scheduler".to_string(),
        timestamp: now,
    };
    let data = serde_json::to_value(&envelope)?;
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: "forge_scheduler".to_string(),
    };
    store
        .append_ext(
            &candidate.workspace_id,
            &format!("workflow:{}", candidate.workflow_id),
            "workflow",
            "trigger.fired",
            data,
            &metadata,
            None,
        )
        .await
        .context("emitting trigger.fired")?;
    Ok(())
}

async fn upsert_state(
    pool: &PgPool,
    workflow_id: &str,
    now: DateTime<Utc>,
    scheduled_for: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO workflow_trigger_state (workflow_id, last_fired_at, last_payload) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (workflow_id) DO UPDATE \
            SET last_fired_at = EXCLUDED.last_fired_at, \
                last_payload  = EXCLUDED.last_payload",
    )
    .bind(workflow_id)
    .bind(now)
    .bind(serde_json::json!({ "scheduled_for": scheduled_for }))
    .execute(pool)
    .await
    .context("upserting workflow_trigger_state")?;
    Ok(())
}

// Trick to silence unused-import warnings when chrono::TimeZone isn't
// needed by every cfg. The cron schedule.after on a chrono-tz Tz still
// requires the trait in scope.
#[allow(dead_code)]
fn _force_tz_in_scope<Tz2: TimeZone>(_t: &Tz2) {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn delay_fires_once_then_returns_none() {
        let trigger = TriggerKind::Delay {
            seconds: 30,
            anchor: DelayAnchor::WorkflowActivatedAt,
        };
        let created = ts("2026-05-05T00:00:00Z");
        // Never fired → next is created + 30s.
        let now = ts("2026-05-05T00:01:00Z");
        let next = next_fire_at(&trigger, created, None, now).unwrap();
        assert_eq!(next, Some(ts("2026-05-05T00:00:30Z")));

        // Already fired → no further fires.
        let next2 = next_fire_at(&trigger, created, next, now).unwrap();
        assert_eq!(next2, None);
    }

    #[test]
    fn interval_fires_at_period_boundary() {
        let trigger = TriggerKind::Interval { period_seconds: 60 };
        let created = ts("2026-05-05T00:00:00Z");
        let now = ts("2026-05-05T00:00:30Z");
        let next = next_fire_at(&trigger, created, None, now).unwrap();
        assert_eq!(next, Some(ts("2026-05-05T00:01:00Z")));
    }

    #[test]
    fn interval_skips_missed_firings_and_lands_on_latest_missed() {
        // Workflow created 10 minutes ago, 1-minute interval, never fired.
        // Expected: next fire is the most recent boundary ≤ now, not all
        // 10 missed boundaries.
        let trigger = TriggerKind::Interval { period_seconds: 60 };
        let created = ts("2026-05-05T00:00:00Z");
        let now = ts("2026-05-05T00:10:30Z");
        let next = next_fire_at(&trigger, created, None, now).unwrap().unwrap();
        // Latest boundary <= now is 00:10:00.
        assert_eq!(next, ts("2026-05-05T00:10:00Z"));
        // And next > now - period, i.e. within the most recent window.
        assert!(next + chrono::Duration::seconds(60) > now);
    }

    #[test]
    fn cron_fires_at_next_match() {
        // Every minute on the hour boundary.
        let trigger = TriggerKind::Cron {
            expression: "0 * * * * *".into(),
            timezone: None,
        };
        let created = ts("2026-05-05T00:00:30Z");
        let now = ts("2026-05-05T00:00:45Z");
        let next = next_fire_at(&trigger, created, None, now).unwrap().unwrap();
        assert_eq!(next, ts("2026-05-05T00:01:00Z"));
    }

    #[test]
    fn cron_with_named_timezone_does_not_panic() {
        let trigger = TriggerKind::Cron {
            expression: "0 0 9 * * *".into(),
            timezone: Some("America/Los_Angeles".into()),
        };
        let created = Utc.with_ymd_and_hms(2026, 5, 5, 0, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 5, 5, 12, 0, 0).unwrap();
        let next = next_fire_at(&trigger, created, None, now).unwrap();
        assert!(next.is_some());
    }

    #[test]
    fn cron_invalid_expression_returns_error() {
        let trigger = TriggerKind::Cron {
            expression: "not a cron".into(),
            timezone: None,
        };
        let created = ts("2026-05-05T00:00:00Z");
        let err = next_fire_at(&trigger, created, None, created).unwrap_err();
        assert!(err.to_string().contains("invalid cron"));
    }

    #[test]
    fn cron_invalid_timezone_returns_error() {
        let trigger = TriggerKind::Cron {
            expression: "0 * * * * *".into(),
            timezone: Some("Earth/Atlantis".into()),
        };
        let created = ts("2026-05-05T00:00:00Z");
        let err = next_fire_at(&trigger, created, None, created).unwrap_err();
        assert!(err.to_string().contains("invalid timezone"));
    }

    #[test]
    fn next_fire_at_rejects_non_schedule_trigger() {
        let trigger = TriggerKind::GithubIssueWebhook {
            repo: "a/b".into(),
            label: "x".into(),
        };
        let created = ts("2026-05-05T00:00:00Z");
        assert!(next_fire_at(&trigger, created, None, created).is_err());
    }
}
