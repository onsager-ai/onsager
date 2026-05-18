//! Integration tests for the public Dogfood showcase projection (spec #407).
//!
//! The contract under test is single-sentence: the public response
//! contains only the allow-listed fields. The acceptance criterion is
//! explicit — "verified by integration test: assert response shape
//! contains *only* the allow-listed fields". This file pins that shape
//! against a real Postgres so future PRs can't quietly add a leaking
//! column to the projection.
//!
//! Skipped when `DATABASE_URL` is unset — the projection reads from
//! spine tables that only the Postgres-backed harness exercises.

use std::collections::BTreeSet;

use chrono::Utc;
use onsager_portal::handlers::showcase::build_projection_for_test;
use onsager_portal::workflow::{GateKind, TriggerKind, Workflow, WorkflowStage};
use onsager_portal::workflow_db;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("spine connect"))
}

/// Seed a workflow with the four gate kinds in declared order. The
/// projection only reads `gate_kind` (the "executor_kind") + count, so
/// the names here can be anything — the public response anonymizes them.
async fn seed_dogfood_workflow(spine: &PgPool, workspace_id: &str) -> String {
    let now = Utc::now();
    let workflow_id = format!("wf_{}", Uuid::new_v4());
    let wf = Workflow {
        id: workflow_id.clone(),
        workspace_id: workspace_id.into(),
        name: "internal-dogfood-name-should-not-leak".into(),
        trigger: TriggerKind::GithubIssueWebhook {
            repo: "onsager-ai/onsager".into(),
            label: "ready-to-implement".into(),
        },
        install_id: 1,
        preset_id: Some("onsager-dogfood".into()),
        active: true,
        created_by: "tester".into(),
        created_at: now,
        updated_at: now,
    };
    let stages = vec![
        WorkflowStage {
            id: Uuid::new_v4().to_string(),
            workflow_id: workflow_id.clone(),
            seq: 0,
            gate_kind: GateKind::AgentSession,
            params: serde_json::json!({}),
        },
        WorkflowStage {
            id: Uuid::new_v4().to_string(),
            workflow_id: workflow_id.clone(),
            seq: 1,
            gate_kind: GateKind::Governance,
            params: serde_json::json!({}),
        },
        WorkflowStage {
            id: Uuid::new_v4().to_string(),
            workflow_id: workflow_id.clone(),
            seq: 2,
            gate_kind: GateKind::ExternalCheck,
            params: serde_json::json!({}),
        },
        WorkflowStage {
            id: Uuid::new_v4().to_string(),
            workflow_id: workflow_id.clone(),
            seq: 3,
            gate_kind: GateKind::ManualApproval,
            params: serde_json::json!({}),
        },
    ];
    workflow_db::insert_workflow_with_stages(spine, &wf, &stages)
        .await
        .expect("seed workflow");
    workflow_id
}

async fn seed_issue_run(
    spine: &PgPool,
    workspace_id: &str,
    workflow_id: &str,
    project_id: &str,
    issue_number: u64,
    state: &str,
    stage_index: Option<i32>,
) -> String {
    let artifact_id = format!("art_iss_{}", Uuid::new_v4().simple());
    let external_ref = format!("github:project:{project_id}:issue:{issue_number}");
    sqlx::query(
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, current_version, \
             external_ref, workspace_id, metadata, workflow_id, current_stage_index) \
         VALUES ($1, 'issue', NULL, NULL, 'tester', $2, 1, $3, $4, \
                 jsonb_build_object('project_id', $5::text, \
                                    'issue_number', $6::bigint, \
                                    'repo', $7::text), \
                 $8, $9)",
    )
    .bind(&artifact_id)
    .bind(state)
    .bind(&external_ref)
    .bind(workspace_id)
    .bind(project_id)
    .bind(issue_number as i64)
    .bind("onsager-ai/onsager")
    .bind(workflow_id)
    .bind(stage_index)
    .execute(spine)
    .await
    .expect("seed issue run");
    artifact_id
}

async fn seed_pr_for_issue(
    spine: &PgPool,
    portal: &PgPool,
    workspace_id: &str,
    issue_artifact_id: &str,
    project_id: &str,
    pr_number: u64,
) -> String {
    let pr_artifact_id = format!("art_pr_{}", Uuid::new_v4().simple());
    let external_ref = format!("github:project:{project_id}:pr:{pr_number}");
    sqlx::query(
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, current_version, \
             external_ref, workspace_id, metadata) \
         VALUES ($1, 'pull_request', NULL, NULL, 'tester', 'released', 1, $2, $3, \
                 jsonb_build_object('project_id', $4::text, \
                                    'pr_number', $5::bigint, \
                                    'repo', $6::text))",
    )
    .bind(&pr_artifact_id)
    .bind(&external_ref)
    .bind(workspace_id)
    .bind(project_id)
    .bind(pr_number as i64)
    .bind("onsager-ai/onsager")
    .execute(spine)
    .await
    .expect("seed PR artifact");

    // horizontal_lineage lives on the portal connection per
    // onsager-portal's split-pool convention (#222).
    sqlx::query(
        "INSERT INTO horizontal_lineage \
            (artifact_id, source_artifact_id, source_version, role) \
         VALUES ($1, $2, 1, 'closes_issue')",
    )
    .bind(&pr_artifact_id)
    .bind(issue_artifact_id)
    .execute(portal)
    .await
    .expect("link PR → issue");
    pr_artifact_id
}

async fn cleanup_workflow(spine: &PgPool, portal: &PgPool, workflow_id: &str) {
    // Drop horizontal_lineage rows that point at this workflow's artifacts.
    let _ = sqlx::query(
        "DELETE FROM horizontal_lineage hl \
          USING artifacts a \
          WHERE (hl.artifact_id = a.artifact_id OR hl.source_artifact_id = a.artifact_id) \
            AND a.workflow_id = $1",
    )
    .bind(workflow_id)
    .execute(portal)
    .await;
    let _ = sqlx::query("DELETE FROM artifacts WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(spine)
        .await;
    let _ = sqlx::query("DELETE FROM workflows WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(spine)
        .await;
}

/// The allow-list this test pins. New top-level keys here are deliberate
/// public-API expansions — adding one without updating this set is a
/// regression. Per-run nested keys live in `RUN_ALLOWED_KEYS`.
const TOP_LEVEL_ALLOWED_KEYS: &[&str] = &[
    "enabled",
    "workflow",
    "runs",
    "stats_7d",
    "last_activity_at",
    "is_quiet",
    "generated_at",
];

const RUN_ALLOWED_KEYS: &[&str] = &[
    "id",
    "status",
    "stages",
    "spec",
    "pr",
    "started_at",
    "updated_at",
];

const STAGE_ALLOWED_KEYS: &[&str] = &["index", "executor_kind", "status"];

const WORKFLOW_ALLOWED_KEYS: &[&str] = &["name", "stage_count", "stages"];

const STATS_KEYS: &[&str] = &["specs_shipped", "prs_merged", "verify_gates_passed"];

fn object_keys(v: &Value) -> BTreeSet<String> {
    v.as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

fn allow_set(allowed: &[&str]) -> BTreeSet<String> {
    allowed.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
async fn projection_exposes_only_allow_listed_fields() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let portal = spine.clone();
    let workspace_id = format!("ws-showcase-{}", Uuid::new_v4());
    let workflow_id = seed_dogfood_workflow(&spine, &workspace_id).await;

    // Released run with a linked PR — exercises the "passed" status path
    // plus the PR-link lookup.
    let project_id = format!("proj-{}", Uuid::new_v4());
    let issue_id = seed_issue_run(
        &spine,
        &workspace_id,
        &workflow_id,
        &project_id,
        407,
        "released",
        Some(3),
    )
    .await;
    let _pr_id =
        seed_pr_for_issue(&spine, &portal, &workspace_id, &issue_id, &project_id, 412).await;

    // In-flight run, parked at stage 1.
    let _ = seed_issue_run(
        &spine,
        &workspace_id,
        &workflow_id,
        &project_id,
        408,
        "under_review",
        Some(1),
    )
    .await;

    let body = build_projection_for_test(&spine, &portal, &workflow_id)
        .await
        .expect("projection succeeds");

    // Top-level shape.
    assert_eq!(
        object_keys(&body),
        allow_set(TOP_LEVEL_ALLOWED_KEYS),
        "top-level keys must be exactly the allow-list"
    );
    assert_eq!(body["enabled"], Value::Bool(true));

    // Workflow block: no `id`, no `workspace_id`, no `created_by`, no
    // `trigger`. Only the user-visible shape.
    assert_eq!(
        object_keys(&body["workflow"]),
        allow_set(WORKFLOW_ALLOWED_KEYS),
        "workflow block keys must be exactly the allow-list"
    );
    let workflow_stages = body["workflow"]["stages"].as_array().expect("stages array");
    assert_eq!(workflow_stages.len(), 4);
    for (i, s) in workflow_stages.iter().enumerate() {
        let keys: BTreeSet<String> = object_keys(s);
        assert_eq!(
            keys,
            ["index", "executor_kind"]
                .iter()
                .map(|x| x.to_string())
                .collect::<BTreeSet<_>>(),
            "workflow.stages[{i}] must only carry index + executor_kind"
        );
    }

    // Stats block shape.
    assert_eq!(
        object_keys(&body["stats_7d"]),
        allow_set(STATS_KEYS),
        "stats_7d must contain exactly the three counter keys"
    );

    // Per-run shape.
    let runs = body["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 2, "two seeded runs surface");
    for run in runs {
        assert_eq!(
            object_keys(run),
            allow_set(RUN_ALLOWED_KEYS),
            "per-run keys must be exactly the allow-list",
        );
        // The opaque id never includes the raw artifact_id.
        let id = run["id"].as_str().expect("run id is string");
        assert!(id.starts_with("run_"));
        assert!(!id.contains("art_iss"));
        assert!(!id.contains("art_pr"));

        // Stage entries carry only the documented keys.
        let stages = run["stages"].as_array().expect("stages array");
        assert_eq!(stages.len(), 4);
        for s in stages {
            assert_eq!(
                object_keys(s),
                allow_set(STAGE_ALLOWED_KEYS),
                "per-stage keys must be exactly the allow-list"
            );
            // No leaked stage name from the workflow_stages table.
            assert!(!s.as_object().unwrap().contains_key("name"));
        }
    }

    // Released run must carry a PR link with just (number, url).
    let released = runs
        .iter()
        .find(|r| r["status"] == Value::String("passed".into()))
        .expect("released run present");
    let pr = &released["pr"];
    let pr_obj = pr.as_object().expect("pr is an object");
    let pr_keys: BTreeSet<String> = pr_obj.keys().cloned().collect();
    assert_eq!(
        pr_keys,
        ["number", "url"]
            .iter()
            .map(|x| x.to_string())
            .collect::<BTreeSet<_>>(),
        "pr block carries only number + url"
    );
    assert_eq!(pr["number"], Value::Number(412.into()));

    // Spec link carries only (number, url) too — no title, no body, no
    // labels.
    let spec = &released["spec"];
    let spec_obj = spec.as_object().expect("spec is an object");
    let spec_keys: BTreeSet<String> = spec_obj.keys().cloned().collect();
    assert_eq!(
        spec_keys,
        ["number", "url"]
            .iter()
            .map(|x| x.to_string())
            .collect::<BTreeSet<_>>(),
        "spec block carries only number + url"
    );

    // Internal workflow name does not leak. The spec mandates "Stages
    // are anonymized to 'Stage 1 / Stage 2 / ...' plus their executor
    // kind — no internal repo paths, no spec body content beyond title."
    // We expose the workflow's name (which is a user-chosen string)
    // here, so we only assert that nothing in the stages array carries
    // any internal stage name.
    for run in runs {
        for stage in run["stages"].as_array().unwrap() {
            assert!(
                !stage.as_object().unwrap().contains_key("name"),
                "stage entries must not carry the internal stage name"
            );
            assert!(
                !stage.as_object().unwrap().contains_key("params"),
                "stage entries must not carry gate params"
            );
        }
    }

    cleanup_workflow(&spine, &portal, &workflow_id).await;
}

#[tokio::test]
async fn projection_marks_quiet_when_no_recent_activity() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let portal = spine.clone();
    let workspace_id = format!("ws-showcase-quiet-{}", Uuid::new_v4());
    let workflow_id = seed_dogfood_workflow(&spine, &workspace_id).await;

    // No runs seeded. The projection should mark itself quiet rather
    // than pretend something is live.
    let body = build_projection_for_test(&spine, &portal, &workflow_id)
        .await
        .expect("projection succeeds");
    assert_eq!(body["is_quiet"], Value::Bool(true));
    assert_eq!(body["last_activity_at"], Value::Null);
    assert_eq!(body["runs"].as_array().unwrap().len(), 0);
    assert_eq!(body["stats_7d"]["specs_shipped"], Value::Number(0.into()));

    cleanup_workflow(&spine, &portal, &workflow_id).await;
}
