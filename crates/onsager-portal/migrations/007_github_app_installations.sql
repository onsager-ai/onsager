-- Onsager #222 Slice 3b: github_app_installations moves into portal.
--
-- The table holds the (workspace, GitHub App install) link that webhook
-- ingestion, OAuth callback, and project onboarding all key on. Slice 3a
-- (PR #254) moved workspaces / workspace_members / projects into the
-- spine; per-installation state stays edge-shaped — only portal needs to
-- decrypt the stored webhook secret cipher and mint installation tokens
-- for the App flow — so it lives in the portal migrations directory next
-- to user_credentials / user_pats / portal_webhook_secrets.
--
-- Portal becomes the sole writer (the install-flow + manual-register
-- routes move with this migration). Stiglab keeps idempotent
-- CREATE TABLE IF NOT EXISTS DDL in `server/db.rs::run_migrations` so
-- SQLite-backed integration tests build the schema without portal in
-- the loop, and so a fresh deploy's first migrator wins. Stiglab also
-- keeps reader functions on the same Postgres table (different
-- connection pool) for `routes/projects.rs` live-data hydration.

CREATE TABLE IF NOT EXISTS github_app_installations (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    install_id BIGINT NOT NULL UNIQUE,
    account_login TEXT NOT NULL,
    account_type TEXT NOT NULL,
    webhook_secret_cipher TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_github_app_installations_workspace_id
    ON github_app_installations (workspace_id);
