mod auth;
mod config;
mod db;
mod http;
mod instance;
mod logging;
mod state;
mod ws_proxy;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::http::router;
use crate::instance::manager::InstanceManager;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::from_env();
    let _log_guard = logging::init(&cfg)?;

    tracing::info!(
        bind = %cfg.bind_addr,
        db = %cfg.database_url,
        public_host = %cfg.public_host,
        game_bin = %cfg.game_bin_path.display(),
        pid = std::process::id(),
        "lobby starting"
    );

    if let Some(path) = cfg.database_url.strip_prefix("sqlite://") {
        if let Some((dir, _)) = path.split_once('/') {
            if !dir.is_empty() && !std::path::Path::new(dir).exists() {
                std::fs::create_dir_all(dir)?;
            }
        }
    }

    let db = db::init_pool(&cfg.database_url).await?;
    let instances = Arc::new(InstanceManager::new(db.clone(), cfg.game_bin_path.clone()));

    let state = Arc::new(AppState {
        db,
        session_ttl_days: cfg.session_ttl_days,
        pow_difficulty: cfg.pow_difficulty,
        public_host: cfg.public_host.clone(),
        instances: instances.clone(),
    });

    // Watchdog: scan for heartbeats every 5s
    {
        let instances = instances.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            loop {
                tick.tick().await;
                let _ = instances.check_timeouts().await;
            }
        });
    }

    let app = router::build(state)
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(http::request_id::middleware));

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    tracing::info!(addr = %cfg.bind_addr, "lobby listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("lobby stopped cleanly");
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("ctrl-c received, shutting down"),
        _ = terminate => tracing::info!("SIGTERM received, shutting down"),
    }
}