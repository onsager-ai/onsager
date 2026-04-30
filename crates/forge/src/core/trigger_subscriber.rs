//! Trigger subscriber (issue #80).
//!
//! Listens for `trigger.fired` events on the spine. Each event identifies
//! the workflow that fired and carries the payload needed to register an
//! artifact (issue number, repo, etc.). The subscriber:
//!
//! 1. Reads the matching [`Workflow`] from the in-memory registry.
//! 2. Registers an artifact of the workflow's declared kind.
//! 3. Calls [`enter_workflow`] so the stage runner picks it up next tick.
//!
//! v1 always produces `github-issue` artifacts — future trigger kinds will
//! choose the right [`Kind`] from the workflow's trigger spec.

use std::sync::Arc;

use async_trait::async_trait;

use onsager_artifact::{Artifact, Kind};
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};

use super::artifact_store::ArtifactStore;
use super::stage_runner::{enter_workflow, StageEvent};
use super::workflow::Workflow;

/// Parsed `trigger.fired` event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerFired {
    pub event_id: i64,
    pub workflow_id: String,
    pub trigger_kind: String,
    pub payload: serde_json::Value,
}

/// What the trigger subscriber does with each fired trigger. Concrete
/// implementations in `cmd/serve.rs` reach into the shared Forge state to
/// register an artifact + enter the workflow; tests inspect captured calls.
#[async_trait]
pub trait TriggerHandler: Send + Sync + 'static {
    async fn on_trigger_fired(&self, event: TriggerFired) -> anyhow::Result<()>;
}

/// Spawn the subscriber against the spine. Returns only when pg_notify closes.
pub async fn run<H: TriggerHandler>(
    store: EventStore,
    handler: H,
    since: Option<i64>,
) -> anyhow::Result<()> {
    let dispatcher = Dispatcher {
        store: store.clone(),
        handler: Arc::new(handler),
    };
    Listener::new(store).with_since(since).run(dispatcher).await
}

struct Dispatcher<H: TriggerHandler> {
    store: EventStore,
    handler: Arc<H>,
}

impl<H: TriggerHandler> Dispatcher<H> {
    async fn load(&self, notification: &EventNotification) -> anyhow::Result<Option<TriggerFired>> {
        let (id, kind) = match notification.table.as_str() {
            "events" => {
                let Some(row) = self.store.get_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                let envelope: FactoryEvent = serde_json::from_value(row.data)?;
                (row.id, envelope.event)
            }
            "events_ext" => {
                let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                // Extension events may write either the FactoryEvent
                // envelope or the bare FactoryEventKind as the data column
                // — same shape question session_listener.rs resolves.
                let raw = row.data;
                if let Ok(envelope) = serde_json::from_value::<FactoryEvent>(raw.clone()) {
                    (row.id, envelope.event)
                } else {
                    let kind: FactoryEventKind = serde_json::from_value(raw)?;
                    (row.id, kind)
                }
            }
            _ => return Ok(None),
        };

        let FactoryEventKind::TriggerFired {
            workflow_id,
            trigger_kind,
            payload,
        } = kind
        else {
            return Ok(None);
        };

        Ok(Some(TriggerFired {
            event_id: id,
            workflow_id,
            trigger_kind,
            payload,
        }))
    }
}

#[async_trait]
impl<H: TriggerHandler> EventHandler for Dispatcher<H> {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "trigger.fired" {
            return Ok(());
        }
        match self.load(&notification).await {
            Ok(Some(evt)) => self.handler.on_trigger_fired(evt).await?,
            Ok(None) => {
                tracing::debug!(
                    id = notification.id,
                    "trigger.fired notification had no matching row or mismatched variant"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load trigger.fired event");
            }
        }
        Ok(())
    }
}

/// Stable identity for an artifact created from a trigger payload.
///
/// Format: `forge:trigger:{workflow_id}:{trigger_kind}:{owner}/{repo}#{n}`.
/// Returns `None` when the payload is missing the fields needed to form a
/// stable key (we never fall back to a random ref — that would re-introduce
/// the duplication this function exists to prevent).
///
/// Anchored on `(workflow_id, issue)`: the same issue under two different
/// workflows produces two distinct artifacts (each workflow drives its own
/// pipeline), but a re-fired trigger for the same `(workflow, issue)`
/// converges on the existing row.
pub fn trigger_external_ref(
    workflow_id: &str,
    trigger_kind: &str,
    payload: &serde_json::Value,
) -> Option<String> {
    let owner = payload.get("repo_owner").and_then(|v| v.as_str())?;
    let repo = payload.get("repo_name").and_then(|v| v.as_str())?;
    let number = payload.get("issue_number").and_then(|v| v.as_u64())?;
    if owner.is_empty() || repo.is_empty() || number == 0 {
        return None;
    }
    Some(format!(
        "forge:trigger:{workflow_id}:{trigger_kind}:{owner}/{repo}#{number}"
    ))
}

/// Pure helper: given a fired trigger + the corresponding workflow, build
/// and register the artifact in the store. Used by the live handler in
/// `cmd/serve.rs` and by tests that want to exercise the logic without a
/// spine.
///
/// Returns the newly registered artifact and the initial
/// [`StageEvent::StageEntered`] that the caller should emit on the spine.
pub fn register_artifact_from_trigger(
    store: &mut ArtifactStore,
    workflow: &Workflow,
    trigger: &TriggerFired,
) -> Option<(Artifact, StageEvent)> {
    if !workflow.active {
        // A fired trigger can outrace a workflow deactivation. Drop the
        // registration rather than silently adding another in-flight
        // artifact to an inactive workflow.
        return None;
    }

    // Pull a displayable name from the payload when present; fall back to
    // the workflow name so we never register a completely anonymous
    // artifact.
    let name = trigger
        .payload
        .get("title")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("{} artifact", workflow.name));

    let owner = trigger
        .payload
        .get("owner")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| "forge".to_string());

    let artifact = Artifact::new(
        Kind::Custom(workflow.trigger_artifact_kind().to_string()),
        name,
        owner,
        "forge",
        vec![],
    );
    let id = artifact.artifact_id.clone();
    store.insert(artifact.clone());

    let entered = enter_workflow(store, &id, workflow)?;
    let registered = store.get(&id).cloned()?;
    Some((registered, entered))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::workflow::{StageSpec, TriggerSpec};
    use onsager_artifact::ArtifactState;

    fn make_workflow(active: bool) -> Workflow {
        Workflow {
            workflow_id: "wf_1".into(),
            name: "issue-to-pr".into(),
            trigger: TriggerSpec::GithubIssueWebhook {
                repo: "a/b".into(),
                label: "ai".into(),
            },
            stages: vec![StageSpec {
                name: "implement".into(),
                target_state: Some(ArtifactState::InProgress),
                gates: vec![],
                params: serde_json::Value::Null,
            }],
            active,
            workspace_id: "ws_test".into(),
            preset_id: None,
            install_id: None,
            created_by: None,
        }
    }

    fn make_trigger() -> TriggerFired {
        TriggerFired {
            event_id: 1,
            workflow_id: "wf_1".into(),
            trigger_kind: "github_issue_webhook".into(),
            payload: serde_json::json!({
                "title": "Fix bug #42",
                "owner": "marvin",
                "issue_number": 42,
                "repo": "a/b"
            }),
        }
    }

    #[test]
    fn registers_artifact_with_workflow_tag() {
        let mut store = ArtifactStore::new();
        let wf = make_workflow(true);
        let trigger = make_trigger();

        let (artifact, event) =
            register_artifact_from_trigger(&mut store, &wf, &trigger).expect("some");
        assert_eq!(artifact.workflow_id.as_deref(), Some("wf_1"));
        assert_eq!(artifact.current_stage_index, Some(0));
        assert_eq!(artifact.kind.to_string(), "github-issue");
        assert_eq!(artifact.name, "Fix bug #42");
        assert_eq!(artifact.owner, "marvin");
        assert_eq!(artifact.state, ArtifactState::InProgress);
        assert!(matches!(event, StageEvent::StageEntered { .. }));
    }

    #[test]
    fn drops_trigger_for_inactive_workflow() {
        let mut store = ArtifactStore::new();
        let wf = make_workflow(false);
        let trigger = make_trigger();
        assert!(register_artifact_from_trigger(&mut store, &wf, &trigger).is_none());
        assert_eq!(store.active_artifacts().len(), 0);
    }

    #[test]
    fn external_ref_is_stable_across_calls() {
        let payload = serde_json::json!({
            "repo_owner": "acme",
            "repo_name": "widgets",
            "issue_number": 42,
            "title": "anything",
        });
        let a = trigger_external_ref("wf_1", "github_issue_webhook", &payload);
        let b = trigger_external_ref("wf_1", "github_issue_webhook", &payload);
        assert_eq!(a, b);
        assert_eq!(
            a.as_deref(),
            Some("forge:trigger:wf_1:github_issue_webhook:acme/widgets#42")
        );
    }

    #[test]
    fn external_ref_distinct_per_workflow() {
        let payload = serde_json::json!({
            "repo_owner": "acme",
            "repo_name": "widgets",
            "issue_number": 42,
        });
        let a = trigger_external_ref("wf_1", "github_issue_webhook", &payload);
        let b = trigger_external_ref("wf_2", "github_issue_webhook", &payload);
        assert_ne!(a, b);
    }

    #[test]
    fn external_ref_returns_none_when_payload_incomplete() {
        // Missing repo_owner / repo_name / issue_number → can't form a
        // stable key, so we don't fabricate one (which would re-introduce
        // duplicates).
        assert!(trigger_external_ref("wf", "k", &serde_json::json!({})).is_none());
        assert!(trigger_external_ref(
            "wf",
            "k",
            &serde_json::json!({ "repo_owner": "a", "repo_name": "b" })
        )
        .is_none());
        assert!(trigger_external_ref(
            "wf",
            "k",
            &serde_json::json!({
                "repo_owner": "",
                "repo_name": "b",
                "issue_number": 1
            })
        )
        .is_none());
        assert!(trigger_external_ref(
            "wf",
            "k",
            &serde_json::json!({
                "repo_owner": "a",
                "repo_name": "b",
                "issue_number": 0
            })
        )
        .is_none());
    }

    #[test]
    fn missing_payload_fields_use_defaults() {
        let mut store = ArtifactStore::new();
        let wf = make_workflow(true);
        let trigger = TriggerFired {
            event_id: 1,
            workflow_id: "wf_1".into(),
            trigger_kind: "github_issue_webhook".into(),
            payload: serde_json::json!({}),
        };
        let (artifact, _) =
            register_artifact_from_trigger(&mut store, &wf, &trigger).expect("some");
        assert_eq!(artifact.name, "issue-to-pr artifact");
        assert_eq!(artifact.owner, "forge");
    }
}
