-- 001_rename_tenant_to_workspace.sql (issue #163, child of #161)
--
-- Hard rename of the stiglab "tenant" surface to "workspace" — the term the
-- rest of the product (and the dashboard) already uses.  Aligns the column
-- names with the spine/dashboard vocabulary so we stop translating
-- `tenant_id ↔ workspace_id` at every seam.
--
-- Tables renamed:
--   * tenants                 -> workspaces
--   * tenant_members          -> workspace_members
--   * tenant_workflows        -> workspace_workflows
--   * tenant_workflow_stages  -> workspace_workflow_stages
--
-- Columns renamed (all):
--   * tenant_id -> workspace_id
--
-- `user_pats.workspace_id` is tightened from NULL-able to NOT NULL.  Backfill
-- rule:
--   * Rows with a non-null pin keep their pin verbatim.
--   * Rows with NULL get pinned to the user's first `workspace_members`
--     membership (lowest `joined_at`, ties broken by `workspace_id`).
--   * Rows with NO membership at all are deleted — they were unusable anyway
--     (a PAT with no workspace context can't address a workspace-scoped
--     route, and after this migration `workspace_id` is mandatory at the DB
--     level).  We log nothing here; auditing the old behaviour is moot.
--
-- This file is the canonical documentation of the migration.  The actual
-- DDL is executed at startup by `crates/stiglab/src/server/db.rs`'s
-- `run_migrations` because stiglab still uses an inlined-SQL migration runner
-- (no sqlx-migrate).  The DDL there is wrapped in idempotent guards so a
-- re-run against a database already on this schema is a no-op.

-- ── Tables ──

ALTER TABLE IF EXISTS tenants RENAME TO workspaces;
ALTER TABLE IF EXISTS tenant_members RENAME TO workspace_members;
ALTER TABLE IF EXISTS tenant_workflows RENAME TO workspace_workflows;
ALTER TABLE IF EXISTS tenant_workflow_stages RENAME TO workspace_workflow_stages;

-- ── Columns ──

ALTER TABLE workspace_members         RENAME COLUMN tenant_id TO workspace_id;
ALTER TABLE github_app_installations  RENAME COLUMN tenant_id TO workspace_id;
ALTER TABLE projects                  RENAME COLUMN tenant_id TO workspace_id;
ALTER TABLE workspace_workflows       RENAME COLUMN tenant_id TO workspace_id;
ALTER TABLE user_pats                 RENAME COLUMN tenant_id TO workspace_id;

-- ── Indexes (drop old, create new) ──

DROP   INDEX IF EXISTS idx_tenant_members_user_id;
CREATE INDEX IF NOT EXISTS idx_workspace_members_user_id
    ON workspace_members (user_id);

DROP   INDEX IF EXISTS idx_github_app_installations_tenant_id;
CREATE INDEX IF NOT EXISTS idx_github_app_installations_workspace_id
    ON github_app_installations (workspace_id);

DROP   INDEX IF EXISTS idx_projects_tenant_id;
CREATE INDEX IF NOT EXISTS idx_projects_workspace_id
    ON projects (workspace_id);

DROP   INDEX IF EXISTS idx_tenant_workflows_tenant_id;
CREATE INDEX IF NOT EXISTS idx_workspace_workflows_workspace_id
    ON workspace_workflows (workspace_id);

DROP   INDEX IF EXISTS idx_tenant_workflows_repo_active;
CREATE INDEX IF NOT EXISTS idx_workspace_workflows_repo_active
    ON workspace_workflows (repo_owner, repo_name, active);

DROP   INDEX IF EXISTS idx_tenant_workflow_stages_workflow_id;
CREATE INDEX IF NOT EXISTS idx_workspace_workflow_stages_workflow_id
    ON workspace_workflow_stages (workflow_id, seq);

-- ── PAT backfill + tighten to NOT NULL ──

-- 1. Pin NULL rows to the user's first workspace membership.
UPDATE user_pats
SET workspace_id = (
    SELECT m.workspace_id
    FROM workspace_members m
    WHERE m.user_id = user_pats.user_id
    ORDER BY m.joined_at ASC, m.workspace_id ASC
    LIMIT 1
)
WHERE workspace_id IS NULL;

-- 2. Drop rows that still have NULL — the user has no membership at all,
--    so the PAT was unusable.  Documented in the file header.
DELETE FROM user_pats WHERE workspace_id IS NULL;

-- 3. Tighten the column.  Postgres-only syntax; SQLite enforces NOT NULL
--    only on freshly-created tables, so the embedded migration runner skips
--    this step on SQLite (the AnyPool tests use sqlite::memory: and rebuild
--    the schema each run, so the new CREATE TABLE already encodes NOT NULL).
ALTER TABLE user_pats ALTER COLUMN workspace_id SET NOT NULL;
