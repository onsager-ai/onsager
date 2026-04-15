-- Onsager Level 2: Artifact Model Schema
-- See specs/artifact-model-v0.1.md for the data model specification.

-- Artifacts table — the core identity and state record.
-- Content lives externally; this table holds metadata and pointers.
CREATE TABLE IF NOT EXISTS artifacts (
    artifact_id  TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    name         TEXT NOT NULL,
    owner        TEXT NOT NULL,
    created_by   TEXT NOT NULL,
    state        TEXT NOT NULL DEFAULT 'draft'
                     CHECK (state IN ('draft', 'in_progress', 'under_review', 'released', 'archived')),
    current_version INTEGER NOT NULL DEFAULT 0,
    consumers    JSONB NOT NULL DEFAULT '[]',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_artifacts_state ON artifacts (state);
CREATE INDEX IF NOT EXISTS idx_artifacts_kind ON artifacts (kind);
CREATE INDEX IF NOT EXISTS idx_artifacts_owner ON artifacts (owner);

-- Artifact versions — each snapshot in an artifact's lifecycle.
-- content_ref is immutable per version (artifact-model §7.7).
CREATE TABLE IF NOT EXISTS artifact_versions (
    artifact_id  TEXT NOT NULL REFERENCES artifacts(artifact_id),
    version      INTEGER NOT NULL,
    content_ref_uri TEXT NOT NULL,
    content_ref_checksum TEXT,
    change_summary TEXT NOT NULL DEFAULT '',
    created_by_session TEXT NOT NULL,
    parent_version INTEGER,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (artifact_id, version)
);

-- Vertical lineage — which session shaped which version.
CREATE TABLE IF NOT EXISTS vertical_lineage (
    id           BIGSERIAL PRIMARY KEY,
    artifact_id  TEXT NOT NULL REFERENCES artifacts(artifact_id),
    version      INTEGER NOT NULL,
    session_id   TEXT NOT NULL,
    recorded_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (artifact_id, version, session_id)
);

CREATE INDEX IF NOT EXISTS idx_vertical_lineage_artifact ON vertical_lineage (artifact_id);

-- Horizontal lineage — which other artifacts were used as inputs.
CREATE TABLE IF NOT EXISTS horizontal_lineage (
    id                  BIGSERIAL PRIMARY KEY,
    artifact_id         TEXT NOT NULL REFERENCES artifacts(artifact_id),
    source_artifact_id  TEXT NOT NULL REFERENCES artifacts(artifact_id),
    source_version      INTEGER NOT NULL,
    role                TEXT NOT NULL,
    recorded_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_horizontal_lineage_artifact ON horizontal_lineage (artifact_id);
CREATE INDEX IF NOT EXISTS idx_horizontal_lineage_source ON horizontal_lineage (source_artifact_id);

-- Quality signals — append-only records about artifact quality.
-- Never updated or deleted (artifact-model §7, §4.6).
CREATE TABLE IF NOT EXISTS quality_signals (
    id           BIGSERIAL PRIMARY KEY,
    artifact_id  TEXT NOT NULL REFERENCES artifacts(artifact_id),
    source       TEXT NOT NULL,
    dimension    TEXT NOT NULL,
    value        JSONB NOT NULL,
    recorded_by  TEXT NOT NULL,
    recorded_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_quality_signals_artifact ON quality_signals (artifact_id);
CREATE INDEX IF NOT EXISTS idx_quality_signals_dimension ON quality_signals (artifact_id, dimension);

-- Trigger to update artifacts.updated_at on state changes.
CREATE OR REPLACE FUNCTION update_artifact_timestamp() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS artifact_updated ON artifacts;
CREATE TRIGGER artifact_updated BEFORE UPDATE ON artifacts
    FOR EACH ROW EXECUTE FUNCTION update_artifact_timestamp();
