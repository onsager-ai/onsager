//! Idempotent seed loader for the factory registry.
//!
//! Loads a [`SeedCatalog`] (typically parsed from a YAML file) and materializes
//! its entries into the registry tables, emitting the corresponding
//! `registry.*` events onto the spine with `actor = "seed"`.
//!
//! Idempotency: the loader checks [`registry_seed_marker`] first. If a seed
//! with the same name has already been applied to the workspace, it returns
//! immediately with zero events. This is the bootstrap-termination rule from
//! issue #14 — the seed runs exactly once; after that, every registry change
//! goes through the normal propose/approve flow.

use chrono::Utc;
use onsager_spine::{
    append_factory_event_tx, EventMetadata, EventStore, FactoryEvent, FactoryEventKind,
};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};

use crate::registry::{
    AgentProfile, RegistryStatus, TypeDefinition, DEFAULT_WORKSPACE, SEED_ACTOR,
};

/// An adapter catalog entry as it appears in a seed file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedAdapter {
    pub adapter_id: String,
    #[serde(default)]
    pub config: serde_json::Value,
}

/// An evaluator catalog entry as it appears in a seed file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedEvaluator {
    pub evaluator_id: String,
    #[serde(default)]
    pub config: serde_json::Value,
}

/// A self-contained seed bundle for one workspace.
///
/// The ordering of apply is: adapters → evaluators → profiles → types. Types
/// may reference the others by id, so the others must be present first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedCatalog {
    /// Unique name used by the idempotency marker. Re-running the loader with
    /// the same `name` for a workspace is a no-op.
    pub name: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub types: Vec<TypeDefinition>,
    #[serde(default)]
    pub adapters: Vec<SeedAdapter>,
    #[serde(default)]
    pub evaluators: Vec<SeedEvaluator>,
    #[serde(default)]
    pub profiles: Vec<AgentProfile>,
}

impl SeedCatalog {
    /// Parse a seed catalog from YAML source.
    pub fn from_yaml(src: &str) -> anyhow::Result<Self> {
        Ok(serde_yaml::from_str(src)?)
    }
}

/// Outcome of a seed apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeedOutcome {
    pub applied: bool,
    pub events_emitted: usize,
}

/// Apply a seed catalog idempotently.
///
/// The entire apply runs in a single transaction: either every row lands
/// (registry tables + marker + events) or nothing does. Rerunning with the
/// same `name`/`workspace_id` pair emits zero events.
pub async fn apply_seed(store: &EventStore, catalog: &SeedCatalog) -> anyhow::Result<SeedOutcome> {
    let workspace_id = catalog
        .workspace_id
        .clone()
        .unwrap_or_else(|| DEFAULT_WORKSPACE.to_owned());
    let seed_name = catalog.name.clone();
    let catalog = catalog.clone();

    store
        .transaction(move |tx| {
            Box::pin(async move {
                apply_in_tx(tx, &workspace_id, &seed_name, &catalog)
                    .await
                    .map_err(to_sqlx_error)
            })
        })
        .await
        .map_err(anyhow::Error::from)
}

async fn apply_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: &str,
    seed_name: &str,
    catalog: &SeedCatalog,
) -> anyhow::Result<SeedOutcome> {
    // Check the marker first — idempotency.
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT workspace_id FROM registry_seed_marker WHERE workspace_id = $1 AND seed_name = $2",
    )
    .bind(workspace_id)
    .bind(seed_name)
    .fetch_optional(&mut **tx)
    .await?;
    if existing.is_some() {
        return Ok(SeedOutcome::default());
    }

    let mut events_emitted = 0usize;
    let status = RegistryStatus::Approved.as_str();

    // Only emit a registry.* event when the row was actually inserted. ON
    // CONFLICT DO NOTHING returns 0 rows affected for duplicates (YAML
    // duplicates or concurrent seeds), and an event in that case would
    // falsely claim a registration happened.

    for adapter in &catalog.adapters {
        let affected = sqlx::query(
            r#"
            INSERT INTO artifact_adapters (adapter_id, workspace_id, revision, status, config)
            VALUES ($1, $2, 1, $3, $4)
            ON CONFLICT (workspace_id, adapter_id) DO NOTHING
            "#,
        )
        .bind(&adapter.adapter_id)
        .bind(workspace_id)
        .bind(status)
        .bind(&adapter.config)
        .execute(&mut **tx)
        .await?
        .rows_affected();

        if affected == 1 {
            let evt = registry_event(FactoryEventKind::AdapterRegistered {
                adapter_id: adapter.adapter_id.clone(),
                workspace_id: workspace_id.to_owned(),
                revision: 1,
            });
            append_factory_event_tx(tx, &evt, &seed_metadata()).await?;
            events_emitted += 1;
        }
    }

    for evaluator in &catalog.evaluators {
        let affected = sqlx::query(
            r#"
            INSERT INTO gate_evaluators (evaluator_id, workspace_id, revision, status, config)
            VALUES ($1, $2, 1, $3, $4)
            ON CONFLICT (workspace_id, evaluator_id) DO NOTHING
            "#,
        )
        .bind(&evaluator.evaluator_id)
        .bind(workspace_id)
        .bind(status)
        .bind(&evaluator.config)
        .execute(&mut **tx)
        .await?
        .rows_affected();

        if affected == 1 {
            let evt = registry_event(FactoryEventKind::GateRegistered {
                evaluator_id: evaluator.evaluator_id.clone(),
                workspace_id: workspace_id.to_owned(),
                revision: 1,
            });
            append_factory_event_tx(tx, &evt, &seed_metadata()).await?;
            events_emitted += 1;
        }
    }

    for profile in &catalog.profiles {
        let config = serde_json::to_value(profile)?;
        let affected = sqlx::query(
            r#"
            INSERT INTO agent_profiles (profile_id, workspace_id, revision, status, config)
            VALUES ($1, $2, 1, $3, $4)
            ON CONFLICT (workspace_id, profile_id) DO NOTHING
            "#,
        )
        .bind(profile.profile_id.as_str())
        .bind(workspace_id)
        .bind(status)
        .bind(&config)
        .execute(&mut **tx)
        .await?
        .rows_affected();

        if affected == 1 {
            let evt = registry_event(FactoryEventKind::ProfileRegistered {
                profile_id: profile.profile_id.as_str().to_owned(),
                workspace_id: workspace_id.to_owned(),
                revision: 1,
            });
            append_factory_event_tx(tx, &evt, &seed_metadata()).await?;
            events_emitted += 1;
        }
    }

    for type_def in &catalog.types {
        let definition = serde_json::to_value(type_def)?;
        let affected = sqlx::query(
            r#"
            INSERT INTO artifact_types (type_id, workspace_id, revision, status, definition)
            VALUES ($1, $2, 1, $3, $4)
            ON CONFLICT (workspace_id, type_id) DO NOTHING
            "#,
        )
        .bind(type_def.type_id.as_str())
        .bind(workspace_id)
        .bind(status)
        .bind(&definition)
        .execute(&mut **tx)
        .await?
        .rows_affected();

        if affected == 1 {
            let evt = registry_event(FactoryEventKind::TypeApproved {
                type_id: type_def.type_id.as_str().to_owned(),
                workspace_id: workspace_id.to_owned(),
                revision: 1,
            });
            append_factory_event_tx(tx, &evt, &seed_metadata()).await?;
            events_emitted += 1;
        }
    }

    // Write the marker last so a partial failure rolls back both the marker
    // and the registry writes (the whole thing is one transaction).
    sqlx::query("INSERT INTO registry_seed_marker (workspace_id, seed_name) VALUES ($1, $2)")
        .bind(workspace_id)
        .bind(seed_name)
        .execute(&mut **tx)
        .await?;

    Ok(SeedOutcome {
        applied: true,
        events_emitted,
    })
}

fn registry_event(kind: FactoryEventKind) -> FactoryEvent {
    FactoryEvent {
        event: kind,
        correlation_id: None,
        causation_id: None,
        actor: SEED_ACTOR.to_owned(),
        timestamp: Utc::now(),
    }
}

fn seed_metadata() -> EventMetadata {
    EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: SEED_ACTOR.to_owned(),
    }
}

/// Convert anyhow errors back into `sqlx::Error` so they can bubble out of
/// the `transaction` closure. The original error text is preserved.
fn to_sqlx_error(err: anyhow::Error) -> sqlx::Error {
    match err.downcast::<sqlx::Error>() {
        Ok(sqlx_err) => sqlx_err,
        Err(other) => sqlx::Error::Protocol(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in base seed parses cleanly and declares the expected
    /// recursive base.
    #[test]
    fn base_yaml_parses() {
        let src = include_str!("../seeds/base.yaml");
        let seed = SeedCatalog::from_yaml(src).expect("base.yaml should parse");
        assert_eq!(seed.name, "base");

        let type_ids: Vec<_> = seed.types.iter().map(|t| t.type_id.as_str()).collect();
        assert!(type_ids.contains(&"TypeDefinition"));
        assert!(type_ids.contains(&"GateEvaluator"));
        assert!(type_ids.contains(&"AgentProfile"));

        assert!(seed
            .evaluators
            .iter()
            .any(|e| e.evaluator_id == "HumanApproval"));
        assert!(seed
            .profiles
            .iter()
            .any(|p| p.profile_id.as_str() == "Human"));
    }

    #[test]
    fn seed_metadata_uses_seed_actor() {
        let meta = seed_metadata();
        assert_eq!(meta.actor, SEED_ACTOR);
        assert_eq!(SEED_ACTOR, "seed");
    }

    #[test]
    fn registry_event_preserves_kind() {
        let evt = registry_event(FactoryEventKind::TypeApproved {
            type_id: "Spec".into(),
            workspace_id: DEFAULT_WORKSPACE.into(),
            revision: 1,
        });
        assert_eq!(evt.actor, SEED_ACTOR);
        assert_eq!(evt.event.event_type(), "registry.type_approved");
        assert_eq!(evt.event.stream_type(), "registry");
        assert_eq!(evt.event.stream_id(), "type:Spec");
    }

    #[test]
    fn seed_outcome_default_is_no_op() {
        let o = SeedOutcome::default();
        assert!(!o.applied);
        assert_eq!(o.events_emitted, 0);
    }

    /// Bootstrap termination: each reference a seed type makes must resolve
    /// inside the same seed. Otherwise the base case is not self-contained.
    #[test]
    fn base_seed_is_self_contained() {
        let src = include_str!("../seeds/base.yaml");
        let seed = SeedCatalog::from_yaml(src).expect("base.yaml should parse");

        let adapter_ids: std::collections::HashSet<_> = seed
            .adapters
            .iter()
            .map(|a| a.adapter_id.as_str())
            .collect();
        let evaluator_ids: std::collections::HashSet<_> = seed
            .evaluators
            .iter()
            .map(|e| e.evaluator_id.as_str())
            .collect();
        let profile_ids: std::collections::HashSet<_> = seed
            .profiles
            .iter()
            .map(|p| p.profile_id.as_str())
            .collect();

        for t in &seed.types {
            assert!(
                adapter_ids.contains(t.adapter_id.as_str()),
                "type {} references unknown adapter {}",
                t.type_id,
                t.adapter_id
            );
            for g in &t.gate_ids {
                assert!(
                    evaluator_ids.contains(g.as_str()),
                    "type {} references unknown evaluator {g}",
                    t.type_id
                );
            }
            if let Some(p) = &t.producer_profile_id {
                assert!(
                    profile_ids.contains(p.as_str()),
                    "type {} references unknown profile {p}",
                    t.type_id
                );
            }
        }
    }
}
