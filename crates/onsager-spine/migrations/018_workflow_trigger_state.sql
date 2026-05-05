-- Sidecar table for schedule-trigger state (#238).
--
-- The scheduler in forge ticks every 5s, finds active workflows whose
-- trigger kind is `cron` / `delay` / `interval`, computes the next
-- fire-at, and emits `TriggerFired` once `next_fire_at <= now()`.
-- Persisting `last_fired_at` here (instead of growing the `workflows`
-- row) keeps the workflow schema kind-agnostic — future trigger
-- categories add their own sidecar tables rather than packing every
-- variant's transient state into a single row.
--
-- Idempotency: `TriggerFired.payload.last_fired_at` carries this
-- timestamp so forge's `trigger_subscriber` dedupes on
-- `(workflow_id, last_fired_at)` if the scheduler restarts mid-emit.

CREATE TABLE IF NOT EXISTS workflow_trigger_state (
    workflow_id   TEXT PRIMARY KEY,
    last_fired_at TIMESTAMPTZ NOT NULL,
    -- The most recent payload we emitted for this workflow. Mostly
    -- diagnostic; the scheduler doesn't read it back.
    last_payload  JSONB,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Workflow row may be deleted; we don't want to keep an
    -- orphaned cursor around if so.
    CONSTRAINT workflow_trigger_state_workflow_fk
        FOREIGN KEY (workflow_id) REFERENCES workflows(workflow_id)
        ON DELETE CASCADE
);

-- Bump `updated_at` on every write so an operator can spot triggers
-- that haven't fired in a long time without scanning emit logs.
CREATE OR REPLACE FUNCTION workflow_trigger_state_touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS workflow_trigger_state_touch ON workflow_trigger_state;
CREATE TRIGGER workflow_trigger_state_touch
BEFORE UPDATE ON workflow_trigger_state
FOR EACH ROW EXECUTE FUNCTION workflow_trigger_state_touch_updated_at();
