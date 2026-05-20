//! Per-project poll scheduler. One tokio task per polling project,
//! ticking at the mode-derived interval.
//!
//! The scheduler is the load-bearing surface for the reconciliation
//! contract — it is what catches missed webhook deliveries. v1 wires
//! the state-table round trip end-to-end (load cursor → poll →
//! advance cursor) but defers the actual spine emit to the webhook-
//! translator refactor (#121 follow-up): emitted events are logged
//! at `info` level so operators can verify the poller is doing
//! useful work before turning on the spine writer.
//!
//! Boot scan happens once at startup; project-add / project-remove
//! lifecycle is a follow-up. Restart the portal to pick up new
//! projects in the v1 slice.

use sqlx::PgPool;
use tokio::time;

use onsager_github::{Adapter, GitHubAdapter, PollOutcome};

use super::mode::IngestionMode;
use super::state::{load_state, touch_polled_at, upsert_state};

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
            .filter(|p| IngestionMode::parse(&p.ingestion_mode).polls())
            .count();
        tracing::info!(
            total = projects.len(),
            polling = polling_count,
            "reconciliation scheduler: boot scan complete"
        );

        for project in projects {
            let mode = IngestionMode::parse(&project.ingestion_mode);
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

async fn run_project_loop(pool: PgPool, project: ProjectRow, mode: IngestionMode) {
    let interval = mode.poll_interval();
    let mut ticker = time::interval(interval);
    // First tick fires immediately; skip it so we don't slam every
    // adapter on portal boot. The reconciler is a backstop — missing
    // the first interval is fine; missing the entire window is not.
    ticker.tick().await;

    tracing::info!(
        project_id = %project.id,
        repo = %format!("{}/{}", project.repo_owner, project.repo_name),
        mode = mode.as_str(),
        interval_secs = interval.as_secs(),
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
                    // operators can validate the cursor is moving
                    // and the adapter sees the resources it should.
                    tracing::info!(
                        project_id = %project.id,
                        adapter_id = adapter.adapter_id(),
                        resource_kind = kind,
                        observed = events.len(),
                        "reconciliation: poll observed events (spine emit deferred)"
                    );
                }
                if let Some(advance) = advance {
                    upsert_state(pool, &advance).await?;
                } else {
                    touch_polled_at(pool, adapter.adapter_id(), &project.workspace_id, kind)
                        .await?;
                }
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
