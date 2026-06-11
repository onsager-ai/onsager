//! Shared HTTP state.

use std::sync::Arc;

use onsager_spine::EventStore;
use sqlx::postgres::PgPool;

use crate::config::Config;
use crate::proxy_cache::ProxyCache;
use crate::push::PushState;
use crate::session_runner::SessionRunner;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub spine: EventStore,
    pub config: Arc<Config>,
    /// In-process agent session executor (spec #583, ADR 0027).
    /// `POST /api/tasks` hands sessions here directly; cancel goes
    /// through it too — no spine dispatch, no agent-node fleet.
    pub session_runner: Arc<SessionRunner>,
    /// Short-TTL cache for `/api/projects/:id/{issues,pulls}` live-hydration
    /// reads (#170 / #222 follow-up 2). Deduplicates GitHub round-trips within
    /// the TTL window (default 60s, set via `PORTAL_PROXY_CACHE_TTL_SECS`).
    pub proxy_cache: Arc<ProxyCache>,
    /// VAPID config + handles for Web Push dispatch (ADR 0020 / spec
    /// #472). `vapid` is `None` when the env vars are unset; the push
    /// endpoints then surface "push disabled" and `dispatch_to_user`
    /// short-circuits.
    pub push: PushState,
}
