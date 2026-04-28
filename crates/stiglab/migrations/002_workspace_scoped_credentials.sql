-- 002_workspace_scoped_credentials.sql (issue #164, child C of #161)
--
-- Per-workspace credential isolation, workspace-scoped sessions, and a
-- belt-and-suspenders UNIQUE on github_app_installations.install_id.
--
-- After this migration:
--   * Credentials are scoped to (user_id, workspace_id, name) — a user has
--     a separate `CLAUDE_CODE_OAUTH_TOKEN` per workspace, not a global one.
--   * Sessions carry `workspace_id` so the agent runner can fetch the
--     workspace's credentials at dispatch time.
--   * `github_app_installations.install_id` is enforced UNIQUE so the
--     webhook ingress can resolve `install_id → workspace_id` deterministically.
--     Per the parent spec (#161 open question, decided in #164):
--     install_id ↔ workspace_id is a 1:1 invariant.
--
-- This file is the canonical documentation; the actual DDL is executed at
-- startup by `crates/stiglab/src/server/db.rs`'s `run_migrations` because
-- stiglab still uses an inlined-SQL migration runner (no sqlx-migrate).

-- ── user_credentials.workspace_id ──
--
-- Backfill rule:
--   1. Add the column nullable.
--   2. Backfill each row to the credential owner's first workspace
--      membership (lowest joined_at, ties broken by workspace_id).
--   3. Drop rows whose owner has no membership at all — those credentials
--      were unreachable anyway under the new per-workspace contract.
--   4. Tighten the column to NOT NULL.
--   5. Replace the unique index with a per-(workspace, user, name) one so
--      the same credential name can exist in multiple workspaces for the
--      same user.

ALTER TABLE user_credentials
    ADD COLUMN IF NOT EXISTS workspace_id TEXT;

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

-- The legacy `UNIQUE(user_id, name)` predates per-workspace scoping and
-- would now block a user from holding the same credential name in two
-- workspaces. Drop it; rely on the new index below.
DROP INDEX IF EXISTS user_credentials_user_id_name_key;
DROP INDEX IF EXISTS idx_user_credentials_user_name;

CREATE UNIQUE INDEX IF NOT EXISTS idx_user_credentials_workspace_user_name
    ON user_credentials (workspace_id, user_id, name);

CREATE INDEX IF NOT EXISTS idx_user_credentials_workspace
    ON user_credentials (workspace_id);

-- ── sessions.workspace_id ──
--
-- Sessions are routed at dispatch time using the dispatcher's workspace
-- context (project's workspace_id, or the caller's workspace for direct
-- task POSTs). Backfill existing rows from the joined project; rows
-- without a project remain NULL — they are pre-#164 personal sessions
-- and will not appear under any `?workspace=` filter (matching the
-- spec's "no merged worldview" intent).

ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS workspace_id TEXT;

UPDATE sessions
SET workspace_id = (
    SELECT p.workspace_id FROM projects p WHERE p.id = sessions.project_id
)
WHERE workspace_id IS NULL AND project_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_sessions_workspace_id
    ON sessions (workspace_id);

-- ── github_app_installations.install_id UNIQUE ──
--
-- The CREATE TABLE above already declares `install_id BIGINT NOT NULL UNIQUE`,
-- but legacy databases created before that declaration carry the column
-- without the constraint. Add a unique index defensively so webhook
-- resolution can rely on the 1:1 invariant.
--
-- Pre-existing duplicates would need manual cleanup before this index can
-- build; if such a database exists in production, this CREATE INDEX will
-- fail and the operator must reconcile the duplicates first.

CREATE UNIQUE INDEX IF NOT EXISTS idx_github_app_installations_install_id_unique
    ON github_app_installations (install_id);
