-- Onsager spec #336: mechanical check for denormalized external state.
--
-- Spec #170 / #171 made external-origin artifacts reference-only: the
-- spine carries identity + our derived lifecycle, the dashboard hydrates
-- provider-authored fields (title, body, author, labels) live through a
-- portal proxy. The contract test in
-- `crates/onsager-portal/tests/reference_only_artifacts.rs` pinned the
-- two helpers that exist today (`upsert_pr_artifact_ref`,
-- `upsert_issue_artifact_ref`, `touch_artifact`) but a future external
-- integration (Linear, Slack, Sentry, …) adding a new `upsert_*_artifact`
-- that stuffs the provider-authored title into `artifacts.name` would be
-- a clean clippy-and-test-passing regression.
--
-- This CHECK constraint encodes the invariant in the schema itself: any
-- row with a non-NULL `external_ref` must have NULL `name` and NULL
-- `owner`. Internal-origin artifacts (no `external_ref`) still write
-- `name` / `owner` freely — the constraint only binds the external case.
--
-- Two-step ordering matters:
--
--   1. NULL out pre-#170 soft-fallback rows that kept their stale
--      denormalized `name` / `owner` (migration 011's "no data migration"
--      decision). Pre-launch posture (CLAUDE.md): no users to protect,
--      the proxy is the canonical source for those fields, no reason to
--      preserve a cached fallback that's just drift waiting to surface.
--   2. Add the CHECK constraint fully validated (no `NOT VALID`).
--      `NOT VALID` would have skipped only the creation-time scan —
--      Postgres still evaluates the CHECK on every subsequent UPDATE,
--      so leaving the legacy rows in place would have broken the next
--      webhook touch (`upsert_pr_artifact_ref`'s
--      `UPDATE artifacts SET state = $2, last_observed_at = NOW()`
--      writes a row whose post-image still violates the predicate).
--      Backfilling first lets the constraint be fully VALID end-to-end.

UPDATE artifacts
    SET name = NULL, owner = NULL
    WHERE external_ref IS NOT NULL
      AND (name IS NOT NULL OR owner IS NOT NULL);

ALTER TABLE artifacts
    DROP CONSTRAINT IF EXISTS artifacts_external_ref_no_provider_fields;

ALTER TABLE artifacts
    ADD CONSTRAINT artifacts_external_ref_no_provider_fields
        CHECK (external_ref IS NULL OR (name IS NULL AND owner IS NULL));
