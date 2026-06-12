-- v2 schema, migration 002 — the run loop (ADR 0029 / M1 #623).
--
-- Workflow is the authored unit (trigger + ordered stages, both jsonb —
-- the executable form is derived at fire time, never persisted twice).
-- Run is the first-class execution record; Artifact is a run product
-- (content XOR external ref, provenance from day one); events is the
-- single append-only audit table — the record, not the protocol.

CREATE TABLE workflows (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces (id),
    name         TEXT NOT NULL,
    trigger      JSONB NOT NULL,
    definition   JSONB NOT NULL,
    active       BOOLEAN NOT NULL DEFAULT FALSE,
    created_by   TEXT NOT NULL REFERENCES users (id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_workflows_workspace ON workflows (workspace_id);

CREATE TABLE runs (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces (id),
    workflow_id  TEXT NOT NULL REFERENCES workflows (id),
    -- queued | running | completed | failed (M3 adds aborted with its
    -- abort endpoint)
    status       TEXT NOT NULL,
    error        TEXT,
    started_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at  TIMESTAMPTZ
);
CREATE INDEX idx_runs_workflow ON runs (workflow_id, started_at DESC);
CREATE INDEX idx_runs_workspace ON runs (workspace_id, started_at DESC);

CREATE TABLE sessions (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces (id),
    run_id       TEXT NOT NULL REFERENCES runs (id),
    -- Bearer token the agent process presents to /mcp/messages.
    -- Session-scoped, short-lived, local to this row.
    token        TEXT NOT NULL UNIQUE,
    -- running | completed | failed
    status       TEXT NOT NULL,
    -- Set by the declare_no_output MCP tool: "delivered nothing on
    -- purpose" is distinguishable from "delivered nothing".
    declared_no_output_reason TEXT,
    error        TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at  TIMESTAMPTZ
);
CREATE INDEX idx_sessions_run ON sessions (run_id);

CREATE TABLE artifacts (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces (id),
    run_id       TEXT REFERENCES runs (id),
    session_id   TEXT REFERENCES sessions (id),
    kind         TEXT NOT NULL,
    name         TEXT NOT NULL,
    -- Internal-authored artifacts carry content; external-referenced
    -- ones (M2: PRs, issues) carry external_ref. Exactly one is set.
    content_uri      TEXT,
    content_checksum TEXT,
    external_ref     JSONB,
    -- {"kind": "uncertain"|"deterministic", "source": "agent"|...}
    provenance   JSONB NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT artifacts_content_xor_external CHECK (
        (content_uri IS NOT NULL) != (external_ref IS NOT NULL)
    )
);
CREATE INDEX idx_artifacts_workspace ON artifacts (workspace_id, created_at DESC);
CREATE INDEX idx_artifacts_run ON artifacts (run_id);

-- The record, not the protocol (ADR 0029): append-only audit log read
-- by the run timeline (and M3's activity feed). Nothing dispatches off
-- it.
CREATE TABLE events (
    id           BIGSERIAL PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    run_id       TEXT,
    kind         TEXT NOT NULL,
    payload      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_events_run ON events (run_id, id);
CREATE INDEX idx_events_workspace ON events (workspace_id, id DESC);
