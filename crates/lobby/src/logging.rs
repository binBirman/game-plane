use std::path::Path;

use anyhow::{Context, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::daily;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::LogFormat;

pub struct LoggingGuard {
    _file_guard: Option<WorkerGuard>,
}

pub fn init(cfg: &crate::config::Config) -> Result<LoggingGuard> {
    // Default filter enables structured info-level logs from game subprocesses
    // (via `lobby::game_log` span) and the `lobby::game_stderr` plain-text
    // fallback at DEBUG. Override with `RUST_LOG` in `lobby.env` to tune.
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,lobby::db=warn,lobby::game_stderr=debug"));

    if let Some(dir) = &cfg.log_file_dir {
        std::fs::create_dir_all(dir).with_context(|| format!("create log dir {dir}"))?;
        cleanup_old_files(Path::new(dir), cfg.log_keep_days);

        let appender = daily(dir, "lobby.log");
        let (file_writer, file_guard) = tracing_appender::non_blocking(appender);

        match cfg.log_format {
            LogFormat::Json => tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(false)
                        .with_target(true)
                        .with_writer(std::io::stdout),
                )
                .with(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(false)
                        .with_target(true)
                        .with_writer(file_writer),
                )
                .init(),
            LogFormat::Text => tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_target(true)
                        .with_line_number(true)
                        .with_writer(std::io::stdout),
                )
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_target(true)
                        .with_line_number(true)
                        .with_writer(file_writer),
                )
                .init(),
        }

        tracing::info!(
            log_format = ?cfg.log_format,
            log_file_dir = %dir,
            log_keep_days = cfg.log_keep_days,
            "logging initialized (stdout + rolling file)"
        );
        Ok(LoggingGuard { _file_guard: Some(file_guard) })
    } else {
        match cfg.log_format {
            LogFormat::Json => tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(false)
                        .with_target(true)
                        .with_writer(std::io::stdout),
                )
                .init(),
            LogFormat::Text => tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_target(true)
                        .with_line_number(true)
                        .with_writer(std::io::stdout),
                )
                .init(),
        }

        tracing::info!(
            log_format = ?cfg.log_format,
            log_file_dir = "<none>",
            "logging initialized (stdout only)"
        );
        Ok(LoggingGuard { _file_guard: None })
    }
}

fn cleanup_old_files(dir: &Path, keep_days: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let Some(cutoff) = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(keep_days.saturating_mul(86_400)))
    else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.starts_with("lobby.") || !name.ends_with(".log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if modified < cutoff && std::fs::remove_file(&path).is_ok() {
            tracing::debug!(file = %path.display(), "removed old log file");
        }
    }
}