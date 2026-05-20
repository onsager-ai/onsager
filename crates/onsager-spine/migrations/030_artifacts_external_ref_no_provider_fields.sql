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
-- `NOT VALID` skips validation of the existing rows. The pre-spec-#170
-- soft-fallback rows kept their stale denormalized `name` / `owner` as
-- best-effort cached display under proxy outage (migration 011's "no
-- data migration" decision); the umbrella's resolved policy was "leave
-- them" and this migration preserves that. New INSERTs and UPDATEs are
-- still checked — `NOT VALID` is forward-looking only.
--
-- If a future spec re-decides the soft-fallback policy, the followup
-- migration nulls the legacy rows and runs
-- `ALTER TABLE artifacts VALIDATE CONSTRAINT artifacts_external_ref_no_provider_fields`
-- to retrofit the check across the whole table.

ALTER TABLE artifacts
    DROP CONSTRAINT IF EXISTS artifacts_external_ref_no_provider_fields;

ALTER TABLE artifacts
    ADD CONSTRAINT artifacts_external_ref_no_provider_fields
        CHECK (external_ref IS NULL OR (name IS NULL AND owner IS NULL))
        NOT VALID;
