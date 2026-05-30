//! Integration tests for run-path version pinning (#511, #506 Phase 2).
//!
//! A plan recovered after a host restart must recompile against the
//! workflow-library version it was *registered* with, not whatever is
//! latest at restart. These tests exercise the full persisted path:
//! register a workflow version → register a plan (which pins
//! `{kind → version}`) → register a *newer* version → recover → assert
//! the recovered snapshot resolves the original version.
//!
//! Skipped when `DATABASE_URL` is unset — matches the portal /
//! `durable_plan_store` integration-test convention. Requires the spine
//! `workflow_library` table (migration 004) plus the execution-plan
//! tables (migrations 006 + 008, ensured idempotently by
//! `onsager_scheduler::migrate::run`).

use std::collections::HashMap;

use onsager_artifact::{ArtifactId, NodeId, Provenance};
use onsager_nodes::PlanId;
use onsager_scheduler::{SchedulerLibrarySnapshot, plan_registry, resolve_kind_versions};
use onsager_substrate::WorkflowLookup;
use onsager_substrate::ids::EdgeId;
use onsager_substrate::spec_plan::{SpecId, SpecPlan, SpecRef};
use onsager_substrate::workflow::{Edge, EdgeRef, EntrySpec, Node, OutputSpec, Workflow};
use onsager_substrate::workflow_library::WorkflowLibrary;
use sqlx::PgPool;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("spine connect"))
}

/// A one-node passthrough workflow whose entry edge carries a `tag`ed
/// artifact id, so two registered versions of the same kind survive a
/// round-trip through the library distinguishably.
fn tagged_workflow(tag: &str) -> Workflow {
    let entry = EdgeId::generate();
    let exit = EdgeId::generate();
    Workflow {
        nodes: vec![Node {
            id: NodeId::generate(),
            executor: Box::new(onsager_substrate::executor::NoOpExecutor),
            inputs: vec![EdgeRef::new(entry)],
            outputs: vec![EdgeRef::new(exit)],
        }],
        edges: vec![
            Edge {
                id: entry,
                artifact_id: ArtifactId::new(format!("art_in_{tag}")),
                requires_deterministic: false,
            },
            Edge {
                id: exit,
                artifact_id: ArtifactId::new(format!("art_out_{tag}")),
                requires_deterministic: false,
            },
        ],
        entry_specs: vec![EntrySpec { edge_id: entry }],
        output_specs: vec![OutputSpec {
            edge_id: exit,
            provenance: Provenance::external_deterministic(),
        }],
    }
}

/// The entry edge's artifact id (`art_in_<tag>`) — the field that tells
/// two registered versions of a kind apart.
fn entry_tag(wf: &Workflow) -> &str {
    let entry_edge = wf.entry_specs[0].edge_id;
    wf.edges
        .iter()
        .find(|e| e.id == entry_edge)
        .map(|e| e.artifact_id.as_str())
        .expect("entry edge present")
}

fn one_spec_plan(kind: &str) -> SpecPlan {
    SpecPlan {
        specs: vec![SpecRef {
            id: SpecId::new("s1"),
            kind: kind.into(),
            inputs: Default::default(),
        }],
        deps: vec![],
    }
}

/// The core DoD: a plan registered against v1, recovered after v2 lands,
/// recompiles against **v1**.
///
/// Mutation check: make `SchedulerLibrarySnapshot::build` ignore the pin
/// (resolve `library.lookup(kind)` unconditionally) and this assertion
/// flips to `art_in_v2` and fails.
#[tokio::test]
async fn recovered_plan_resolves_pinned_version_not_latest() {
    let Some(pool) = try_pool().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    onsager_scheduler::migrate::run(&pool)
        .await
        .expect("ensure execution-plan tables");
    let library = WorkflowLibrary::new(pool.clone());

    // Unique kind per run so reruns don't collide on workflow_library.
    let kind = format!("pin-test-{}", uuid::Uuid::new_v4());

    // Register version 1, then pin a plan against it.
    let v1 = library
        .register(&kind, &tagged_workflow("v1"))
        .await
        .expect("register v1");
    assert_eq!(v1, 1);

    let plan = one_spec_plan(&kind);
    let kind_versions = resolve_kind_versions(&plan, &library)
        .await
        .expect("resolve versions");
    assert_eq!(kind_versions.get(&kind), Some(&1), "pinned at v1");

    let plan_id = PlanId::new(format!("plan-{}", uuid::Uuid::new_v4()));
    plan_registry::register_plan(&pool, &plan_id, "default", None, &plan, &kind_versions)
        .await
        .expect("register plan");

    // A NEW version lands after registration — latest() now points at it.
    let v2 = library
        .register(&kind, &tagged_workflow("v2"))
        .await
        .expect("register v2");
    assert_eq!(v2, 2);
    assert_eq!(
        entry_tag(&library.lookup(&kind).await.unwrap().unwrap()),
        "art_in_v2",
        "latest() now resolves v2",
    );

    // Recover: the persisted pin must round-trip and resolve v1.
    let recovered = plan_registry::list_recoverable(&pool)
        .await
        .expect("list recoverable");
    let rec = recovered
        .iter()
        .find(|r| r.plan_id.as_str() == plan_id.as_str())
        .expect("our plan is recoverable");
    assert_eq!(
        rec.kind_versions.get(&kind),
        Some(&1),
        "pin survived the JSONB round-trip",
    );

    let snapshot = SchedulerLibrarySnapshot::build(&rec.spec_plan, &library, &rec.kind_versions)
        .await
        .expect("build snapshot");
    assert_eq!(
        entry_tag(snapshot.by_kind(&kind).expect("kind resolved")),
        "art_in_v1",
        "recovery recompiles against the pinned v1, not latest v2",
    );

    // Don't leave a `running` row behind for other tests' recovery scans.
    plan_registry::set_status(&pool, &plan_id, "completed")
        .await
        .unwrap();
}

/// A legacy row (registered before #511) carries an empty
/// `kind_versions` map and recovers via `latest()` exactly as before.
#[tokio::test]
async fn absent_pin_recovers_via_latest() {
    let Some(pool) = try_pool().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    onsager_scheduler::migrate::run(&pool)
        .await
        .expect("ensure execution-plan tables");
    let library = WorkflowLibrary::new(pool.clone());

    let kind = format!("absent-pin-{}", uuid::Uuid::new_v4());
    library
        .register(&kind, &tagged_workflow("v1"))
        .await
        .expect("register v1");
    library
        .register(&kind, &tagged_workflow("v2"))
        .await
        .expect("register v2");

    let plan = one_spec_plan(&kind);
    let no_pins: HashMap<String, i32> = HashMap::new();
    let snapshot = SchedulerLibrarySnapshot::build(&plan, &library, &no_pins)
        .await
        .expect("build snapshot");
    assert_eq!(
        entry_tag(snapshot.by_kind(&kind).expect("kind resolved")),
        "art_in_v2",
        "an absent pin resolves latest()",
    );
}

/// A pinned version that was later retired (`retired_at IS NOT NULL`)
/// falls back to `latest()` — `lookup_versioned` treats the retired row
/// as absent, so `build` takes the warn-and-fall-back branch.
#[tokio::test]
async fn retired_pin_falls_back_to_latest() {
    let Some(pool) = try_pool().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    onsager_scheduler::migrate::run(&pool)
        .await
        .expect("ensure execution-plan tables");
    let library = WorkflowLibrary::new(pool.clone());

    let kind = format!("retired-pin-{}", uuid::Uuid::new_v4());
    library
        .register(&kind, &tagged_workflow("v1"))
        .await
        .expect("register v1");
    library
        .register(&kind, &tagged_workflow("v2"))
        .await
        .expect("register v2");

    // Retire v1 out from under the pin.
    sqlx::query(
        "UPDATE workflow_library SET retired_at = NOW() \
         WHERE spec_kind = $1 AND version = 1",
    )
    .bind(&kind)
    .execute(&pool)
    .await
    .expect("retire v1");

    // lookup_versioned ignores the retired row entirely.
    assert!(
        library.lookup_versioned(&kind, 1).await.unwrap().is_none(),
        "a retired version is invisible to lookup_versioned",
    );

    // Pin still points at v1; build must warn and fall back to latest v2.
    let plan = one_spec_plan(&kind);
    let pins = HashMap::from([(kind.clone(), 1)]);
    let snapshot = SchedulerLibrarySnapshot::build(&plan, &library, &pins)
        .await
        .expect("build snapshot");
    assert_eq!(
        entry_tag(snapshot.by_kind(&kind).expect("kind resolved")),
        "art_in_v2",
        "a retired pin falls back to latest()",
    );
}
