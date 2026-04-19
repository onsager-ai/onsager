//! Shared HTTP state.

use std::sync::Arc;

use onsager_spine::EventStore;
use sqlx::postgres::PgPool;

use crate::config::Config;
use crate::gate::GateClient;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub spine: EventStore,
    pub config: Arc<Config>,
    pub gate: Arc<GateClient>,
}
