//! Per-project poll scheduler. One tokio task per polling project,
//! ticking at the mode-derived interval.
//!
//! The scheduler is the load-bearing surface for the reconciliation
//! contract — it is what catches missed webhook deliveries. v1 wires
//! the load-cursor → poll → log-observation path end-to-end but
//! deliberately does NOT call `upsert_state` to advance the cursor:
//! the contract documented in `onsager-github::polling` is "cursor
//! advances only on successful emit", and the spine emit path is
//! deferred to the webhook-translator refactor (#121 follow-up).
//! Advancing the cursor before emit lands would permanently skip
//! reconciliation on the affected window once the emit slice ships.
//!
//! Boot scan happens once at startup; project-add / project-remove
//! lifecycle is a follow-up. Restart the portal to pick up new
//! projects in the v1 slice.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use sqlx::PgPool;
use tokio::time::{self, Instant};

use onsager_github::{Adapter, GitHubAdapter, PollOutcome};

use super::mode::IngestionMode;
use super::state::{load_state, touch_polled_at};

/// Project row shape the scheduler needs. A narrow projection over
/// `projects` — we don't want the full row coupling.
#[derive(Debug, Clone)]
struct ProjectRow {
    id: String,
    workspace_id: String,
    repo_owner: String,
    repo_name: String,
    ingestion_mode: String,
}

/// Boot-time entry point: query every project, classify by
/// ingestion mode, and spawn a poll task for each one that polls.
/// Returns immediately; the spawned tasks run for the lifetime of
/// the portal (no graceful shutdown wired today — the listener
/// pattern across portal uses the same posture).
pub fn spawn_all(pool: PgPool) {
    tokio::spawn(async move {
        let projects = match load_polling_projects(&pool).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "reconciliation scheduler: failed to enumerate projects on boot"
                );
                return;
            }
        };

        let polling_count = projects
            .iter()
            .filter(|p| IngestionMode::parse(&p.ingestion_mode).0.polls())
            .count();
        tracing::info!(
            total = projects.len(),
            polling = polling_count,
            "reconciliation scheduler: boot scan complete"
        );

        for project in projects {
            let (mode, unknown) = IngestionMode::parse(&project.ingestion_mode);
            if unknown {
                // Forward-rolled or typo'd ingestion_mode value:
                // we fall back to the default mode but warn once
                // per project so the misconfiguration surfaces in
                // logs instead of being silently absorbed.
                tracing::warn!(
                    project_id = %project.id,
                    ingestion_mode = %project.ingestion_mode,
                    fallback = mode.as_str(),
                    "reconciliation: unknown ingestion_mode; falling back to default"
                );
            }
            if !mode.polls() {
                continue;
            }
            let pool = pool.clone();
            tokio::spawn(async move {
                run_project_loop(pool, project, mode).await;
            });
        }
    });
}

async fn load_polling_projects(pool: &PgPool) -> Result<Vec<ProjectRow>, sqlx::Error> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT id, workspace_id, repo_owner, repo_name, ingestion_mode
        FROM projects
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, workspace_id, repo_owner, repo_name, ingestion_mode)| ProjectRow {
                id,
                workspace_id,
                repo_owner,
                repo_name,
                ingestion_mode,
            },
        )
        .collect())
}

/// Deterministic per-project offset within `[0, interval)`. Spreads
/// the boot-aligned tick storm across the interval window so the
/// scheduler doesn't periodically hammer GitHub + Postgres with
/// every project's poll firing at the same instant. Hash is
/// deterministic across restarts (same project_id → same offset)
/// which is what we want for diagnosis — a noisy tick keeps the
/// same wall-clock position.
fn project_offset(project_id: &str, interval: Duration) -> Duration {
    let mut hasher = DefaultHasher::new();
    project_id.hash(&mut hasher);
    let h = hasher.finish();
    let interval_nanos = (interval.as_nanos() as u64).max(1);
    Duration::from_nanos(h % interval_nanos)
}

async fn run_project_loop(pool: PgPool, project: ProjectRow, mode: IngestionMode) {
    let interval = mode.poll_interval();
    let offset = project_offset(&project.id, interval);
    // Start the ticker at `now + offset` so two projects with the
    // same mode don't fire on the same tokio timer tick. After the
    // first tick the interval handles subsequent firings; the
    // offset is paid once.
    let start = Instant::now() + offset;
    let mut ticker = time::interval_at(start, interval);

    tracing::info!(
        project_id = %project.id,
        repo = %format!("{}/{}", project.repo_owner, project.repo_name),
        mode = mode.as_str(),
        interval_secs = interval.as_secs(),
        offset_ms = offset.as_millis() as u64,
        "reconciliation: starting poll loop"
    );

    loop {
        ticker.tick().await;
        if let Err(e) = tick_project(&pool, &project).await {
            tracing::warn!(
                project_id = %project.id,
                error = %e,
                "reconciliation: tick failed; will retry on next interval"
            );
        }
    }
}

async fn tick_project(pool: &PgPool, project: &ProjectRow) -> anyhow::Result<()> {
    // v1: unauthenticated reads. The installation-token path will
    // land with the webhook-translator refactor (#121 follow-up)
    // so the poller and the webhook share the credential resolver.
    // For now an unauth'd poll works for public repos and surfaces
    // a 401 in logs for private ones, which is the local-dev
    // baseline the spec calls for.
    let adapter = GitHubAdapter::new(
        project.id.clone(),
        project.repo_owner.clone(),
        project.repo_name.clone(),
        None,
    );

    for kind in [GitHubAdapter::KIND_ISSUE, GitHubAdapter::KIND_PULL_REQUEST] {
        let state = load_state(pool, adapter.adapter_id(), &project.workspace_id, kind).await?;
        match adapter.poll_since(&state).await {
            Ok(PollOutcome { events, advance }) => {
                if !events.is_empty() {
                    // Spine emit lands with the webhook-translator
                    // refactor (#121 follow-up). For now we log so
                    // operators can validate the adapter sees the
                    // resources it should.
                    tracing::info!(
                        project_id = %project.id,
                        adapter_id = adapter.adapter_id(),
                        resource_kind = kind,
                        observed = events.len(),
                        proposed_advance = advance.is_some(),
                        "reconciliation: poll observed events (spine emit + cursor advance deferred)"
                    );
                }
                // IMPORTANT: do NOT call `upsert_state` here. The
                // "cursor advances only on successful emit" contract
                // (see `onsager-github::polling` module docs) means
                // we can't move the cursor past unemitted events —
                // doing so would permanently skip reconciliation on
                // the affected window once the emit path lands.
                // Stamp `last_polled_at` for liveness instead; the
                // cursor stays at `state.last_seen_*` until the
                // follow-up wires `upsert_state` after a successful
                // spine emit.
                touch_polled_at(pool, adapter.adapter_id(), &project.workspace_id, kind).await?;
            }
            Err(e) => {
                // Don't advance the cursor on error — the next tick
                // retries the same window. Stamp `last_polled_at`
                // so the poll-loop liveness signal is honest.
                tracing::warn!(
                    project_id = %project.id,
                    resource_kind = kind,
                    error = %e,
                    "reconciliation: poll_since failed"
                );
                touch_polled_at(pool, adapter.adapter_id(), &project.workspace_id, kind).await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_offset_is_deterministic_and_bounded() {
        let interval = Duration::from_secs(300);
        let a = project_offset("proj_abc", interval);
        let b = project_offset("proj_abc", interval);
        let c = project_offset("proj_xyz", interval);
        assert_eq!(a, b, "same project id must produce the same offset");
        assert_ne!(a, c, "different project ids should differ (collision rare)");
        assert!(a < interval);
        assert!(c < interval);
    }
}
