-- v2 schema, migration 003 — GitHub App installations (M2 #624).
--
-- One row per App installation, maintained by the App's own webhook
-- (`installation` created/deleted events). The run's repo-access token
-- is minted against the installation whose account owns the trigger
-- repo. Reference-only: GitHub owns everything else about the install.

CREATE TABLE github_app_installations (
    installation_id BIGINT PRIMARY KEY,
    account_login   TEXT NOT NULL,
    account_kind    TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_gh_installs_account ON github_app_installations (account_login);
