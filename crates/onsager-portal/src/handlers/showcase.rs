//! Public Dogfood showcase projection (spec #407).
//!
//! `GET /api/showcase/dogfood` is the read-only public surface that
//! powers `app.onsager.ai/showcase/dogfood` — the live reference Axis 2's
//! landing hero links to. The route is unauthenticated by design: the
//! evaluator clicks "see live runs ↗" from an OSS README or a marketing
//! page and lands on something concrete, with no auth wall.
//!
//! ## Sanitization (the load-bearing piece)
//!
//! The dashboard's authenticated workflow API surfaces internal state
//! (full stage names, gate params, agent transcripts, governance
//! verdicts, retry reasons). This route serves the same data through a
//! strict allow-list: stage names are anonymized to `Stage N · <executor>`,
//! spec issues expose only `(number, url)`, PRs only `(number, url)`,
//! and nothing else from the artifact/event chain crosses the boundary.
//! `tests/showcase_dogfood.rs` pins the response shape so future fields
//! on the source tables can't accidentally leak.
//!
//! ## Freshness
//!
//! The projection is cached in-process for 60 seconds (the same TTL
//! shape `proxy_cache` uses for live-hydration reads — different cache
//! instance, distinct concern). Within a TTL window every request
//! returns identical bytes; expiry triggers a single fresh build that
//! the next caller sees. No coalescing — under genuine load two
//! simultaneous misses will each rebuild, which is cheap because the
//! projection is a handful of indexed reads.
//!
//! ## Configuration
//!
//! The workflow we project is selected by `SHOWCASE_DOGFOOD_WORKFLOW_ID`
//! (Config::showcase_dogfood_workflow_id). When unset, the route
//! short-circuits to `{ "enabled": false, ... }` so the static page can
//! render a placeholder rather than 404. In production this env var
//! pins the Onsager-managing-Onsager workflow row in the Cloud
//! instance's internal workspace; OSS self-hosters can repoint it at
//! their own dogfood workflow if they want.
//!
//! ## Quiet-week handling
//!
//! When the most recent run's `updated_at` is older than 7 days, the
//! response sets `is_quiet = true` and the dashboard renders the
//! "Last activity: <relative ts>" footer instead of inventing fake
//! liveness. A red factory shows up as failed runs — that's the
//! commitment behind the surface.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

use crate::state::AppState;
use crate::workflow_db;

/// How many runs the projection surfaces. Hardcoded per spec — 10 is
/// enough to read a week's worth of activity at the current dogfood
/// cadence without forcing the reader to scroll.
const RUN_LIMIT: i64 = 10;

/// TTL for the per-process projection cache. The spec pins 60s as the
/// freshness floor; a static rather than env-tunable value keeps the
/// public surface predictable across replicas.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Cached projection payload. One slot, replaced atomically on miss.
#[derive(Clone)]
struct CachedPayload {
    inserted: Instant,
    body: Value,
}

/// Process-local single-slot cache. The showcase has exactly one
/// resource (the dogfood workflow projection), so a HashMap would just
/// add ceremony.
#[derive(Default)]
struct ProjectionCache {
    slot: Mutex<Option<CachedPayload>>,
}

impl ProjectionCache {
    fn get_fresh(&self) -> Option<Value> {
        let slot = self.slot.lock().ok()?;
        let entry = slot.as_ref()?;
        if entry.inserted.elapsed() <= CACHE_TTL {
            Some(entry.body.clone())
        } else {
            None
        }
    }

    fn put(&self, body: Value) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = Some(CachedPayload {
                inserted: Instant::now(),
                body,
            });
        }
    }
}

/// Lazily-initialized process-local cache. The handler shares one
/// instance across requests; nothing else touches it.
fn cache() -> &'static Arc<ProjectionCache> {
    use std::sync::OnceLock;
    static CELL: OnceLock<Arc<ProjectionCache>> = OnceLock::new();
    CELL.get_or_init(|| Arc::new(ProjectionCache::default()))
}

/// GET /api/showcase/dogfood — public, unauthenticated, 60s cached.
pub async fn get_dogfood(State(state): State<AppState>) -> Response {
    if let Some(body) = cache().get_fresh() {
        return Json(body).into_response();
    }

    let workflow_id = match state.config.showcase_dogfood_workflow_id.as_deref() {
        Some(id) if !id.is_empty() => id,
        _ => {
            let body = disabled_payload();
            cache().put(body.clone());
            return Json(body).into_response();
        }
    };

    let body = match build_projection(&state, workflow_id).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "showcase: projection failed");
            // Public surface — fail soft so a transient DB hiccup
            // doesn't surface a 500 to an evaluator. Reuse the
            // disabled shape verbatim so the `{ enabled: false }`
            // allow-list contract still holds; the failure signal is
            // `enabled: false` plus the server-side log.
            let b = disabled_payload();
            cache().put(b.clone());
            return Json(b).into_response();
        }
    };
    cache().put(body.clone());
    Json(body).into_response()
}

fn disabled_payload() -> Value {
    json!({
        "enabled": false,
        "workflow": Value::Null,
        "runs": [],
        "stats_7d": {
            "specs_shipped": 0,
            "prs_merged": 0,
            "verify_gates_passed": 0,
        },
        "last_activity_at": Value::Null,
        "is_quiet": true,
        "generated_at": Utc::now().to_rfc3339(),
    })
}

#[derive(Debug)]
struct StageMeta {
    executor_kind: String,
}

#[derive(Debug, sqlx::FromRow)]
struct RunRow {
    artifact_id: String,
    kind: String,
    state: String,
    current_stage_index: Option<i32>,
    workflow_parked_reason: Option<String>,
    external_ref: Option<String>,
    metadata: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Build the projection payload directly against the spine + portal
/// pools. The HTTP handler is a thin wrapper that adds the 60s cache
/// and serves the result over JSON; tests exercise this seam to assert
/// the response shape without spinning up a full axum router.
pub async fn build_projection_for_test(
    spine: &PgPool,
    portal: &PgPool,
    workflow_id: &str,
) -> anyhow::Result<Value> {
    build_projection_inner(spine, portal, workflow_id).await
}

async fn build_projection(state: &AppState, workflow_id: &str) -> anyhow::Result<Value> {
    build_projection_inner(state.spine.pool(), &state.pool, workflow_id).await
}

async fn build_projection_inner(
    spine: &PgPool,
    portal: &PgPool,
    workflow_id: &str,
) -> anyhow::Result<Value> {
    let workflow = workflow_db::get_workflow(spine, workflow_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("dogfood workflow {workflow_id} not found"))?;
    let stages = workflow_db::list_stages_for_workflow(spine, workflow_id).await?;

    let stage_meta: Vec<StageMeta> = stages
        .iter()
        .map(|s| StageMeta {
            executor_kind: s.gate_kind.to_string(),
        })
        .collect();

    let rows: Vec<RunRow> = sqlx::query_as::<_, RunRow>(
        "SELECT artifact_id, kind, state, current_stage_index, workflow_parked_reason, \
                external_ref, metadata, created_at, updated_at \
           FROM artifacts \
          WHERE workflow_id = $1 \
          ORDER BY updated_at DESC \
          LIMIT $2",
    )
    .bind(workflow_id)
    .bind(RUN_LIMIT)
    .fetch_all(spine)
    .await?;

    let mut runs = Vec::with_capacity(rows.len());
    for row in &rows {
        runs.push(project_run(spine, portal, row, &stage_meta).await?);
    }

    let last_activity_at = rows.first().map(|r| r.updated_at);
    let is_quiet = last_activity_at
        .map(|ts| (Utc::now() - ts).num_days() >= 7)
        .unwrap_or(true);

    let stats = compute_stats_7d(spine, workflow_id, &workflow.workspace_id).await?;

    Ok(json!({
        "enabled": true,
        "workflow": {
            "name": workflow.name,
            "stage_count": stages.len(),
            "stages": stage_meta
                .iter()
                .enumerate()
                .map(|(i, m)| json!({
                    "index": i + 1,
                    "executor_kind": m.executor_kind,
                }))
                .collect::<Vec<_>>(),
        },
        "runs": runs,
        "stats_7d": stats,
        "last_activity_at": last_activity_at.map(|ts| ts.to_rfc3339()),
        "is_quiet": is_quiet,
        "generated_at": Utc::now().to_rfc3339(),
    }))
}

/// Project a single run row + its stage chain into the sanitized public
/// shape. Mirrors the status mapping in `handlers/workflows::project_run`
/// — kept inline so the public projection can never drift to expose
/// fields the authenticated route adds later.
async fn project_run(
    spine: &PgPool,
    portal: &PgPool,
    row: &RunRow,
    stages: &[StageMeta],
) -> anyhow::Result<Value> {
    let current_idx = row
        .current_stage_index
        .and_then(|i| usize::try_from(i).ok());
    let archived = row.state == "archived";
    let released = row.state == "released";
    let parked = row.workflow_parked_reason.is_some();

    let stage_entries: Vec<Value> = stages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let status = match (released, archived, parked, current_idx) {
                (true, _, _, _) => "passed",
                (_, true, _, Some(idx)) if i < idx => "passed",
                (_, true, _, Some(idx)) if i == idx => "failed",
                (_, true, _, _) => "pending",
                (_, _, true, Some(idx)) if i < idx => "passed",
                (_, _, true, Some(idx)) if i == idx => "blocked",
                (_, _, _, Some(idx)) if i < idx => "passed",
                _ => "pending",
            };
            json!({
                "index": i + 1,
                "executor_kind": m.executor_kind,
                "status": status,
            })
        })
        .collect();

    let run_status = if released {
        "passed"
    } else if archived {
        "failed"
    } else if parked {
        "blocked"
    } else {
        "pending"
    };

    let spec = spec_link_for_row(row);
    let pr = pr_link_for_run(spine, portal, &row.artifact_id, &row.kind).await?;

    Ok(json!({
        "id": run_status_id(&row.artifact_id),
        "status": run_status,
        "stages": stage_entries,
        "spec": spec,
        "pr": pr,
        "started_at": row.created_at.to_rfc3339(),
        "updated_at": row.updated_at.to_rfc3339(),
    }))
}

/// Opaque, deterministic id for a run on the public surface. Drops the
/// internal `art_iss_<uuid>` shape so an evaluator can't infer artifact
/// kinds or DB ids from the URL. The 12-char prefix is plenty unique
/// across 10 runs.
fn run_status_id(artifact_id: &str) -> String {
    let h = fnv1a_hex(artifact_id);
    format!("run_{}", &h[..12])
}

/// FNV-1a 64-bit hash → lowercase hex. Non-cryptographic, deterministic,
/// std-only. Plenty unique for the 10-runs surface; opacity here is
/// "don't leak the internal `art_iss_<uuid>` shape", not adversarial
/// resistance.
fn fnv1a_hex(input: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug)]
struct SpecLink {
    project_id: Option<String>,
    issue_number: Option<u64>,
}

fn parse_external_ref(ext: &str) -> SpecLink {
    let mut parts = ext.splitn(5, ':');
    let _ = parts.next(); // "github"
    let _ = parts.next(); // "project"
    let project_id = parts.next().map(|s| s.to_string());
    let _kind = parts.next(); // "issue" | "pr"
    let number = parts.next().and_then(|s| s.parse::<u64>().ok());
    SpecLink {
        project_id,
        issue_number: number,
    }
}

fn spec_link_for_row(row: &RunRow) -> Value {
    let Some(ext) = row.external_ref.as_deref() else {
        return Value::Null;
    };
    if row.kind != "issue" {
        return Value::Null;
    }
    let link = parse_external_ref(ext);
    let (Some(_project_id), Some(number)) = (link.project_id.clone(), link.issue_number) else {
        return Value::Null;
    };
    let repo = repo_from_metadata(&row.metadata);
    let url = repo
        .as_deref()
        .map(|r| format!("https://github.com/{r}/issues/{number}"));
    json!({
        "number": number,
        "url": url,
    })
}

/// Best-effort `(owner, name)` extraction. The portal's webhook + spec
/// listeners stamp `project_id`/`issue_number` into metadata but not
/// the owner/name pair — the canonical mapping lives on the `projects`
/// row. For v1 we leave URL construction up to the dashboard via the
/// project lookup elsewhere; here we surface the link only when
/// metadata carries an explicit `repo: "owner/name"` hint.
fn repo_from_metadata(metadata: &Value) -> Option<String> {
    metadata
        .get("repo")
        .and_then(|v| v.as_str())
        .map(String::from)
}

async fn pr_link_for_run(
    spine: &PgPool,
    portal: &PgPool,
    run_artifact_id: &str,
    run_kind: &str,
) -> anyhow::Result<Value> {
    if run_kind != "issue" {
        // PR-keyed runs surface the PR directly via their own external_ref.
        return Ok(Value::Null);
    }
    // PR → issue linkage lives on `horizontal_lineage` (role='closes_issue').
    // The reverse lookup finds the PR opened against this run's spec.
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT hl.artifact_id \
           FROM horizontal_lineage hl \
          WHERE hl.source_artifact_id = $1 AND hl.role = 'closes_issue' \
          ORDER BY hl.id DESC LIMIT 1",
    )
    .bind(run_artifact_id)
    .fetch_optional(portal)
    .await?;
    let Some((pr_artifact_id,)) = row else {
        return Ok(Value::Null);
    };
    let pr: Option<(Option<String>, Value)> =
        sqlx::query_as("SELECT external_ref, metadata FROM artifacts WHERE artifact_id = $1")
            .bind(&pr_artifact_id)
            .fetch_optional(spine)
            .await?;
    let Some((ext, meta)) = pr else {
        return Ok(Value::Null);
    };
    let Some(ext) = ext else {
        return Ok(Value::Null);
    };
    let link = parse_external_ref(&ext);
    let Some(number) = link.issue_number else {
        return Ok(Value::Null);
    };
    let repo = repo_from_metadata(&meta);
    let url = repo
        .as_deref()
        .map(|r| format!("https://github.com/{r}/pull/{number}"));
    Ok(json!({
        "number": number,
        "url": url,
    }))
}

async fn compute_stats_7d(
    spine: &PgPool,
    workflow_id: &str,
    workspace_id: &str,
) -> anyhow::Result<Value> {
    // Specs shipped: runs of this workflow that reached `released`
    // in the last 7 days. One spec = one run = one released artifact.
    let specs_shipped: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT FROM artifacts \
          WHERE workflow_id = $1 \
            AND state = 'released' \
            AND updated_at >= NOW() - INTERVAL '7 days'",
    )
    .bind(workflow_id)
    .fetch_one(spine)
    .await?
    .get::<i64, _>(0);

    // PRs merged: `git.pr_merged` events scoped to the workflow's
    // workspace, last 7 days. Filter on the indexed `workspace_id`
    // column (migration 016) rather than a JSONB hint — the
    // `FactoryEventKind::GitPrMerged` payload carries
    // `(artifact_id, pr_number, merge_sha)` and no top-level
    // `workspace_id`, so the legacy JSONB predicate would always
    // miss. Workspace-level scoping is good enough for the dogfood
    // instance where the workspace hosts only this one workflow.
    let prs_merged: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT FROM events_ext \
          WHERE event_type = 'git.pr_merged' \
            AND workspace_id = $1 \
            AND created_at >= NOW() - INTERVAL '7 days'",
    )
    .bind(workspace_id)
    .fetch_one(spine)
    .await?
    .get::<i64, _>(0);

    // Verify gates passed: `synodic.gate_verdict` events whose inner
    // `GateVerdict` enum tag is `"allow"`. The enum lives at
    // `data.verdict.verdict` (FactoryEventKind serializes as
    // `{type, ..., verdict: <GateVerdict>}` and `GateVerdict` is
    // `#[serde(tag = "verdict")]`). Same indexed-column scoping as
    // above.
    let verify_passed: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT FROM events_ext \
          WHERE event_type = 'synodic.gate_verdict' \
            AND workspace_id = $1 \
            AND data->'verdict'->>'verdict' = 'allow' \
            AND created_at >= NOW() - INTERVAL '7 days'",
    )
    .bind(workspace_id)
    .fetch_one(spine)
    .await?
    .get::<i64, _>(0);

    Ok(json!({
        "specs_shipped": specs_shipped,
        "prs_merged": prs_merged,
        "verify_gates_passed": verify_passed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_payload_shape() {
        let body = disabled_payload();
        assert_eq!(body["enabled"], json!(false));
        assert_eq!(body["runs"], json!([]));
        assert_eq!(body["is_quiet"], json!(true));
        assert_eq!(body["stats_7d"]["specs_shipped"], json!(0));
        assert_eq!(body["stats_7d"]["prs_merged"], json!(0));
        assert_eq!(body["stats_7d"]["verify_gates_passed"], json!(0));
        assert!(body.get("generated_at").is_some());
    }

    #[test]
    fn parse_external_ref_extracts_pieces() {
        let link = parse_external_ref("github:project:proj_abc:issue:407");
        assert_eq!(link.project_id.as_deref(), Some("proj_abc"));
        assert_eq!(link.issue_number, Some(407));

        let link = parse_external_ref("github:project:proj_xyz:pr:412");
        assert_eq!(link.project_id.as_deref(), Some("proj_xyz"));
        assert_eq!(link.issue_number, Some(412));
    }

    #[test]
    fn parse_external_ref_handles_garbage() {
        let link = parse_external_ref("not-a-ref");
        assert!(link.project_id.is_none() || link.issue_number.is_none());
    }

    #[test]
    fn run_status_id_is_opaque_and_deterministic() {
        let a = run_status_id("art_iss_abcdef1234567890abcdef1234567890");
        let b = run_status_id("art_iss_abcdef1234567890abcdef1234567890");
        assert_eq!(a, b);
        assert!(a.starts_with("run_"));
        assert_eq!(a.len(), "run_".len() + 12);
        // Different inputs produce different outputs (probabilistically).
        let c = run_status_id("art_iss_0000000000000000000000000000ffff");
        assert_ne!(a, c);
    }

    #[test]
    fn spec_link_only_emits_for_issue_kind() {
        let row = RunRow {
            artifact_id: "art_pr_x".into(),
            kind: "pull_request".into(),
            state: "released".into(),
            current_stage_index: None,
            workflow_parked_reason: None,
            external_ref: Some("github:project:p1:pr:5".into()),
            metadata: json!({"repo": "onsager-ai/onsager"}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(spec_link_for_row(&row), Value::Null);
    }

    #[test]
    fn spec_link_constructs_url_when_repo_known() {
        let row = RunRow {
            artifact_id: "art_iss_x".into(),
            kind: "issue".into(),
            state: "released".into(),
            current_stage_index: None,
            workflow_parked_reason: None,
            external_ref: Some("github:project:p1:issue:407".into()),
            metadata: json!({"repo": "onsager-ai/onsager"}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let v = spec_link_for_row(&row);
        assert_eq!(v["number"], json!(407));
        assert_eq!(
            v["url"],
            json!("https://github.com/onsager-ai/onsager/issues/407")
        );
    }

    #[test]
    fn spec_link_null_when_repo_unknown() {
        // Without a `repo` hint we still emit the number so the
        // dashboard could resolve it later. URL stays null.
        let row = RunRow {
            artifact_id: "art_iss_x".into(),
            kind: "issue".into(),
            state: "released".into(),
            current_stage_index: None,
            workflow_parked_reason: None,
            external_ref: Some("github:project:p1:issue:407".into()),
            metadata: json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let v = spec_link_for_row(&row);
        assert_eq!(v["number"], json!(407));
        assert_eq!(v["url"], Value::Null);
    }
}
