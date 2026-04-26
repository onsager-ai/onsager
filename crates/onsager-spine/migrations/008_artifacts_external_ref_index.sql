-- Onsager spine: index on artifacts.external_ref.
--
-- The forge trigger subscriber dedups `trigger.fired` events by looking up
-- an existing artifact via its stable external_ref (e.g.
-- `forge:trigger:{wf}:github_issue_webhook:{owner}/{repo}#{n}`). Without an
-- index, that lookup is a sequential scan on `artifacts` for every webhook
-- delivery — fine while the table is tiny, a problem as soon as it grows.
--
-- Partial index: most rows have `external_ref IS NULL` (created via the
-- manual REST path or older subsystems), so indexing only non-NULL values
-- keeps the index small while still serving the dedup query.

CREATE INDEX IF NOT EXISTS idx_artifacts_external_ref
    ON artifacts (external_ref)
    WHERE external_ref IS NOT NULL;
