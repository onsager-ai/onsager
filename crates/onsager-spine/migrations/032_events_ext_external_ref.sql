-- Onsager #121 — events_ext idempotency key (adapter_id, external_ref).
--
-- The reconciliation contract (#121) has the webhook receiver and the
-- poll-based reconciler write to the same idempotency key: whichever
-- side arrives first wins; the second is a silent no-op via DB
-- constraint, not application-level coordination. The shape of that
-- key is `(adapter_id, external_ref)` where `external_ref` is the
-- adapter-normalized stable identity of the underlying resource
-- (e.g. `github:project:<id>:issue:42` for GitHub-sourced issues —
-- the existing format already in use on `artifacts.external_ref`).
--
-- Today's `events_ext` rows do not carry these fields explicitly;
-- adapter identity and external refs live in the `data`/`metadata`
-- JSONB blobs and there is no unique constraint enforcing dedup.
-- This migration adds the columns nullable so backfill is not
-- required for existing rows, and lays down a partial unique index
-- that only constrains rows where both columns are set — webhook /
-- poller emitters opt in by populating them.
--
-- Adapter-aware emitters (#121 follow-up: webhook translator refactor)
-- will populate these columns. Once every emit path is populated,
-- a follow-up spec can ratchet the partial index into a full one
-- or replace it with a stricter check.
--
-- See spec #121 § Design / "Dedup is unique-index, not coordination".

ALTER TABLE events_ext
    ADD COLUMN IF NOT EXISTS adapter_id   TEXT,
    ADD COLUMN IF NOT EXISTS external_ref TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS uniq_events_ext_adapter_external_ref
    ON events_ext (adapter_id, external_ref)
    WHERE adapter_id IS NOT NULL AND external_ref IS NOT NULL;
