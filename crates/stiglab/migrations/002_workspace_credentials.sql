-- 002_workspace_credentials.sql (issue #164, child of #161)
--
-- Per-workspace credential scope. Credentials (CLAUDE_CODE_OAUTH_TOKEN,
-- ANTHROPIC_API_KEY, etc.) were stored as global per-(user, name) rows
-- and silently shipped to every agent session that user launched —
-- regardless of which workspace the session belonged to. After this
-- migration each row is tied to exactly one workspace, the route moves
-- under `/api/workspaces/:workspace/credentials`, and stiglab's session
-- launcher looks up the credential by `(user_id, session.workspace_id)`
-- instead of "any of the user's credentials of this name".
--
-- Webhook ingress: this same migration installs the
-- `UNIQUE(install_id)` invariant on `github_app_installations` (1:1
-- install→workspace, per the parent #161 open question). The
-- fresh-DB CREATE TABLE in `run_migrations` already declares the
-- column UNIQUE; the unique-index added below covers existing
-- Postgres deployments that shipped without the constraint.
--
-- Sessions also gain a `workspace_id` column so `/api/sessions?workspace=`
-- can filter directly without joining through projects (sessions without
-- a project — direct/personal sessions — keep `workspace_id IS NULL` and
-- aren't surfaced by any workspace-scoped list).
--
-- Backfill rule mirrors the PAT rule from #163:
--   * `user_credentials.workspace_id IS NULL` rows get pinned to the
--     user's first `workspace_members` membership (lowest `joined_at`,
--     ties broken by `workspace_id`).
--   * Rows for users with zero memberships are deleted — they're
--     unreachable from the new route shape.
--   * The column is then tightened to `NOT NULL`.
--   * The legacy `UNIQUE(user_id, name)` is replaced with
--     `UNIQUE(user_id, workspace_id, name)` so two workspaces may
--     legitimately host the same credential name for the same user.
--
-- This file is the canonical documentation of the migration. The live
-- migration is the inlined DDL in
-- `crates/stiglab/src/server/db.rs::run_migrations`, wrapped in
-- idempotent guards so a re-run is a no-op.

-- ── Per-workspace credentials ──

ALTER TABLE user_credentials ADD COLUMN IF NOT EXISTS workspace_id TEXT;

UPDATE user_credentials
SET workspace_id = (
    SELECT m.workspace_id
    FROM workspace_members m
    WHERE m.user_id = user_credentials.user_id
    ORDER BY m.joined_at ASC, m.workspace_id ASC
    LIMIT 1
)
WHERE workspace_id IS NULL;

DELETE FROM user_credentials WHERE workspace_id IS NULL;

ALTER TABLE user_credentials ALTER COLUMN workspace_id SET NOT NULL;

-- Drop the legacy unique key (created by the original
-- `UNIQUE(user_id, name)` table definition) and replace it with the
-- workspace-aware shape.  The exact constraint name is Postgres-default
-- (`<table>_<col>_<col>_key`); SQLite uses an implicit index that the
-- new CREATE TABLE supersedes on rebuild.
ALTER TABLE user_credentials
    DROP CONSTRAINT IF EXISTS user_credentials_user_id_name_key;
DROP INDEX IF EXISTS user_credentials_user_id_name_key;

CREATE UNIQUE INDEX IF NOT EXISTS idx_user_credentials_user_workspace_name
    ON user_credentials (user_id, workspace_id, name);

-- ── Sessions: workspace_id ──

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS workspace_id TEXT;

UPDATE sessions
SET workspace_id = (
    SELECT p.workspace_id FROM projects p
    WHERE p.id = sessions.project_id
)
WHERE workspace_id IS NULL AND project_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_sessions_workspace_id
    ON sessions (workspace_id);

-- ── Webhook → workspace 1:1 invariant ──

CREATE UNIQUE INDEX IF NOT EXISTS idx_github_app_installations_install_id
    ON github_app_installations (install_id);
