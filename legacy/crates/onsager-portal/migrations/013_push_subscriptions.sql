-- Portal-owned schema: Web Push subscriptions for HITL notifications
-- (ADR 0020 § Invocation channels, spec #472).
--
-- Each row is one device + browser combination the user has authorised
-- to receive push. A user can have multiple subscriptions (laptop,
-- phone, work browser, …); push dispatch fans out to all of them and
-- prunes rows that the push service marks as `gone` (HTTP 410).
--
-- `endpoint` is the push service URL (FCM / APNs / Mozilla autopush /
-- Edge / …) the browser handed back from `pushManager.subscribe()`.
-- `p256dh_key` and `auth_secret` are the keys used to encrypt the push
-- payload — they're public (per the Web Push standard) but still
-- user-private, so we keep them in portal alongside other user state.
-- No FK on `workspace_id` — same pattern as other portal tables; joins
-- happen at the app layer.

CREATE TABLE IF NOT EXISTS push_subscriptions (
    id           TEXT         NOT NULL PRIMARY KEY,
    user_id      TEXT         NOT NULL,
    -- Subscriptions are scoped to the workspace the user was in when
    -- they enabled push. Dispatch can fan to all of a user's
    -- workspace-scoped subscriptions or filter to one workspace.
    workspace_id TEXT         NOT NULL,
    endpoint     TEXT         NOT NULL,
    p256dh_key   TEXT         NOT NULL,
    auth_secret  TEXT         NOT NULL,
    user_agent   TEXT,
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,
    -- One row per (user, endpoint). Re-subscribing the same browser
    -- upserts the keys instead of duplicating the row.
    UNIQUE (user_id, endpoint)
);

CREATE INDEX IF NOT EXISTS idx_push_subscriptions_user
    ON push_subscriptions (user_id);
CREATE INDEX IF NOT EXISTS idx_push_subscriptions_workspace
    ON push_subscriptions (workspace_id);
