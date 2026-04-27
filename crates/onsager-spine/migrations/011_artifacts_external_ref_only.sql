-- Onsager spec #170 / #171: reference-only artifacts for external items.
--
-- External-source artifacts (PRs, GitHub issues, future Linear/Slack) no
-- longer denormalize provider-owned fields (`name` = title, `owner` = author
-- login) into the spine. The portal proxies live to GitHub for those; the
-- spine row carries identity + our derived state only.
--
-- Three changes:
--   1. `artifacts.name` and `artifacts.owner` become nullable. Existing rows
--      keep their stale denormalized values as best-effort cached display
--      under proxy outage; new external-source writes set them NULL.
--   2. Add `last_observed_at TIMESTAMPTZ` so the dashboard can render
--      "last seen N min ago" placeholders when the proxy cache misses and
--      GitHub is rate-limited or unreachable (#170 fail-open decision).
--   3. No data migration. This is purely additive — existing reads keep
--      working, and the portal stops writing the columns going forward.

ALTER TABLE artifacts
    ALTER COLUMN name DROP NOT NULL,
    ALTER COLUMN owner DROP NOT NULL;

ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS last_observed_at TIMESTAMPTZ;
