use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::Message;
use sqlx::AnyPool;
use tokio::sync::{mpsc, RwLock};

use crate::server::config::ServerConfig;
use crate::server::proxy_cache::ProxyCache;
use crate::server::spine::SpineEmitter;

pub type WsSender = mpsc::UnboundedSender<Message>;

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
    /// Short-TTL cache for `/api/projects/:id/{issues,pulls}` live-hydration
    /// reads (#170). Reference-only artifact rows don't carry GitHub-authored
    /// fields; the proxy fetches them on demand and this cache deduplicates
    /// in-flight reads inside the TTL window (default 60s).
    pub proxy_cache: Arc<ProxyCache>,
}

impl AppState {
    pub fn new(db: AnyPool, config: ServerConfig, spine: Option<SpineEmitter>) -> Self {
        AppState {
            db,
            agents: Arc::new(RwLock::new(HashMap::new())),
            config,
            spine,
            proxy_cache: Arc::new(ProxyCache::from_env()),
        }
    }
}
