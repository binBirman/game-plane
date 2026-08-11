mod auth;
mod config;
mod db;
mod games;
mod http;
mod instance;
mod logging;
mod ratelimit;
mod state;
mod ws_proxy;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::games::GameRegistry;
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
        public_port = cfg.public_port,
        game_bin = %cfg.game_bin_path.display(),
        games_toml = %cfg.games_toml.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "<none>".into()),
        pid = std::process::id(),
        "lobby starting"
    );

    if let Some(path) = cfg.database_url.strip_prefix("sqlite://") {
        if let Some((dir, _)) = path.split_once('/') {
            if !dir.is_empty() && !std::path::Path::new(dir).exists() {
                std::fs::create_dir_all(dir).with_context(|| format!("mkdir {dir}"))?;
            }
        }
    }

    let db = db::init_pool(&cfg.database_url).await?;

    // Load games registry: prefer LOBBY_GAMES_TOML, otherwise fall back to a single
    // auto-registered entry pointing at LOBBY_GAME_BIN (tictactoe default).
    let games: Arc<GameRegistry> = if let Some(path) = &cfg.games_toml {
        match GameRegistry::from_file(path, cfg.game_bin_path.clone()) {
            Ok(reg) => Arc::new(reg),
            Err(e) => {
                tracing::error!(error = %e, "failed to load games.toml; falling back to default");
                Arc::new(
                    GameRegistry::new(cfg.game_bin_path.clone())
                        .with_default("tictactoe", "井字棋"),
                )
            }
        }
    } else {
        Arc::new(
            GameRegistry::new(cfg.game_bin_path.clone())
                .with_default("tictactoe", "井字棋"),
        )
    };
    tracing::info!(
        games = games.list_enabled().len(),
        "game registry loaded"
    );
    for g in games.list_enabled() {
        match g.resolve_binary() {
            crate::games::registry::BinResolve::Ok => {
                tracing::info!(game_type = %g.r#type, bin = %g.binary.display(), "game binary ok");
            }
            crate::games::registry::BinResolve::NotFound(why) => {
                tracing::error!(
                    game_type = %g.r#type,
                    bin = %g.binary.display(),
                    why = %why,
                    "game binary NOT FOUND at startup — POST /api/rooms/:id/start will return 503"
                );
            }
            crate::games::registry::BinResolve::NotExecutable => {
                tracing::error!(
                    game_type = %g.r#type,
                    bin = %g.binary.display(),
                    "game binary NOT EXECUTABLE at startup — chmod +x the binary"
                );
            }
        }
    }

    let instances = Arc::new(InstanceManager::new(db.clone(), cfg.game_bin_path.clone()));

    let state = Arc::new(AppState {
        db: db.clone(),
        session_ttl_days: cfg.session_ttl_days,
        pow_difficulty: cfg.pow_difficulty,
        public_host: cfg.public_host.clone(),
        public_port: cfg.public_port,
        game_bin_path: cfg.game_bin_path.clone(),
        games: games.clone(),
        instances: instances.clone(),
        rl_register: crate::ratelimit::RateLimiter::new(
            std::time::Duration::from_secs(60),
            cfg.rate_limit_register_per_min,
        ),
        rl_login: crate::ratelimit::RateLimiter::new(
            std::time::Duration::from_secs(60),
            cfg.rate_limit_login_per_min,
        ),
        rl_captcha: crate::ratelimit::RateLimiter::new(
            std::time::Duration::from_secs(60),
            cfg.rate_limit_captcha_per_min,
        ),
    });

    // Watchdog: heartbeat timeouts every 5s
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

    // Session GC: purge expired tokens every hour.
    {
        let db = db.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(3600));
            loop {
                tick.tick().await;
                let res = sqlx::query("DELETE FROM sessions WHERE expires_at < datetime('now')")
                    .execute(&db)
                    .await;
                match res {
                    Ok(r) => tracing::info!(deleted = r.rows_affected(), "session gc"),
                    Err(e) => tracing::warn!(error = %e, "session gc failed"),
                }
            }
        });
    }

    // Orphan instance sweep: a previous lobby may have died mid-game, leaving
    // `game_instances.status IN ('starting','ready','running')` rows whose
    // sub-process is long gone and `rooms.status='Running'` without a live
    // instance. Mark them abnormal; if their room is still 'Running', roll
    // it back to 'Waiting' so the host can re-start.
    {
        let db = db.clone();
        match sqlx::query(
            "UPDATE game_instances
             SET status = 'abnormal', end_time = datetime('now')
             WHERE status IN ('starting','ready','running')",
        )
        .execute(&db)
        .await
        {
            Ok(r) => {
                let n = r.rows_affected();
                if n > 0 {
                    tracing::warn!(orphans = n, "orphaned game_instances marked abnormal at startup");
                    // Roll any rooms that pointed to live-running state back
                    // to 'Waiting' so they're playable again. We use the
                    // existence of an instance row that's now abnormal as
                    // the signal; only flip Running rooms (Finished is fine).
                    let r2 = sqlx::query(
                        "UPDATE rooms
                         SET status = 'Waiting'
                         WHERE status = 'Running'
                           AND EXISTS (
                             SELECT 1 FROM game_instances
                             WHERE game_instances.room_id = rooms.room_id
                               AND game_instances.status = 'abnormal'
                               AND game_instances.end_time IS NOT NULL
                           )",
                    )
                    .execute(&db)
                    .await;
                    if let Ok(r2) = r2 {
                        tracing::info!(rooms = r2.rows_affected(), "rooms rolled back to Waiting");
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "orphan sweep failed"),
        }
    }

    let app = router::build(state)
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(http::request_id::middleware));

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    tracing::info!(addr = %cfg.bind_addr, "lobby listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    tracing::info!("lobby: stopping all game instances gracefully");
    instances.shutdown_all().await;
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