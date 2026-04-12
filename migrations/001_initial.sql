-- Onsager Core Level 1: Event Stream Schema

-- Core events table (append-only)
CREATE TABLE IF NOT EXISTS events (
    id          BIGSERIAL PRIMARY KEY,
    stream_id   TEXT NOT NULL,
    stream_type TEXT NOT NULL,
    event_type  TEXT NOT NULL,
    data        JSONB NOT NULL,
    metadata    JSONB NOT NULL DEFAULT '{}',
    sequence    BIGINT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(stream_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_events_stream ON events (stream_id, sequence);
CREATE INDEX IF NOT EXISTS idx_events_type ON events (event_type);
CREATE INDEX IF NOT EXISTS idx_events_created ON events (created_at);

-- Extension events table (wide JSON, namespaced by component)
CREATE TABLE IF NOT EXISTS events_ext (
    id           BIGSERIAL PRIMARY KEY,
    stream_id    TEXT NOT NULL,
    namespace    TEXT NOT NULL,
    event_type   TEXT NOT NULL,
    data         JSONB NOT NULL,
    metadata     JSONB NOT NULL DEFAULT '{}',
    ref_event_id BIGINT REFERENCES events(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_events_ext_stream ON events_ext (stream_id);
CREATE INDEX IF NOT EXISTS idx_events_ext_namespace ON events_ext (namespace, event_type);

-- pg_notify trigger for real-time subscription
CREATE OR REPLACE FUNCTION notify_event() RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('onsager_events', json_build_object(
        'table', TG_TABLE_NAME,
        'id', NEW.id,
        'stream_id', NEW.stream_id,
        'event_type', NEW.event_type
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
