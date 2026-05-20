-- Onsager #121 — Per-project ingestion mode selector.
--
-- Three modes (#121 § Design / "Ingestion-mode selector"):
--   * webhook+reconciler — default; webhooks for low latency,
--     poller as reconciliation backstop at a low frequency.
--   * polling-only      — local-dev / webhook-less installs; full-
--     rate polling, no public URL required.
--   * webhook-only      — opt out of the reconciler (not
--     recommended; silent drops become permanent).
--
-- Stored as a free-form TEXT column with a CHECK constraint rather
-- than a Postgres ENUM so adding a fourth mode later is one
-- migration, not a `CREATE TYPE` dance.

ALTER TABLE projects
    ADD COLUMN IF NOT EXISTS ingestion_mode TEXT NOT NULL
        DEFAULT 'webhook+reconciler';

ALTER TABLE projects
    DROP CONSTRAINT IF EXISTS projects_ingestion_mode_check;

ALTER TABLE projects
    ADD CONSTRAINT projects_ingestion_mode_check
        CHECK (ingestion_mode IN
            ('webhook+reconciler', 'polling-only', 'webhook-only'));
