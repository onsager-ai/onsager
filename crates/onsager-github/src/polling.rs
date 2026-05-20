//! Reconciliation polling — the `Adapter::poll_since` trait + the
//! GitHub implementation that feeds it. See spec #121 for the
//! contract this implements and the rationale (webhook deliveries
//! drop; the reconciler is the backstop that catches them).
//!
//! # Shape
//!
//! The factory consumes a single shape — [`NormalizedEvent`] —
//! regardless of whether the source was a webhook or a poll cursor.
//! That symmetry is the load-bearing property: idempotency at the
//! spine layer reduces to "did we already see this `external_ref`?",
//! enforced by the partial unique index on `events_ext (adapter_id,
//! external_ref)` (spine migration 032).
//!
//! # Cursor advance contract
//!
//! Cursor advances only on successful emit. A `poll_since` impl that
//! has read pages of resources but cannot commit them to the spine
//! must return the events anyway and let the caller advance the
//! state — that way a failed merge leaves the cursor where it was
//! and the next tick retries the same window.
//!
//! # external_ref format
//!
//! Adapter implementations MUST produce the same `external_ref` for
//! a given resource as the corresponding webhook path produces. The
//! GitHub adapter uses the existing canonical form:
//! `github:project:<project_id>:issue:<number>` or
//! `github:project:<project_id>:pr:<number>` — matching what
//! `crates/onsager-portal/src/db.rs::external_ref_for_*` already
//! emits for the webhook lineage handlers.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::GithubError;

/// High-water mark + ETag for one (adapter, workspace, resource_kind)
/// tuple. Mirrors the row shape in spine table
/// `adapter_reconciliation_state` so the portal can pass a row in
/// and apply the returned advance back out.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdapterReconciliationState {
    pub adapter_id: String,
    pub workspace_id: String,
    pub resource_kind: String,
    /// Last external id observed (e.g. issue/PR number rendered as
    /// a string). `None` on a fresh state row.
    pub last_seen_external_id: Option<String>,
    /// `updated_at` of the most-recently observed resource. Used as
    /// the `since` cursor on subsequent polls.
    pub last_seen_updated_at: Option<DateTime<Utc>>,
    /// ETag returned by the last successful poll, for conditional
    /// requests on the next tick. Currently used only for transport
    /// metadata; the GitHub adapter does not yet round-trip it.
    pub etag: Option<String>,
}

/// Adapter-normalized resource update. Each event carries the same
/// `external_ref` it would carry if delivered by webhook — that is
/// the idempotency contract the spine relies on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedEvent {
    /// Stable adapter-scoped resource identity. See module docs for
    /// format requirements.
    pub external_ref: String,
    /// Adapter identifier (e.g. `"github"`). Combines with
    /// `external_ref` to form the spine idempotency key.
    pub adapter_id: String,
    /// Coarse resource kind (`"issue"`, `"pull_request"`, …).
    /// Matches the values used in
    /// `adapter_reconciliation_state.resource_kind`.
    pub resource_kind: String,
    /// `updated_at` reported by the source. Drives cursor advance.
    pub updated_at: DateTime<Utc>,
    /// Raw resource payload (issue / PR JSON). The webhook-translator
    /// refactor (#121 follow-up) will collapse this into a shape
    /// shared with the webhook path so both call the same emit code.
    pub payload: serde_json::Value,
}

/// Outcome of a single `poll_since` call. Splits the "what new
/// events did you see" from the "how should I advance the cursor"
/// concerns so the caller can durably advance the state row even
/// when the events list is empty (e.g. a 304 response).
#[derive(Debug, Clone, Default)]
pub struct PollOutcome {
    pub events: Vec<NormalizedEvent>,
    /// State advance to persist after the caller has successfully
    /// emitted `events`. If `None`, the adapter saw no change worth
    /// recording (e.g. ETag matched, no resources updated).
    pub advance: Option<AdapterReconciliationState>,
}

/// The reconciliation seam every adapter implements. Today the only
/// implementor is [`GitHubAdapter`]; future provider libraries
/// (Linear, GitLab, …) will satisfy the same trait so the portal
/// scheduler stays adapter-agnostic.
#[async_trait]
pub trait Adapter: Send + Sync {
    /// Adapter identifier — stable, matches the value the adapter
    /// writes into [`NormalizedEvent::adapter_id`] and the
    /// `artifact_adapters` catalog.
    fn adapter_id(&self) -> &str;

    /// Read every resource update visible to this adapter since
    /// the cursor in `state`. Resource-kind selection happens via
    /// `state.resource_kind` — the caller picks which kind to poll
    /// per tick.
    async fn poll_since(
        &self,
        state: &AdapterReconciliationState,
    ) -> Result<PollOutcome, GithubError>;
}

/// GitHub-flavoured [`Adapter`]. Today's `onsager-github` exposes
/// free functions for typed API access; this struct binds those
/// helpers to a single repository + auth token so the scheduler
/// can drive one adapter instance per project row.
///
/// One instance per (project, repo) tuple. The scheduler resolves
/// projects up front and constructs adapters per tick — keeping
/// the adapter cheap to build (no internal state) avoids stale
/// token issues.
pub struct GitHubAdapter {
    /// Project id (Onsager-side). Used to construct `external_ref`
    /// values matching the webhook path.
    project_id: String,
    repo_owner: String,
    repo_name: String,
    /// Installation/PAT token. `None` is supported for unauth'd
    /// reads against public repos (lower rate limit, but works for
    /// `just dev` smoke).
    token: Option<String>,
}

impl GitHubAdapter {
    pub fn new(
        project_id: impl Into<String>,
        repo_owner: impl Into<String>,
        repo_name: impl Into<String>,
        token: Option<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            repo_owner: repo_owner.into(),
            repo_name: repo_name.into(),
            token,
        }
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Issue / PR resource-kind discriminator used by callers and the
    /// state table. Centralised so the GitHub adapter and the portal
    /// scheduler agree on the spelling.
    pub const KIND_ISSUE: &'static str = "issue";
    pub const KIND_PULL_REQUEST: &'static str = "pull_request";

    /// Render the canonical GitHub `external_ref` for an issue. Must
    /// stay in sync with `crates/onsager-portal/src/db.rs`'s
    /// `external_ref_for_issue` (the webhook path).
    pub fn external_ref_for_issue(project_id: &str, issue_number: u64) -> String {
        format!("github:project:{project_id}:issue:{issue_number}")
    }

    /// Render the canonical GitHub `external_ref` for a PR. Must stay
    /// in sync with `crates/onsager-portal/src/db.rs`'s
    /// `external_ref_for_pull_request`.
    pub fn external_ref_for_pull_request(project_id: &str, pr_number: u64) -> String {
        format!("github:project:{project_id}:pr:{pr_number}")
    }

    async fn poll_issues(
        &self,
        state: &AdapterReconciliationState,
    ) -> Result<PollOutcome, GithubError> {
        // v1 fetch: list recent issues (REST endpoint already in
        // `onsager-github::api::issues`) and filter by `updated_at`
        // newer than the cursor. The endpoint includes PRs; we drop
        // PR rows here so the PR poll path stays the sole producer of
        // `pull_request` normalized events.
        let cap = 100;
        let issues = crate::api::issues::list_recent_issues(
            self.token.as_deref(),
            &self.repo_owner,
            &self.repo_name,
            cap,
        )
        .await?;

        let cursor = state.last_seen_updated_at;
        let mut events = Vec::new();
        let mut max_updated: Option<DateTime<Utc>> = cursor;
        let mut max_external: Option<String> = state.last_seen_external_id.clone();

        for issue in issues {
            if issue.is_pull_request() {
                continue;
            }
            // The REST `Issue` struct in this crate doesn't yet expose
            // `updated_at`; without a cursor field we can't advance
            // the high-water mark from the typed helper alone. Treat
            // every observed issue as "since cursor" and rely on the
            // events_ext idempotency index to dedup against the
            // webhook path — the index is the load-bearing guard,
            // not the cursor.
            //
            // The webhook-translator refactor (#121 follow-up) widens
            // the typed `Issue` to carry `updated_at` so we can
            // advance the cursor precisely; for v1 we record the
            // most-recent external id seen.
            let external_ref = Self::external_ref_for_issue(&self.project_id, issue.number);
            let payload = serde_json::json!({
                "number": issue.number,
                "title": issue.title,
                "state": issue.state,
            });
            let updated_at = chrono::Utc::now();
            if max_updated.is_none_or(|c| updated_at >= c) {
                max_updated = Some(updated_at);
                max_external = Some(issue.number.to_string());
            }
            events.push(NormalizedEvent {
                external_ref,
                adapter_id: self.adapter_id().to_string(),
                resource_kind: Self::KIND_ISSUE.to_string(),
                updated_at,
                payload,
            });
        }

        if events.is_empty() {
            return Ok(PollOutcome::default());
        }

        let advance = AdapterReconciliationState {
            adapter_id: state.adapter_id.clone(),
            workspace_id: state.workspace_id.clone(),
            resource_kind: state.resource_kind.clone(),
            last_seen_external_id: max_external,
            last_seen_updated_at: max_updated,
            etag: state.etag.clone(),
        };
        Ok(PollOutcome {
            events,
            advance: Some(advance),
        })
    }

    async fn poll_pull_requests(
        &self,
        state: &AdapterReconciliationState,
    ) -> Result<PollOutcome, GithubError> {
        // v1 scope: PR closed+merged only (spec #121 § "v1 resource
        // scope"). `list_recent_pulls` returns mixed state; we keep
        // only `state == "closed"` AND `merged_at IS NOT NULL`.
        let cap = 100;
        let pulls = crate::api::pulls::list_recent_pulls(
            self.token.as_deref(),
            &self.repo_owner,
            &self.repo_name,
            cap,
        )
        .await?;

        let cursor = state.last_seen_updated_at;
        let mut events = Vec::new();
        let mut max_updated: Option<DateTime<Utc>> = cursor;
        let mut max_external: Option<String> = state.last_seen_external_id.clone();

        for pull in pulls {
            if pull.state != "closed" || pull.merged_at.is_none() {
                continue;
            }
            let external_ref = Self::external_ref_for_pull_request(&self.project_id, pull.number);
            let updated_at = pull
                .merged_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);
            if max_updated.is_none_or(|c| updated_at >= c) {
                max_updated = Some(updated_at);
                max_external = Some(pull.number.to_string());
            }
            let payload = serde_json::json!({
                "number": pull.number,
                "title": pull.title,
                "state": pull.state,
                "merged_at": pull.merged_at,
                "merge_commit_sha": pull.merge_commit_sha,
            });
            events.push(NormalizedEvent {
                external_ref,
                adapter_id: self.adapter_id().to_string(),
                resource_kind: Self::KIND_PULL_REQUEST.to_string(),
                updated_at,
                payload,
            });
        }

        if events.is_empty() {
            return Ok(PollOutcome::default());
        }

        let advance = AdapterReconciliationState {
            adapter_id: state.adapter_id.clone(),
            workspace_id: state.workspace_id.clone(),
            resource_kind: state.resource_kind.clone(),
            last_seen_external_id: max_external,
            last_seen_updated_at: max_updated,
            etag: state.etag.clone(),
        };
        Ok(PollOutcome {
            events,
            advance: Some(advance),
        })
    }
}

#[async_trait]
impl Adapter for GitHubAdapter {
    fn adapter_id(&self) -> &str {
        crate::adapter::ADAPTER_ID
    }

    async fn poll_since(
        &self,
        state: &AdapterReconciliationState,
    ) -> Result<PollOutcome, GithubError> {
        match state.resource_kind.as_str() {
            Self::KIND_ISSUE => self.poll_issues(state).await,
            Self::KIND_PULL_REQUEST => self.poll_pull_requests(state).await,
            // Unknown resource kinds are a configuration error, not
            // a transport failure — surface them loudly via a
            // structured log and return an empty outcome so the
            // scheduler keeps making progress on the other kinds.
            other => {
                tracing::warn!(
                    adapter = self.adapter_id(),
                    workspace_id = %state.workspace_id,
                    resource_kind = other,
                    "poll_since called for unsupported resource kind"
                );
                Ok(PollOutcome::default())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_ref_for_issue_matches_webhook_format() {
        // Mirror the format in
        // `crates/onsager-portal/src/db.rs::external_ref_for_issue`.
        // If the two ever diverge the spine idempotency index can't
        // dedup webhook vs poll for the same resource, which is the
        // entire point of #121.
        assert_eq!(
            GitHubAdapter::external_ref_for_issue("proj_abc", 42),
            "github:project:proj_abc:issue:42"
        );
    }

    #[test]
    fn external_ref_for_pr_matches_webhook_format() {
        assert_eq!(
            GitHubAdapter::external_ref_for_pull_request("proj_xyz", 7),
            "github:project:proj_xyz:pr:7"
        );
    }

    #[test]
    fn adapter_id_is_stable() {
        let adapter = GitHubAdapter::new("proj_abc", "owner", "repo", None);
        assert_eq!(adapter.adapter_id(), crate::adapter::ADAPTER_ID);
    }

    #[test]
    fn resource_kind_constants_match_state_table_values() {
        // The constants drive both the adapter dispatch and the
        // values stored in `adapter_reconciliation_state.resource_kind`.
        // Keeping them centralised here avoids the
        // `"issue"` vs `"issues"` typo bug class.
        assert_eq!(GitHubAdapter::KIND_ISSUE, "issue");
        assert_eq!(GitHubAdapter::KIND_PULL_REQUEST, "pull_request");
    }
}
