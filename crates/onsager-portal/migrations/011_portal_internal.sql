-- Portal-owned operational tables that do not map cleanly to the auth or
-- external-integration groupings above. All three tables existed as inline
-- DDL in migrate.rs; this file is the canonical source.
--
-- factory_tasks — backlog rows materialized from spec-labeled GitHub issues.
-- pr_gate_verdicts — per-(pr_artifact_id, head_sha) gate evaluation record,
--   used for idempotency so replayed webhooks don't double-evaluate.
-- pr_branch_links — per-session branch hint, used to attach vertical_lineage
--   when the PR webhook arrives. Also created by stiglab's startup path;
--   both sides use CREATE TABLE IF NOT EXISTS so whoever runs first wins.

CREATE TABLE IF NOT EXISTS factory_tasks (
    id         TEXT        NOT NULL PRIMARY KEY,
    project_id TEXT        NOT NULL,
    source     TEXT        NOT NULL DEFAULT 'manual',
    source_ref TEXT        NOT NULL,
    title      TEXT        NOT NULL,
    body       TEXT,
    state      TEXT        NOT NULL DEFAULT 'queued',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, source_ref)
);

CREATE INDEX IF NOT EXISTS idx_factory_tasks_project ON factory_tasks (project_id);
CREATE INDEX IF NOT EXISTS idx_factory_tasks_state   ON factory_tasks (state);

CREATE TABLE IF NOT EXISTS pr_gate_verdicts (
    pr_artifact_id TEXT        NOT NULL,
    head_sha       TEXT        NOT NULL,
    verdict        TEXT        NOT NULL,
    recorded_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (pr_artifact_id, head_sha)
);

CREATE TABLE IF NOT EXISTS pr_branch_links (
    session_id  TEXT   NOT NULL PRIMARY KEY,
    project_id  TEXT,
    branch      TEXT   NOT NULL,
    pr_number   BIGINT,
    recorded_at TEXT   NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pr_branch_links_lookup
    ON pr_branch_links (project_id, branch);
