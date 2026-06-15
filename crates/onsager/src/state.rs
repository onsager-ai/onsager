//! Shared application state handed to every handler.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
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
    /// Concurrency cap on in-flight agent sessions (ADR 0030 MVP stance):
    /// a fire registers and starts immediately, but its task waits here
    /// for a slot before launching the `claude` process, so the number of
    /// live sessions never exceeds `config.max_concurrent_runs`. This is
    /// the one cheap knob that bounds the single-process ceiling; the
    /// fleet (per-machine slots, Σ capacity) stays deferred.
    pub run_slots: Arc<Semaphore>,
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
        }
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
