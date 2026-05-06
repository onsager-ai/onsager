//! Shared HTTP state.

use std::sync::Arc;

use onsager_spine::EventStore;
use sqlx::postgres::PgPool;

use crate::config::Config;
use crate::gate::GateClient;
use crate::proxy_cache::ProxyCache;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub spine: EventStore,
    pub config: Arc<Config>,
    pub gate: Arc<GateClient>,
    /// Short-TTL cache for `/api/projects/:id/{issues,pulls}` live-hydration
    /// reads (#170 / #222 follow-up 2). Deduplicates GitHub round-trips within
    /// the TTL window (default 60s, set via `PORTAL_PROXY_CACHE_TTL_SECS`).
    pub proxy_cache: Arc<ProxyCache>,
}
