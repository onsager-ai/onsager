use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::Message;
use sqlx::AnyPool;
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::server::config::ServerConfig;
use crate::server::proxy_cache::ProxyCache;
use crate::server::spine::SpineEmitter;

pub type WsSender = mpsc::UnboundedSender<Message>;

/// Capacity of the per-process session-completion broadcast channel.
///
/// `tokio::sync::broadcast` overwrites the oldest message on overflow; the
/// `wait` endpoint treats `Lagged` as a no-op (it just falls through to the
/// next iteration / DB read). Sized for many concurrent waiters per terminal
/// transition without dropping useful events under normal load.
const SESSION_COMPLETION_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug)]
pub struct AgentConnection {
    #[allow(dead_code)]
    pub node_id: String,
    pub sender: WsSender,
}

#[derive(Clone)]
pub struct AppState {
    pub db: AnyPool,
    pub agents: Arc<RwLock<HashMap<String, AgentConnection>>>,
    pub config: ServerConfig,
    pub spine: Option<SpineEmitter>,
    /// Broadcasts `session_id` whenever a session reaches a terminal state
    /// (Done or Failed). Powers `GET /api/shaping/{id}?wait=Ns` so the
    /// status endpoint doesn't need to poll the database (issue #31).
    pub session_completion_tx: broadcast::Sender<String>,
    /// Short-TTL cache for `/api/projects/:id/{issues,pulls}` live-hydration
    /// reads (#170). Reference-only artifact rows don't carry GitHub-authored
    /// fields; the proxy fetches them on demand and this cache deduplicates
    /// in-flight reads inside the TTL window (default 60s).
    pub proxy_cache: Arc<ProxyCache>,
}

impl AppState {
    pub fn new(db: AnyPool, config: ServerConfig, spine: Option<SpineEmitter>) -> Self {
        let (session_completion_tx, _) = broadcast::channel(SESSION_COMPLETION_CHANNEL_CAPACITY);
        AppState {
            db,
            agents: Arc::new(RwLock::new(HashMap::new())),
            config,
            spine,
            session_completion_tx,
            proxy_cache: Arc::new(ProxyCache::from_env()),
        }
    }
}
