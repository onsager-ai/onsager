use crate::core::{Session, SessionState};
use chrono::Utc;
use sqlx::AnyPool;

pub async fn insert_session(pool: &AnyPool, session: &Session) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               artifact_id, artifact_version, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_sessions(pool: &AnyPool) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// Sessions assigned to `node_id` that were created while the agent was
/// not connected (state still `Pending`). The agent registration handler
/// drains these on (re)connect so a session created during a brief
/// disconnect doesn't sit in `Pending` forever.
pub async fn list_pending_sessions_for_node(
    pool: &AnyPool,
    node_id: &str,
) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE node_id = $1 AND state = 'pending' \
         ORDER BY created_at ASC",
    )
    .bind(node_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_session(pool: &AnyPool, session_id: &str) -> anyhow::Result<Option<Session>> {
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

/// Look up an existing session by its idempotency key.
///
/// `request_id` from a `ShapingRequest` (or the `Idempotency-Key` header) is
/// used as the key so that a Forge retry on a dropped connection collapses
/// onto the original session instead of dispatching a second agent
/// (issue #31).
pub async fn find_session_by_idempotency_key(
    pool: &AnyPool,
    key: &str,
) -> anyhow::Result<Option<Session>> {
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE idempotency_key = $1 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

/// Insert a session bound to an idempotency key.
///
/// Returns `Ok(true)` on a fresh insert, `Ok(false)` when a row with the same
/// key already existed and the insert was skipped (via `ON CONFLICT DO
/// NOTHING`). Callers should re-lookup on `false` to recover the winning
/// session id.
///
/// The database's unique index on `idempotency_key` is the authoritative
/// guard against concurrent POSTs with the same key — the lookup-before-
/// insert path in the handler is a fast path, not a correctness barrier.
pub async fn insert_session_with_idempotency_key(
    pool: &AnyPool,
    session: &Session,
    idempotency_key: &str,
) -> anyhow::Result<bool> {
    let affected = sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               artifact_id, artifact_version, idempotency_key, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT (idempotency_key) DO NOTHING",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(idempotency_key)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn update_session_state(
    pool: &AnyPool,
    session_id: &str,
    state: SessionState,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE sessions SET state = $1, updated_at = $2 WHERE id = $3")
        .bind(state.to_string())
        .bind(Utc::now().to_rfc3339())
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically claim a `pending` session by transitioning it to the given
/// state. Returns `true` only when this caller won the race (the row was
/// `pending` at update time and exactly one row was affected).
///
/// Used by the agent reconnect drain so a session that's already been
/// claimed by another path (e.g. the create-time WebSocket dispatch, or
/// a parallel reconnect that beat us) doesn't get sent twice.
pub async fn claim_pending_session(
    pool: &AnyPool,
    session_id: &str,
    new_state: SessionState,
) -> anyhow::Result<bool> {
    let affected = sqlx::query(
        "UPDATE sessions SET state = $1, updated_at = $2 \
         WHERE id = $3 AND state = 'pending'",
    )
    .bind(new_state.to_string())
    .bind(Utc::now().to_rfc3339())
    .bind(session_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

// ── Session Logs (append-only) ──

pub async fn append_session_log(
    pool: &AnyPool,
    session_id: &str,
    chunk: &str,
    stream: &str,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    // Use a subquery to get the next sequence number for this session
    sqlx::query(
        "INSERT INTO session_logs (id, session_id, seq, chunk, stream, created_at)
         VALUES ($1, $2, COALESCE((SELECT MAX(seq) FROM session_logs WHERE session_id = $2), 0) + 1, $3, $4, $5)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(chunk)
    .bind(stream)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Get all log chunks for a session, ordered by sequence number.
pub async fn get_session_logs(pool: &AnyPool, session_id: &str) -> anyhow::Result<Vec<LogChunk>> {
    let rows = sqlx::query_as::<_, LogChunkRow>(
        "SELECT chunk, stream, created_at FROM session_logs WHERE session_id = $1 ORDER BY seq ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// Get log chunks added after a given sequence number (for incremental SSE).
pub async fn get_session_logs_after(
    pool: &AnyPool,
    session_id: &str,
    after_seq: i64,
) -> anyhow::Result<Vec<LogChunkWithSeq>> {
    let rows = sqlx::query_as::<_, LogChunkWithSeqRow>(
        "SELECT seq, chunk, stream, created_at FROM session_logs WHERE session_id = $1 AND seq > $2 ORDER BY seq ASC",
    )
    .bind(session_id)
    .bind(after_seq)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

// ── Updated Session queries (user-scoped) ──

pub async fn insert_session_with_user(
    pool: &AnyPool,
    session: &Session,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    insert_session_with_user_project_workspace(pool, session, user_id, None, None).await
}

/// Insert a session with workspace + project context.
///
/// `workspace_id` is what the runner reads back at dispatch to fetch
/// per-workspace credentials (#164). `project_id` is the existing
/// workspace-owned project link from #59. A session can have a
/// workspace without a project (direct task POST), but never a project
/// without a workspace — callers resolve the workspace from the project
/// before this call.
pub async fn insert_session_with_user_and_project(
    pool: &AnyPool,
    session: &Session,
    user_id: Option<&str>,
    project_id: Option<&str>,
) -> anyhow::Result<()> {
    insert_session_with_user_project_workspace(pool, session, user_id, project_id, None).await
}

pub async fn insert_session_with_user_project_workspace(
    pool: &AnyPool,
    session: &Session,
    user_id: Option<&str>,
    project_id: Option<&str>,
    workspace_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               user_id, project_id, workspace_id, artifact_id, artifact_version, \
                               created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(user_id)
    .bind(project_id)
    .bind(workspace_id)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// List sessions owned by `user_id` and scoped to `workspace_id`.
///
/// Pre-#164 sessions with a NULL `workspace_id` (personal direct-task
/// POSTs that predate the column) never appear here — the spec's
/// "no merged worldview" intent is enforced by the filter, not the read.
pub async fn list_sessions_for_user_in_workspace(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: &str,
) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE user_id = $1 AND workspace_id = $2 \
         ORDER BY created_at DESC",
    )
    .bind(user_id)
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_session_owner(pool: &AnyPool, session_id: &str) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM sessions WHERE id = $1 AND user_id IS NOT NULL",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Workspace context for a session, used by the detail/log endpoints to
/// 404 callers that aren't members of the owning workspace, and by the
/// session-launch path to fetch the right credential set.
pub async fn get_session_workspace(
    pool: &AnyPool,
    session_id: &str,
) -> anyhow::Result<Option<String>> {
    let row =
        sqlx::query_scalar::<_, Option<String>>("SELECT workspace_id FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.flatten())
}

// ── Public log types ──

#[allow(dead_code)]
pub struct LogChunk {
    pub chunk: String,
    pub stream: String,
    pub created_at: String,
}

#[allow(dead_code)]
pub struct LogChunkWithSeq {
    pub seq: i64,
    pub chunk: String,
    pub stream: String,
    pub created_at: String,
}

// ── Row types ──

#[derive(sqlx::FromRow)]
struct LogChunkRow {
    chunk: String,
    stream: String,
    created_at: String,
}

impl From<LogChunkRow> for LogChunk {
    fn from(row: LogChunkRow) -> Self {
        LogChunk {
            chunk: row.chunk,
            stream: row.stream,
            created_at: row.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct LogChunkWithSeqRow {
    seq: i32,
    chunk: String,
    stream: String,
    created_at: String,
}

impl From<LogChunkWithSeqRow> for LogChunkWithSeq {
    fn from(row: LogChunkWithSeqRow) -> Self {
        LogChunkWithSeq {
            seq: row.seq as i64,
            chunk: row.chunk,
            stream: row.stream,
            created_at: row.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    task_id: String,
    node_id: String,
    state: String,
    prompt: String,
    output: Option<String>,
    working_dir: Option<String>,
    artifact_id: Option<String>,
    artifact_version: Option<i32>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<SessionRow> for Session {
    type Error = anyhow::Error;

    fn try_from(row: SessionRow) -> anyhow::Result<Self> {
        Ok(Session {
            id: row.id,
            task_id: row.task_id,
            node_id: row.node_id,
            state: row
                .state
                .parse()
                .map_err(|e: crate::core::StiglabError| anyhow::anyhow!(e))?,
            prompt: row.prompt,
            output: row.output,
            working_dir: row.working_dir,
            artifact_id: row.artifact_id,
            artifact_version: row.artifact_version,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)?.with_timezone(&Utc),
        })
    }
}
