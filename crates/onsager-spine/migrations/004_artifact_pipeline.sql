-- Onsager Level 3: Factory pipeline foundations.
-- See GitHub issue #14 for the design: artifact types, adapters, registry-as-artifacts.
--
-- Load-bearing decisions this migration enshrines:
--   1. Types are data (DB-backed registry), not code.
--   2. Artifacts are thin handles: (type_id, adapter_id, external_ref, metadata).
--   3. workspace_id exists on every row from day 1; default 'default'.
--   4. Registry entries (types, adapters, evaluators, profiles) are themselves artifacts,
--      mutated via spine events — this table is a materialized view of that event log.

-- -- Artifact extensions ------------------------------------------------------
-- Additive columns on the existing artifacts table (migration 002). Existing
-- rows get NULL external refs and the default workspace.

ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS type_id       TEXT,
    ADD COLUMN IF NOT EXISTS adapter_id    TEXT,
    ADD COLUMN IF NOT EXISTS external_ref  TEXT,
    ADD COLUMN IF NOT EXISTS metadata      JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS workspace_id  TEXT  NOT NULL DEFAULT 'default';

CREATE INDEX IF NOT EXISTS idx_artifacts_type       ON artifacts (type_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_workspace  ON artifacts (workspace_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_adapter    ON artifacts (adapter_id);

-- -- Registry tables ----------------------------------------------------------
-- Each registry table is the current projection of a stream of registry events
-- (`type.proposed`, `type.approved`, …). Writes here must go through the event
-- store so the audit trail and projection stay in sync.

-- Artifact type catalog.
CREATE TABLE IF NOT EXISTS artifact_types (
    type_id       TEXT        NOT NULL,
    workspace_id  TEXT        NOT NULL DEFAULT 'default',
    revision      INTEGER     NOT NULL DEFAULT 1,
    status        TEXT        NOT NULL DEFAULT 'approved'
                      CHECK (status IN ('proposed', 'approved', 'deprecated')),
    definition    JSONB       NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (workspace_id, type_id)
);

CREATE INDEX IF NOT EXISTS idx_artifact_types_status ON artifact_types (status);

-- Gate evaluator catalog. Each evaluator is identified by a stable id and
-- carries its config (thresholds, required reviewers, etc.) as JSON so that
-- changing parameters is a registry update, not a code change.
CREATE TABLE IF NOT EXISTS gate_evaluators (
    evaluator_id  TEXT        NOT NULL,
    workspace_id  TEXT        NOT NULL DEFAULT 'default',
    revision      INTEGER     NOT NULL DEFAULT 1,
    status        TEXT        NOT NULL DEFAULT 'approved'
                      CHECK (status IN ('proposed', 'approved', 'deprecated')),
    config        JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (workspace_id, evaluator_id)
);

-- Agent profile catalog. Profiles are reusable bundles of (role, system prompt,
-- tools, model) that types reference by id so one change propagates everywhere.
CREATE TABLE IF NOT EXISTS agent_profiles (
    profile_id    TEXT        NOT NULL,
    workspace_id  TEXT        NOT NULL DEFAULT 'default',
    revision      INTEGER     NOT NULL DEFAULT 1,
    status        TEXT        NOT NULL DEFAULT 'approved'
                      CHECK (status IN ('proposed', 'approved', 'deprecated')),
    config        JSONB       NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (workspace_id, profile_id)
);

-- Adapter catalog. Adapters bind a type to an external system of record
-- (GitHub issue/PR, Railway env, git tag, Notion page, …).
CREATE TABLE IF NOT EXISTS artifact_adapters (
    adapter_id    TEXT        NOT NULL,
    workspace_id  TEXT        NOT NULL DEFAULT 'default',
    revision      INTEGER     NOT NULL DEFAULT 1,
    status        TEXT        NOT NULL DEFAULT 'approved'
                      CHECK (status IN ('proposed', 'approved', 'deprecated')),
    config        JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (workspace_id, adapter_id)
);

-- -- Bootstrap termination marker --------------------------------------------
-- Records the fact that a seed catalog was applied, so the seed loader can be
-- idempotent: rerunning emits zero events once a workspace has been seeded.
CREATE TABLE IF NOT EXISTS registry_seed_marker (
    workspace_id  TEXT        NOT NULL,
    seed_name     TEXT        NOT NULL,
    seeded_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (workspace_id, seed_name)
);

-- -- Updated-at triggers -----------------------------------------------------
CREATE OR REPLACE FUNCTION update_registry_timestamp() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS artifact_types_updated   ON artifact_types;
CREATE TRIGGER artifact_types_updated   BEFORE UPDATE ON artifact_types
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();

DROP TRIGGER IF EXISTS gate_evaluators_updated  ON gate_evaluators;
CREATE TRIGGER gate_evaluators_updated  BEFORE UPDATE ON gate_evaluators
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();

DROP TRIGGER IF EXISTS agent_profiles_updated   ON agent_profiles;
CREATE TRIGGER agent_profiles_updated   BEFORE UPDATE ON agent_profiles
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();

DROP TRIGGER IF EXISTS artifact_adapters_updated ON artifact_adapters;
CREATE TRIGGER artifact_adapters_updated BEFORE UPDATE ON artifact_adapters
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();
