use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Starting,
    Ready,
    Running,
    Finished,
    Abnormal,
    Stopped,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Finished => "finished",
            Self::Abnormal => "abnormal",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event")]
pub enum GameEvent {
    #[serde(rename = "ready")]
    Ready { port: Option<u16> },
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "finished")]
    Finished,
    #[serde(rename = "shutdown")]
    Shutdown,
    #[serde(rename = "heartbeat")]
    Heartbeat,
}

#[derive(Debug, Serialize)]
#[serde(tag = "cmd")]
pub enum GameCommand {
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "stop")]
    Stop { reason: String },
}

struct ActiveInstance {
    port: u16,
    #[allow(dead_code)]
    status: Status,
    last_heartbeat: Instant,
    stdin_tx: mpsc::Sender<String>,
}

pub struct InstanceManager {
    db: SqlitePool,
    bin_path: PathBuf,
    instances: Arc<Mutex<HashMap<i64, ActiveInstance>>>,
}

impl InstanceManager {
    pub fn new(db: SqlitePool, bin_path: PathBuf) -> Self {
        Self {
            db,
            bin_path,
            instances: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn spawn(
        self: &Arc<Self>,
        room_id: i64,
        game_type: &str,
        players: Vec<(i64, String)>,
    ) -> Result<i64> {
        let port = allocate_port().await?;

        let init_payload = serde_json::json!({
            "room_id": room_id,
            "game_type": game_type,
            "listen": format!("127.0.0.1:{}", port),
            "players": players.iter().map(|(uid, session)| {
                serde_json::json!({"uid": uid, "session": session})
            }).collect::<Vec<_>>(),
        });
        let init_line = format!("{}\n", init_payload);

        let mut child = Command::new(&self.bin_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawn {}", self.bin_path.display()))?;

        let pid = child.id();
        let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;

        stdin.write_all(init_line.as_bytes()).await?;
        stdin.flush().await?;

        let row: (i64,) = sqlx::query_as(
            "INSERT INTO game_instances (room_id, pid, port, status, start_time) VALUES (?, ?, ?, 'starting', datetime('now')) RETURNING instance_id"
        )
        .bind(room_id)
        .bind(pid.map(|p| p as i64))
        .bind(port as i64)
        .fetch_one(&self.db)
        .await
        .map_err(|e| anyhow!("db insert instance: {e}"))?;
        let instance_id = row.0;

        // stdin writer task
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(16);
        tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                if stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        // stderr → log
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                tracing::debug!(target: "lobby::game_stderr", instance_id, "{}", line);
            }
        });

        // stdout event reader
        let db = self.db.clone();
        let instances = self.instances.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let evt: serde_json::Result<GameEvent> = serde_json::from_str(&line);
                let mut g = instances.lock().await;
                match evt {
                    Ok(GameEvent::Ready { .. }) => {
                        info!(instance_id, "game ready");
                        if let Some(h) = g.get_mut(&instance_id) {
                            h.status = Status::Ready;
                        }
                        let _ = sqlx::query("UPDATE game_instances SET status='ready' WHERE instance_id=?")
                            .bind(instance_id).execute(&db).await;
                    }
                    Ok(GameEvent::Running) => {
                        info!(instance_id, "game running");
                        if let Some(h) = g.get_mut(&instance_id) {
                            h.status = Status::Running;
                        }
                        let _ = sqlx::query("UPDATE game_instances SET status='running' WHERE instance_id=?")
                            .bind(instance_id).execute(&db).await;
                    }
                    Ok(GameEvent::Finished) => {
                        info!(instance_id, "game finished");
                        if let Some(h) = g.get_mut(&instance_id) {
                            h.status = Status::Finished;
                        }
                        let _ = sqlx::query("UPDATE game_instances SET status='finished' WHERE instance_id=?")
                            .bind(instance_id).execute(&db).await;
                    }
                    Ok(GameEvent::Shutdown) => {
                        info!(instance_id, "game shutdown");
                        if let Some(h) = g.get_mut(&instance_id) {
                            h.status = Status::Stopped;
                        }
                    }
                    Ok(GameEvent::Heartbeat) => {
                        if let Some(h) = g.get_mut(&instance_id) {
                            h.last_heartbeat = Instant::now();
                        }
                    }
                    Err(e) => {
                        warn!(instance_id, error=%e, line=%line, "bad event line");
                    }
                }
                drop(g);
            }
            // stdout closed: abnormal if still active
            let mut g = instances.lock().await;
            if let Some(h) = g.get_mut(&instance_id) {
                if !matches!(h.status, Status::Stopped | Status::Finished | Status::Abnormal) {
                    h.status = Status::Abnormal;
                    let _ = sqlx::query("UPDATE game_instances SET status='abnormal', end_time=datetime('now') WHERE instance_id=?")
                        .bind(instance_id).execute(&db).await;
                }
            }
        });

        self.instances.lock().await.insert(instance_id, ActiveInstance {
            port,
            status: Status::Starting,
            last_heartbeat: Instant::now(),
            stdin_tx,
        });

        Ok(instance_id)
    }

    pub async fn lookup(&self, instance_id: i64) -> Option<(u16, Status)> {
        let g = self.instances.lock().await;
        g.get(&instance_id).map(|h| (h.port, h.status))
    }

    pub async fn stop(&self, instance_id: i64, reason: &str) {
        let mut g = self.instances.lock().await;
        if let Some(h) = g.get_mut(&instance_id) {
            let line = serde_json::to_string(&GameCommand::Stop { reason: reason.into() }).unwrap();
            let _ = h.stdin_tx.send(line).await;
            h.status = Status::Stopped;
        }
    }

    pub async fn send_start(&self, instance_id: i64) {
        let mut g = self.instances.lock().await;
        if let Some(h) = g.get_mut(&instance_id) {
            let line = serde_json::to_string(&GameCommand::Start).unwrap();
            let _ = h.stdin_tx.send(line).await;
        }
    }

    /// Run watchdog: mark abnormal any instance without heartbeat for >15s.
    pub async fn check_timeouts(&self) -> Vec<i64> {
        let mut timed_out = Vec::new();
        let mut g = self.instances.lock().await;
        for (id, h) in g.iter_mut() {
            if matches!(h.status, Status::Stopped | Status::Finished | Status::Abnormal) {
                continue;
            }
            if h.last_heartbeat.elapsed() > Duration::from_secs(15) {
                warn!(instance_id = id, "heartbeat timeout");
                h.status = Status::Abnormal;
                let _ = sqlx::query("UPDATE game_instances SET status='abnormal', end_time=datetime('now') WHERE instance_id=?")
                    .bind(id).execute(&self.db).await;
                timed_out.push(*id);
            }
        }
        timed_out
    }
}

async fn allocate_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}