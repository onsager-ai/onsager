-- Spine: event bus foundation.
--
-- Two append-only tables form the coordination backbone:
--   events     — typed factory events (pg_notify on every INSERT)
--   events_ext — wide extension events carrying adapter-specific payloads
--
-- The notify_event trigger publishes to 'onsager_events' on every INSERT
-- so listeners never poll; the JSONB payload carries correlation_id so
-- portal can await a specific response without a DB round-trip (spec #223).

CREATE TABLE IF NOT EXISTS events (
    id             BIGSERIAL   PRIMARY KEY,
    stream_id      TEXT        NOT NULL,
    stream_type    TEXT        NOT NULL,
    event_type     TEXT        NOT NULL,
    data           JSONB       NOT NULL,
    metadata       JSONB       NOT NULL DEFAULT '{}',
    sequence       BIGINT      NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    correlation_id UUID,
    UNIQUE (stream_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_events_stream          ON events (stream_id, sequence);
CREATE INDEX IF NOT EXISTS idx_events_type            ON events (event_type);
CREATE INDEX IF NOT EXISTS idx_events_created         ON events (created_at);
CREATE INDEX IF NOT EXISTS idx_events_correlation_id  ON events (correlation_id)
    WHERE correlation_id IS NOT NULL;

-- Extension events carry namespaced adapter payloads.
-- workspace_id is a first-class column (not embedded in data JSONB) to
-- support indexed per-workspace filtering.
-- adapter_id + external_ref are the idempotency key for dedup between
-- the webhook receiver and the reconciliation poller (spec #121): whichever
-- arrives first wins; the other is a silent no-op via the partial unique index.
CREATE TABLE IF NOT EXISTS events_ext (
    id             BIGSERIAL   PRIMARY KEY,
    stream_id      TEXT        NOT NULL,
    namespace      TEXT        NOT NULL,
    event_type     TEXT        NOT NULL,
    data           JSONB       NOT NULL,
    metadata       JSONB       NOT NULL DEFAULT '{}',
    ref_event_id   BIGINT      REFERENCES events(id),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    correlation_id UUID,
    workspace_id   TEXT        NOT NULL DEFAULT 'default',
    adapter_id     TEXT,
    external_ref   TEXT
);

CREATE INDEX IF NOT EXISTS idx_events_ext_stream          ON events_ext (stream_id);
CREATE INDEX IF NOT EXISTS idx_events_ext_namespace       ON events_ext (namespace, event_type);
CREATE INDEX IF NOT EXISTS idx_events_ext_correlation_id  ON events_ext (correlation_id)
    WHERE correlation_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_ext_workspace_id    ON events_ext (workspace_id, id DESC);

CREATE UNIQUE INDEX IF NOT EXISTS uniq_events_ext_adapter_external_ref
    ON events_ext (adapter_id, external_ref)
    WHERE adapter_id IS NOT NULL AND external_ref IS NOT NULL;

CREATE OR REPLACE FUNCTION notify_event() RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('onsager_events', json_build_object(
        'table',          TG_TABLE_NAME,
        'id',             NEW.id,
        'stream_id',      NEW.stream_id,
        'event_type',     NEW.event_type,
        'correlation_id', NEW.correlation_id
    )::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS events_notify ON events;
CREATE TRIGGER events_notify AFTER INSERT ON events
    FOR EACH ROW EXECUTE FUNCTION notify_event();

DROP TRIGGER IF EXISTS events_ext_notify ON events_ext;
CREATE TRIGGER events_ext_notify AFTER INSERT ON events_ext
    FOR EACH ROW EXECUTE FUNCTION notify_event();
