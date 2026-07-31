use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,
    pub session_ttl_days: i64,
    pub log_format: LogFormat,
    pub log_file_dir: Option<String>,
    pub log_keep_days: u64,
    pub pow_difficulty: u32,
    pub public_host: String,
    pub game_bin_path: std::path::PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let bind_addr = env::var("LOBBY_BIND").unwrap_or_else(|_| "0.0.0.0:8192".to_string());
        let database_url =
            env::var("LOBBY_DATABASE_URL").unwrap_or_else(|_| "sqlite://data/lobby.db?mode=rwc".to_string());
        let session_ttl_days = env::var("LOBBY_SESSION_TTL_DAYS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(7);
        let log_format = match env::var("LOBBY_LOG_FORMAT").as_deref() {
            Ok("json") => LogFormat::Json,
            _ => LogFormat::Text,
        };
        let log_file_dir = env::var("LOBBY_LOG_FILE_DIR").ok().filter(|s| !s.is_empty());
        let log_keep_days = env::var("LOBBY_LOG_KEEP_DAYS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(7);
        let pow_difficulty = env::var("LOBBY_POW_DIFFICULTY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(crate::auth::pow::DEFAULT_DIFFICULTY);
        let public_host = env::var("LOBBY_PUBLIC_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let game_bin_path = env::var("LOBBY_GAME_BIN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("game"));
        Self {
            bind_addr,
            database_url,
            session_ttl_days,
            log_format,
            log_file_dir,
            log_keep_days,
            pow_difficulty,
            public_host,
            game_bin_path,
        }
    }
}