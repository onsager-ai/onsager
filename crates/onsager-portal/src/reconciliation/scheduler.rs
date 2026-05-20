//! Per-project poll scheduler. One tokio task per polling project,
//! ticking at the mode-derived interval.
//!
//! The scheduler is the load-bearing surface for the reconciliation
//! contract — it is what catches missed webhook deliveries. Each
//! tick:
//!
//!   1. Loads the cursor for `(adapter, workspace, resource_kind)`.
//!   2. Calls `Adapter::poll_since` and gets a `Vec<NormalizedEvent>`
//!      plus a proposed cursor advance.
//!   3. Routes every event through the shared
//!      [`super::translator`] (the same one the webhook handler calls
//!      after parsing) and emits the resulting `RoutedEvent`s via
//!      [`super::emit::emit_routed_events`]. `(adapter_id,
//!      external_ref)` dedup on `events_ext` collapses any race with
//!      a sibling webhook delivery to a silent no-op.
//!   4. Advances the cursor only on successful emit — the
//!      "advance only on emit" contract from
//!      `onsager-github::polling` means a failure leaves the cursor
//!      where it was so the next tick retries the same window.
//!
//! Boot scan happens once at startup; project-add / project-remove
//! lifecycle is a follow-up. Restart the portal to pick up new
//! projects in the v1 slice.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use onsager_github::api::issues::Issue;
use onsager_github::api::pulls::Pull;
use onsager_github::{Adapter, GitHubAdapter, NormalizedEvent, PollOutcome};
use onsager_spine::{EventStore, RoutedEvent, WorkflowMatch, WorkflowTrigger};
use sqlx::PgPool;
use tokio::time::{self, Instant};

use super::emit::emit_routed_events;
use super::mode::IngestionMode;
use super::state::{load_state, touch_polled_at, upsert_state};
use super::translator::{translate_issue, translate_pull_request};

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
pub fn spawn_all(pool: PgPool, spine: EventStore) {
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
            let spine = spine.clone();
            tokio::spawn(async move {
                run_project_loop(pool, spine, project, mode).await;
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

async fn run_project_loop(
    pool: PgPool,
    spine: EventStore,
    project: ProjectRow,
    mode: IngestionMode,
) {
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
        if let Err(e) = tick_project(&pool, &spine, &project).await {
            tracing::warn!(
                project_id = %project.id,
                error = %e,
                "reconciliation: tick failed; will retry on next interval"
            );
        }
    }
}

async fn tick_project(
    pool: &PgPool,
    spine: &EventStore,
    project: &ProjectRow,
) -> anyhow::Result<()> {
    // v1: unauthenticated reads. Per-installation credential
    // resolution is a deferred follow-up listed on spec #430 (open
    // questions / Alignment). For now an unauth'd poll works for
    // public repos and surfaces a 401 in logs for private ones,
    // which is the local-dev baseline the spec calls for.
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
                let observed = events.len();
                let emit_ok = emit_normalized(pool, spine, project, kind, &events).await;
                // Advance the cursor ONLY when emit succeeded. The
                // "cursor advances only on successful emit" contract
                // (see `onsager-github::polling`) means a failure
                // leaves the cursor where it was so the next tick
                // retries the same window. Dedup at the spine layer
                // turns the retry of an already-emitted event into a
                // silent no-op.
                if emit_ok && let Some(advanced) = advance.as_ref() {
                    upsert_state(pool, advanced).await?;
                    tracing::info!(
                        project_id = %project.id,
                        adapter_id = adapter.adapter_id(),
                        resource_kind = kind,
                        observed,
                        "reconciliation: poll emitted + cursor advanced"
                    );
                } else {
                    // Stamp `last_polled_at` so the liveness signal
                    // is honest even when there were no events to
                    // emit, or when the emit pipeline failed and we
                    // intentionally did not advance.
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

/// Translate a batch of [`NormalizedEvent`]s for one resource kind
/// into [`RoutedEvent`]s via the shared translator and emit them to
/// the spine. Returns `true` when every event in the batch was
/// translated and emit was attempted without an error; the caller
/// uses this signal to decide whether to advance the cursor. An
/// empty batch is a trivial success.
async fn emit_normalized(
    pool: &PgPool,
    spine: &EventStore,
    project: &ProjectRow,
    resource_kind: &str,
    events: &[NormalizedEvent],
) -> bool {
    if events.is_empty() {
        return true;
    }
    let mut routed: Vec<RoutedEvent> = Vec::new();

    for ev in events {
        match resource_kind {
            GitHubAdapter::KIND_ISSUE => {
                match serde_json::from_value::<Issue>(ev.payload.clone()) {
                    Ok(issue) => {
                        let by_label = match collect_label_workflows(pool, project, &issue).await {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::warn!(
                                    project_id = %project.id,
                                    issue_number = issue.number,
                                    error = %e,
                                    "reconciliation: failed to load label workflows"
                                );
                                return false;
                            }
                        };
                        if by_label.is_empty() {
                            continue;
                        }
                        routed.extend(translate_issue(
                            &issue,
                            &project.repo_owner,
                            &project.repo_name,
                            Some(&project.id),
                            &by_label,
                        ));
                    }
                    Err(e) => {
                        tracing::warn!(
                            project_id = %project.id,
                            external_ref = %ev.external_ref,
                            error = %e,
                            "reconciliation: failed to parse Issue payload"
                        );
                        return false;
                    }
                }
            }
            GitHubAdapter::KIND_PULL_REQUEST => {
                match serde_json::from_value::<Pull>(ev.payload.clone()) {
                    Ok(pull) => {
                        let candidates =
                            match crate::workflow_db::find_active_pull_request_closed_workflows(
                                pool,
                                &project.repo_owner,
                                &project.repo_name,
                            )
                            .await
                            {
                                Ok(c) => c,
                                Err(e) => {
                                    tracing::warn!(
                                        project_id = %project.id,
                                        pr_number = pull.number,
                                        error = %e,
                                        "reconciliation: failed to load PR workflows"
                                    );
                                    return false;
                                }
                            };
                        let triggers: Vec<WorkflowTrigger> = candidates
                            .into_iter()
                            .map(|w| WorkflowTrigger {
                                id: w.id,
                                workspace_id: w.workspace_id,
                                trigger: w.trigger,
                            })
                            .collect();
                        routed.extend(translate_pull_request(
                            &pull,
                            &project.repo_owner,
                            &project.repo_name,
                            Some(&project.id),
                            &triggers,
                        ));
                    }
                    Err(e) => {
                        tracing::warn!(
                            project_id = %project.id,
                            external_ref = %ev.external_ref,
                            error = %e,
                            "reconciliation: failed to parse Pull payload"
                        );
                        return false;
                    }
                }
            }
            other => {
                tracing::warn!(
                    project_id = %project.id,
                    resource_kind = other,
                    "reconciliation: unsupported resource_kind"
                );
            }
        }
    }

    emit_routed_events(spine, routed, &project.workspace_id, "portal-reconciler").await;
    true
}

/// Look up the workflows that match each of `issue`'s labels. The
/// poller has no `labeled` action context (only the current shape of
/// the issue), so we query per-label. Empty map → nothing to fire.
async fn collect_label_workflows(
    pool: &PgPool,
    project: &ProjectRow,
    issue: &Issue,
) -> anyhow::Result<HashMap<String, Vec<WorkflowMatch>>> {
    let mut by_label: HashMap<String, Vec<WorkflowMatch>> = HashMap::new();
    // The poller polls unauthenticated in v1, so we have no
    // install_id to scope by. Use the workspace-scoped query so a
    // matching workflow on this project's workspace is found
    // regardless of install_id — when the credential follow-up
    // lands, scope tightens to (install_id, repo, label).
    for label in &issue.labels {
        let workflows = crate::workflow_db::find_active_github_workflows_for_label_in_workspace(
            pool,
            &project.workspace_id,
            &project.repo_owner,
            &project.repo_name,
            &label.name,
        )
        .await?;
        if workflows.is_empty() {
            continue;
        }
        let matches: Vec<WorkflowMatch> = workflows
            .into_iter()
            .map(|w| WorkflowMatch {
                id: w.id,
                workspace_id: w.workspace_id,
                trigger_kind_tag: w.trigger.kind_tag().to_string(),
            })
            .collect();
        by_label.insert(label.name.clone(), matches);
    }
    Ok(by_label)
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
