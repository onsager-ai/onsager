-- Sidecar cursor table for outbox-row triggers (#239).
--
-- Each `OutboxRow` workflow gets one row here. The forge poller reads
-- the watched table with
--
--     SELECT id, ... FROM <table>
--      WHERE id > $cursor AND <where_clause>
--      ORDER BY id ASC
--
-- and advances the cursor atomically with the `TriggerFired` emit.
-- This keeps the watched tables read-only from the trigger system's
-- perspective — no intrusive `processed_at` / `consumed_by` columns,
-- which would force schema changes on tables we don't own.

CREATE TABLE IF NOT EXISTS outbox_trigger_cursor (
    workflow_id   TEXT PRIMARY KEY,
    -- Highest `id` we have already emitted a TriggerFired for.
    -- Default 0: any non-negative `id` row will be picked up.
    last_seen_id  BIGINT NOT NULL DEFAULT 0,
    last_seen_at  TIMESTAMPTZ,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT outbox_trigger_cursor_workflow_fk
        FOREIGN KEY (workflow_id) REFERENCES workflows(workflow_id)
        ON DELETE CASCADE
);

CREATE OR REPLACE FUNCTION outbox_trigger_cursor_touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS outbox_trigger_cursor_touch ON outbox_trigger_cursor;
CREATE TRIGGER outbox_trigger_cursor_touch
BEFORE UPDATE ON outbox_trigger_cursor
FOR EACH ROW EXECUTE FUNCTION outbox_trigger_cursor_touch_updated_at();
