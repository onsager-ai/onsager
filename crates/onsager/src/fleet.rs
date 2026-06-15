//! The fleet (ADR 0030): agent sessions run on **machines**, not only
//! in the control-plane process. A machine dials *out* over a WebSocket,
//! authenticates with a shared token, registers a name, and subscribes
//! to work; the control plane dispatches a session and replays the
//! machine's event stream into Postgres + the SSE feed.
//!
//! This is the M2 happy path: dispatch + event-flow inversion + abort.
//! Deferred (ADR 0030): durable at-most-once leases / reclaim (M3) and
//! the dashboard fleet view (M4). When no machine is connected, dispatch
//! falls back to running the session in-process — the degenerate "local
//! machine = default machine" case, byte-compatible with ADR 0029.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::sessions::{
    ClonableRepo, SessionError, SessionOutcome, SessionRequest, SessionSink, run_agent_session,
};
use crate::state::AppState;

/// The dispatchable unit of work (ADR 0030 D1): everything a machine
/// needs to run one agent session. Persistence and liveness stay
/// control-plane-side, so this carries no pool or feed.
pub struct SessionJob {
    pub session_id: String,
    pub token: String,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub user_prompt: String,
    pub mcp_url: Option<String>,
    pub env: Vec<(String, String)>,
    pub repos: Vec<ClonableRepo>,
}

/// A repo on the wire (ADR 0030). Mirrors [`ClonableRepo`]; the minted
/// token travels to the machine that will clone with it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoWire {
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub token: Option<String>,
}

impl From<ClonableRepo> for RepoWire {
    fn from(r: ClonableRepo) -> Self {
        Self {
            owner: r.owner,
            name: r.name,
            default_branch: r.default_branch,
            token: r.token,
        }
    }
}

impl From<RepoWire> for ClonableRepo {
    fn from(r: RepoWire) -> Self {
        Self {
            owner: r.owner,
            name: r.name,
            default_branch: r.default_branch,
            token: r.token,
        }
    }
}

/// Control plane → machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ControlFrame {
    /// Run this session.
    Assign {
        session_id: String,
        token: String,
        model: Option<String>,
        system_prompt: Option<String>,
        user_prompt: String,
        mcp_url: Option<String>,
        env: Vec<(String, String)>,
        repos: Vec<RepoWire>,
    },
    /// Kill a running session's process tree.
    Abort { session_id: String },
}

/// Machine → control plane.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WorkerFrame {
    /// First frame after connect: claim a name.
    Register { name: String },
    /// A durable, normalized session event (to be persisted + fanned out).
    Event {
        session_id: String,
        seq: i32,
        kind: String,
        payload: serde_json::Value,
    },
    /// A transient token delta (live feed only).
    Delta { session_id: String, text: String },
    /// The session finished; carries the outcome.
    Ended {
        session_id: String,
        ok: bool,
        output: Option<String>,
        error: Option<String>,
    },
    /// Liveness keepalive.
    Heartbeat,
}

// ---------------------------------------------------------------------------
// Control-plane side: the WebSocket endpoint + dispatch.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ConnectParams {
    pub token: String,
    pub name: String,
}

/// `GET /api/fleet/connect` — a machine dials in. Token-gated (fail
/// closed: no `ONSAGER_MACHINE_TOKEN` configured → reject every machine,
/// like the GitHub webhook).
pub async fn connect(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(params): Query<ConnectParams>,
) -> Response {
    match state.config.machine_token.as_deref() {
        Some(expected) if !expected.is_empty() && expected == params.token => {}
        _ => {
            tracing::warn!(machine = %params.name, "fleet connect rejected: bad token");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    let name = params.name.clone();
    ws.on_upgrade(move |socket| handle_machine(socket, state, name))
}

/// Own one machine connection for its lifetime: register it, pump
/// outgoing control frames, and replay incoming worker frames into
/// Postgres + the SSE feed. On disconnect, fail the machine's in-flight
/// sessions (at-most-once — ADR 0030 D4; durable reclaim is M3).
async fn handle_machine(socket: WebSocket, state: AppState, name: String) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ControlFrame>();

    state
        .machines
        .lock()
        .expect("machines lock")
        .insert(name.clone(), out_tx);
    tracing::info!(machine = %name, "machine connected");

    // Outgoing pump: serialize control frames to the socket.
    let writer = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            let Ok(text) = serde_json::to_string(&frame) else {
                continue;
            };
            if ws_tx.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Incoming: replay worker frames.
    while let Some(Ok(msg)) = ws_rx.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(frame) = serde_json::from_str::<WorkerFrame>(&text) else {
            tracing::warn!(machine = %name, "undecodable worker frame");
            continue;
        };
        match frame {
            WorkerFrame::Register { .. } | WorkerFrame::Heartbeat => {}
            WorkerFrame::Delta { session_id, text } => {
                if let Some(feed) = state.feeds.lock().expect("feeds lock").get(&session_id) {
                    let _ = feed.send(serde_json::json!({
                        "kind": "agent.text_delta",
                        "payload": { "text": text },
                    }));
                }
            }
            WorkerFrame::Event {
                session_id,
                seq,
                kind,
                payload,
            } => {
                let feed = state
                    .feeds
                    .lock()
                    .expect("feeds lock")
                    .get(&session_id)
                    .cloned();
                if let Some(feed) = feed {
                    crate::sessions::persist_session_event(
                        &state.pool,
                        &feed,
                        &session_id,
                        seq,
                        &kind,
                        &payload,
                    )
                    .await;
                }
            }
            WorkerFrame::Ended {
                session_id,
                ok,
                output,
                error,
            } => {
                state
                    .remote_sessions
                    .lock()
                    .expect("remote_sessions lock")
                    .remove(&session_id);
                if let Some(done) = state
                    .pending
                    .lock()
                    .expect("pending lock")
                    .remove(&session_id)
                {
                    let outcome = if ok {
                        Ok(output.unwrap_or_default())
                    } else {
                        Err(error.unwrap_or_else(|| "machine reported failure".into()))
                    };
                    let _ = done.send(outcome);
                }
            }
        }
    }

    // Teardown: deregister and fail anything still in flight on this
    // machine so its run resolves instead of hanging forever.
    writer.abort();
    state.machines.lock().expect("machines lock").remove(&name);
    let orphaned: Vec<String> = {
        let mut rs = state.remote_sessions.lock().expect("remote_sessions lock");
        let ids: Vec<String> = rs
            .iter()
            .filter(|(_, m)| **m == name)
            .map(|(s, _)| s.clone())
            .collect();
        for id in &ids {
            rs.remove(id);
        }
        ids
    };
    for id in orphaned {
        if let Some(done) = state.pending.lock().expect("pending lock").remove(&id) {
            let _ = done.send(Err(format!("machine {name} disconnected mid-session")));
        }
    }
    tracing::info!(machine = %name, "machine disconnected");
}

/// Run one session: on a connected machine if any, else in-process.
/// The return contract matches [`run_agent_session`], so the scheduler
/// is unchanged below the seam (ADR 0030).
pub async fn dispatch(
    state: &AppState,
    job: SessionJob,
    feed: tokio::sync::broadcast::Sender<serde_json::Value>,
    pgid_slot: Arc<std::sync::Mutex<Option<i32>>>,
) -> Result<SessionOutcome, SessionError> {
    let machine = pick_machine(state);
    let Some((name, tx)) = machine else {
        // Degenerate case: no machine connected → run locally, exactly
        // as the single-process system did (ADR 0029).
        return run_agent_session(SessionRequest {
            session_id: job.session_id,
            token: job.token,
            model: job.model,
            system_prompt: job.system_prompt,
            user_prompt: job.user_prompt,
            mcp_url: job.mcp_url,
            env: job.env,
            repos: job.repos,
            pgid_slot,
            sink: SessionSink::Local {
                pool: state.pool.clone(),
                feed,
            },
        })
        .await;
    };

    // Remote: register the completion channel before sending, so a fast
    // machine's Ended can never race ahead of the pending entry.
    let (done_tx, done_rx) = oneshot::channel::<Result<String, String>>();
    state
        .pending
        .lock()
        .expect("pending lock")
        .insert(job.session_id.clone(), done_tx);
    state
        .remote_sessions
        .lock()
        .expect("remote_sessions lock")
        .insert(job.session_id.clone(), name.clone());

    let assign = ControlFrame::Assign {
        session_id: job.session_id.clone(),
        token: job.token,
        model: job.model,
        system_prompt: job.system_prompt,
        user_prompt: job.user_prompt,
        mcp_url: job.mcp_url,
        env: job.env,
        repos: job.repos.into_iter().map(RepoWire::from).collect(),
    };
    if tx.send(assign).is_err() {
        state
            .pending
            .lock()
            .expect("pending lock")
            .remove(&job.session_id);
        state
            .remote_sessions
            .lock()
            .expect("remote_sessions lock")
            .remove(&job.session_id);
        return Err(SessionError(format!(
            "machine {name} went away before assign"
        )));
    }
    tracing::info!(machine = %name, session_id = %job.session_id, "session dispatched");

    match done_rx.await {
        Ok(Ok(output)) => Ok(SessionOutcome { output }),
        Ok(Err(e)) => Err(SessionError(e)),
        Err(_) => Err(SessionError("machine connection dropped".into())),
    }
}

/// Pick a machine to run on. Round-robin/least-loaded is M-later; for
/// now, any connected machine (ADR 0030 D5 — placement stays dumb).
fn pick_machine(state: &AppState) -> Option<(String, mpsc::UnboundedSender<ControlFrame>)> {
    let machines = state.machines.lock().expect("machines lock");
    machines
        .iter()
        .next()
        .map(|(name, tx)| (name.clone(), tx.clone()))
}

/// Best-effort abort of a remote session: tell the owning machine to
/// kill the process tree. No-op for local sessions (the abort endpoint
/// handles those via the pgid).
pub fn abort_remote_session(state: &AppState, session_id: &str) {
    let machine = state
        .remote_sessions
        .lock()
        .expect("remote_sessions lock")
        .get(session_id)
        .cloned();
    let Some(machine) = machine else { return };
    if let Some(tx) = state.machines.lock().expect("machines lock").get(&machine) {
        let _ = tx.send(ControlFrame::Abort {
            session_id: session_id.to_string(),
        });
    }
}

// ---------------------------------------------------------------------------
// Worker side: dial out, register, run assigned sessions.
// ---------------------------------------------------------------------------

/// Worker configuration, env-only (a worker has no database).
pub struct WorkerConfig {
    /// Control-plane WebSocket base, e.g. `ws://127.0.0.1:3002`.
    pub control_url: String,
    pub token: String,
    pub name: String,
}

impl WorkerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let control_url = std::env::var("ONSAGER_CONTROL_URL").map_err(|_| {
            anyhow::anyhow!("ONSAGER_CONTROL_URL is required (e.g. ws://host:3002)")
        })?;
        let token = std::env::var("ONSAGER_MACHINE_TOKEN")
            .map_err(|_| anyhow::anyhow!("ONSAGER_MACHINE_TOKEN is required"))?;
        let name = std::env::var("ONSAGER_MACHINE_NAME").unwrap_or_else(|_| "local".to_string());
        Ok(Self {
            control_url,
            token,
            name,
        })
    }
}

/// Run the worker: connect, register, and serve assigned sessions until
/// the connection drops. (Reconnect/backoff is M3 — for now the
/// supervisor that launches the worker restarts it.)
pub async fn run_worker(config: WorkerConfig) -> anyhow::Result<()> {
    use tokio_tungstenite::tungstenite::Message as TMessage;

    let url = format!(
        "{}/api/fleet/connect?token={}&name={}",
        config.control_url.trim_end_matches('/'),
        urlencoding(&config.token),
        urlencoding(&config.name),
    );
    tracing::info!(machine = %config.name, "connecting to control plane");
    let (ws, _) = tokio_tungstenite::connect_async(&url).await?;
    let (mut ws_tx, mut ws_rx) = ws.split();

    // Outgoing frames (events, deltas, ended, register) funnel through
    // one channel so concurrent sessions never interleave a partial send.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WorkerFrame>();
    let writer = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            let Ok(text) = serde_json::to_string(&frame) else {
                continue;
            };
            if ws_tx.send(TMessage::Text(text)).await.is_err() {
                break;
            }
        }
    });

    out_tx.send(WorkerFrame::Register {
        name: config.name.clone(),
    })?;
    tracing::info!(machine = %config.name, "registered; awaiting work");

    // In-flight sessions, for abort: session_id → (task, pgid).
    let inflight: Arc<std::sync::Mutex<std::collections::HashMap<String, InflightSession>>> =
        Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

    while let Some(Ok(msg)) = ws_rx.next().await {
        let text = match msg {
            TMessage::Text(t) => t,
            TMessage::Close(_) => break,
            TMessage::Ping(_) | TMessage::Pong(_) | TMessage::Binary(_) | TMessage::Frame(_) => {
                continue;
            }
        };
        let Ok(frame) = serde_json::from_str::<ControlFrame>(&text) else {
            tracing::warn!("undecodable control frame");
            continue;
        };
        match frame {
            ControlFrame::Assign {
                session_id,
                token,
                model,
                system_prompt,
                user_prompt,
                mcp_url,
                env,
                repos,
            } => {
                let out = out_tx.clone();
                let pgid_slot = Arc::new(std::sync::Mutex::new(None));
                let inflight_map = inflight.clone();
                let sid = session_id.clone();
                let task = tokio::spawn(run_assigned(
                    SessionJob {
                        session_id: session_id.clone(),
                        token,
                        model,
                        system_prompt,
                        user_prompt,
                        mcp_url,
                        env,
                        repos: repos.into_iter().map(ClonableRepo::from).collect(),
                    },
                    out.clone(),
                    pgid_slot.clone(),
                    inflight_map.clone(),
                ));
                inflight.lock().expect("inflight lock").insert(
                    sid,
                    InflightSession {
                        abort: task.abort_handle(),
                        pgid: pgid_slot,
                    },
                );
            }
            ControlFrame::Abort { session_id } => {
                if let Some(s) = inflight.lock().expect("inflight lock").remove(&session_id) {
                    s.abort.abort();
                    let pgid = *s.pgid.lock().expect("pgid lock");
                    if let Some(pgid) = pgid
                        && pgid > 1
                    {
                        // SIGKILL the whole process group (claude spawns trees).
                        unsafe {
                            libc::kill(-pgid, libc::SIGKILL);
                        }
                    }
                }
            }
        }
    }

    writer.abort();
    tracing::info!(machine = %config.name, "disconnected from control plane");
    Ok(())
}

struct InflightSession {
    abort: tokio::task::AbortHandle,
    pgid: Arc<std::sync::Mutex<Option<i32>>>,
}

/// Run one assigned session to completion and report the outcome.
async fn run_assigned(
    job: SessionJob,
    out: mpsc::UnboundedSender<WorkerFrame>,
    pgid_slot: Arc<std::sync::Mutex<Option<i32>>>,
    inflight: Arc<std::sync::Mutex<std::collections::HashMap<String, InflightSession>>>,
) {
    let session_id = job.session_id.clone();
    let outcome = run_agent_session(SessionRequest {
        session_id: job.session_id.clone(),
        token: job.token,
        model: job.model,
        system_prompt: job.system_prompt,
        user_prompt: job.user_prompt,
        mcp_url: job.mcp_url,
        env: job.env,
        repos: job.repos,
        pgid_slot,
        sink: SessionSink::Remote {
            frames: out.clone(),
        },
    })
    .await;
    inflight.lock().expect("inflight lock").remove(&session_id);
    let ended = match outcome {
        Ok(o) => WorkerFrame::Ended {
            session_id,
            ok: true,
            output: Some(o.output),
            error: None,
        },
        Err(e) => WorkerFrame::Ended {
            session_id,
            ok: false,
            output: None,
            error: Some(e.to_string()),
        },
    };
    let _ = out.send(ended);
}

/// Minimal percent-encoding for the connect query params.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_frame_round_trips() {
        let f = ControlFrame::Assign {
            session_id: "sess_1".into(),
            token: "tok".into(),
            model: Some("claude-opus-4-8".into()),
            system_prompt: None,
            user_prompt: "do it".into(),
            mcp_url: Some("http://cp/mcp/messages".into()),
            env: vec![("CLAUDE_CODE_OAUTH_TOKEN".into(), "x".into())],
            repos: vec![RepoWire {
                owner: "acme".into(),
                name: "widgets".into(),
                default_branch: "main".into(),
                token: Some("ght".into()),
            }],
        };
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<ControlFrame>(&json).unwrap(), f);
    }

    #[test]
    fn worker_frame_round_trips() {
        for f in [
            WorkerFrame::Register {
                name: "local".into(),
            },
            WorkerFrame::Event {
                session_id: "s".into(),
                seq: 3,
                kind: "agent.text".into(),
                payload: serde_json::json!({"text": "hi"}),
            },
            WorkerFrame::Delta {
                session_id: "s".into(),
                text: "partial".into(),
            },
            WorkerFrame::Ended {
                session_id: "s".into(),
                ok: true,
                output: Some("done".into()),
                error: None,
            },
            WorkerFrame::Heartbeat,
        ] {
            let json = serde_json::to_string(&f).unwrap();
            assert_eq!(serde_json::from_str::<WorkerFrame>(&json).unwrap(), f);
        }
    }

    #[test]
    fn repo_wire_is_lossless() {
        let r = ClonableRepo {
            owner: "a".into(),
            name: "b".into(),
            default_branch: "main".into(),
            token: Some("t".into()),
        };
        let back: ClonableRepo = RepoWire::from(r.clone()).into();
        assert_eq!(r, back);
    }

    #[tokio::test]
    async fn dispatch_runs_locally_when_no_machine() {
        // No machine connected → the local sink path is taken. We assert
        // selection (not execution) by checking the registry is empty.
        let state = crate::state::AppState::for_test();
        assert!(pick_machine(&state).is_none());
    }

    #[tokio::test]
    async fn machine_connects_registers_and_bad_token_is_rejected() {
        use std::time::Duration;

        // Boot the real router in-process with a machine token set.
        let mut config = crate::config::Config::for_test();
        config.machine_token = Some("secret".into());
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused")
            .expect("lazy pool");
        let state = crate::state::AppState::new(pool, config);
        let app = crate::http::router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Right token + name → upgrades and registers in the fleet.
        let good = format!("ws://{addr}/api/fleet/connect?token=secret&name=test-machine");
        let (_ws, _resp) = tokio_tungstenite::connect_async(&good)
            .await
            .expect("handshake should succeed");
        let mut registered = false;
        for _ in 0..100 {
            if state.machines.lock().unwrap().contains_key("test-machine") {
                registered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(registered, "machine should register on connect");

        // Wrong token → the upgrade is refused (fail closed).
        let bad = format!("ws://{addr}/api/fleet/connect?token=wrong&name=intruder");
        assert!(
            tokio_tungstenite::connect_async(&bad).await.is_err(),
            "bad token must be rejected"
        );
    }
}
