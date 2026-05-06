-- Onsager #222 Slice 3a: workspaces / workspace_members / projects move
-- into the spine.
--
-- These three tables are cross-cutting — `artifacts.workspace_id` joins
-- to `workspaces.id` (no FK constraint declared, in keeping with the
-- rest of stiglab/spine schema; app-layer enforcement only), every
-- subsystem that joins agent-session work to a tenant boundary needs
-- `workspace_members` for membership checks, and `projects` is the
-- (workspace, repo) pair that webhook ingestion and workflow CRUD both
-- index on. Per Lever B, the schema lives where the data is canonical
-- — in the spine — rather than in the subsystem that happens to own a
-- CRUD route over it.
--
-- Slice 2a's pattern: portal becomes the only writer to these tables;
-- stiglab keeps its own `db::*` reads (same database, separate connection
-- pool) for the in-process needs of `tasks.rs` / `sessions.rs` / the
-- workflow runtime. Stiglab also keeps idempotent `CREATE TABLE IF NOT
-- EXISTS` fallbacks in `server/db.rs::run_migrations` so SQLite-backed
-- integration tests build the schema without a portal binary in the loop,
-- and so a fresh deploy's first migrator wins.
--
-- Slice 3b moves `github_app_installations` to portal's migrations
-- directory. This migration leaves that table in stiglab's runtime path
-- until 3b lands.

CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_members (
    workspace_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    joined_at TEXT NOT NULL,
    PRIMARY KEY (workspace_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_workspace_members_user_id
    ON workspace_members (user_id);

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    github_app_installation_id TEXT NOT NULL,
    repo_owner TEXT NOT NULL,
    repo_name TEXT NOT NULL,
    default_branch TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(workspace_id, repo_owner, repo_name)
);

CREATE INDEX IF NOT EXISTS idx_projects_workspace_id
    ON projects (workspace_id);
