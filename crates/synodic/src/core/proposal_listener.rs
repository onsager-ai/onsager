//! Rule-proposal spine listener (issue #36 Step 2).
//!
//! Tails `ising.rule_proposed` events off the event spine, deserializes the
//! payload into the typed [`IsingRuleProposed`] variant, and inserts the
//! proposal into the `rule_proposals` queue.
//!
//! Classification:
//! - `safe_auto` proposals are inserted as `approved` and the listener
//!   applies the rule change inline. Today the only wired action is
//!   `Retire`, which disables the matching rule. Future action kinds can
//!   extend [`apply_auto_action`] without a migration because the raw
//!   action is stored as JSON alongside the proposal.
//! - `review_required` proposals sit in the queue at status `pending` for
//!   a human or supervisor to resolve via the HTTP API.
//!
//! The proposal listener is a pure consumer of the spine — it does not emit
//! Synodic events back. The follow-up `synodic.rule_proposed` / `rule_approved`
//! bookkeeping events remain the responsibility of Synodic's existing HTTP
//! routes for rule creation, so the listener doesn't duplicate them.

use std::sync::Arc;

use async_trait::async_trait;
use onsager_spine::factory_event::{FactoryEventKind, RuleProposalAction, RuleProposalClass};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};

use crate::core::storage::{CreateRuleProposal, Storage, UpdateRule};

/// Run a rule_proposed listener forever. Returns only if the underlying
/// pg_notify channel closes.
pub async fn run(
    store: EventStore,
    storage: Arc<dyn Storage>,
    since: Option<i64>,
) -> anyhow::Result<()> {
    let dispatcher = Dispatcher {
        store: store.clone(),
        storage,
    };
    Listener::new(store).with_since(since).run(dispatcher).await
}

struct Dispatcher {
    store: EventStore,
    storage: Arc<dyn Storage>,
}

#[async_trait]
impl EventHandler for Dispatcher {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "ising.rule_proposed" {
            return Ok(());
        }
        if notification.table != "events_ext" {
            return Ok(());
        }

        let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
            return Ok(());
        };

        let kind: FactoryEventKind = match serde_json::from_value(row.data.clone()) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    id = row.id,
                    "synodic: rule_proposed payload parse failed: {e}"
                );
                return Ok(());
            }
        };

        let FactoryEventKind::IsingRuleProposed {
            insight_id,
            signal_kind,
            subject_ref,
            proposed_action,
            class,
            rationale,
            confidence,
        } = kind
        else {
            return Ok(());
        };

        let (initial_status, is_safe_auto) = match class {
            RuleProposalClass::SafeAuto => (Some("approved".to_string()), true),
            RuleProposalClass::ReviewRequired => (None, false),
        };

        let created = self
            .storage
            .create_rule_proposal(CreateRuleProposal {
                insight_id: insight_id.clone(),
                signal_kind: signal_kind.clone(),
                subject_ref: subject_ref.clone(),
                proposed_action: serde_json::to_value(&proposed_action)?,
                class: match class {
                    RuleProposalClass::SafeAuto => "safe_auto".into(),
                    RuleProposalClass::ReviewRequired => "review_required".into(),
                },
                rationale,
                confidence,
                initial_status,
            })
            .await?;

        tracing::info!(
            insight_id = %insight_id,
            signal_kind = %signal_kind,
            subject_ref = %subject_ref,
            proposal_id = %created.id,
            confidence,
            safe_auto = is_safe_auto,
            "synodic: ingested rule proposal"
        );

        if is_safe_auto {
            if let Err(e) = apply_auto_action(&*self.storage, &proposed_action).await {
                // Don't block the listener on a single failed auto-apply —
                // the proposal stays in the queue as `approved` for the
                // operator to re-apply manually.
                tracing::error!(
                    proposal_id = %created.id,
                    "synodic: auto-apply failed, leaving as approved for manual retry: {e}"
                );
            }
        }

        Ok(())
    }
}

/// Apply a proposed rule change without a human in the loop. Only the
/// `Retire` action is wired today; `Rewrite` and `Introduce` still need
/// a human signature because they change rule semantics rather than
/// merely disabling noise.
async fn apply_auto_action(
    storage: &dyn Storage,
    action: &RuleProposalAction,
) -> anyhow::Result<()> {
    match action {
        RuleProposalAction::Retire { rule_id } => {
            // The Ising producer uses the subject_ref (artifact kind) as a
            // placeholder rule_id when grouping is by kind rather than by
            // rule. If no rule exists with that id, skip the apply — the
            // proposal is still logged.
            match storage.get_rule(rule_id).await? {
                Some(_) => {
                    storage
                        .update_rule(
                            rule_id,
                            UpdateRule {
                                enabled: Some(false),
                                ..Default::default()
                            },
                        )
                        .await?;
                    tracing::info!(rule_id, "synodic: auto-retired rule");
                    Ok(())
                }
                None => {
                    tracing::debug!(
                        rule_id,
                        "synodic: retire proposal references no matching rule; skipping apply"
                    );
                    Ok(())
                }
            }
        }
        RuleProposalAction::Rewrite { .. } | RuleProposalAction::Introduce { .. } => {
            tracing::debug!(
                ?action,
                "synodic: auto-apply not implemented for this action"
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::storage::pool::create_storage;

    async fn test_storage() -> Arc<dyn Storage> {
        let url = "sqlite://:memory:";
        let storage = create_storage(url)
            .await
            .expect("in-memory sqlite storage must connect");
        Arc::from(storage)
    }

    #[tokio::test]
    async fn auto_retire_skips_missing_rule() {
        // A safe-auto Retire proposal whose rule_id doesn't exist in Synodic
        // (e.g. Ising grouped by artifact kind) must NOT error — the
        // proposal stays recorded and the listener moves on.
        let storage = test_storage().await;
        let action = RuleProposalAction::Retire {
            rule_id: "nonexistent_kind".into(),
        };
        apply_auto_action(&*storage, &action)
            .await
            .expect("should no-op rather than error");
    }

    #[tokio::test]
    async fn rewrite_and_introduce_are_noops_for_now() {
        let storage = test_storage().await;
        apply_auto_action(
            &*storage,
            &RuleProposalAction::Rewrite {
                rule_id: "r1".into(),
                suggested_condition: None,
            },
        )
        .await
        .expect("rewrite no-op");
        apply_auto_action(
            &*storage,
            &RuleProposalAction::Introduce {
                subject_ref: "code".into(),
                suggested_condition: None,
            },
        )
        .await
        .expect("introduce no-op");
    }

    #[tokio::test]
    async fn create_proposal_is_idempotent_on_insight_id() {
        // Dedup contract: redelivery of the same insight must return the
        // original row rather than insert a duplicate.
        let storage = test_storage().await;

        let make = || CreateRuleProposal {
            insight_id: "ins_dup".into(),
            signal_kind: "repeated_gate_override".into(),
            subject_ref: "code".into(),
            proposed_action: serde_json::json!({
                "action": "retire",
                "rule_id": "code",
            }),
            class: "review_required".into(),
            rationale: "80% override rate".into(),
            confidence: 0.82,
            initial_status: None,
        };

        let first = storage.create_rule_proposal(make()).await.unwrap();
        let second = storage.create_rule_proposal(make()).await.unwrap();
        assert_eq!(first.id, second.id, "redelivery must map to the same row");

        let pending = storage.list_rule_proposals(Some("pending")).await.unwrap();
        assert_eq!(pending.len(), 1, "no duplicate inserted");
    }

    #[tokio::test]
    async fn resolve_pending_then_double_resolve_errors() {
        let storage = test_storage().await;
        let created = storage
            .create_rule_proposal(CreateRuleProposal {
                insight_id: "ins_solo".into(),
                signal_kind: "x".into(),
                subject_ref: "code".into(),
                proposed_action: serde_json::json!({}),
                class: "review_required".into(),
                rationale: "test".into(),
                confidence: 0.5,
                initial_status: None,
            })
            .await
            .unwrap();

        storage
            .resolve_rule_proposal(&created.id, "rejected", Some("no".into()))
            .await
            .expect("first resolve");

        let err = storage
            .resolve_rule_proposal(&created.id, "approved", None)
            .await
            .expect_err("second resolve must error");
        assert!(err.to_string().contains("already resolved"));
    }

    #[tokio::test]
    async fn resolve_rejects_invalid_status() {
        let storage = test_storage().await;
        let created = storage
            .create_rule_proposal(CreateRuleProposal {
                insight_id: "ins_bad_status".into(),
                signal_kind: "x".into(),
                subject_ref: "code".into(),
                proposed_action: serde_json::json!({}),
                class: "review_required".into(),
                rationale: "test".into(),
                confidence: 0.5,
                initial_status: None,
            })
            .await
            .unwrap();

        let err = storage
            .resolve_rule_proposal(&created.id, "deleted", None)
            .await
            .expect_err("invalid status must error");
        assert!(err.to_string().contains("invalid proposal status"));
    }
}
