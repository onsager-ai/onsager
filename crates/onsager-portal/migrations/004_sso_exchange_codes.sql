-- Portal-owned schema: short-lived opaque codes used by cross-environment
-- SSO delegation. The owner mints a code at the OAuth callback and 302s
-- to the relying party; the relying party redeems server-to-server with a
-- shared bearer secret to learn the user identity.
--
-- `redeemed_at` is NULL until a relying party successfully exchanges the
-- code for the user identity; the UPDATE that flips it is the single-use
-- gate (see `auth_db::redeem_sso_exchange_code`).
--
-- Spec #222 Slice 5 — moved from stiglab's inline `CREATE TABLE` block.

CREATE TABLE IF NOT EXISTS sso_exchange_codes (
    code           TEXT PRIMARY KEY,
    user_id        TEXT NOT NULL,
    return_to_host TEXT NOT NULL,
    expires_at     TEXT NOT NULL,
    redeemed_at    TEXT,
    created_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sso_exchange_codes_expires_at
    ON sso_exchange_codes (expires_at);
