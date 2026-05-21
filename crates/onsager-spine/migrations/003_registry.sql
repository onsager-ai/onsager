-- Factory pipeline registry.
--
-- Registry entries are data, not code: types, adapters, evaluators, and
-- agent profiles are DB-backed. Each table is the current projection of
-- a stream of registry events (type.proposed, type.approved, …); writes
-- must go through the event store so the audit trail and projection stay
-- in sync.
--
-- workspace_id on every row supports workspace-scoped listing.
-- registry_seed_marker records applied seed catalogs for idempotency.

CREATE OR REPLACE FUNCTION update_registry_timestamp() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Artifact type catalog.
CREATE TABLE IF NOT EXISTS artifact_types (
    workspace_id TEXT        NOT NULL DEFAULT 'default',
    type_id      TEXT        NOT NULL,
    revision     INTEGER     NOT NULL DEFAULT 1,
    status       TEXT        NOT NULL DEFAULT 'approved'
                     CHECK (status IN ('proposed', 'approved', 'deprecated')),
    definition   JSONB       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, type_id)
);

CREATE INDEX IF NOT EXISTS idx_artifact_types_status ON artifact_types (status);

DROP TRIGGER IF EXISTS artifact_types_updated ON artifact_types;
CREATE TRIGGER artifact_types_updated BEFORE UPDATE ON artifact_types
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();

-- Gate evaluator catalog. Config carries thresholds, required reviewers, etc.
-- Changing parameters is a registry update, not a code change.
CREATE TABLE IF NOT EXISTS gate_evaluators (
    workspace_id TEXT        NOT NULL DEFAULT 'default',
    evaluator_id TEXT        NOT NULL,
    revision     INTEGER     NOT NULL DEFAULT 1,
    status       TEXT        NOT NULL DEFAULT 'approved'
                     CHECK (status IN ('proposed', 'approved', 'deprecated')),
    config       JSONB       NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, evaluator_id)
);

DROP TRIGGER IF EXISTS gate_evaluators_updated ON gate_evaluators;
CREATE TRIGGER gate_evaluators_updated BEFORE UPDATE ON gate_evaluators
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();

-- Agent profile catalog. Profiles bundle (role, system prompt, tools, model)
-- so one profile change propagates to every workflow that references it.
CREATE TABLE IF NOT EXISTS agent_profiles (
    workspace_id TEXT        NOT NULL DEFAULT 'default',
    profile_id   TEXT        NOT NULL,
    revision     INTEGER     NOT NULL DEFAULT 1,
    status       TEXT        NOT NULL DEFAULT 'approved'
                     CHECK (status IN ('proposed', 'approved', 'deprecated')),
    config       JSONB       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, profile_id)
);

DROP TRIGGER IF EXISTS agent_profiles_updated ON agent_profiles;
CREATE TRIGGER agent_profiles_updated BEFORE UPDATE ON agent_profiles
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();

-- Adapter catalog. Adapters bind a type to an external system of record
-- (GitHub issue/PR, Railway env, git tag, Notion page, …).
CREATE TABLE IF NOT EXISTS artifact_adapters (
    workspace_id TEXT        NOT NULL DEFAULT 'default',
    adapter_id   TEXT        NOT NULL,
    revision     INTEGER     NOT NULL DEFAULT 1,
    status       TEXT        NOT NULL DEFAULT 'approved'
                     CHECK (status IN ('proposed', 'approved', 'deprecated')),
    config       JSONB       NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, adapter_id)
);

DROP TRIGGER IF EXISTS artifact_adapters_updated ON artifact_adapters;
CREATE TRIGGER artifact_adapters_updated BEFORE UPDATE ON artifact_adapters
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();

-- Seed marker: records which seed catalogs have been applied so re-seeding
-- is idempotent (re-run emits zero events once a workspace has been seeded).
CREATE TABLE IF NOT EXISTS registry_seed_marker (
    workspace_id TEXT        NOT NULL,
    seed_name    TEXT        NOT NULL,
    seeded_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, seed_name)
);
