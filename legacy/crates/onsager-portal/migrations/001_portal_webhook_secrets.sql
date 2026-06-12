-- Portal-owned schema: per-workspace webhook signature secret for self-hosted
-- PAT mode (spec #222 open question 3 / parent #220).
--
-- When portal creates a repo webhook on the user's behalf via a workspace PAT
-- with `admin:repo_hook`, the webhook secret needs a stable, per-workspace
-- store separate from the App-level `GITHUB_APP_WEBHOOK_SECRET` env var.
--
-- One row per workspace (matches the workspace ↔ install scope established by
-- #161). Generated at install time; surfaced to the user once via the
-- dashboard; only `secret_hash` is persisted thereafter.
--
-- `workspace_id` is intentionally TEXT without a foreign key to `workspaces`:
-- the `workspaces` table currently lives in stiglab's runtime migrations
-- (target: spine, per spec #222's schema split slice). Once the workspaces
-- move into spine, this FK can be added in a follow-up migration without
-- relaxing any constraint here.

CREATE TABLE IF NOT EXISTS portal_webhook_secrets (
    workspace_id TEXT        PRIMARY KEY,
    secret_hash  TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);
