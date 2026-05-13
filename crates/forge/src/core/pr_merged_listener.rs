//! Spine listener for `git.pr_merged` → archive originating issue artifact
//! (spec #273, PR 2).
//!
//! When a PR is merged:
//! 1. Look up the `Kind::PullRequest` artifact from the event.
//! 2. Follow the `horizontal_lineage` back to the originating issue artifact.
//! 3. Archive the issue artifact (state → `archived`).
//! 4. Emit `artifact.archived` on the spine.
//!
//! If there is no lineage row (PR was not opened by the factory, or was
//! opened before spec #273 landed), archival is skipped silently.

use async_trait::async_trait;
use onsager_artifact::ArtifactId;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventMetadata, EventNotification, EventStore, Listener};

/// Run the pr-merged → archive listener forever. Returns only when the
/// pg_notify channel closes.
pub async fn run(store: EventStore, since: Option<i64>) -> anyhow::Result<()> {
    let handler = PrMergedHandler {
        store: store.clone(),
    };
    Listener::new(store).with_since(since).run(handler).await
}

struct PrMergedHandler {
    store: EventStore,
}

impl PrMergedHandler {
    async fn handle_pr_merged(&self, pr_artifact_id: &ArtifactId) -> anyhow::Result<()> {
        let pool = self.store.pool();

        // Find the issue artifact linked to this PR via horizontal_lineage.
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT hl.source_artifact_id, a.workspace_id \
               FROM horizontal_lineage hl \
               JOIN artifacts a ON a.artifact_id = hl.source_artifact_id \
              WHERE hl.artifact_id = $1 AND hl.role = 'closes_issue' \
              ORDER BY hl.id DESC LIMIT 1",
        )
        .bind(pr_artifact_id.as_str())
        .fetch_optional(pool)
        .await?;

        let Some((issue_artifact_id, workspace_id)) = row else {
            // PR was not opened by the factory (no lineage row); skip silently.
            return Ok(());
        };

        // Archive the issue artifact. The UPDATE returns the row only when
        // the state actually changed (not already archived), so we can use
        // the result to decide whether to emit the event.
        let archived: Option<(String,)> = sqlx::query_as(
            "UPDATE artifacts SET state = 'archived', updated_at = NOW() \
              WHERE artifact_id = $1 AND state != 'archived' \
          RETURNING artifact_id",
        )
        .bind(&issue_artifact_id)
        .fetch_optional(pool)
        .await?;

        let Some(_) = archived else {
            // Already archived or not found — no-op.
            return Ok(());
        };

        // Emit artifact.archived.
        let event = FactoryEventKind::ArtifactArchived {
            artifact_id: ArtifactId::new(issue_artifact_id.clone()),
            reason: "pr_merged".to_string(),
        };
        let metadata = EventMetadata {
            actor: "forge".into(),
            ..Default::default()
        };
        let stream_id = format!("forge:{issue_artifact_id}");
        if let Err(e) = self
            .store
            .append_ext(
                &workspace_id,
                &stream_id,
                "forge",
                "artifact.archived",
                serde_json::to_value(&event)?,
                &metadata,
                None,
            )
            .await
        {
            tracing::warn!(
                artifact_id = %issue_artifact_id,
                error = %e,
                "pr_merged_listener: failed to emit artifact.archived"
            );
        } else {
            tracing::info!(
                pr_artifact_id = %pr_artifact_id,
                issue_artifact_id = %issue_artifact_id,
                "pr_merged_listener: issue artifact archived"
            );
        }

        Ok(())
    }
}

#[async_trait]
impl EventHandler for PrMergedHandler {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "git.pr_merged" {
            return Ok(());
        }

        let kind = match notification.table.as_str() {
            "events" => {
                let Some(row) = self.store.get_event_by_id(notification.id).await? else {
                    return Ok(());
                };
                let envelope: FactoryEvent = serde_json::from_value(row.data)?;
                envelope.event
            }
            "events_ext" => {
                let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
                    return Ok(());
                };
                serde_json::from_value::<FactoryEventKind>(row.data)?
            }
            _ => return Ok(()),
        };

        let FactoryEventKind::GitPrMerged { artifact_id, .. } = kind else {
            return Ok(());
        };

        if let Err(e) = self.handle_pr_merged(&artifact_id).await {
            tracing::warn!(
                pr_artifact_id = %artifact_id,
                error = %e,
                "pr_merged_listener: handler error"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_spine::factory_event::FactoryEventKind;

    #[test]
    fn event_type_filter_literal_present() {
        // Smoke-test: the event type string used in the listener matches the
        // check-events lint pattern. The guard `if notification.event_type !=
        // "git.pr_merged"` is what check-events scans for.
        let expected = "git.pr_merged";
        let kind = FactoryEventKind::GitPrMerged {
            artifact_id: ArtifactId::new("art_test"),
            pr_number: 1,
            merge_sha: "abc".into(),
        };
        assert_eq!(kind.event_type(), expected);
    }
}
