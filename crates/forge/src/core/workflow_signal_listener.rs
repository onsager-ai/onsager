//! Workflow signal listener (issue #80).
//!
//! Translates spine events into [`SignalCache`] entries so the workflow
//! stage runner's `external-check` and `manual-approval` gates resolve.
//!
//! Event mapping (producers live in the stiglab webhook receiver):
//!
//! | Spine event                | Signal kind (cache key)               |
//! |----------------------------|---------------------------------------|
//! | `git.ci_completed`         | `ci:<check_name>`                     |
//! | `git.pr_merged`            | `pr_merged`                           |
//! | `git.pr_closed`            | `pr_closed`                           |
//! | `stiglab.session_completed`| `agent_session` → Success             |
//! | `stiglab.session_failed`   | `agent_session` → Failure(error)      |
//!
//! `session_failed` (issue #156) is the failure-side counterpart of
//! `session_completed`: without it, an agent that crashes on launch (no
//! credentials, OAuth expired, panic) leaves the agent-session gate
//! pending, the artifact stalls at stage 0, and the next tick re-dispatches
//! a new session that also fails. With it, the gate fails loudly and the
//! artifact parks in `workflow_parked_reason` so the dashboard can render
//! a "needs reconnect" affordance.

use async_trait::async_trait;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};

use super::signal_cache::{Signal, SignalCache, SignalOutcome};
use super::workflow_gates::{external_check_signal_kind, AGENT_SESSION_SIGNAL};

/// Pure classifier: turn a [`FactoryEventKind`] into the signal it maps to
/// (if any). Returned tuple is `(artifact_id, Signal)`.
///
/// Split out as a pure function so tests can exercise every branch
/// without a spine connection.
pub fn classify_signal(kind: &FactoryEventKind) -> Option<(String, Signal)> {
    match kind {
        FactoryEventKind::GitCiCompleted {
            artifact_id,
            check_name,
            conclusion,
            ..
        } => {
            let outcome = if conclusion.eq_ignore_ascii_case("success") {
                SignalOutcome::Success
            } else {
                SignalOutcome::Failure(conclusion.clone())
            };
            Some((
                artifact_id.as_str().to_string(),
                Signal {
                    kind: external_check_signal_kind(check_name),
                    outcome,
                },
            ))
        }
        FactoryEventKind::GitPrMerged { artifact_id, .. } => Some((
            artifact_id.as_str().to_string(),
            Signal {
                kind: "pr_merged".into(),
                outcome: SignalOutcome::Success,
            },
        )),
        FactoryEventKind::GitPrClosed { artifact_id, .. } => Some((
            artifact_id.as_str().to_string(),
            Signal {
                kind: "pr_closed".into(),
                outcome: SignalOutcome::Success,
            },
        )),
        FactoryEventKind::StiglabSessionCompleted {
            artifact_id: Some(aid),
            ..
        } => Some((
            aid.clone(),
            Signal {
                kind: AGENT_SESSION_SIGNAL.into(),
                outcome: SignalOutcome::Success,
            },
        )),
        FactoryEventKind::StiglabSessionFailed {
            artifact_id: Some(aid),
            error,
            ..
        } => Some((
            aid.clone(),
            Signal {
                kind: AGENT_SESSION_SIGNAL.into(),
                outcome: SignalOutcome::Failure(error.clone()),
            },
        )),
        _ => None,
    }
}

/// Run the signal listener forever. Returns only if pg_notify closes.
pub async fn run(
    store: EventStore,
    signals: SignalCache,
    since: Option<i64>,
) -> anyhow::Result<()> {
    let handler = SignalHandler {
        store: store.clone(),
        signals,
    };
    Listener::new(store).with_since(since).run(handler).await
}

struct SignalHandler {
    store: EventStore,
    signals: SignalCache,
}

impl SignalHandler {
    async fn load_kind(
        &self,
        notification: &EventNotification,
    ) -> anyhow::Result<Option<FactoryEventKind>> {
        match notification.table.as_str() {
            "events" => {
                let Some(row) = self.store.get_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                let envelope: FactoryEvent = serde_json::from_value(row.data)?;
                Ok(Some(envelope.event))
            }
            "events_ext" => {
                let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                let raw = row.data;
                if let Ok(envelope) = serde_json::from_value::<FactoryEvent>(raw.clone()) {
                    Ok(Some(envelope.event))
                } else {
                    let kind: FactoryEventKind = serde_json::from_value(raw)?;
                    Ok(Some(kind))
                }
            }
            _ => Ok(None),
        }
    }
}

#[async_trait]
impl EventHandler for SignalHandler {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        // Fast reject on event types that can never be signals.
        let interesting = matches!(
            notification.event_type.as_str(),
            "git.ci_completed"
                | "git.pr_merged"
                | "git.pr_closed"
                | "stiglab.session_completed"
                | "stiglab.session_failed"
        );
        if !interesting {
            return Ok(());
        }

        let Some(kind) = self.load_kind(&notification).await? else {
            return Ok(());
        };

        if let Some((artifact_id, signal)) = classify_signal(&kind) {
            self.signals.push(&artifact_id, signal);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::ArtifactId;

    #[test]
    fn ci_success_classifies_as_pass_signal() {
        let kind = FactoryEventKind::GitCiCompleted {
            artifact_id: ArtifactId::new("art_ci"),
            pr_number: 1,
            check_name: "ci/test".into(),
            conclusion: "success".into(),
        };
        let (aid, sig) = classify_signal(&kind).expect("classified");
        assert_eq!(aid, "art_ci");
        assert_eq!(sig.kind, "ci:ci/test");
        assert_eq!(sig.outcome, SignalOutcome::Success);
    }

    #[test]
    fn ci_failure_classifies_as_failure_signal() {
        let kind = FactoryEventKind::GitCiCompleted {
            artifact_id: ArtifactId::new("art_ci_fail"),
            pr_number: 2,
            check_name: "ci/lint".into(),
            conclusion: "failure".into(),
        };
        let (_, sig) = classify_signal(&kind).expect("classified");
        assert_eq!(sig.kind, "ci:ci/lint");
        match sig.outcome {
            SignalOutcome::Failure(r) => assert_eq!(r, "failure"),
            other => panic!("expected Failure, got {other:?}"),
        }
    }

    #[test]
    fn pr_merged_classifies_as_pr_merged_signal() {
        let kind = FactoryEventKind::GitPrMerged {
            artifact_id: ArtifactId::new("art_pr"),
            pr_number: 7,
            merge_sha: "abc".into(),
        };
        let (aid, sig) = classify_signal(&kind).expect("classified");
        assert_eq!(aid, "art_pr");
        assert_eq!(sig.kind, "pr_merged");
        assert_eq!(sig.outcome, SignalOutcome::Success);
    }

    #[test]
    fn pr_closed_classifies_as_pr_closed_signal() {
        let kind = FactoryEventKind::GitPrClosed {
            artifact_id: ArtifactId::new("art_closed"),
            pr_number: 7,
        };
        let (_, sig) = classify_signal(&kind).expect("classified");
        assert_eq!(sig.kind, "pr_closed");
    }

    #[test]
    fn session_completed_with_artifact_id_classifies_as_agent_session() {
        let kind = FactoryEventKind::StiglabSessionCompleted {
            session_id: "s".into(),
            request_id: "r".into(),
            duration_ms: 1,
            artifact_id: Some("art_sess".into()),
            token_usage: None,
            branch: None,
            pr_number: None,
        };
        let (aid, sig) = classify_signal(&kind).expect("classified");
        assert_eq!(aid, "art_sess");
        assert_eq!(sig.kind, AGENT_SESSION_SIGNAL);
    }

    #[test]
    fn session_completed_without_artifact_id_is_not_a_signal() {
        let kind = FactoryEventKind::StiglabSessionCompleted {
            session_id: "s".into(),
            request_id: "r".into(),
            duration_ms: 1,
            artifact_id: None,
            token_usage: None,
            branch: None,
            pr_number: None,
        };
        assert!(classify_signal(&kind).is_none());
    }

    #[test]
    fn session_failed_with_artifact_id_classifies_as_agent_session_failure() {
        let kind = FactoryEventKind::StiglabSessionFailed {
            session_id: "s".into(),
            request_id: "r".into(),
            error: "stdout closed without result event".into(),
            artifact_id: Some("art_failed".into()),
        };
        let (aid, sig) = classify_signal(&kind).expect("classified");
        assert_eq!(aid, "art_failed");
        assert_eq!(sig.kind, AGENT_SESSION_SIGNAL);
        match sig.outcome {
            SignalOutcome::Failure(reason) => {
                assert_eq!(reason, "stdout closed without result event")
            }
            other => panic!("expected Failure, got {other:?}"),
        }
    }

    #[test]
    fn session_failed_without_artifact_id_is_not_a_signal() {
        // Direct task POSTs (no artifact link) shouldn't write to the
        // signal cache — only workflow-dispatched sessions carry an
        // artifact_id so the gate has somewhere to land the failure.
        let kind = FactoryEventKind::StiglabSessionFailed {
            session_id: "s".into(),
            request_id: "r".into(),
            error: "anything".into(),
            artifact_id: None,
        };
        assert!(classify_signal(&kind).is_none());
    }

    #[test]
    fn unrelated_events_do_not_classify() {
        let kind = FactoryEventKind::ForgeIdleTick;
        assert!(classify_signal(&kind).is_none());
    }

    #[test]
    fn integrates_with_signal_cache() {
        let cache = SignalCache::new();
        let kind = FactoryEventKind::GitPrMerged {
            artifact_id: ArtifactId::new("art_int"),
            pr_number: 1,
            merge_sha: "x".into(),
        };
        let (aid, sig) = classify_signal(&kind).expect("classified");
        cache.push(&aid, sig);
        assert_eq!(
            cache.get("art_int", "pr_merged"),
            Some(SignalOutcome::Success)
        );
    }
}
