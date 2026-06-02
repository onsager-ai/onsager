use std::time::Duration;

use anyhow::Context;
use sqlx::AnyPool;
use sqlx::pool::PoolOptions;

mod credentials;
mod github_installations;
mod nodes;
mod pats;
mod projects;
mod sessions;
mod users;
mod workspaces;

#[cfg(test)]
mod tests;

// ── Re-exports ──

pub use credentials::{
    UserCredential, delete_user_credential, get_all_user_credential_values,
    get_user_credential_value, get_user_credentials, set_user_credential, user_credential_exists,
    user_has_credential_in,
};

pub use github_installations::{
    count_projects_for_installation, delete_github_app_installation, get_github_app_installation,
    get_github_app_installation_by_install_id, get_install_webhook_secret_cipher,
    insert_github_app_installation, list_github_app_installations_for_workspace,
};

pub use nodes::{
    find_least_loaded_node, find_node_by_name, get_node, list_nodes, update_node_heartbeat,
    update_node_status, upsert_node,
};

pub use pats::{
    UserPat, find_pats_by_prefix, insert_user_pat, list_user_pats, revoke_user_pat, touch_user_pat,
};

pub use projects::{
    count_live_sessions_for_project, delete_project, get_project, insert_project,
    list_projects_for_user, list_projects_for_workspace,
};

pub use sessions::{
    LogChunk, LogChunkWithSeq, append_session_log, claim_pending_session,
    find_session_by_idempotency_key, get_session, get_session_logs, get_session_logs_after,
    get_session_owner, get_session_workspace, insert_session, insert_session_with_idempotency_key,
    insert_session_with_user, insert_session_with_user_and_project,
    insert_session_with_user_project_workspace, list_pending_sessions_for_node, list_sessions,
    list_sessions_for_user_in_workspace, update_session_state,
};

pub use users::{
    AuthSession, create_auth_session, get_auth_session, get_user, get_user_by_github_id,
    upsert_user,
};

pub use workspaces::{
    WorkspaceMemberWithUser, get_workspace, get_workspace_by_slug, insert_workspace,
    insert_workspace_member, insert_workspace_with_creator, is_workspace_member,
    list_workspace_members, list_workspace_members_with_users, list_workspaces_for_user,
};

// ── Pool initialisation ──

pub async fn init_pool(database_url: &str) -> anyhow::Result<AnyPool> {
    let is_sqlite = database_url.starts_with("sqlite://");
    // For SQLite: ensure parent directory exists and enable create-if-missing
    let connect_url = if is_sqlite {
        let path = database_url.trim_start_matches("sqlite://");
        if let Some(parent) = std::path::Path::new(path).parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await?;
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
    run_migrations(&pool, is_sqlite).await?;
    Ok(pool)
}

/// Run a legacy migration statement that is meaningful only on the live
/// Postgres install.
///
/// On SQLite — the unit-test backend, which rebuilds the schema fresh from
/// the `CREATE TABLE IF NOT EXISTS` statements in `run_migrations` — the
/// statement is skipped entirely. On Postgres it executes and **errors
/// propagate**: a real failure (permissions, unexpected state) must surface
/// from `run_migrations` rather than being swallowed by the old
/// `let _ = …execute().await;` idiom, so a half-applied migration cannot
/// silently diverge (issue #550).
///
/// Every statement routed here is written to be idempotent on Postgres
/// (guarded `DO $$` blocks for renames, `IF EXISTS` for drops, `SET NOT
/// NULL` is a no-op once applied, backfills are `WHERE … IS NULL`), so a
/// re-run against an already-migrated database is a no-op, not an error.
async fn run_pg_only(pool: &AnyPool, is_sqlite: bool, sql: &str) -> anyhow::Result<()> {
    if is_sqlite {
        return Ok(());
    }
    sqlx::query(sql)
        .execute(pool)
        .await
        .with_context(|| format!("postgres-only migration statement failed: {sql}"))?;
    Ok(())
}

pub async fn run_migrations(pool: &AnyPool, is_sqlite: bool) -> anyhow::Result<()> {
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
    //
    // Portal owns writes to `users`, `auth_sessions`, and
    // `sso_exchange_codes` post-#222 Slice 5
    // (`crates/onsager-portal/migrations/002–004`). The DDL below stays
    // idempotently here as a fallback so:
    //   1. Stiglab's SQLite-backed unit tests build the schema without a
    //      portal binary in the loop.
    //   2. On a fresh deploy, whichever process migrates first wins —
    //      same-shape `CREATE TABLE IF NOT EXISTS` makes the loser a
    //      no-op. Mirrors the `pr_branch_links` pattern below.
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

    // Per-workspace credential isolation lands in the 002 block below.
    // Fresh databases get the new schema directly; legacy databases pick
    // up the column + constraints via the ALTER chain in the 002 section.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS user_credentials (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            workspace_id TEXT NOT NULL,
            name TEXT NOT NULL,
            encrypted_value TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(workspace_id, user_id, name)
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
    //
    // Spec #222 Slice 2b moved PAT mint/list/revoke to portal
    // (`crates/onsager-portal/migrations/005_user_pats.sql`). Portal is the
    // only writer; stiglab still reads this table from its `AuthUser`
    // extractor for the credentials/workspaces/projects/workflows routes
    // that haven't moved yet (Slices 2a/3/4). The DDL below stays
    // idempotently here as a fallback so SQLite-backed unit tests build
    // the schema without a portal binary in the loop, and on a fresh
    // deploy whichever process migrates first wins (same-shape
    // `CREATE TABLE IF NOT EXISTS` makes the loser a no-op).
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
    // These statements carry a legacy "tenant"-schema installation forward.
    // They are meaningful only on Postgres (the live install) and run via
    // `run_pg_only`; the SQLite unit-test backend rebuilds the schema fresh
    // from the `CREATE TABLE IF NOT EXISTS` statements below, so they are
    // skipped there. Each rename is wrapped in a guarded `DO $$` block
    // (mirroring the canonical spine `004_workflows.sql` shape) so it is a
    // no-op on a fresh or already-migrated DB; a real failure (permissions,
    // unexpected state) now propagates from `run_migrations` instead of being
    // swallowed (issue #550). The deletion rule for `user_pats` rows with no
    // `workspace_members` membership is enforced by the PAT backfill below.
    //
    // The two table renames precede the column renames deliberately: hardening
    // only the column renames would let a failed `tenants`/`tenant_members`
    // rename silently strand data under the old names — the same failure class
    // this change closes — so both halves of the rename are routed here.
    run_pg_only(
        pool,
        is_sqlite,
        "DO $$ BEGIN \
           IF EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'tenants') \
              AND NOT EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'workspaces') THEN \
             ALTER TABLE tenants RENAME TO workspaces; \
           END IF; \
         END $$",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "DO $$ BEGIN \
           IF EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'tenant_members') \
              AND NOT EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'workspace_members') THEN \
             ALTER TABLE tenant_members RENAME TO workspace_members; \
           END IF; \
         END $$",
    )
    .await?;
    // Note: pre-#163 `tenant_workflows` / `tenant_workflow_stages` are
    // not renamed here — Lever D (#149) collapses them into the spine
    // `workflows` / `workflow_stages` tables, so any stiglab DB still
    // carrying the old names just lives with stale rows that the spine
    // migration sweeps. The remaining renames (workspaces, members,
    // installations, projects, user_pats) stay because those tables
    // remain stiglab-owned.
    run_pg_only(
        pool,
        is_sqlite,
        "DO $$ BEGIN \
           IF EXISTS (SELECT FROM information_schema.columns \
                      WHERE table_name = 'workspace_members' AND column_name = 'tenant_id') \
              AND NOT EXISTS (SELECT FROM information_schema.columns \
                      WHERE table_name = 'workspace_members' AND column_name = 'workspace_id') THEN \
             ALTER TABLE workspace_members RENAME COLUMN tenant_id TO workspace_id; \
           END IF; \
         END $$",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "DO $$ BEGIN \
           IF EXISTS (SELECT FROM information_schema.columns \
                      WHERE table_name = 'github_app_installations' AND column_name = 'tenant_id') \
              AND NOT EXISTS (SELECT FROM information_schema.columns \
                      WHERE table_name = 'github_app_installations' AND column_name = 'workspace_id') THEN \
             ALTER TABLE github_app_installations RENAME COLUMN tenant_id TO workspace_id; \
           END IF; \
         END $$",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "DO $$ BEGIN \
           IF EXISTS (SELECT FROM information_schema.columns \
                      WHERE table_name = 'projects' AND column_name = 'tenant_id') \
              AND NOT EXISTS (SELECT FROM information_schema.columns \
                      WHERE table_name = 'projects' AND column_name = 'workspace_id') THEN \
             ALTER TABLE projects RENAME COLUMN tenant_id TO workspace_id; \
           END IF; \
         END $$",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "DO $$ BEGIN \
           IF EXISTS (SELECT FROM information_schema.columns \
                      WHERE table_name = 'user_pats' AND column_name = 'tenant_id') \
              AND NOT EXISTS (SELECT FROM information_schema.columns \
                      WHERE table_name = 'user_pats' AND column_name = 'workspace_id') THEN \
             ALTER TABLE user_pats RENAME COLUMN tenant_id TO workspace_id; \
           END IF; \
         END $$",
    )
    .await?;
    // Old indexes referencing the renamed tables/columns can outlive the
    // rename on Postgres if the index name doesn't auto-update; drop the
    // legacy ones explicitly.  All `IF EXISTS`, all idempotent, routed
    // through `run_pg_only` so a real failure surfaces (issue #550).
    run_pg_only(
        pool,
        is_sqlite,
        "DROP INDEX IF EXISTS idx_tenant_members_user_id",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "DROP INDEX IF EXISTS idx_github_app_installations_tenant_id",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "DROP INDEX IF EXISTS idx_projects_tenant_id",
    )
    .await?;

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

    // spec #545: rename legacy `projects` → `workspace_repos` on existing
    // installs. Postgres-only guarded `DO $$` block routed through
    // `run_pg_only` — skipped on SQLite (tests rebuild from the CREATE TABLE
    // below); on Postgres a real failure propagates instead of being
    // swallowed (issue #550).
    run_pg_only(
        pool,
        is_sqlite,
        "DO $$ \
         BEGIN \
           IF EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'projects') \
              AND NOT EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'workspace_repos') THEN \
             ALTER TABLE projects RENAME TO workspace_repos; \
             ALTER INDEX IF EXISTS idx_projects_workspace_id RENAME TO idx_workspace_repos_workspace_id; \
           END IF; \
         END $$",
    )
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS workspace_repos (
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

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_workspace_repos_workspace_id ON workspace_repos (workspace_id)",
    )
    .execute(pool)
    .await?;

    // PAT backfill: rows with NULL `workspace_id` get pinned to the user's
    // first workspace membership; rows with no membership at all are
    // deleted (unusable — a workspace-less PAT can't address any
    // workspace-scoped route, and after this migration `workspace_id` is
    // mandatory at the API layer).  All three statements are idempotent on
    // Postgres (the backfill/delete are `WHERE workspace_id IS NULL`; the
    // `SET NOT NULL` is a no-op once applied), so they run via `run_pg_only`
    // — skipped on SQLite (tests rebuild the schema each run from the new
    // CREATE TABLE above), errors propagated on Postgres (issue #550).
    run_pg_only(
        pool,
        is_sqlite,
        "UPDATE user_pats \
         SET workspace_id = ( \
             SELECT m.workspace_id FROM workspace_members m \
             WHERE m.user_id = user_pats.user_id \
             ORDER BY m.joined_at ASC, m.workspace_id ASC \
             LIMIT 1 \
         ) \
         WHERE workspace_id IS NULL",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "DELETE FROM user_pats WHERE workspace_id IS NULL",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "ALTER TABLE user_pats ALTER COLUMN workspace_id SET NOT NULL",
    )
    .await?;

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

    // Workflows (issue #81): the schema lives on the spine
    // (`workflows` / `workflow_stages`) post-Lever D (#149). Stiglab
    // writes through `workflow_db.rs` against the spine pool — no
    // private table here. See spine migrations 006/010/012/013.

    // ── 002_workspace_scoped_credentials (issue #164) ────────────────
    //
    // Per-workspace credential isolation, workspace-scoped sessions, and a
    // belt-and-suspenders UNIQUE on github_app_installations.install_id.
    // See `migrations/002_workspace_scoped_credentials.sql` for the
    // canonical SQL and rationale.

    let _ = sqlx::query("ALTER TABLE user_credentials ADD COLUMN workspace_id TEXT")
        .execute(pool)
        .await;
    // Backfill + `SET NOT NULL` mirror the PAT block above: idempotent on
    // Postgres, routed through `run_pg_only` (skipped on SQLite, errors
    // propagated on Postgres — issue #550).
    run_pg_only(
        pool,
        is_sqlite,
        "UPDATE user_credentials \
         SET workspace_id = ( \
             SELECT m.workspace_id FROM workspace_members m \
             WHERE m.user_id = user_credentials.user_id \
             ORDER BY m.joined_at ASC, m.workspace_id ASC \
             LIMIT 1 \
         ) \
         WHERE workspace_id IS NULL",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "DELETE FROM user_credentials WHERE workspace_id IS NULL",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "ALTER TABLE user_credentials ALTER COLUMN workspace_id SET NOT NULL",
    )
    .await?;
    // Drop the legacy `(user_id, name)` unique constraint — its modern
    // equivalent is `(workspace_id, user_id, name)`. Both names cover the
    // index/constraint forms Postgres might have created. All `IF EXISTS`,
    // idempotent, routed through `run_pg_only`.
    run_pg_only(
        pool,
        is_sqlite,
        "DROP INDEX IF EXISTS user_credentials_user_id_name_key",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "DROP INDEX IF EXISTS idx_user_credentials_user_name",
    )
    .await?;
    run_pg_only(
        pool,
        is_sqlite,
        "ALTER TABLE user_credentials DROP CONSTRAINT IF EXISTS user_credentials_user_id_name_key",
    )
    .await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_user_credentials_workspace_user_name \
         ON user_credentials (workspace_id, user_id, name)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_user_credentials_workspace \
         ON user_credentials (workspace_id)",
    )
    .execute(pool)
    .await?;

    // sessions.workspace_id — backfill from project; rows without a
    // project remain NULL (legacy personal sessions). The backfill is
    // idempotent (`WHERE workspace_id IS NULL`), so it runs via
    // `run_pg_only` — skipped on SQLite, errors propagated on Postgres
    // (issue #550).
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN workspace_id TEXT")
        .execute(pool)
        .await;
    run_pg_only(
        pool,
        is_sqlite,
        "UPDATE sessions \
         SET workspace_id = ( \
             SELECT p.workspace_id FROM workspace_repos p WHERE p.id = sessions.project_id \
         ) \
         WHERE workspace_id IS NULL AND project_id IS NOT NULL",
    )
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_workspace_id ON sessions (workspace_id)")
        .execute(pool)
        .await?;

    // Belt-and-suspenders UNIQUE(install_id) for legacy databases created
    // before the constraint was added to CREATE TABLE. Webhook resolution
    // depends on the 1:1 install_id ↔ workspace_id invariant.
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_github_app_installations_install_id_unique \
         ON github_app_installations (install_id)",
    )
    .execute(pool)
    .await?;

    Ok(())
}
