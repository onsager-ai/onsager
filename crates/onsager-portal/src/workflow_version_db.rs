//! Workflow versioning CRUD against the spine `workflow_versions` /
//! `workflow_changes` / `workflow_edit_policies` tables (spec #337,
//! Phase 2 substrate).
//!
//! Portal is the sole writer of the workflow surface (Slice 4 of #222);
//! these tables follow the same convention.
//!
//! The module deliberately ships the substrate first: typed shapes,
//! create/publish/retire/revert CRUD, audit-row helper, and the
//! workspace edit-policy resolver. The dashboard surfaces (version
//! history panel, revert action, diff view) and the export/import
//! CLI/MCP tool are explicit follow-ups under #337, not silent scope
//! reduction.

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::str::FromStr;

use crate::workflow::Workflow;

// ── Public types ───────────────────────────────────────────────────────

/// Edit-mode policy for a workspace. `Direct` writes and publishes in
/// one step; `DraftThenPublish` parks the edit as a draft until an
/// explicit publish promotes it. Default for new workspaces is
/// `DraftThenPublish` (matches the spine `workflow_edit_policies`
/// column default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditMode {
    Direct,
    DraftThenPublish,
}

impl EditMode {
    pub fn as_str(self) -> &'static str {
        match self {
            EditMode::Direct => "direct",
            EditMode::DraftThenPublish => "draft_then_publish",
        }
    }
}

impl FromStr for EditMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s {
            "direct" => Ok(EditMode::Direct),
            "draft_then_publish" => Ok(EditMode::DraftThenPublish),
            other => anyhow::bail!("unknown edit_mode: {other}"),
        }
    }
}

/// One row of `workflow_versions`. `content` is the canonical JSONB
/// snapshot (trigger + stages today; the deferred composition fields
/// — `uses`, `needs`, `triggers_on_admit`, `composite_criteria` — will
/// extend it additively).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowVersion {
    pub version_id: String,
    pub workflow_id: String,
    pub version_label: String,
    pub content: serde_json::Value,
    pub parent_version_id: Option<String>,
    pub state: VersionState,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionState {
    Draft,
    Published,
    Retired,
}

impl VersionState {
    pub fn as_str(self) -> &'static str {
        match self {
            VersionState::Draft => "draft",
            VersionState::Published => "published",
            VersionState::Retired => "retired",
        }
    }
}

impl FromStr for VersionState {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s {
            "draft" => Ok(VersionState::Draft),
            "published" => Ok(VersionState::Published),
            "retired" => Ok(VersionState::Retired),
            other => anyhow::bail!("unknown version state: {other}"),
        }
    }
}

/// One row of `workflow_changes`. Carries before/after content
/// snapshots so a diff view can render without re-joining version
/// rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowChange {
    pub change_id: i64,
    pub workflow_id: String,
    pub version_id: Option<String>,
    pub actor: String,
    pub action: ChangeAction,
    pub before_content: Option<serde_json::Value>,
    pub after_content: Option<serde_json::Value>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    CreateDraft,
    EditDraft,
    Publish,
    Retire,
    Revert,
}

impl ChangeAction {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeAction::CreateDraft => "create_draft",
            ChangeAction::EditDraft => "edit_draft",
            ChangeAction::Publish => "publish",
            ChangeAction::Retire => "retire",
            ChangeAction::Revert => "revert",
        }
    }
}

impl FromStr for ChangeAction {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s {
            "create_draft" => Ok(ChangeAction::CreateDraft),
            "edit_draft" => Ok(ChangeAction::EditDraft),
            "publish" => Ok(ChangeAction::Publish),
            "retire" => Ok(ChangeAction::Retire),
            "revert" => Ok(ChangeAction::Revert),
            other => anyhow::bail!("unknown change action: {other}"),
        }
    }
}

/// Canonical content shape for a workflow version. JSONB on the wire;
/// this is the typed view for callers that want to construct/inspect
/// snapshots without poking at raw JSON.
///
/// Forward-compatible: the deferred composition fields (`uses`,
/// `needs`, `triggers_on_admit`, `composite_criteria`) attach to the
/// per-stage entries and to the top-level snapshot when they land.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalContent {
    pub id: String,
    pub name: String,
    pub workspace_id: String,
    pub install_id: String,
    pub trigger: CanonicalTrigger,
    #[serde(default)]
    pub stages: Vec<CanonicalStage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalTrigger {
    pub kind: String,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalStage {
    pub stage_order: i32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_state: Option<String>,
    pub gates: serde_json::Value,
    pub params: serde_json::Value,
}

// ── Snapshot builder ───────────────────────────────────────────────────

/// Build a canonical JSONB snapshot from the spine `workflows` +
/// `workflow_stages` rows in scope. This is the same shape the
/// migration-025 backfill produces, so v1 versions from existing
/// workflows and v1 versions from new workflows are byte-equivalent
/// for the same input.
pub fn build_snapshot(
    workflow: &Workflow,
    stage_rows: &[(
        i32,
        &str,
        Option<&str>,
        &serde_json::Value,
        &serde_json::Value,
    )],
) -> serde_json::Value {
    let (trigger_kind, trigger_config) = workflow.trigger.to_storage();
    let stages: Vec<serde_json::Value> = stage_rows
        .iter()
        .map(|(order, name, target_state, gates, params)| {
            let mut obj = serde_json::Map::new();
            obj.insert("stage_order".into(), serde_json::json!(order));
            obj.insert("name".into(), serde_json::json!(name));
            obj.insert("target_state".into(), serde_json::json!(target_state));
            obj.insert("gates".into(), (*gates).clone());
            obj.insert("params".into(), (*params).clone());
            serde_json::Value::Object(obj)
        })
        .collect();
    serde_json::json!({
        "id": workflow.id,
        "name": workflow.name,
        "workspace_id": workflow.workspace_id,
        "install_id": workflow.install_id.to_string(),
        "trigger": {
            "kind": trigger_kind,
            "config": trigger_config,
        },
        "stages": stages,
    })
}

// ── Versions CRUD ──────────────────────────────────────────────────────

/// Insert a new version row. Caller chooses the state (draft for chat
/// edits under `draft_then_publish`, published for `direct` or v1
/// creation) and is responsible for writing the matching
/// `workflow_changes` audit row in the same logical step.
#[allow(clippy::too_many_arguments)]
pub async fn insert_version(
    pool: &PgPool,
    version_id: &str,
    workflow_id: &str,
    version_label: &str,
    content: &serde_json::Value,
    parent_version_id: Option<&str>,
    state: VersionState,
    created_by: &str,
) -> anyhow::Result<WorkflowVersion> {
    let published_at = match state {
        VersionState::Published => Some(Utc::now()),
        _ => None,
    };
    let row = sqlx::query(
        "INSERT INTO workflow_versions ( \
            version_id, workflow_id, version_label, content, \
            parent_version_id, state, created_by, published_at \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         RETURNING version_id, workflow_id, version_label, content, \
                   parent_version_id, state, created_by, created_at, \
                   published_at",
    )
    .bind(version_id)
    .bind(workflow_id)
    .bind(version_label)
    .bind(content)
    .bind(parent_version_id)
    .bind(state.as_str())
    .bind(created_by)
    .bind(published_at)
    .fetch_one(pool)
    .await
    .context("insert workflow_versions row")?;
    row_to_version(row)
}

/// Mark a version `published` and set `published_at`. Caller is
/// responsible for ensuring this is the new active version (typically
/// followed by [`set_active_version`] and a retire of the prior
/// active version).
pub async fn publish_version(pool: &PgPool, version_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE workflow_versions \
            SET state = 'published', \
                published_at = COALESCE(published_at, NOW()) \
          WHERE version_id = $1 \
            AND state = 'draft'",
    )
    .bind(version_id)
    .execute(pool)
    .await
    .context("publish workflow_versions row")?;
    Ok(())
}

/// Mark a version `retired`. Retired versions are still readable for
/// audit / diff purposes; they're just not the active version.
pub async fn retire_version(pool: &PgPool, version_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE workflow_versions \
            SET state = 'retired' \
          WHERE version_id = $1 \
            AND state = 'published'",
    )
    .bind(version_id)
    .execute(pool)
    .await
    .context("retire workflow_versions row")?;
    Ok(())
}

/// Update the workflow's pointer to its currently active version.
pub async fn set_active_version(
    pool: &PgPool,
    workflow_id: &str,
    version_id: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE workflows \
            SET active_version_id = $1 \
          WHERE workflow_id = $2",
    )
    .bind(version_id)
    .bind(workflow_id)
    .execute(pool)
    .await
    .context("set workflows.active_version_id")?;
    Ok(())
}

pub async fn get_version(
    pool: &PgPool,
    version_id: &str,
) -> anyhow::Result<Option<WorkflowVersion>> {
    let row = sqlx::query(
        "SELECT version_id, workflow_id, version_label, content, \
                parent_version_id, state, created_by, created_at, \
                published_at \
           FROM workflow_versions WHERE version_id = $1",
    )
    .bind(version_id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_version).transpose()
}

pub async fn list_versions_for_workflow(
    pool: &PgPool,
    workflow_id: &str,
) -> anyhow::Result<Vec<WorkflowVersion>> {
    let rows = sqlx::query(
        "SELECT version_id, workflow_id, version_label, content, \
                parent_version_id, state, created_by, created_at, \
                published_at \
           FROM workflow_versions \
          WHERE workflow_id = $1 \
          ORDER BY created_at ASC",
    )
    .bind(workflow_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_version).collect()
}

/// Look up the active (published) version for a workflow via the
/// `workflows.active_version_id` pointer. Returns `None` if no
/// workflow row, or no active version yet pointed to.
pub async fn get_active_version_for_workflow(
    pool: &PgPool,
    workflow_id: &str,
) -> anyhow::Result<Option<WorkflowVersion>> {
    let version_id: Option<String> =
        sqlx::query_scalar("SELECT active_version_id FROM workflows WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_optional(pool)
            .await?
            .flatten();
    match version_id {
        Some(vid) => get_version(pool, &vid).await,
        None => Ok(None),
    }
}

// ── Changes (audit) CRUD ───────────────────────────────────────────────

/// Append a row to `workflow_changes`. Action-specific snapshot fields
/// are caller-supplied so the audit row exactly reflects what changed
/// without re-deriving from the version table.
#[allow(clippy::too_many_arguments)]
pub async fn insert_change(
    pool: &PgPool,
    workflow_id: &str,
    version_id: Option<&str>,
    actor: &str,
    action: ChangeAction,
    before_content: Option<&serde_json::Value>,
    after_content: Option<&serde_json::Value>,
    reason: Option<&str>,
) -> anyhow::Result<i64> {
    let change_id: i64 = sqlx::query_scalar(
        "INSERT INTO workflow_changes ( \
            workflow_id, version_id, actor, action, \
            before_content, after_content, reason \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7) \
         RETURNING change_id",
    )
    .bind(workflow_id)
    .bind(version_id)
    .bind(actor)
    .bind(action.as_str())
    .bind(before_content)
    .bind(after_content)
    .bind(reason)
    .fetch_one(pool)
    .await
    .context("insert workflow_changes row")?;
    Ok(change_id)
}

pub async fn list_changes_for_workflow(
    pool: &PgPool,
    workflow_id: &str,
    limit: i64,
) -> anyhow::Result<Vec<WorkflowChange>> {
    let rows = sqlx::query(
        "SELECT change_id, workflow_id, version_id, actor, action, \
                before_content, after_content, reason, created_at \
           FROM workflow_changes \
          WHERE workflow_id = $1 \
          ORDER BY created_at DESC, change_id DESC \
          LIMIT $2",
    )
    .bind(workflow_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_change).collect()
}

// ── Edit policies ──────────────────────────────────────────────────────

/// Resolve a workspace's edit mode. Defaults to `DraftThenPublish`
/// when no row is present — the same default the schema applies for
/// new workspaces.
pub async fn get_edit_mode(pool: &PgPool, workspace_id: &str) -> anyhow::Result<EditMode> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT edit_mode FROM workflow_edit_policies WHERE workspace_id = $1")
            .bind(workspace_id)
            .fetch_optional(pool)
            .await?;
    match raw {
        Some(s) => EditMode::from_str(&s),
        None => Ok(EditMode::DraftThenPublish),
    }
}

/// Set (upsert) a workspace's edit mode. The application-layer
/// invariant — "can't weaken while in-flight runs exist" — is
/// enforced by the caller because it requires joining against
/// `artifacts` to know what's in flight.
pub async fn set_edit_mode(
    pool: &PgPool,
    workspace_id: &str,
    mode: EditMode,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO workflow_edit_policies (workspace_id, edit_mode, updated_at) \
              VALUES ($1, $2, NOW()) \
         ON CONFLICT (workspace_id) DO UPDATE \
            SET edit_mode = EXCLUDED.edit_mode, \
                updated_at = NOW()",
    )
    .bind(workspace_id)
    .bind(mode.as_str())
    .execute(pool)
    .await
    .context("upsert workflow_edit_policies row")?;
    Ok(())
}

// ── Row → struct helpers ───────────────────────────────────────────────

fn row_to_version(row: sqlx::postgres::PgRow) -> anyhow::Result<WorkflowVersion> {
    let state_raw: String = row.try_get("state")?;
    Ok(WorkflowVersion {
        version_id: row.try_get("version_id")?,
        workflow_id: row.try_get("workflow_id")?,
        version_label: row.try_get("version_label")?,
        content: row.try_get("content")?,
        parent_version_id: row.try_get("parent_version_id")?,
        state: VersionState::from_str(&state_raw)?,
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
        published_at: row.try_get("published_at")?,
    })
}

fn row_to_change(row: sqlx::postgres::PgRow) -> anyhow::Result<WorkflowChange> {
    let action_raw: String = row.try_get("action")?;
    Ok(WorkflowChange {
        change_id: row.try_get("change_id")?,
        workflow_id: row.try_get("workflow_id")?,
        version_id: row.try_get("version_id")?,
        actor: row.try_get("actor")?,
        action: ChangeAction::from_str(&action_raw)?,
        before_content: row.try_get("before_content")?,
        after_content: row.try_get("after_content")?,
        reason: row.try_get("reason")?,
        created_at: row.try_get("created_at")?,
    })
}

// ── ID synthesis ───────────────────────────────────────────────────────

/// Stable v1 version id for a workflow — matches the deterministic
/// `wfv_<workflow_id>` pattern used by migration 025's backfill so a
/// freshly-created v1 version and a backfilled v1 version share an
/// id when keyed by the same workflow.
pub fn v1_version_id(workflow_id: &str) -> String {
    format!("wfv_{workflow_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::Workflow;
    use chrono::TimeZone;
    use onsager_spine::TriggerKind;

    fn sample_workflow() -> Workflow {
        Workflow {
            id: "wf_abc".into(),
            workspace_id: "ws_1".into(),
            name: "demo".into(),
            trigger: TriggerKind::GithubIssueWebhook {
                repo: "owner/repo".into(),
                label: "planned".into(),
            },
            install_id: 42,
            preset_id: None,
            active: false,
            created_by: "user-1".into(),
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn snapshot_roundtrips_through_canonical_content() {
        let wf = sample_workflow();
        let gates = serde_json::json!([{"kind": "agent_session"}]);
        let params = serde_json::json!({"shaping_intent": {}});
        let stage_rows = vec![(0_i32, "agent-session", Some("in_progress"), &gates, &params)];
        let snap = build_snapshot(&wf, &stage_rows);

        let typed: CanonicalContent = serde_json::from_value(snap.clone()).unwrap();
        assert_eq!(typed.id, "wf_abc");
        assert_eq!(typed.name, "demo");
        assert_eq!(typed.workspace_id, "ws_1");
        assert_eq!(typed.install_id, "42");
        assert_eq!(typed.trigger.kind, "github_issue_webhook");
        assert_eq!(typed.trigger.config["repo"], "owner/repo");
        assert_eq!(typed.stages.len(), 1);
        assert_eq!(typed.stages[0].stage_order, 0);
        assert_eq!(typed.stages[0].name, "agent-session");
        assert_eq!(typed.stages[0].target_state.as_deref(), Some("in_progress"));
    }

    #[test]
    fn v1_version_id_is_deterministic() {
        assert_eq!(v1_version_id("wf_abc"), "wfv_wf_abc");
        assert_eq!(v1_version_id("wf_abc"), v1_version_id("wf_abc"));
    }

    #[test]
    fn edit_mode_round_trip() {
        for m in [EditMode::Direct, EditMode::DraftThenPublish] {
            assert_eq!(EditMode::from_str(m.as_str()).unwrap(), m);
        }
        assert!(EditMode::from_str("nope").is_err());
    }

    #[test]
    fn version_state_round_trip() {
        for s in [
            VersionState::Draft,
            VersionState::Published,
            VersionState::Retired,
        ] {
            assert_eq!(VersionState::from_str(s.as_str()).unwrap(), s);
        }
        assert!(VersionState::from_str("garbage").is_err());
    }

    #[test]
    fn change_action_round_trip() {
        for a in [
            ChangeAction::CreateDraft,
            ChangeAction::EditDraft,
            ChangeAction::Publish,
            ChangeAction::Retire,
            ChangeAction::Revert,
        ] {
            assert_eq!(ChangeAction::from_str(a.as_str()).unwrap(), a);
        }
        assert!(ChangeAction::from_str("squish").is_err());
    }
}
