use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::Message;
use sqlx::AnyPool;
use tokio::sync::{mpsc, RwLock};

use crate::server::config::ServerConfig;
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
}

impl AppState {
    pub fn new(db: AnyPool, config: ServerConfig, spine: Option<SpineEmitter>) -> Self {
        AppState {
            db,
            agents: Arc::new(RwLock::new(HashMap::new())),
            config,
            spine,
        }
    }
}
