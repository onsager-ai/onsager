-- Workspace + workflow substrate.
--
-- Workspaces and projects are cross-cutting identity tables; portal is the
-- only writer, every subsystem reads from the spine pool. ingestion_mode
-- controls how the reconciliation poller operates (spec #121).
--
-- Workflows are declarative production-line blueprints: trigger + ordered
-- stages + a versioned content history (spec #337):
--   workflows              — identity + active version pointer
--   workflow_stages        — current stage chain (drives the forge stage runner)
--   workflow_versions      — versioned content snapshots (audit + rollback)
--   workflow_changes       — audit trail of every state-changing operation
--   workflow_edit_policies — per-workspace edit mode (direct / draft_then_publish)
--   workflow_trigger_state — sidecar cursor for schedule-based triggers (#238)
--   outbox_trigger_cursor  — sidecar cursor for outbox-row triggers (#239)
--   workflow_library       — template catalog indexed by spec_kind + version
--
-- artifacts.workflow_id and artifacts.workflow_version_id are added at the
-- bottom of this file (as ALTER TABLE) because their FK targets are created
-- here first.

-- ── Workspaces ───────────────────────────────────────────────────────────────

-- Timestamps stored as TEXT: matches the existing Rust read/write path
-- (stiglab SQLite heritage); upgrading to TIMESTAMPTZ is a follow-up.
CREATE TABLE IF NOT EXISTS workspaces (
    id         TEXT NOT NULL PRIMARY KEY,
    slug       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_members (
    workspace_id TEXT NOT NULL,
    user_id      TEXT NOT NULL,
    joined_at    TEXT NOT NULL,
    PRIMARY KEY (workspace_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_workspace_members_user_id ON workspace_members (user_id);

CREATE TABLE IF NOT EXISTS projects (
    id                         TEXT NOT NULL PRIMARY KEY,
    workspace_id               TEXT NOT NULL,
    github_app_installation_id TEXT NOT NULL,
    repo_owner                 TEXT NOT NULL,
    repo_name                  TEXT NOT NULL,
    default_branch             TEXT NOT NULL,
    created_at                 TEXT NOT NULL,
    ingestion_mode             TEXT NOT NULL DEFAULT 'webhook+reconciler',
    UNIQUE (workspace_id, repo_owner, repo_name),
    CONSTRAINT projects_ingestion_mode_check
        CHECK (ingestion_mode IN ('webhook+reconciler', 'polling-only', 'webhook-only'))
);

CREATE INDEX IF NOT EXISTS idx_projects_workspace_id ON projects (workspace_id);

-- ── Workflows ────────────────────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION update_workflow_timestamp() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- workflows.active_version_id FK is added after workflow_versions is created
-- (forward reference: workflow_versions references workflows).
CREATE TABLE IF NOT EXISTS workflows (
    workflow_id    TEXT        NOT NULL PRIMARY KEY,
    name           TEXT        NOT NULL,
    -- trigger_kind is validated at the application layer (registry-driven),
    -- not via a CHECK constraint, so adding new trigger kinds needs no DDL.
    trigger_kind   TEXT        NOT NULL,
    trigger_config JSONB       NOT NULL DEFAULT '{}',
    active         BOOLEAN     NOT NULL DEFAULT FALSE,
    preset_id      TEXT,
    install_id     TEXT,
    created_by     TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    workspace_id   TEXT        NOT NULL DEFAULT 'default'
);

CREATE INDEX IF NOT EXISTS idx_workflows_active       ON workflows (active);
CREATE INDEX IF NOT EXISTS idx_workflows_trigger_kind ON workflows (trigger_kind);
CREATE INDEX IF NOT EXISTS idx_workflows_workspace    ON workflows (workspace_id);

DROP TRIGGER IF EXISTS workflow_updated ON workflows;
CREATE TRIGGER workflow_updated BEFORE UPDATE ON workflows
    FOR EACH ROW EXECUTE FUNCTION update_workflow_timestamp();

-- Ordered stage chain; each stage's gates must resolve before the runner
-- advances. stage_order is 0-based.
CREATE TABLE IF NOT EXISTS workflow_stages (
    workflow_id  TEXT    NOT NULL REFERENCES workflows(workflow_id) ON DELETE CASCADE,
    stage_order  INTEGER NOT NULL,
    name         TEXT    NOT NULL,
    target_state TEXT
        CHECK (target_state IS NULL OR target_state IN
               ('draft', 'in_progress', 'under_review', 'released', 'archived')),
    gates        JSONB   NOT NULL DEFAULT '[]',
    params       JSONB   NOT NULL DEFAULT '{}',
    PRIMARY KEY (workflow_id, stage_order)
);

CREATE INDEX IF NOT EXISTS idx_workflow_stages_workflow
    ON workflow_stages (workflow_id, stage_order);

-- Versioned content snapshots (spec #337). Each publish creates one row;
-- the workflow's active_version_id pointer (added below) tracks the current.
CREATE TABLE IF NOT EXISTS workflow_versions (
    version_id        TEXT        NOT NULL PRIMARY KEY,
    workflow_id       TEXT        NOT NULL REFERENCES workflows(workflow_id) ON DELETE CASCADE,
    version_label     TEXT        NOT NULL,
    content           JSONB       NOT NULL,
    parent_version_id TEXT        REFERENCES workflow_versions(version_id),
    state             TEXT        NOT NULL
        CHECK (state IN ('draft', 'published', 'retired')),
    created_by        TEXT        NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at      TIMESTAMPTZ,
    UNIQUE (workflow_id, version_label)
);

CREATE INDEX IF NOT EXISTS idx_workflow_versions_workflow
    ON workflow_versions (workflow_id, state);

-- Now that workflow_versions exists, add the active-version pointer.
ALTER TABLE workflows
    ADD COLUMN IF NOT EXISTS active_version_id TEXT
        REFERENCES workflow_versions(version_id);

-- Audit trail: one row per state-changing operation (spec #337).
CREATE TABLE IF NOT EXISTS workflow_changes (
    change_id      BIGSERIAL   NOT NULL PRIMARY KEY,
    workflow_id    TEXT        NOT NULL REFERENCES workflows(workflow_id) ON DELETE CASCADE,
    version_id     TEXT        REFERENCES workflow_versions(version_id) ON DELETE SET NULL,
    actor          TEXT        NOT NULL,
    action         TEXT        NOT NULL
        CHECK (action IN ('create_draft', 'edit_draft', 'publish', 'retire', 'revert')),
    before_content JSONB,
    after_content  JSONB,
    reason         TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workflow_changes_workflow
    ON workflow_changes (workflow_id, created_at DESC);

-- Per-workspace edit mode. Weakening from draft_then_publish to direct
-- is blocked at the application layer while any workflow has in-flight runs.
CREATE TABLE IF NOT EXISTS workflow_edit_policies (
    workspace_id TEXT        NOT NULL PRIMARY KEY,
    edit_mode    TEXT        NOT NULL DEFAULT 'draft_then_publish'
        CHECK (edit_mode IN ('direct', 'draft_then_publish')),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Sidecar cursor for schedule-based triggers (spec #238). Separate from the
-- workflows row so future trigger categories can add their own sidecars
-- without growing the workflow schema.
CREATE OR REPLACE FUNCTION workflow_trigger_state_touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE IF NOT EXISTS workflow_trigger_state (
    workflow_id   TEXT        NOT NULL PRIMARY KEY,
    last_fired_at TIMESTAMPTZ NOT NULL,
    last_payload  JSONB,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT workflow_trigger_state_workflow_fk
        FOREIGN KEY (workflow_id) REFERENCES workflows(workflow_id) ON DELETE CASCADE
);

DROP TRIGGER IF EXISTS workflow_trigger_state_touch ON workflow_trigger_state;
CREATE TRIGGER workflow_trigger_state_touch
    BEFORE UPDATE ON workflow_trigger_state
    FOR EACH ROW EXECUTE FUNCTION workflow_trigger_state_touch_updated_at();

-- Sidecar cursor for outbox-row triggers (spec #239). Keeps watched tables
-- read-only from the trigger system's perspective — no intrusive
-- processed_at / consumed_by columns.
CREATE OR REPLACE FUNCTION outbox_trigger_cursor_touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE IF NOT EXISTS outbox_trigger_cursor (
    workflow_id  TEXT   NOT NULL PRIMARY KEY,
    last_seen_id BIGINT NOT NULL DEFAULT 0,
    last_seen_at TIMESTAMPTZ,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT outbox_trigger_cursor_workflow_fk
        FOREIGN KEY (workflow_id) REFERENCES workflows(workflow_id) ON DELETE CASCADE
);

DROP TRIGGER IF EXISTS outbox_trigger_cursor_touch ON outbox_trigger_cursor;
CREATE TRIGGER outbox_trigger_cursor_touch
    BEFORE UPDATE ON outbox_trigger_cursor
    FOR EACH ROW EXECUTE FUNCTION outbox_trigger_cursor_touch_updated_at();

-- Workflow template catalog (spec #351 / ADR 0016). Maps spec_kind → reusable
-- Workflow template. One row per (spec_kind, version); latest active version
-- wins. The unique constraint name is keyed by the substrate's DuplicateKind
-- error so unrelated constraints don't get misclassified.
CREATE TABLE IF NOT EXISTS workflow_library (
    id            TEXT        NOT NULL PRIMARY KEY,
    spec_kind     TEXT        NOT NULL,
    version       INTEGER     NOT NULL,
    workflow_json JSONB       NOT NULL,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at    TIMESTAMPTZ,
    CONSTRAINT workflow_library_kind_version_unique UNIQUE (spec_kind, version)
);

-- Partial index for the hot path "latest active version for a kind".
-- Filtered to non-retired rows so retired tail rows don't bloat the scan.
CREATE INDEX IF NOT EXISTS idx_workflow_library_active_kind
    ON workflow_library (spec_kind, version DESC)
    WHERE retired_at IS NULL;

-- ── Artifact workflow columns ─────────────────────────────────────────────────
--
-- These FK columns reference tables created above, so they must be added
-- after this file's table definitions are in place.

ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS workflow_id TEXT
        REFERENCES workflows(workflow_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_artifacts_workflow_id ON artifacts (workflow_id);

ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS workflow_version_id TEXT
        REFERENCES workflow_versions(version_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_artifacts_workflow_version ON artifacts (workflow_version_id);
