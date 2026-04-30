-- Onsager #223 — typed correlation_id on the spine.
--
-- The portal-feedback contract (read / fast write / slow write) needs a
-- first-class column to correlate an HTTP request with the events it
-- produces. Until now correlation_id only lived inside the JSON payload
-- and the metadata blob, which means:
--   - filtering on it requires JSON traversal, no usable index;
--   - the pg_notify payload doesn't carry it, so subscribers can't filter
--     without round-tripping to the DB for every notification.
--
-- This migration:
--   1. Adds a UUID column to both `events` and `events_ext` (NULL allowed
--      — background events without an originating HTTP request).
--   2. Adds a partial index on each so portal's `await_response` can
--      lookup by correlation_id in O(log n) without scanning the JSONB.
--   3. Extends the `notify_event` trigger to include `correlation_id` in
--      the pg_notify payload, letting in-process subscribers filter
--      without a DB roundtrip.
--
-- The column is *not* derived from JSON by a trigger — writers must bind
-- it explicitly. `EventStore::append_factory_event{,_tx}` /
-- `append_ext` parse `EventMetadata.correlation_id` (with a fallback to
-- `FactoryEvent.correlation_id`) as a UUID and bind to `$N`. Pre-#223
-- producers that stop at the JSON envelope simply land NULL in the
-- column — which is fine, the partial index is `WHERE correlation_id IS
-- NOT NULL`, and only portal-minted UUIDs need to be awaited.

ALTER TABLE events     ADD COLUMN IF NOT EXISTS correlation_id UUID;
ALTER TABLE events_ext ADD COLUMN IF NOT EXISTS correlation_id UUID;

CREATE INDEX IF NOT EXISTS idx_events_correlation_id
    ON events (correlation_id)
    WHERE correlation_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_events_ext_correlation_id
    ON events_ext (correlation_id)
    WHERE correlation_id IS NOT NULL;

-- Extend the notify_event trigger to surface correlation_id in the
-- pg_notify payload. Subscribers (notably portal::feedback::await_response)
-- read the field directly off the notification and never have to query
-- the row to know whether it belongs to the request they're awaiting.
CREATE OR REPLACE FUNCTION notify_event() RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('onsager_events', json_build_object(
        'table', TG_TABLE_NAME,
        'id', NEW.id,
        'stream_id', NEW.stream_id,
        'event_type', NEW.event_type,
        'correlation_id', NEW.correlation_id
    )::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
