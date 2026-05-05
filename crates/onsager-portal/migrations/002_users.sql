-- Portal-owned schema: identity for everyone who signs in to Onsager
-- (real GitHub OAuth users plus the negative-id dev user seeded in
-- debug builds).
--
-- Spec #222 Slice 5 — moved from stiglab's inline `CREATE TABLE` block
-- to the portal migrations directory. Stiglab still reads this table
-- through its `AuthUser` extractor (cookie validation) until Slice 2/3
-- moves the dependent routes; portal is the only writer.
--
-- `id` is a UUID/string PK so the on-the-wire shape stays stable across
-- backends. `github_id` is the external identity key — `UNIQUE` so OAuth
-- callbacks can `ON CONFLICT(github_id) DO UPDATE` without a separate
-- upsert path.

CREATE TABLE IF NOT EXISTS users (
    id                TEXT PRIMARY KEY,
    github_id         BIGINT NOT NULL UNIQUE,
    github_login      TEXT NOT NULL,
    github_name       TEXT,
    github_avatar_url TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
