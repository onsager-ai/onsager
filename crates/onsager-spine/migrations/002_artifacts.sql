-- Artifact model: core identity, versioning, lineage, and quality signals.
--
-- External-origin artifacts (PRs, GitHub issues, future Linear/Slack) are
-- reference-only: the spine carries identity + our derived lifecycle; the
-- dashboard hydrates provider-owned fields (title, body, author) live
-- through a portal proxy (spec #170). The CHECK constraint encodes this
-- invariant: any row with a non-NULL external_ref must have NULL name and
-- owner (spec #336, enforced at INSERT time).
--
-- Note: workflow_id and workflow_version_id FK columns are added in
-- 004_workflows.sql after the referenced tables exist.

CREATE TABLE IF NOT EXISTS artifacts (
    artifact_id     TEXT        NOT NULL PRIMARY KEY,
    kind            TEXT        NOT NULL,
    -- NULL for external-origin artifacts (reference-only per spec #170/336).
    name            TEXT,
    owner           TEXT,
    created_by      TEXT        NOT NULL,
    state           TEXT        NOT NULL DEFAULT 'draft'
                        CHECK (state IN ('draft', 'in_progress', 'under_review', 'released', 'archived')),
    current_version INTEGER     NOT NULL DEFAULT 0,
    consumers       JSONB       NOT NULL DEFAULT '[]',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Registry pointers (type catalog, adapter, external system of record).
    type_id         TEXT,
    adapter_id      TEXT,
    external_ref    TEXT,
    metadata        JSONB       NOT NULL DEFAULT '{}',
    workspace_id    TEXT        NOT NULL DEFAULT 'default',
    -- git_context carries repo/branch/PR linkage for PR artifacts.
    git_context     JSONB,
    -- Provenance: whether content is deterministic/external/uncertain (spec #348).
    -- Default covers legacy and externally-ingested rows produced before the
    -- workflow runtime existed.
    provenance      JSONB       NOT NULL DEFAULT '{"kind": "deterministic", "source": "external"}',
    produced_by_node UUID,
    -- last_observed_at: dashboard renders "last seen N min ago" on cache miss.
    last_observed_at TIMESTAMPTZ,
    -- Workflow execution state — populated by the workflow runtime.
    -- workflow_id and workflow_version_id are added as FK columns in
    -- 004_workflows.sql after workflows / workflow_versions are created.
    current_stage_index    INTEGER,
    workflow_parked_reason TEXT,
    -- External-ref constraint: provider-authored fields must not be denormalized
    -- into the spine row when external_ref is set.
    CONSTRAINT artifacts_external_ref_no_provider_fields
        CHECK (external_ref IS NULL OR (name IS NULL AND owner IS NULL))
);

CREATE INDEX IF NOT EXISTS idx_artifacts_state              ON artifacts (state);
CREATE INDEX IF NOT EXISTS idx_artifacts_kind               ON artifacts (kind);
CREATE INDEX IF NOT EXISTS idx_artifacts_owner              ON artifacts (owner);
CREATE INDEX IF NOT EXISTS idx_artifacts_type               ON artifacts (type_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_workspace          ON artifacts (workspace_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_adapter            ON artifacts (adapter_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_external_ref       ON artifacts (external_ref)
    WHERE external_ref IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_artifacts_provenance         ON artifacts USING gin (provenance jsonb_path_ops);
CREATE INDEX IF NOT EXISTS idx_artifacts_produced_by_node   ON artifacts (produced_by_node)
    WHERE produced_by_node IS NOT NULL;

-- Artifact versions: immutable snapshots (content_ref never updated).
CREATE TABLE IF NOT EXISTS artifact_versions (
    artifact_id          TEXT        NOT NULL REFERENCES artifacts(artifact_id),
    version              INTEGER     NOT NULL,
    content_ref_uri      TEXT        NOT NULL,
    content_ref_checksum TEXT,
    change_summary       TEXT        NOT NULL DEFAULT '',
    created_by_session   TEXT        NOT NULL,
    parent_version       INTEGER,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (artifact_id, version)
);

-- Vertical lineage: which session shaped which version.
CREATE TABLE IF NOT EXISTS vertical_lineage (
    id          BIGSERIAL   NOT NULL PRIMARY KEY,
    artifact_id TEXT        NOT NULL REFERENCES artifacts(artifact_id),
    version     INTEGER     NOT NULL,
    session_id  TEXT        NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (artifact_id, version, session_id)
);

CREATE INDEX IF NOT EXISTS idx_vertical_lineage_artifact ON vertical_lineage (artifact_id);

-- Horizontal lineage: which other artifacts were used as inputs.
CREATE TABLE IF NOT EXISTS horizontal_lineage (
    id                 BIGSERIAL   NOT NULL PRIMARY KEY,
    artifact_id        TEXT        NOT NULL REFERENCES artifacts(artifact_id),
    source_artifact_id TEXT        NOT NULL REFERENCES artifacts(artifact_id),
    source_version     INTEGER     NOT NULL,
    role               TEXT        NOT NULL,
    recorded_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_horizontal_lineage_artifact ON horizontal_lineage (artifact_id);
CREATE INDEX IF NOT EXISTS idx_horizontal_lineage_source   ON horizontal_lineage (source_artifact_id);

-- Quality signals: append-only records about artifact quality (never updated).
CREATE TABLE IF NOT EXISTS quality_signals (
    id          BIGSERIAL   NOT NULL PRIMARY KEY,
    artifact_id TEXT        NOT NULL REFERENCES artifacts(artifact_id),
    source      TEXT        NOT NULL,
    dimension   TEXT        NOT NULL,
    value       JSONB       NOT NULL,
    recorded_by TEXT        NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_quality_signals_artifact  ON quality_signals (artifact_id);
CREATE INDEX IF NOT EXISTS idx_quality_signals_dimension ON quality_signals (artifact_id, dimension);

CREATE OR REPLACE FUNCTION update_artifact_timestamp() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS artifact_updated ON artifacts;
CREATE TRIGGER artifact_updated BEFORE UPDATE ON artifacts
    FOR EACH ROW EXECUTE FUNCTION update_artifact_timestamp();
