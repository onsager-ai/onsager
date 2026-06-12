-- v2 schema, migration 001 — identity and tenancy (ADR 0029 / M0 #622).
--
-- Only the tables this milestone's routes consume. The run-loop tables
-- (workflows, runs, sessions, artifacts, events) land with their
-- consumers in M1 — no producer without a consumer.
--
-- Applied once by the boot-time runner (`db::migrate`), which records
-- filenames in `schema_migrations`; files are immutable after merge.

CREATE TABLE users (
    id                TEXT PRIMARY KEY,
    github_id         BIGINT NOT NULL UNIQUE,
    github_login      TEXT NOT NULL,
    github_name       TEXT,
    github_avatar_url TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE auth_sessions (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users (id),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_auth_sessions_user_id ON auth_sessions (user_id);

CREATE TABLE workspaces (
    id         TEXT PRIMARY KEY,
    slug       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    created_by TEXT NOT NULL REFERENCES users (id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE workspace_members (
    workspace_id TEXT NOT NULL REFERENCES workspaces (id),
    user_id      TEXT NOT NULL REFERENCES users (id),
    joined_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, user_id)
);

-- Workspace-scoped secrets (CLAUDE_CODE_OAUTH_TOKEN, ...), AES-256-GCM
-- encrypted with ONSAGER_CREDENTIAL_KEY; values never leave the server.
-- Workspace-level (not per-user): roles are owner-only in v2.
CREATE TABLE credentials (
    workspace_id TEXT NOT NULL REFERENCES workspaces (id),
    name         TEXT NOT NULL,
    value_cipher TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, name)
);
