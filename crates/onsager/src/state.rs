//! Shared application state handed to every handler.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use sqlx::PgPool;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::task::AbortHandle;

use crate::config::Config;
use crate::fleet::ControlFrame;

/// What the abort endpoint needs to stop a run: the task's abort
/// handle plus the live session's process-group id. `kill_on_drop`
/// only reaches the direct child; real agent sessions spawn process
/// trees, so abort also SIGKILLs the negative pgid (M3 #625).
#[derive(Clone)]
pub struct RunHandle {
    pub abort: AbortHandle,
    pub pgid: Arc<Mutex<Option<i32>>>,
}

/// A connected machine (ADR 0030): the outbound control-frame channel
/// plus the last time we heard from it (any frame), for lease expiry.
#[derive(Clone)]
pub struct MachineConn {
    pub tx: mpsc::UnboundedSender<ControlFrame>,
    pub last_seen: Arc<Mutex<Instant>>,
}

impl MachineConn {
    pub fn touch(&self) {
        *self.last_seen.lock().expect("last_seen lock") = Instant::now();
    }
}

/// Connected machines: name → connection (ADR 0030).
pub type MachineRegistry = Arc<Mutex<HashMap<String, MachineConn>>>;
/// Dispatched-but-unfinished remote sessions: session_id → completion.
pub type PendingSessions = Arc<Mutex<HashMap<String, oneshot::Sender<Result<String, String>>>>>;

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
    /// Concurrency cap on in-flight agent sessions (ADR 0030 MVP stance):
    /// a fire registers and starts immediately, but its task waits here
    /// for a slot before launching the `claude` process, so the number of
    /// live sessions never exceeds `config.max_concurrent_runs`. This is
    /// the one cheap knob that bounds the single-process ceiling; the
    /// fleet (per-machine slots, Σ capacity) stays deferred.
    pub run_slots: Arc<Semaphore>,
    /// Connected fleet machines (ADR 0030): name → outbound control-frame
    /// channel. A machine registers on WS connect, deregisters on
    /// disconnect. Empty = the degenerate single-process case.
    pub machines: MachineRegistry,
    /// Dispatched-but-unfinished remote sessions: session_id → completion
    /// channel. The WS receive loop resolves these on `Ended`.
    pub pending: PendingSessions,
    /// session_id → owning machine, so abort can route a kill frame and
    /// disconnect can fail the right in-flight sessions (at-most-once).
    pub remote_sessions: Arc<Mutex<HashMap<String, String>>>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> Self {
        let run_slots = Arc::new(Semaphore::new(config.max_concurrent_runs));
        Self {
            pool,
            config,
            running: Arc::new(Mutex::new(HashMap::new())),
            feeds: Arc::new(Mutex::new(HashMap::new())),
            run_slots,
            machines: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            remote_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// An `AppState` with a lazy (never-connected) pool, for unit tests
    /// that exercise only in-memory state (registries, semaphore).
    #[cfg(test)]
    pub fn for_test() -> Self {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused")
            .expect("lazy pool");
        Self::new(pool, Config::for_test())
    }

    /// Wait for a run slot. The returned permit is held for the run's
    /// lifetime and releases the slot on drop. The semaphore is never
    /// closed, so the acquire cannot fail.
    pub async fn acquire_run_slot(&self) -> OwnedSemaphorePermit {
        self.run_slots
            .clone()
            .acquire_owned()
            .await
            .expect("run-slot semaphore is never closed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    fn state_with_cap(cap: usize) -> AppState {
        let mut config = Config::for_test();
        config.max_concurrent_runs = cap;
        // `connect_lazy` never touches the network — fine for a state that
        // only exercises the in-memory semaphore.
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused")
            .expect("lazy pool");
        AppState::new(pool, config)
    }

    #[tokio::test]
    async fn semaphore_sized_from_config() {
        let state = state_with_cap(3);
        assert_eq!(state.run_slots.available_permits(), 3);
    }

    #[tokio::test]
    async fn acquire_run_slot_bounds_concurrency() {
        let state = state_with_cap(1);
        let _held = state.acquire_run_slot().await;
        // The only slot is taken: a second acquire must not be grantable.
        assert_eq!(state.run_slots.available_permits(), 0);
        assert!(state.run_slots.clone().try_acquire_owned().is_err());
        drop(_held);
        assert_eq!(state.run_slots.available_permits(), 1);
    }
}
