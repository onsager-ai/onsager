-- Onsager 0.2 substrate — issue #361 (OBS-01).
--
-- ADR 0013 makes Observers the second substrate citizen: they
-- subscribe to spine events, run analysis off the hot path, and emit
-- typed outputs (QualitySignal / Insight / Alert) into this table.
-- Observers cannot write to `events` / `events_ext`; that's the
-- constitutive "observers audit, not manage" property. This table
-- is observer-only.
--
-- Schema (issue #361 § "Approach"):
--   - one row per emitted `ObserverOutput`
--   - `kind` is a closed enum (CHECK constraint) — `quality_signal`,
--     `insight`, `alert`; matches the Rust `ObserverOutputKind`.
--   - `triggered_by_event_id` carries the `events.id` of the row
--     that caused this output, so the dashboard can render
--     "Insight raised because of event #123". Nullable because some
--     observers may emit outputs not anchored to a single event (a
--     periodic summary, an aggregate at flush time). FK with
--     ON DELETE SET NULL — losing the trigger row drops the link but
--     keeps the audit trail.
--   - `payload` is JSONB carrying the full `ObserverOutput` JSON
--     (including the `kind` discriminator); the runtime's record
--     path serializes via `serde_json::to_value`.
--
-- Pre-launch posture — brand-new table; no backfill obligations.

CREATE TABLE IF NOT EXISTS observer_outputs (
    id                    BIGSERIAL PRIMARY KEY,
    observer_id           TEXT NOT NULL,
    kind                  TEXT NOT NULL
        CHECK (kind IN ('quality_signal', 'insight', 'alert')),
    triggered_by_event_id BIGINT
        REFERENCES events(id) ON DELETE SET NULL,
    payload               JSONB NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Dashboard reads are dominated by "show me the latest outputs from
-- observer X" and "show me the latest alerts (any observer)". Both
-- want id-descending paging.
CREATE INDEX IF NOT EXISTS observer_outputs_observer_id_id_idx
    ON observer_outputs (observer_id, id DESC);
CREATE INDEX IF NOT EXISTS observer_outputs_kind_id_idx
    ON observer_outputs (kind, id DESC);

-- Triggered-by lookup: "what did observers say about event #X" is
-- the dashboard's drill-down from the event timeline into the
-- observer audit trail.
CREATE INDEX IF NOT EXISTS observer_outputs_triggered_by_event_id_idx
    ON observer_outputs (triggered_by_event_id)
    WHERE triggered_by_event_id IS NOT NULL;
