//! Shared application state handed to every handler.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use tokio::task::AbortHandle;

use crate::config::Config;

/// What the abort endpoint needs to stop a run: the task's abort
/// handle plus the live session's process-group id. `kill_on_drop`
/// only reaches the direct child; real agent sessions spawn process
/// trees, so abort also SIGKILLs the negative pgid (M3 #625).
#[derive(Clone)]
pub struct RunHandle {
    pub abort: AbortHandle,
    pub pgid: Arc<Mutex<Option<i32>>>,
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Config,
    /// In-flight runs (M3 #625). The scheduler registers on fire and
    /// deregisters on finish; the abort endpoint aborts the task and
    /// kills the session's process group.
    pub running: Arc<Mutex<HashMap<String, RunHandle>>>,
    /// Live session feeds (#634): one broadcast channel per running
    /// session, carrying durable `agent.*` events and transient
    /// `agent.text_delta` drafts to SSE subscribers. In-process only —
    /// one process is the whole factory (ADR 0029), so no pg_notify.
    pub feeds: Arc<Mutex<HashMap<String, tokio::sync::broadcast::Sender<serde_json::Value>>>>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> Self {
        Self {
            pool,
            config,
            running: Arc::new(Mutex::new(HashMap::new())),
            feeds: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
