use std::sync::Arc;

use sqlx::SqlitePool;

use crate::instance::manager::InstanceManager;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub session_ttl_days: i64,
    pub pow_difficulty: u32,
    pub public_host: String,
    pub instances: Arc<InstanceManager>,
}

pub type SharedState = Arc<AppState>;