//! Session and node read/write for the portal-owned session surface
//! (spec #222 Follow-up 3).
//!
//! Uses `PgPool` (same as every other portal DB module). Timestamps are
//! stored as RFC3339 TEXT in the shared Postgres schema (created by
//! stiglab's migration path) and parsed on read.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPool;

use crate::core::{LogChunk, LogChunkWithSeq, Node, NodeStatus, Session, SessionState};

// ── Row types ────────────────────────────────────────────────────────────────

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
    fn try_from(r: SessionRow) -> anyhow::Result<Self> {
        Ok(Session {
            id: r.id,
            task_id: r.task_id,
            node_id: r.node_id,
            state: r.state.parse()?,
            prompt: r.prompt,
            output: r.output,
            working_dir: r.working_dir,
            artifact_id: r.artifact_id,
            artifact_version: r.artifact_version,
            created_at: DateTime::parse_from_rfc3339(&r.created_at)?.with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&r.updated_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct NodeRow {
    id: String,
    name: String,
    hostname: String,
    status: String,
    max_sessions: i32,
    active_sessions: i32,
    last_heartbeat: String,
    registered_at: String,
}

impl TryFrom<NodeRow> for Node {
    type Error = anyhow::Error;
    fn try_from(r: NodeRow) -> anyhow::Result<Self> {
        Ok(Node {
            id: r.id,
            name: r.name,
            hostname: r.hostname,
            status: r.status.parse::<NodeStatus>()?,
            max_sessions: r.max_sessions as u32,
            active_sessions: r.active_sessions as u32,
            last_heartbeat: DateTime::parse_from_rfc3339(&r.last_heartbeat)?.with_timezone(&Utc),
            registered_at: DateTime::parse_from_rfc3339(&r.registered_at)?.with_timezone(&Utc),
        })
    }
}

impl std::str::FromStr for NodeStatus {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "online" => Ok(Self::Online),
            "offline" => Ok(Self::Offline),
            "draining" => Ok(Self::Draining),
            _ => Err(anyhow::anyhow!("invalid node status: {s}")),
        }
    }
}

#[derive(sqlx::FromRow)]
struct LogChunkRow {
    chunk: String,
    stream: String,
    created_at: String,
}

#[derive(sqlx::FromRow)]
struct LogChunkWithSeqRow {
    seq: i64,
    chunk: String,
    stream: String,
}

// ── Session reads ─────────────────────────────────────────────────────────────

pub async fn list_sessions_for_user_in_workspace(
    pool: &PgPool,
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

pub async fn get_session(pool: &PgPool, session_id: &str) -> anyhow::Result<Option<Session>> {
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

pub async fn get_session_owner(
    pool: &PgPool,
    session_id: &str,
) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM sessions WHERE id = $1 AND user_id IS NOT NULL",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_session_workspace(
    pool: &PgPool,
    session_id: &str,
) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, Option<String>>(
        "SELECT workspace_id FROM sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.flatten())
}

pub async fn get_session_logs(
    pool: &PgPool,
    session_id: &str,
) -> anyhow::Result<Vec<LogChunk>> {
    let rows = sqlx::query_as::<_, LogChunkRow>(
        "SELECT chunk, stream, created_at FROM session_logs \
         WHERE session_id = $1 ORDER BY seq ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LogChunk {
            chunk: r.chunk,
            stream: r.stream,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn get_session_logs_after(
    pool: &PgPool,
    session_id: &str,
    after_seq: i64,
) -> anyhow::Result<Vec<LogChunkWithSeq>> {
    let rows = sqlx::query_as::<_, LogChunkWithSeqRow>(
        "SELECT seq, chunk, stream FROM session_logs \
         WHERE session_id = $1 AND seq > $2 ORDER BY seq ASC",
    )
    .bind(session_id)
    .bind(after_seq)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LogChunkWithSeq {
            seq: r.seq,
            chunk: r.chunk,
            stream: r.stream,
        })
        .collect())
}

// ── Session writes ────────────────────────────────────────────────────────────

pub async fn insert_session_with_user_project_workspace(
    pool: &PgPool,
    session: &Session,
    user_id: Option<&str>,
    project_id: Option<&str>,
    workspace_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               user_id, project_id, workspace_id, artifact_id, artifact_version, \
                               created_at, updated_at) \
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

pub async fn update_session_state(
    pool: &PgPool,
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

// ── Node reads ────────────────────────────────────────────────────────────────

pub async fn list_nodes(pool: &PgPool) -> anyhow::Result<Vec<Node>> {
    let rows = sqlx::query_as::<_, NodeRow>(
        "SELECT id, name, hostname, status, max_sessions, active_sessions, \
                last_heartbeat, registered_at FROM nodes",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_node(pool: &PgPool, node_id: &str) -> anyhow::Result<Option<Node>> {
    let row = sqlx::query_as::<_, NodeRow>(
        "SELECT id, name, hostname, status, max_sessions, active_sessions, \
                last_heartbeat, registered_at FROM nodes WHERE id = $1",
    )
    .bind(node_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn find_least_loaded_node(pool: &PgPool) -> anyhow::Result<Option<Node>> {
    let cutoff = (Utc::now() - chrono::Duration::seconds(120)).to_rfc3339();
    let row = sqlx::query_as::<_, NodeRow>(
        "SELECT id, name, hostname, status, max_sessions, active_sessions, \
                last_heartbeat, registered_at \
         FROM nodes \
         WHERE status = 'online' AND active_sessions < max_sessions AND last_heartbeat > $1 \
         ORDER BY CAST(active_sessions AS REAL) / CAST(max_sessions AS REAL) ASC \
         LIMIT 1",
    )
    .bind(&cutoff)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}
