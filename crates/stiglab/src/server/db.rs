use std::time::Duration;

use crate::core::{
    GitHubAppInstallation, Node, NodeStatus, Project, Session, SessionState, User, Workspace,
    WorkspaceMember,
};
use chrono::Utc;
use sqlx::pool::PoolOptions;
use sqlx::AnyPool;

pub async fn init_pool(database_url: &str) -> anyhow::Result<AnyPool> {
    // For SQLite: ensure parent directory exists and enable create-if-missing
    let connect_url = if database_url.starts_with("sqlite://") {
        let path = database_url.trim_start_matches("sqlite://");
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        // Append mode=rwc so SQLx creates the file if it doesn't exist
        if database_url.contains('?') {
            format!("{database_url}&mode=rwc")
        } else {
            format!("{database_url}?mode=rwc")
        }
    } else {
        database_url.to_string()
    };

    // Install drivers
    sqlx::any::install_default_drivers();

    let pool = tokio::time::timeout(
        Duration::from_secs(10),
        PoolOptions::new()
            .acquire_timeout(Duration::from_secs(10))
            .connect(&connect_url),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timed out while connecting to database"))??;
    run_migrations(&pool).await?;
    Ok(pool)
}

pub async fn run_migrations(pool: &AnyPool) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS nodes (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            hostname TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'online',
            max_sessions INTEGER NOT NULL DEFAULT 4,
            active_sessions INTEGER NOT NULL DEFAULT 0,
            last_heartbeat TEXT NOT NULL,
            registered_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL,
            node_id TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'pending',
            prompt TEXT NOT NULL,
            output TEXT,
            working_dir TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS session_logs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            chunk TEXT NOT NULL,
            stream TEXT NOT NULL DEFAULT 'stdout',
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_session_logs_session_id ON session_logs (session_id, seq)",
    )
    .execute(pool)
    .await?;

    // ── Auth tables ──

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            github_id BIGINT NOT NULL UNIQUE,
            github_login TEXT NOT NULL,
            github_name TEXT,
            github_avatar_url TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS auth_sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_auth_sessions_user_id ON auth_sessions (user_id)")
        .execute(pool)
        .await?;

    // Short-lived opaque codes used by cross-environment SSO delegation.
    // `redeemed_at` is NULL until a relying party successfully exchanges
    // the code for the user identity; the UPDATE that flips it is the
    // single-use gate (see `redeem_sso_exchange_code`).
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS sso_exchange_codes (
            code TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            return_to_host TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            redeemed_at TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_sso_exchange_codes_expires_at \
         ON sso_exchange_codes (expires_at)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS user_credentials (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            workspace_id TEXT,
            name TEXT NOT NULL,
            encrypted_value TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(user_id, workspace_id, name)
        )",
    )
    .execute(pool)
    .await?;

    // Personal Access Tokens (issue #143). Server-issued bearer tokens that
    // authenticate non-browser callers as a specific user. The full token is
    // never stored — `token_hash` is the SHA-256 hex of the raw token, and
    // `token_prefix` is the first 12 characters for display + indexed lookup.
    // `workspace_id` is the workspace this PAT addresses; required since
    // issue #163 (renamed from the legacy `tenant_id`).  Cross-workspace
    // calls 403.  `revoked_at` is a soft-delete: revoked rows stay for audit
    // but fail verification.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS user_pats (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            workspace_id TEXT NOT NULL,
            name TEXT NOT NULL,
            token_prefix TEXT NOT NULL,
            token_hash TEXT NOT NULL,
            scopes TEXT NOT NULL DEFAULT '[\"*\"]',
            expires_at TEXT,
            last_used_at TEXT,
            last_used_ip TEXT,
            last_used_user_agent TEXT,
            created_at TEXT NOT NULL,
            revoked_at TEXT,
            UNIQUE(user_id, name)
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_user_pats_user_id ON user_pats (user_id)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_user_pats_token_prefix ON user_pats (token_prefix)",
    )
    .execute(pool)
    .await?;

    // Add user_id column to sessions if it doesn't exist.
    // Swallow duplicate-column errors on both SQLite and PostgreSQL.
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN user_id TEXT")
        .execute(pool)
        .await;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions (user_id)")
        .execute(pool)
        .await?;

    // Issue #14 phase 2: link sessions to the artifact they're shaping.
    // Try the ALTERs unconditionally; the errors are swallowed when the
    // columns already exist (both SQLite and Postgres return a distinct
    // error for duplicate columns, which we don't surface here).
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN artifact_id TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN artifact_version INTEGER")
        .execute(pool)
        .await;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_artifact_id ON sessions (artifact_id)")
        .execute(pool)
        .await?;

    // Issue #31: idempotency key for POST /api/shaping. Same swallow-on-
    // duplicate pattern as above for cross-backend ALTER compatibility.
    //
    // The index is UNIQUE so the database enforces at-most-one session per
    // key — the application lookup is just the fast path; concurrent inserts
    // with the same key are caught by a unique-violation at commit and
    // translated back to "return existing session". Both SQLite and Postgres
    // treat NULL values as distinct in a unique index, so sessions without an
    // idempotency key don't collide.
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN idempotency_key TEXT")
        .execute(pool)
        .await;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_idempotency_key \
         ON sessions (idempotency_key)",
    )
    .execute(pool)
    .await?;

    // ── Workspace / membership / GitHub App / project tables (issue #59;
    //    renamed from "tenant" → "workspace" in issue #163).
    //
    // The block below reuses the same swallow-on-error pattern the earlier
    // ALTERs use.  On a fresh DB the rename statements all fail with "no
    // such table / column" and the subsequent CREATE TABLE IF NOT EXISTS
    // statements build the schema directly under the new names.  On an
    // existing DB the renames run once, the CREATE TABLE IF NOT EXISTS
    // statements are no-ops, and the column-rename ALTERs realign the
    // surface.  See `migrations/001_rename_tenant_to_workspace.sql` for the
    // canonical SQL; the deletion rule for `user_pats` rows with no
    // `workspace_members` membership is documented there.
    let _ = sqlx::query("ALTER TABLE tenants RENAME TO workspaces")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tenant_members RENAME TO workspace_members")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tenant_workflows RENAME TO workspace_workflows")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tenant_workflow_stages RENAME TO workspace_workflow_stages")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE workspace_members RENAME COLUMN tenant_id TO workspace_id")
        .execute(pool)
        .await;
    let _ =
        sqlx::query("ALTER TABLE github_app_installations RENAME COLUMN tenant_id TO workspace_id")
            .execute(pool)
            .await;
    let _ = sqlx::query("ALTER TABLE projects RENAME COLUMN tenant_id TO workspace_id")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE workspace_workflows RENAME COLUMN tenant_id TO workspace_id")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE user_pats RENAME COLUMN tenant_id TO workspace_id")
        .execute(pool)
        .await;
    // Old indexes referencing the renamed tables/columns can outlive the
    // rename on Postgres if the index name doesn't auto-update; drop the
    // legacy ones explicitly.  All `IF EXISTS`, all idempotent.
    let _ = sqlx::query("DROP INDEX IF EXISTS idx_tenant_members_user_id")
        .execute(pool)
        .await;
    let _ = sqlx::query("DROP INDEX IF EXISTS idx_github_app_installations_tenant_id")
        .execute(pool)
        .await;
    let _ = sqlx::query("DROP INDEX IF EXISTS idx_projects_tenant_id")
        .execute(pool)
        .await;
    let _ = sqlx::query("DROP INDEX IF EXISTS idx_tenant_workflows_tenant_id")
        .execute(pool)
        .await;
    let _ = sqlx::query("DROP INDEX IF EXISTS idx_tenant_workflows_repo_active")
        .execute(pool)
        .await;
    let _ = sqlx::query("DROP INDEX IF EXISTS idx_tenant_workflow_stages_workflow_id")
        .execute(pool)
        .await;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS workspaces (
            id TEXT PRIMARY KEY,
            slug TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            created_by TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS workspace_members (
            workspace_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            joined_at TEXT NOT NULL,
            PRIMARY KEY (workspace_id, user_id)
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_workspace_members_user_id ON workspace_members (user_id)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS github_app_installations (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            install_id BIGINT NOT NULL UNIQUE,
            account_login TEXT NOT NULL,
            account_type TEXT NOT NULL,
            webhook_secret_cipher TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_github_app_installations_workspace_id \
         ON github_app_installations (workspace_id)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            github_app_installation_id TEXT NOT NULL,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            default_branch TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(workspace_id, repo_owner, repo_name)
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_projects_workspace_id ON projects (workspace_id)")
        .execute(pool)
        .await?;

    // PAT backfill: rows with NULL `workspace_id` get pinned to the user's
    // first workspace membership; rows with no membership at all are
    // deleted (unusable — a workspace-less PAT can't address any
    // workspace-scoped route, and after this migration `workspace_id` is
    // mandatory at the API layer).  Postgres-only `ALTER COLUMN ... SET
    // NOT NULL` is gated on the live install (SQLite tests rebuild the
    // schema each run from the new CREATE TABLE above).  See
    // `migrations/001_rename_tenant_to_workspace.sql` for the canonical
    // SQL.
    let _ = sqlx::query(
        "UPDATE user_pats \
         SET workspace_id = ( \
             SELECT m.workspace_id FROM workspace_members m \
             WHERE m.user_id = user_pats.user_id \
             ORDER BY m.joined_at ASC, m.workspace_id ASC \
             LIMIT 1 \
         ) \
         WHERE workspace_id IS NULL",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM user_pats WHERE workspace_id IS NULL")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE user_pats ALTER COLUMN workspace_id SET NOT NULL")
        .execute(pool)
        .await;

    // Attach sessions to projects (nullable; pre-existing sessions stay personal).
    // Same swallow-on-duplicate ALTER pattern as earlier migrations for
    // cross-backend compatibility.
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN project_id TEXT")
        .execute(pool)
        .await;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_project_id ON sessions (project_id)")
        .execute(pool)
        .await?;

    // Session↔PR correlation hand-off table (issue #60). Stiglab writes a row
    // at session completion; onsager-portal reads it on `pull_request.opened`
    // to attach vertical_lineage. The portal also creates this table at its
    // own startup — declaring it here guarantees stiglab never races the
    // portal's first migrate on a fresh deploy.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pr_branch_links (
            session_id  TEXT PRIMARY KEY,
            project_id  TEXT,
            branch      TEXT NOT NULL,
            pr_number   BIGINT,
            recorded_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_pr_branch_links_lookup \
         ON pr_branch_links (project_id, branch)",
    )
    .execute(pool)
    .await?;

    // ── Workflows (issue #81) ──────────────────────────────────────────
    //
    // `workspace_workflows` — declarative blueprint persisted per
    // workspace.  The trigger is currently GitHub-specific so `repo_owner`,
    // `repo_name`, `trigger_label`, and `install_id` are stored inline; a
    // second trigger kind would factor these out.
    //
    // `workspace_workflow_stages` — ordered stage chain; stages walk in
    // ascending `seq` order, never reorder.  `params` holds kind-specific
    // config as free-form JSON (TEXT for SQLite/Postgres portability
    // through AnyPool).
    //
    // No FK declarations — the rest of the stiglab schema uses app-layer
    // referential checks for the same SQLite/Postgres-portability reasons
    // documented above.
    //
    // Named `workspace_workflows` / `workspace_workflow_stages` (renamed
    // from `tenant_workflows` in #163) to keep the workspace-scoped
    // blueprint distinct from the onsager-spine `006_workflows.sql`
    // migration's `workflows` table — that table is the factory workflow
    // runtime; this one is the stiglab workspace-level blueprint store.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS workspace_workflows (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            name TEXT NOT NULL,
            trigger_kind TEXT NOT NULL,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            trigger_label TEXT NOT NULL,
            install_id BIGINT NOT NULL,
            preset_id TEXT,
            active INTEGER NOT NULL DEFAULT 0,
            created_by TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_workspace_workflows_workspace_id \
         ON workspace_workflows (workspace_id)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_workspace_workflows_repo_active \
         ON workspace_workflows (repo_owner, repo_name, active)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS workspace_workflow_stages (
            id TEXT PRIMARY KEY,
            workflow_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            gate_kind TEXT NOT NULL,
            params TEXT NOT NULL,
            UNIQUE(workflow_id, seq)
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_workspace_workflow_stages_workflow_id \
         ON workspace_workflow_stages (workflow_id, seq)",
    )
    .execute(pool)
    .await?;

    // ── Per-workspace credentials (issue #164).
    //
    // Credentials were originally global per (user, name).  Issue #164 ties
    // each credential row to a specific workspace so a user with access to
    // multiple workspaces no longer ships the same OAuth token to agent
    // sessions launched in unrelated workspaces.  See the canonical SQL at
    // `migrations/0NN_workspace_credentials.sql` (human documentation; the
    // live migration is the inlined DDL below).
    //
    // The CREATE TABLE above already declares the column on a fresh DB; the
    // ALTERs here cover existing installs.  Backfill rule mirrors the PAT
    // backfill from #163: rows get pinned to the user's first workspace
    // membership; rows for users with zero memberships are deleted (an
    // unscoped credential post-migration is unreachable from the new
    // route shape and would be permanently orphaned).
    let _ = sqlx::query("ALTER TABLE user_credentials ADD COLUMN workspace_id TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "UPDATE user_credentials \
         SET workspace_id = ( \
             SELECT m.workspace_id FROM workspace_members m \
             WHERE m.user_id = user_credentials.user_id \
             ORDER BY m.joined_at ASC, m.workspace_id ASC \
             LIMIT 1 \
         ) \
         WHERE workspace_id IS NULL",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM user_credentials WHERE workspace_id IS NULL")
        .execute(pool)
        .await;
    // Postgres-only NOT NULL tightening; SQLite tests rebuild fresh from the
    // CREATE TABLE above so the column is already typed correctly there.
    let _ = sqlx::query("ALTER TABLE user_credentials ALTER COLUMN workspace_id SET NOT NULL")
        .execute(pool)
        .await;
    // The legacy unique constraint was UNIQUE(user_id, name).  After the
    // workspace split, two different workspaces may legitimately host the
    // same credential name for the same user — drop the legacy index and
    // re-create it as UNIQUE(user_id, workspace_id, name).  The fresh-DB
    // CREATE TABLE above already uses the new shape.
    let _ = sqlx::query(
        "ALTER TABLE user_credentials DROP CONSTRAINT user_credentials_user_id_name_key",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("DROP INDEX IF EXISTS user_credentials_user_id_name_key")
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_user_credentials_user_workspace_name \
         ON user_credentials (user_id, workspace_id, name)",
    )
    .execute(pool)
    .await;

    // ── Sessions: workspace_id (issue #164).
    //
    // Sessions were workspace-scoped only transitively, via the optional
    // `project_id` column.  That left direct/personal sessions unfilterable
    // and made the parent #161 contract test (W1 user must not see W2
    // sessions) unprovable.  Backfill from the project's workspace where
    // possible; sessions with no project stay NULL and the API filter just
    // doesn't surface them in any workspace-scoped list.
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN workspace_id TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "UPDATE sessions \
         SET workspace_id = ( \
             SELECT p.workspace_id FROM projects p \
             WHERE p.id = sessions.project_id \
         ) \
         WHERE workspace_id IS NULL AND project_id IS NOT NULL",
    )
    .execute(pool)
    .await;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_sessions_workspace_id \
         ON sessions (workspace_id)",
    )
    .execute(pool)
    .await?;

    // ── github_app_installations: enforce 1:1 install_id → workspace_id
    //    (issue #164).
    //
    // The fresh-DB CREATE TABLE above already declares
    // `install_id BIGINT NOT NULL UNIQUE`, so this is a no-op for new
    // deploys.  Older Postgres installs may have shipped without the
    // UNIQUE — re-installing the same GitHub App into a different
    // workspace must be a re-install flow, not a row update, and the
    // database is the right place to enforce that contract.
    let _ = sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_github_app_installations_install_id \
         ON github_app_installations (install_id)",
    )
    .execute(pool)
    .await;

    Ok(())
}

// ── Node CRUD ──

pub async fn upsert_node(pool: &AnyPool, node: &Node) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO nodes (id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT(id) DO UPDATE SET
            name = $2, hostname = $3, status = $4, max_sessions = $5,
            active_sessions = $6, last_heartbeat = $7",
    )
    .bind(&node.id)
    .bind(&node.name)
    .bind(&node.hostname)
    .bind(node.status.to_string())
    .bind(node.max_sessions as i32)
    .bind(node.active_sessions as i32)
    .bind(node.last_heartbeat.to_rfc3339())
    .bind(node.registered_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_nodes(pool: &AnyPool) -> anyhow::Result<Vec<Node>> {
    let rows = sqlx::query_as::<_, NodeRow>("SELECT id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at FROM nodes")
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn update_node_heartbeat(
    pool: &AnyPool,
    node_id: &str,
    active_sessions: u32,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE nodes SET last_heartbeat = $1, active_sessions = $2 WHERE id = $3")
        .bind(Utc::now().to_rfc3339())
        .bind(active_sessions as i32)
        .bind(node_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_node_status(
    pool: &AnyPool,
    node_id: &str,
    status: NodeStatus,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE nodes SET status = $1 WHERE id = $2")
        .bind(status.to_string())
        .bind(node_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_least_loaded_node(pool: &AnyPool) -> anyhow::Result<Option<Node>> {
    // Exclude stale nodes (no heartbeat in last 2 minutes) to avoid dispatching
    // to dead nodes whose hostname changed across redeploys.
    let cutoff = (Utc::now() - chrono::Duration::seconds(120)).to_rfc3339();
    let row = sqlx::query_as::<_, NodeRow>(
        "SELECT id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at
         FROM nodes
         WHERE status = 'online' AND active_sessions < max_sessions AND last_heartbeat > $1
         ORDER BY CAST(active_sessions AS REAL) / CAST(max_sessions AS REAL) ASC
         LIMIT 1",
    )
    .bind(&cutoff)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn find_node_by_name(pool: &AnyPool, name: &str) -> anyhow::Result<Option<Node>> {
    let row = sqlx::query_as::<_, NodeRow>(
        "SELECT id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at FROM nodes WHERE name = $1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn get_node(pool: &AnyPool, node_id: &str) -> anyhow::Result<Option<Node>> {
    let row = sqlx::query_as::<_, NodeRow>(
        "SELECT id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at FROM nodes WHERE id = $1",
    )
    .bind(node_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

// ── Session CRUD ──

pub async fn insert_session(pool: &AnyPool, session: &Session) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               artifact_id, artifact_version, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_sessions(pool: &AnyPool) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// Sessions assigned to `node_id` that were created while the agent was
/// not connected (state still `Pending`). The agent registration handler
/// drains these on (re)connect so a session created during a brief
/// disconnect doesn't sit in `Pending` forever.
pub async fn list_pending_sessions_for_node(
    pool: &AnyPool,
    node_id: &str,
) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE node_id = $1 AND state = 'pending' \
         ORDER BY created_at ASC",
    )
    .bind(node_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_session(pool: &AnyPool, session_id: &str) -> anyhow::Result<Option<Session>> {
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

/// Look up an existing session by its idempotency key.
///
/// `request_id` from a `ShapingRequest` (or the `Idempotency-Key` header) is
/// used as the key so that a Forge retry on a dropped connection collapses
/// onto the original session instead of dispatching a second agent
/// (issue #31).
pub async fn find_session_by_idempotency_key(
    pool: &AnyPool,
    key: &str,
) -> anyhow::Result<Option<Session>> {
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE idempotency_key = $1 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

/// Insert a session bound to an idempotency key.
///
/// Returns `Ok(true)` on a fresh insert, `Ok(false)` when a row with the same
/// key already existed and the insert was skipped (via `ON CONFLICT DO
/// NOTHING`). Callers should re-lookup on `false` to recover the winning
/// session id.
///
/// The database's unique index on `idempotency_key` is the authoritative
/// guard against concurrent POSTs with the same key — the lookup-before-
/// insert path in the handler is a fast path, not a correctness barrier.
pub async fn insert_session_with_idempotency_key(
    pool: &AnyPool,
    session: &Session,
    idempotency_key: &str,
) -> anyhow::Result<bool> {
    insert_session_with_idempotency_key_and_workspace(pool, session, idempotency_key, None, None)
        .await
}

/// Variant of `insert_session_with_idempotency_key` that also persists
/// `user_id` and `workspace_id` (issue #164).  The shaping path uses
/// this so workspace-scoped session listing has a non-NULL column to
/// filter on, and so credential lookup at agent dispatch can run
/// against the same workspace.
pub async fn insert_session_with_idempotency_key_and_workspace(
    pool: &AnyPool,
    session: &Session,
    idempotency_key: &str,
    user_id: Option<&str>,
    workspace_id: Option<&str>,
) -> anyhow::Result<bool> {
    let affected = sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               user_id, workspace_id, artifact_id, artifact_version, \
                               idempotency_key, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         ON CONFLICT (idempotency_key) DO NOTHING",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(user_id)
    .bind(workspace_id)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(idempotency_key)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn update_session_state(
    pool: &AnyPool,
    session_id: &str,
    state: SessionState,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE sessions SET state = $1, updated_at = $2 WHERE id = $3")
        .bind(state.to_string())
        .bind(Utc::now().to_rfc3339())
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically claim a `pending` session by transitioning it to the given
/// state. Returns `true` only when this caller won the race (the row was
/// `pending` at update time and exactly one row was affected).
///
/// Used by the agent reconnect drain so a session that's already been
/// claimed by another path (e.g. the create-time WebSocket dispatch, or
/// a parallel reconnect that beat us) doesn't get sent twice.
pub async fn claim_pending_session(
    pool: &AnyPool,
    session_id: &str,
    new_state: SessionState,
) -> anyhow::Result<bool> {
    let affected = sqlx::query(
        "UPDATE sessions SET state = $1, updated_at = $2 \
         WHERE id = $3 AND state = 'pending'",
    )
    .bind(new_state.to_string())
    .bind(Utc::now().to_rfc3339())
    .bind(session_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

// ── Session Logs (append-only) ──

pub async fn append_session_log(
    pool: &AnyPool,
    session_id: &str,
    chunk: &str,
    stream: &str,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    // Use a subquery to get the next sequence number for this session
    sqlx::query(
        "INSERT INTO session_logs (id, session_id, seq, chunk, stream, created_at)
         VALUES ($1, $2, COALESCE((SELECT MAX(seq) FROM session_logs WHERE session_id = $2), 0) + 1, $3, $4, $5)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(chunk)
    .bind(stream)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Get all log chunks for a session, ordered by sequence number.
pub async fn get_session_logs(pool: &AnyPool, session_id: &str) -> anyhow::Result<Vec<LogChunk>> {
    let rows = sqlx::query_as::<_, LogChunkRow>(
        "SELECT chunk, stream, created_at FROM session_logs WHERE session_id = $1 ORDER BY seq ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// Get log chunks added after a given sequence number (for incremental SSE).
pub async fn get_session_logs_after(
    pool: &AnyPool,
    session_id: &str,
    after_seq: i64,
) -> anyhow::Result<Vec<LogChunkWithSeq>> {
    let rows = sqlx::query_as::<_, LogChunkWithSeqRow>(
        "SELECT seq, chunk, stream, created_at FROM session_logs WHERE session_id = $1 AND seq > $2 ORDER BY seq ASC",
    )
    .bind(session_id)
    .bind(after_seq)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

#[allow(dead_code)]
pub struct LogChunk {
    pub chunk: String,
    pub stream: String,
    pub created_at: String,
}

#[allow(dead_code)]
pub struct LogChunkWithSeq {
    pub seq: i64,
    pub chunk: String,
    pub stream: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct LogChunkRow {
    chunk: String,
    stream: String,
    created_at: String,
}

impl From<LogChunkRow> for LogChunk {
    fn from(row: LogChunkRow) -> Self {
        LogChunk {
            chunk: row.chunk,
            stream: row.stream,
            created_at: row.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct LogChunkWithSeqRow {
    seq: i32,
    chunk: String,
    stream: String,
    created_at: String,
}

impl From<LogChunkWithSeqRow> for LogChunkWithSeq {
    fn from(row: LogChunkWithSeqRow) -> Self {
        LogChunkWithSeq {
            seq: row.seq as i64,
            chunk: row.chunk,
            stream: row.stream,
            created_at: row.created_at,
        }
    }
}

// ── User CRUD ──

pub async fn upsert_user(pool: &AnyPool, user: &User) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT(github_id) DO UPDATE SET
            github_login = $3, github_name = $4, github_avatar_url = $5, updated_at = $7",
    )
    .bind(&user.id)
    .bind(user.github_id)
    .bind(&user.github_login)
    .bind(&user.github_name)
    .bind(&user.github_avatar_url)
    .bind(user.created_at.to_rfc3339())
    .bind(user.updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_user_by_github_id(pool: &AnyPool, github_id: i64) -> anyhow::Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at FROM users WHERE github_id = $1",
    )
    .bind(github_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn get_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

// ── Auth Session CRUD ──

pub struct AuthSession {
    pub id: String,
    pub user_id: String,
    pub user: User,
    pub expires_at: chrono::DateTime<Utc>,
    pub created_at: chrono::DateTime<Utc>,
}

pub async fn create_auth_session(
    pool: &AnyPool,
    session_id: &str,
    user_id: &str,
    expires_at: chrono::DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO auth_sessions (id, user_id, expires_at, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(expires_at.to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_auth_session(
    pool: &AnyPool,
    session_id: &str,
) -> anyhow::Result<Option<AuthSession>> {
    let row = sqlx::query_as::<_, AuthSessionRow>(
        "SELECT a.id, a.user_id, a.expires_at, a.created_at,
                u.github_id, u.github_login, u.github_name, u.github_avatar_url,
                u.created_at as user_created_at, u.updated_at as user_updated_at
         FROM auth_sessions a JOIN users u ON a.user_id = u.id
         WHERE a.id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else { return Ok(None) };

    let expires_at = chrono::DateTime::parse_from_rfc3339(&row.expires_at)?.with_timezone(&Utc);
    if expires_at < Utc::now() {
        // Expired — clean up and return None
        let _ = delete_auth_session(pool, session_id).await;
        return Ok(None);
    }

    let user = User {
        id: row.user_id.clone(),
        github_id: row.github_id,
        github_login: row.github_login,
        github_name: row.github_name,
        github_avatar_url: row.github_avatar_url,
        created_at: chrono::DateTime::parse_from_rfc3339(&row.user_created_at)?.with_timezone(&Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&row.user_updated_at)?.with_timezone(&Utc),
    };

    Ok(Some(AuthSession {
        id: row.id,
        user_id: row.user_id,
        user,
        expires_at,
        created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
    }))
}

pub async fn delete_auth_session(pool: &AnyPool, session_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM auth_sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── SSO Exchange Codes (cross-env delegation) ──

pub async fn insert_sso_exchange_code(
    pool: &AnyPool,
    code: &str,
    user_id: &str,
    return_to_host: &str,
    expires_at: chrono::DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sso_exchange_codes (code, user_id, return_to_host, expires_at, redeemed_at, created_at)
         VALUES ($1, $2, $3, $4, NULL, $5)",
    )
    .bind(code)
    .bind(user_id)
    .bind(return_to_host)
    .bind(expires_at.to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Atomically consume an exchange code. The UPDATE is the single-use gate —
/// it succeeds for exactly one caller (the one who sees `rows_affected == 1`);
/// concurrent or repeat calls get `None`. The return-to-host check runs in the
/// UPDATE predicate so a code issued for host A can't be redeemed by host B
/// even if both are in the owner's allowlist.
pub async fn redeem_sso_exchange_code(
    pool: &AnyPool,
    code: &str,
    return_to_host: &str,
) -> anyhow::Result<Option<User>> {
    let now = Utc::now().to_rfc3339();
    let rows = sqlx::query(
        "UPDATE sso_exchange_codes
         SET redeemed_at = $1
         WHERE code = $2
           AND redeemed_at IS NULL
           AND expires_at > $1
           AND return_to_host = $3",
    )
    .bind(&now)
    .bind(code)
    .bind(return_to_host)
    .execute(pool)
    .await?;

    if rows.rows_affected() == 0 {
        return Ok(None);
    }

    let user_id: String =
        sqlx::query_scalar("SELECT user_id FROM sso_exchange_codes WHERE code = $1")
            .bind(code)
            .fetch_one(pool)
            .await?;
    get_user(pool, &user_id).await
}

// ── User Credentials CRUD ──

pub struct UserCredential {
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn set_user_credential(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: &str,
    name: &str,
    encrypted_value: &str,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO user_credentials \
            (id, user_id, workspace_id, name, encrypted_value, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $6) \
         ON CONFLICT(user_id, workspace_id, name) \
            DO UPDATE SET encrypted_value = $5, updated_at = $6",
    )
    .bind(&id)
    .bind(user_id)
    .bind(workspace_id)
    .bind(name)
    .bind(encrypted_value)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_user_credentials(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: &str,
) -> anyhow::Result<Vec<UserCredential>> {
    let rows = sqlx::query_as::<_, UserCredentialRow>(
        "SELECT name, created_at, updated_at FROM user_credentials \
         WHERE user_id = $1 AND workspace_id = $2 ORDER BY name",
    )
    .bind(user_id)
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| UserCredential {
            name: r.name,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

pub async fn get_user_credential_value(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: &str,
    name: &str,
) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT encrypted_value FROM user_credentials \
         WHERE user_id = $1 AND workspace_id = $2 AND name = $3",
    )
    .bind(user_id)
    .bind(workspace_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_all_user_credential_values(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    let rows = sqlx::query_as::<_, CredentialKvRow>(
        "SELECT name, encrypted_value FROM user_credentials \
         WHERE user_id = $1 AND workspace_id = $2",
    )
    .bind(user_id)
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.name, r.encrypted_value))
        .collect())
}

pub async fn delete_user_credential(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: &str,
    name: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM user_credentials \
         WHERE user_id = $1 AND workspace_id = $2 AND name = $3",
    )
    .bind(user_id)
    .bind(workspace_id)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn user_credential_exists(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: &str,
    name: &str,
) -> anyhow::Result<bool> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT name FROM user_credentials \
         WHERE user_id = $1 AND workspace_id = $2 AND name = $3",
    )
    .bind(user_id)
    .bind(workspace_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// True if the user has at least one credential row matching one of
/// `names` in `workspace_id`. Used by the workflow-activate gate
/// (issue #156) to refuse activation when the owner has no Claude auth
/// credential in the workflow's workspace — without this check, the
/// workflow would be active but every session would fail with "stdout
/// closed without result event".
///
/// Checks by exact name match because the Claude CLI keys on specific
/// env var names (`CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`).
/// A user with only custom-named credentials would silently activate
/// into a doomed workflow without the name filter.
pub async fn user_has_credential_in(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: &str,
    names: &[&str],
) -> anyhow::Result<bool> {
    if names.is_empty() {
        return Ok(false);
    }
    // Build `name IN ($3, $4, ...)` with placeholders matched to the
    // sqlx binding count — sqlx-AnyPool doesn't speak Postgres array
    // params portably across SQLite.  $1 = user_id, $2 = workspace_id.
    let placeholders: Vec<String> = (3..=names.len() + 2).map(|i| format!("${i}")).collect();
    let sql = format!(
        "SELECT name FROM user_credentials \
         WHERE user_id = $1 AND workspace_id = $2 AND name IN ({}) LIMIT 1",
        placeholders.join(", ")
    );
    let mut q = sqlx::query_scalar::<_, String>(&sql)
        .bind(user_id)
        .bind(workspace_id);
    for n in names {
        q = q.bind(*n);
    }
    let row = q.fetch_optional(pool).await?;
    Ok(row.is_some())
}

// ── Personal Access Tokens (issue #143) ──

#[derive(Debug, Clone)]
pub struct UserPat {
    pub id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub name: String,
    pub token_prefix: String,
    pub expires_at: Option<chrono::DateTime<Utc>>,
    pub last_used_at: Option<chrono::DateTime<Utc>>,
    pub last_used_ip: Option<String>,
    pub last_used_user_agent: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub revoked_at: Option<chrono::DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct UserPatRow {
    id: String,
    user_id: String,
    workspace_id: Option<String>,
    name: String,
    token_prefix: String,
    expires_at: Option<String>,
    last_used_at: Option<String>,
    last_used_ip: Option<String>,
    last_used_user_agent: Option<String>,
    created_at: String,
    revoked_at: Option<String>,
}

fn parse_optional_ts(v: Option<String>) -> anyhow::Result<Option<chrono::DateTime<Utc>>> {
    match v {
        Some(s) => Ok(Some(
            chrono::DateTime::parse_from_rfc3339(&s)?.with_timezone(&Utc),
        )),
        None => Ok(None),
    }
}

impl TryFrom<UserPatRow> for UserPat {
    type Error = anyhow::Error;

    fn try_from(row: UserPatRow) -> anyhow::Result<Self> {
        // Schema is NOT NULL post-#163; surfacing NULL here would mean an
        // older DB that hasn't run the backfill — fail loudly rather than
        // re-introduce the Option higher up the stack.
        let workspace_id = row.workspace_id.ok_or_else(|| {
            anyhow::anyhow!("user_pats.workspace_id is NULL; run migration backfill")
        })?;
        Ok(UserPat {
            id: row.id,
            user_id: row.user_id,
            workspace_id,
            name: row.name,
            token_prefix: row.token_prefix,
            expires_at: parse_optional_ts(row.expires_at)?,
            last_used_at: parse_optional_ts(row.last_used_at)?,
            last_used_ip: row.last_used_ip,
            last_used_user_agent: row.last_used_user_agent,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
            revoked_at: parse_optional_ts(row.revoked_at)?,
        })
    }
}

const PAT_FIELDS: &str = "id, user_id, workspace_id, name, token_prefix, expires_at, \
                          last_used_at, last_used_ip, last_used_user_agent, created_at, revoked_at";

#[allow(clippy::too_many_arguments)]
pub async fn insert_user_pat(
    pool: &AnyPool,
    id: &str,
    user_id: &str,
    workspace_id: &str,
    name: &str,
    token_prefix: &str,
    token_hash: &str,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_pats (id, user_id, workspace_id, name, token_prefix, token_hash, \
                                scopes, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(user_id)
    .bind(workspace_id)
    .bind(name)
    .bind(token_prefix)
    .bind(token_hash)
    .bind("[\"*\"]")
    .bind(expires_at.map(|d| d.to_rfc3339()))
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_user_pats(pool: &AnyPool, user_id: &str) -> anyhow::Result<Vec<UserPat>> {
    let q =
        format!("SELECT {PAT_FIELDS} FROM user_pats WHERE user_id = $1 ORDER BY created_at DESC");
    let rows = sqlx::query_as::<_, UserPatRow>(&q)
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// Look up candidate PATs by token prefix. The caller must verify the
/// `token_hash` against the presented token in constant time before
/// accepting any row.
pub async fn find_pats_by_prefix(
    pool: &AnyPool,
    token_prefix: &str,
) -> anyhow::Result<Vec<(UserPat, String)>> {
    let q = format!("SELECT {PAT_FIELDS}, token_hash FROM user_pats WHERE token_prefix = $1");
    let rows = sqlx::query_as::<_, UserPatWithHashRow>(&q)
        .bind(token_prefix)
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|r| {
            let hash = r.token_hash.clone();
            let pat: UserPat = UserPatRow {
                id: r.id,
                user_id: r.user_id,
                workspace_id: r.workspace_id,
                name: r.name,
                token_prefix: r.token_prefix,
                expires_at: r.expires_at,
                last_used_at: r.last_used_at,
                last_used_ip: r.last_used_ip,
                last_used_user_agent: r.last_used_user_agent,
                created_at: r.created_at,
                revoked_at: r.revoked_at,
            }
            .try_into()?;
            Ok((pat, hash))
        })
        .collect()
}

#[derive(sqlx::FromRow)]
struct UserPatWithHashRow {
    id: String,
    user_id: String,
    workspace_id: Option<String>,
    name: String,
    token_prefix: String,
    expires_at: Option<String>,
    last_used_at: Option<String>,
    last_used_ip: Option<String>,
    last_used_user_agent: Option<String>,
    created_at: String,
    revoked_at: Option<String>,
    token_hash: String,
}

pub async fn revoke_user_pat(pool: &AnyPool, user_id: &str, pat_id: &str) -> anyhow::Result<bool> {
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        "UPDATE user_pats SET revoked_at = $1 \
         WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(&now)
    .bind(pat_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn touch_user_pat(
    pool: &AnyPool,
    pat_id: &str,
    ip: Option<&str>,
    user_agent: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE user_pats SET last_used_at = $1, last_used_ip = $2, last_used_user_agent = $3 \
         WHERE id = $4",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(ip)
    .bind(user_agent)
    .bind(pat_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Updated Session queries (user-scoped) ──

pub async fn insert_session_with_user(
    pool: &AnyPool,
    session: &Session,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    insert_session_with_user_and_project(pool, session, user_id, None, None).await
}

/// Variant that also binds a `project_id` when the session is scoped to a
/// workspace-owned project (issue #59). Pre-existing sessions with a null
/// `project_id` remain personal sessions forever.
///
/// `workspace_id` was added in issue #164 so list endpoints can filter
/// without joining through projects. Sessions with no workspace context
/// (legacy direct-shaping callers) keep `workspace_id IS NULL` and never
/// surface in any workspace-scoped list.
pub async fn insert_session_with_user_and_project(
    pool: &AnyPool,
    session: &Session,
    user_id: Option<&str>,
    project_id: Option<&str>,
    workspace_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               user_id, project_id, workspace_id, artifact_id, artifact_version, \
                               created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(user_id)
    .bind(project_id)
    .bind(workspace_id)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_sessions_for_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// List sessions scoped to a workspace (issue #164).
///
/// Returns only sessions whose `workspace_id` column matches.  Sessions
/// without a workspace (legacy/direct-dispatch — see
/// `insert_session_with_user_and_project`) are not surfaced by any
/// workspace-scoped list.
pub async fn list_sessions_for_workspace(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE workspace_id = $1 ORDER BY created_at DESC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_session_owner(pool: &AnyPool, session_id: &str) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM sessions WHERE id = $1 AND user_id IS NOT NULL",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Look up the workspace a session is scoped to. `Ok(None)` covers two
/// cases that callers handle the same way: the session row doesn't exist
/// (404 from the route), or it exists but predates the workspace_id
/// column / was created without workspace context (also 404 from any
/// workspace-scoped route — there's no workspace to authz against).
pub async fn get_session_workspace_id(
    pool: &AnyPool,
    session_id: &str,
) -> anyhow::Result<Option<String>> {
    let row =
        sqlx::query_scalar::<_, Option<String>>("SELECT workspace_id FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.flatten())
}

// ── Row types for sqlx ──

#[derive(sqlx::FromRow)]
struct NodeRow {
    id: String,
    name: String,
    hostname: String,
    status: String,
    max_sessions: i32,
    active_sessions: i32,
    last_heartbeat: String,
    registered_at: String,
}

impl TryFrom<NodeRow> for Node {
    type Error = anyhow::Error;

    fn try_from(row: NodeRow) -> anyhow::Result<Self> {
        Ok(Node {
            id: row.id,
            name: row.name,
            hostname: row.hostname,
            status: row
                .status
                .parse()
                .map_err(|e: crate::core::StiglabError| anyhow::anyhow!(e))?,
            max_sessions: row.max_sessions as u32,
            active_sessions: row.active_sessions as u32,
            last_heartbeat: chrono::DateTime::parse_from_rfc3339(&row.last_heartbeat)?
                .with_timezone(&Utc),
            registered_at: chrono::DateTime::parse_from_rfc3339(&row.registered_at)?
                .with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    task_id: String,
    node_id: String,
    state: String,
    prompt: String,
    output: Option<String>,
    working_dir: Option<String>,
    artifact_id: Option<String>,
    artifact_version: Option<i32>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<SessionRow> for Session {
    type Error = anyhow::Error;

    fn try_from(row: SessionRow) -> anyhow::Result<Self> {
        Ok(Session {
            id: row.id,
            task_id: row.task_id,
            node_id: row.node_id,
            state: row
                .state
                .parse()
                .map_err(|e: crate::core::StiglabError| anyhow::anyhow!(e))?,
            prompt: row.prompt,
            output: row.output,
            working_dir: row.working_dir,
            artifact_id: row.artifact_id,
            artifact_version: row.artifact_version,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    github_id: i64,
    github_login: String,
    github_name: Option<String>,
    github_avatar_url: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<UserRow> for User {
    type Error = anyhow::Error;

    fn try_from(row: UserRow) -> anyhow::Result<Self> {
        Ok(User {
            id: row.id,
            github_id: row.github_id,
            github_login: row.github_login,
            github_name: row.github_name,
            github_avatar_url: row.github_avatar_url,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct AuthSessionRow {
    id: String,
    user_id: String,
    expires_at: String,
    created_at: String,
    // User fields from join
    github_id: i64,
    github_login: String,
    github_name: Option<String>,
    github_avatar_url: Option<String>,
    user_created_at: String,
    user_updated_at: String,
}

#[derive(sqlx::FromRow)]
struct UserCredentialRow {
    name: String,
    created_at: String,
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct CredentialKvRow {
    name: String,
    encrypted_value: String,
}

// ── Workspace / membership / installation / project CRUD (issue #59;
//    renamed in #163).

pub async fn insert_workspace(pool: &AnyPool, workspace: &Workspace) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO workspaces (id, slug, name, created_by, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&workspace.id)
    .bind(&workspace.slug)
    .bind(&workspace.name)
    .bind(&workspace.created_by)
    .bind(workspace.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Atomically insert a workspace and its creator-as-member row.  Either
/// both rows land or neither does — prevents a failed `workspace_members`
/// insert from leaving an orphan workspace that permanently consumes its
/// slug.
pub async fn insert_workspace_with_creator(
    pool: &AnyPool,
    workspace: &Workspace,
    member: &WorkspaceMember,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO workspaces (id, slug, name, created_by, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&workspace.id)
    .bind(&workspace.slug)
    .bind(&workspace.name)
    .bind(&workspace.created_by)
    .bind(workspace.created_at.to_rfc3339())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, joined_at) VALUES ($1, $2, $3)",
    )
    .bind(&member.workspace_id)
    .bind(&member.user_id)
    .bind(member.joined_at.to_rfc3339())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn get_workspace(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Option<Workspace>> {
    let row = sqlx::query_as::<_, WorkspaceRow>(
        "SELECT id, slug, name, created_by, created_at FROM workspaces WHERE id = $1",
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn list_workspaces_for_user(
    pool: &AnyPool,
    user_id: &str,
) -> anyhow::Result<Vec<Workspace>> {
    let rows = sqlx::query_as::<_, WorkspaceRow>(
        "SELECT w.id, w.slug, w.name, w.created_by, w.created_at \
         FROM workspaces w \
         JOIN workspace_members m ON w.id = m.workspace_id \
         WHERE m.user_id = $1 \
         ORDER BY w.created_at ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn insert_workspace_member(
    pool: &AnyPool,
    member: &WorkspaceMember,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, joined_at) VALUES ($1, $2, $3)",
    )
    .bind(&member.workspace_id)
    .bind(&member.user_id)
    .bind(member.joined_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn is_workspace_member(
    pool: &AnyPool,
    workspace_id: &str,
    user_id: &str,
) -> anyhow::Result<bool> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

pub async fn list_workspace_members(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<WorkspaceMember>> {
    let rows = sqlx::query_as::<_, WorkspaceMemberRow>(
        "SELECT workspace_id, user_id, joined_at \
         FROM workspace_members WHERE workspace_id = $1 ORDER BY joined_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// `WorkspaceMember` enriched with the member's GitHub profile so the
/// dashboard can render `@login` + avatar instead of the opaque user UUID.
/// `LEFT JOIN` so a member row whose `users` row was somehow removed still
/// surfaces (with nullable GitHub fields) rather than silently disappearing
/// from the workspace's member list.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceMemberWithUser {
    pub workspace_id: String,
    pub user_id: String,
    pub joined_at: chrono::DateTime<Utc>,
    pub github_login: Option<String>,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
}

#[derive(sqlx::FromRow)]
struct WorkspaceMemberWithUserRow {
    workspace_id: String,
    user_id: String,
    joined_at: String,
    github_login: Option<String>,
    github_name: Option<String>,
    github_avatar_url: Option<String>,
}

impl TryFrom<WorkspaceMemberWithUserRow> for WorkspaceMemberWithUser {
    type Error = anyhow::Error;

    fn try_from(row: WorkspaceMemberWithUserRow) -> anyhow::Result<Self> {
        Ok(WorkspaceMemberWithUser {
            workspace_id: row.workspace_id,
            user_id: row.user_id,
            joined_at: chrono::DateTime::parse_from_rfc3339(&row.joined_at)?.with_timezone(&Utc),
            github_login: row.github_login,
            github_name: row.github_name,
            github_avatar_url: row.github_avatar_url,
        })
    }
}

pub async fn list_workspace_members_with_users(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<WorkspaceMemberWithUser>> {
    let rows = sqlx::query_as::<_, WorkspaceMemberWithUserRow>(
        "SELECT m.workspace_id, m.user_id, m.joined_at, \
                u.github_login, u.github_name, u.github_avatar_url \
         FROM workspace_members m \
         LEFT JOIN users u ON u.id = m.user_id \
         WHERE m.workspace_id = $1 \
         ORDER BY m.joined_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn insert_github_app_installation(
    pool: &AnyPool,
    install: &GitHubAppInstallation,
    webhook_secret_cipher: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO github_app_installations (id, workspace_id, install_id, account_login, \
                                               account_type, webhook_secret_cipher, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&install.id)
    .bind(&install.workspace_id)
    .bind(install.install_id)
    .bind(&install.account_login)
    .bind(install.account_type.to_string())
    .bind(webhook_secret_cipher)
    .bind(install.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_github_app_installations_for_workspace(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<GitHubAppInstallation>> {
    let rows = sqlx::query_as::<_, GitHubAppInstallationRow>(
        "SELECT id, workspace_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE workspace_id = $1 ORDER BY created_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_github_app_installation(
    pool: &AnyPool,
    install_id: &str,
) -> anyhow::Result<Option<GitHubAppInstallation>> {
    let row = sqlx::query_as::<_, GitHubAppInstallationRow>(
        "SELECT id, workspace_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE id = $1",
    )
    .bind(install_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

/// Look up an installation by its **numeric GitHub install_id** (not the
/// internal UUID).  Used by the install callback to detect idempotent
/// re-runs vs. cross-workspace linkage conflicts before inserting.
pub async fn get_github_app_installation_by_install_id(
    pool: &AnyPool,
    install_id: i64,
) -> anyhow::Result<Option<GitHubAppInstallation>> {
    let row = sqlx::query_as::<_, GitHubAppInstallationRow>(
        "SELECT id, workspace_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE install_id = $1",
    )
    .bind(install_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn delete_github_app_installation(
    pool: &AnyPool,
    install_id: &str,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM github_app_installations WHERE id = $1")
        .bind(install_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Count projects that still reference a given installation. Used by the
/// delete-installation route for an app-layer referential-integrity check:
/// the tables do not declare FK constraints (consistent with the rest of
/// stiglab, which uses AnyPool across SQLite/Postgres — SQLite needs
/// `PRAGMA foreign_keys = ON` to enforce FKs and the rest of the schema
/// matches this convention), so callers must gate destructive operations
/// explicitly.
pub async fn count_projects_for_installation(
    pool: &AnyPool,
    install_id: &str,
) -> anyhow::Result<i64> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE github_app_installation_id = $1")
            .bind(install_id)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

pub async fn insert_project(pool: &AnyPool, project: &Project) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO projects (id, workspace_id, github_app_installation_id, repo_owner, \
                               repo_name, default_branch, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&project.id)
    .bind(&project.workspace_id)
    .bind(&project.github_app_installation_id)
    .bind(&project.repo_owner)
    .bind(&project.repo_name)
    .bind(&project.default_branch)
    .bind(project.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_project(pool: &AnyPool, project_id: &str) -> anyhow::Result<Option<Project>> {
    let row = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, workspace_id, github_app_installation_id, repo_owner, repo_name, \
                default_branch, created_at \
         FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn list_projects_for_workspace(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<Project>> {
    let rows = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, workspace_id, github_app_installation_id, repo_owner, repo_name, \
                default_branch, created_at \
         FROM projects WHERE workspace_id = $1 ORDER BY created_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn list_projects_for_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Vec<Project>> {
    let rows = sqlx::query_as::<_, ProjectRow>(
        "SELECT p.id, p.workspace_id, p.github_app_installation_id, p.repo_owner, p.repo_name, \
                p.default_branch, p.created_at \
         FROM projects p \
         JOIN workspace_members m ON p.workspace_id = m.workspace_id \
         WHERE m.user_id = $1 \
         ORDER BY p.created_at ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn delete_project(pool: &AnyPool, project_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Count sessions attached to a project that are not in a terminal state.
/// Used to block project deletion while live sessions reference it (no
/// cascade, no soft-delete in v1).
pub async fn count_live_sessions_for_project(
    pool: &AnyPool,
    project_id: &str,
) -> anyhow::Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions \
         WHERE project_id = $1 AND state NOT IN ('done', 'failed')",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

// ── Row types (Workspace / membership / installation / project) ──

#[derive(sqlx::FromRow)]
struct WorkspaceRow {
    id: String,
    slug: String,
    name: String,
    created_by: String,
    created_at: String,
}

impl TryFrom<WorkspaceRow> for Workspace {
    type Error = anyhow::Error;

    fn try_from(row: WorkspaceRow) -> anyhow::Result<Self> {
        Ok(Workspace {
            id: row.id,
            slug: row.slug,
            name: row.name,
            created_by: row.created_by,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct WorkspaceMemberRow {
    workspace_id: String,
    user_id: String,
    joined_at: String,
}

impl TryFrom<WorkspaceMemberRow> for WorkspaceMember {
    type Error = anyhow::Error;

    fn try_from(row: WorkspaceMemberRow) -> anyhow::Result<Self> {
        Ok(WorkspaceMember {
            workspace_id: row.workspace_id,
            user_id: row.user_id,
            joined_at: chrono::DateTime::parse_from_rfc3339(&row.joined_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct GitHubAppInstallationRow {
    id: String,
    workspace_id: String,
    install_id: i64,
    account_login: String,
    account_type: String,
    created_at: String,
}

impl TryFrom<GitHubAppInstallationRow> for GitHubAppInstallation {
    type Error = anyhow::Error;

    fn try_from(row: GitHubAppInstallationRow) -> anyhow::Result<Self> {
        Ok(GitHubAppInstallation {
            id: row.id,
            workspace_id: row.workspace_id,
            install_id: row.install_id,
            account_login: row.account_login,
            account_type: row
                .account_type
                .parse()
                .map_err(|e: crate::core::StiglabError| anyhow::anyhow!(e))?,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: String,
    workspace_id: String,
    github_app_installation_id: String,
    repo_owner: String,
    repo_name: String,
    default_branch: String,
    created_at: String,
}

impl TryFrom<ProjectRow> for Project {
    type Error = anyhow::Error;

    fn try_from(row: ProjectRow) -> anyhow::Result<Self> {
        Ok(Project {
            id: row.id,
            workspace_id: row.workspace_id,
            github_app_installation_id: row.github_app_installation_id,
            repo_owner: row.repo_owner,
            repo_name: row.repo_name,
            default_branch: row.default_branch,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        GitHubAccountType, Project, Session, SessionState, Workspace, WorkspaceMember,
    };
    use chrono::Utc;
    use uuid::Uuid;

    async fn test_pool() -> AnyPool {
        sqlx::any::install_default_drivers();
        let pool = PoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("failed to connect to sqlite in-memory");
        run_migrations(&pool)
            .await
            .expect("migrations should succeed");
        pool
    }

    async fn seed_user(pool: &AnyPool, user_id: &str) {
        // Derive a stable non-colliding github_id from the user_id bytes.
        let github_id: i64 = user_id.bytes().fold(0i64, |acc, b| acc * 131 + b as i64);
        sqlx::query(
            "INSERT INTO users (id, github_id, github_login, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $4)",
        )
        .bind(user_id)
        .bind(github_id)
        .bind(user_id)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
    }

    fn new_workspace(created_by: &str) -> Workspace {
        Workspace {
            id: Uuid::new_v4().to_string(),
            slug: format!("workspace-{}", Uuid::new_v4().simple()),
            name: "Test Workspace".to_string(),
            created_by: created_by.to_string(),
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn workspace_crud_roundtrip() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;

        let workspace = new_workspace("u1");
        insert_workspace(&pool, &workspace).await.unwrap();

        let fetched = get_workspace(&pool, &workspace.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, workspace.id);
        assert_eq!(fetched.slug, workspace.slug);
    }

    #[tokio::test]
    async fn membership_query_and_list_workspaces_for_user() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        seed_user(&pool, "u2").await;

        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();
        insert_workspace_member(
            &pool,
            &WorkspaceMember {
                workspace_id: w.id.clone(),
                user_id: "u1".to_string(),
                joined_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        assert!(is_workspace_member(&pool, &w.id, "u1").await.unwrap());
        assert!(!is_workspace_member(&pool, &w.id, "u2").await.unwrap());

        let u1_workspaces = list_workspaces_for_user(&pool, "u1").await.unwrap();
        assert_eq!(u1_workspaces.len(), 1);
        assert_eq!(u1_workspaces[0].id, w.id);

        let u2_workspaces = list_workspaces_for_user(&pool, "u2").await.unwrap();
        assert!(u2_workspaces.is_empty());
    }

    #[tokio::test]
    async fn list_workspace_members_with_users_joins_github_profile() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();
        insert_workspace_member(
            &pool,
            &WorkspaceMember {
                workspace_id: w.id.clone(),
                user_id: "u1".to_string(),
                joined_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        let members = list_workspace_members_with_users(&pool, &w.id)
            .await
            .unwrap();
        assert_eq!(members.len(), 1);
        // `seed_user` writes `github_login = user_id`, so this exercises the
        // JOIN without needing to fixture a realistic avatar URL.
        assert_eq!(members[0].user_id, "u1");
        assert_eq!(members[0].github_login.as_deref(), Some("u1"));
    }

    #[tokio::test]
    async fn installation_and_project_crud() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            install_id: 42,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::Organization,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, Some("ciphertext"))
            .await
            .unwrap();

        let installs = list_github_app_installations_for_workspace(&pool, &w.id)
            .await
            .unwrap();
        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].install_id, 42);

        let project = Project {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        let projects = list_projects_for_workspace(&pool, &w.id).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].repo_name, "widgets");
    }

    #[tokio::test]
    async fn delete_project_blocks_on_live_sessions() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            install_id: 7,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::Organization,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, None)
            .await
            .unwrap();

        let project = Project {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        let session = Session {
            id: Uuid::new_v4().to_string(),
            task_id: Uuid::new_v4().to_string(),
            node_id: "node-1".to_string(),
            state: SessionState::Running,
            prompt: "hello".to_string(),
            output: None,
            working_dir: None,
            artifact_id: None,
            artifact_version: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        insert_session_with_user_and_project(
            &pool,
            &session,
            Some("u1"),
            Some(&project.id),
            Some(&project.workspace_id),
        )
        .await
        .unwrap();

        let live = count_live_sessions_for_project(&pool, &project.id)
            .await
            .unwrap();
        assert_eq!(live, 1);

        // Transition to a terminal state — live count should drop to zero.
        update_session_state(&pool, &session.id, SessionState::Done)
            .await
            .unwrap();
        let live = count_live_sessions_for_project(&pool, &project.id)
            .await
            .unwrap();
        assert_eq!(live, 0);

        delete_project(&pool, &project.id).await.unwrap();
        assert!(get_project(&pool, &project.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn insert_workspace_with_creator_is_atomic() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        let m = WorkspaceMember {
            workspace_id: w.id.clone(),
            user_id: "u1".to_string(),
            joined_at: Utc::now(),
        };
        insert_workspace_with_creator(&pool, &w, &m).await.unwrap();
        assert!(get_workspace(&pool, &w.id).await.unwrap().is_some());
        assert!(is_workspace_member(&pool, &w.id, "u1").await.unwrap());

        // Reusing the same slug must fail and — because the helper uses a
        // transaction — must not create a new member row either.
        let w2 = Workspace {
            id: Uuid::new_v4().to_string(),
            slug: w.slug.clone(),
            ..new_workspace("u1")
        };
        let m2 = WorkspaceMember {
            workspace_id: w2.id.clone(),
            user_id: "u1".to_string(),
            joined_at: Utc::now(),
        };
        assert!(insert_workspace_with_creator(&pool, &w2, &m2)
            .await
            .is_err());
        assert!(get_workspace(&pool, &w2.id).await.unwrap().is_none());
        assert!(!is_workspace_member(&pool, &w2.id, "u1").await.unwrap());
    }

    #[tokio::test]
    async fn count_projects_for_installation_blocks_delete() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            install_id: 99,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::Organization,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, None)
            .await
            .unwrap();

        assert_eq!(
            count_projects_for_installation(&pool, &install.id)
                .await
                .unwrap(),
            0
        );

        let project = Project {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        assert_eq!(
            count_projects_for_installation(&pool, &install.id)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn list_projects_for_user_follows_membership() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        seed_user(&pool, "u2").await;

        let w1 = new_workspace("u1");
        insert_workspace(&pool, &w1).await.unwrap();
        insert_workspace_member(
            &pool,
            &WorkspaceMember {
                workspace_id: w1.id.clone(),
                user_id: "u1".to_string(),
                joined_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            workspace_id: w1.id.clone(),
            install_id: 1,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::User,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, None)
            .await
            .unwrap();
        let project = Project {
            id: Uuid::new_v4().to_string(),
            workspace_id: w1.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        let u1_projects = list_projects_for_user(&pool, "u1").await.unwrap();
        assert_eq!(u1_projects.len(), 1);

        let u2_projects = list_projects_for_user(&pool, "u2").await.unwrap();
        assert!(u2_projects.is_empty());
    }
}
