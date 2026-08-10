use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;

use crate::instance::manager::InstanceManager;
use crate::ratelimit::RateLimiter;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub session_ttl_days: i64,
    pub pow_difficulty: u32,
    pub public_host: String,
    pub public_port: u16,
    #[allow(dead_code)]
    pub game_bin_path: PathBuf,
    pub games: Arc<crate::games::registry::GameRegistry>,
    pub instances: Arc<InstanceManager>,
    pub rl_register: RateLimiter,
    pub rl_login: RateLimiter,
    pub rl_captcha: RateLimiter,
}

pub type SharedState = Arc<AppState>;