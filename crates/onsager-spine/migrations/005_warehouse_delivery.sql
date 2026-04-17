-- Onsager Level 4: Warehouse & Delivery v0.1
-- See specs/warehouse-and-delivery-v0.1.md for the data model specification.
--
-- Adds three concepts:
--   1. bundles        — immutable, content-addressed snapshots of a released artifact.
--   2. deliveries     — per-consumer delivery records with independent retry.
--   3. consumer_sinks — the enabled external destinations for a given artifact.
--
-- Also augments artifacts with current_bundle_id / bundle_history per §4.1.

-- -- Artifact augmentation ----------------------------------------------------
ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS current_bundle_id TEXT,
    ADD COLUMN IF NOT EXISTS bundle_history    JSONB NOT NULL DEFAULT '[]'::jsonb;

-- -- Bundles ------------------------------------------------------------------
-- A bundle is immutable once sealed. Writers must never UPDATE any column
-- after the initial INSERT. The (artifact_id, version) uniqueness constraint
-- enforces invariant §9.2 (monotonic versions).
CREATE TABLE IF NOT EXISTS bundles (
    bundle_id      TEXT        PRIMARY KEY,
    artifact_id    TEXT        NOT NULL REFERENCES artifacts(artifact_id),
    version        INTEGER     NOT NULL,
    supersedes     TEXT        REFERENCES bundles(bundle_id),
    manifest       JSONB       NOT NULL,
    content_ref    TEXT        NOT NULL,
    sealed_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sealed_by      TEXT        NOT NULL,
    metadata       JSONB       NOT NULL DEFAULT '{}'::jsonb,

    UNIQUE (artifact_id, version)
);

CREATE INDEX IF NOT EXISTS idx_bundles_artifact ON bundles (artifact_id);
CREATE INDEX IF NOT EXISTS idx_bundles_supersedes ON bundles (supersedes);

-- Now that bundles exists, add the FK from artifacts.current_bundle_id.
-- (DO block so the migration is idempotent.)
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'artifacts_current_bundle_fkey'
    ) THEN
        ALTER TABLE artifacts
            ADD CONSTRAINT artifacts_current_bundle_fkey
            FOREIGN KEY (current_bundle_id) REFERENCES bundles(bundle_id);
    END IF;
END;
$$;

-- -- Consumer sinks -----------------------------------------------------------
-- The enabled external destinations for a given artifact. Named `consumer_sinks`
-- (not `consumers`) to avoid colliding with the existing `consumers` JSONB
-- column on the artifacts table, which holds declared consumer identities.
CREATE TABLE IF NOT EXISTS consumer_sinks (
    consumer_id    TEXT        PRIMARY KEY,
    artifact_id    TEXT        NOT NULL REFERENCES artifacts(artifact_id),
    kind           TEXT        NOT NULL,
    config         JSONB       NOT NULL DEFAULT '{}'::jsonb,
    retry_policy   JSONB       NOT NULL DEFAULT '{}'::jsonb,
    enabled        BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_consumer_sinks_artifact ON consumer_sinks (artifact_id);
CREATE INDEX IF NOT EXISTS idx_consumer_sinks_enabled  ON consumer_sinks (enabled) WHERE enabled;

-- -- Deliveries ---------------------------------------------------------------
-- Per (bundle, consumer) delivery attempts with independent retry.
-- At-least-once semantics (invariant §9.6) — consumers must idempotent-key on
-- (bundle_id, consumer_id).
CREATE TABLE IF NOT EXISTS deliveries (
    delivery_id    TEXT        PRIMARY KEY,
    bundle_id      TEXT        NOT NULL REFERENCES bundles(bundle_id),
    consumer_id    TEXT        NOT NULL REFERENCES consumer_sinks(consumer_id),
    kind           TEXT        NOT NULL
                       CHECK (kind IN ('initial', 'rework')),
    prior_receipt  JSONB,
    status         TEXT        NOT NULL DEFAULT 'pending'
                       CHECK (status IN ('pending', 'in_flight', 'succeeded', 'failed', 'abandoned')),
    attempts       INTEGER     NOT NULL DEFAULT 0,
    last_error     TEXT,
    receipt        JSONB,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- One row per (bundle, consumer): retries update attempts/status in place,
    -- not via additional rows. This enforces the at-least-once idempotency
    -- model (§9.6) at the schema level.
    UNIQUE (bundle_id, consumer_id)
);

CREATE INDEX IF NOT EXISTS idx_deliveries_bundle  ON deliveries (bundle_id);
CREATE INDEX IF NOT EXISTS idx_deliveries_status  ON deliveries (status);
CREATE INDEX IF NOT EXISTS idx_deliveries_pending ON deliveries (status)
    WHERE status = 'pending';

-- -- Updated-at trigger ---------------------------------------------------
-- Reuse update_registry_timestamp() from migration 004.
DROP TRIGGER IF EXISTS consumer_sinks_updated ON consumer_sinks;
CREATE TRIGGER consumer_sinks_updated BEFORE UPDATE ON consumer_sinks
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();

DROP TRIGGER IF EXISTS deliveries_updated ON deliveries;
CREATE TRIGGER deliveries_updated BEFORE UPDATE ON deliveries
    FOR EACH ROW EXECUTE FUNCTION update_registry_timestamp();
