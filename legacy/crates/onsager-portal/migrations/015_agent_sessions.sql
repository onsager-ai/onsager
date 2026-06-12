-- Portal owns the chat-session tables now that stiglab is retired
-- (ADR 0027 / spec #583). Stiglab's startup `CREATE TABLE IF NOT EXISTS`
-- path died with the crate, so the DDL moves here: an existing deploy
-- no-ops (same-shape IF NOT EXISTS), a fresh deploy gets the final
-- shape directly (the legacy ALTER-chain columns are inlined).

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    -- Constant 'portal' post-#583: the session runs in-process; the
    -- column survives so the dashboard's session wire shape is stable.
    node_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    prompt TEXT NOT NULL,
    output TEXT,
    working_dir TEXT,
    user_id TEXT,
    project_id TEXT,
    workspace_id TEXT,
    artifact_id TEXT,
    artifact_version INTEGER,
    idempotency_key TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_artifact_id ON sessions (artifact_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_idempotency_key
    ON sessions (idempotency_key);

CREATE TABLE IF NOT EXISTS session_logs (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    chunk TEXT NOT NULL,
    stream TEXT NOT NULL DEFAULT 'stdout',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_logs_session_id
    ON session_logs (session_id, seq);

-- The agent-node fleet is gone — dispatch is in-process, no external
-- nodes ever registered in production (pre-launch). Drop its table.
DROP TABLE IF EXISTS nodes;
