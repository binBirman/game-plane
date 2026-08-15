use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

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
    #[allow(dead_code)]
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
    Finished { #[serde(default)] result: serde_json::Value },
    #[serde(rename = "shutdown")]
    Shutdown,
    #[serde(rename = "heartbeat")]
    Heartbeat,
    /// A player took a game action (predict / play / posterior). Lobby bumps
    /// `game_instances.last_action_at` so the stale-room cleanup can tell a
    /// live game apart from an abandoned one.
    #[serde(rename = "action")]
    Action,
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
    room_id: i64,
    port: u16,
    status: Status,
    last_heartbeat: Instant,
    stdin_tx: mpsc::Sender<String>,
    child: Arc<Mutex<Option<Child>>>,
}

pub struct InstanceManager {
    db: SqlitePool,
    #[allow(dead_code)]
    default_bin: PathBuf,
    instances: Arc<Mutex<HashMap<i64, ActiveInstance>>>,
}

impl InstanceManager {
    pub fn new(db: SqlitePool, default_bin: PathBuf) -> Self {
        Self {
            db,
            default_bin,
            instances: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn spawn(
        self: &Arc<Self>,
        room_id: i64,
        game_type: &str,
        bin: &std::path::Path,
        init_config: Option<serde_json::Value>,
        players: Vec<protocol::PlayerInit>,
    ) -> Result<i64> {
        let port = allocate_port().await?;

        let init_payload = serde_json::json!({
            "room_id": room_id,
            "game_type": game_type,
            "listen": format!("127.0.0.1:{}", port),
            "players": players,
            "config": init_config,
        });
        let init_line = format!("{}\n", init_payload);

        let mut child = Command::new(bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawn {}", bin.display()))?;

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

        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                tracing::debug!(target: "lobby::game_stderr", instance_id, "{}", line);
            }
        });

        let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(Some(child)));

        let db = self.db.clone();
        let instances = self.instances.clone();
        let child_for_reader = child_slot.clone();
        let stdin_tx_for_reader = stdin_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let evt: serde_json::Result<GameEvent> = serde_json::from_str(&line);
                let mut g = instances.lock().await;
                match evt {
                    Ok(GameEvent::Ready { port: ready_port }) => {
                        info!(instance_id, ?ready_port, "game ready");
                        if let Some(h) = g.get_mut(&instance_id) {
                            h.status = Status::Ready;
                        }
                        let _ = sqlx::query("UPDATE game_instances SET status='ready' WHERE instance_id=?")
                            .bind(instance_id).execute(&db).await;
                        let _ = sqlx::query("UPDATE rooms SET status='Running' WHERE room_id=?")
                            .bind(h_room_id(&g, instance_id)).execute(&db).await;
                        // Notify game it can formally start the match.
                        let _ = stdin_tx_for_reader
                            .send(serde_json::to_string(&GameCommand::Start).unwrap())
                            .await;
                    }
                    Ok(GameEvent::Running) => {
                        info!(instance_id, "game running");
                        if let Some(h) = g.get_mut(&instance_id) {
                            h.status = Status::Running;
                        }
                        let _ = sqlx::query("UPDATE game_instances SET status='running' WHERE instance_id=?")
                            .bind(instance_id).execute(&db).await;
                    }
                    Ok(GameEvent::Finished { result }) => {
                        info!(instance_id, ?result, "game finished");
                        if let Some(h) = g.get_mut(&instance_id) {
                            h.status = Status::Finished;
                        }
                        let _ = sqlx::query("UPDATE game_instances SET status='finished', end_time=datetime('now') WHERE instance_id=?")
                            .bind(instance_id).execute(&db).await;
                        let _ = sqlx::query("UPDATE rooms SET status='Finished' WHERE room_id=?")
                            .bind(h_room_id(&g, instance_id)).execute(&db).await;
                        // Reap child + drop instance record.
                        drop(g);
                        if let Some(mut c) = child_for_reader.lock().await.take() {
                            let _ = c.start_kill();
                            let _ = c.wait().await;
                        }
                        instances.lock().await.remove(&instance_id);
                        return;
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
                    Ok(GameEvent::Action) => {
                        // A player acted — keep the instance alive for cleanup.
                        let _ = sqlx::query(
                            "UPDATE game_instances SET last_action_at = datetime('now') WHERE instance_id=?",
                        )
                        .bind(instance_id)
                        .execute(&db)
                        .await;
                    }
                    Err(e) => {
                        warn!(instance_id, error=%e, line=%line, "bad event line");
                    }
                }
                drop(g);
            }
            // stdout closed
            let mut g = instances.lock().await;
            if let Some(h) = g.get_mut(&instance_id) {
                if !matches!(h.status, Status::Stopped | Status::Finished | Status::Abnormal) {
                    h.status = Status::Abnormal;
                    let room = h.room_id;
                    let _ = sqlx::query("UPDATE game_instances SET status='abnormal', end_time=datetime('now') WHERE instance_id=?")
                        .bind(instance_id).execute(&db).await;
                    let _ = sqlx::query("UPDATE rooms SET status='Waiting' WHERE room_id=?")
                        .bind(room).execute(&db).await;
                }
            }
        });

        let child_for_store = child_slot.clone();
        self.instances.lock().await.insert(instance_id, ActiveInstance {
            room_id,
            port,
            status: Status::Starting,
            last_heartbeat: Instant::now(),
            stdin_tx,
            child: child_for_store,
        });

        // Spawn timeout watchdog for this instance (10s to reach ready).
        let instances_wd = self.instances.clone();
        let db_wd = self.db.clone();
        let child_wd = child_slot.clone();
        let stdin_tx_wd = self.instances.clone(); // unused below; ref only
        let _ = stdin_tx_wd;
        tokio::spawn(async move {
            let start = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let mut g = instances_wd.lock().await;
                let Some(h) = g.get_mut(&instance_id) else { return; };
                if matches!(h.status, Status::Ready | Status::Running | Status::Finished | Status::Stopped | Status::Abnormal) {
                    return;
                }
                if start.elapsed() > READY_TIMEOUT {
                    warn!(instance_id, "ready timeout, killing");
                    h.status = Status::Abnormal;
                    let room = h.room_id;
                    let _ = sqlx::query("UPDATE game_instances SET status='abnormal', end_time=datetime('now') WHERE instance_id=?")
                        .bind(instance_id).execute(&db_wd).await;
                    let _ = sqlx::query("UPDATE rooms SET status='Waiting' WHERE room_id=?")
                        .bind(room).execute(&db_wd).await;
                    drop(g);
                    if let Some(mut c) = child_wd.lock().await.take() {
                        let _ = c.start_kill();
                        let _ = c.wait().await;
                    }
                    instances_wd.lock().await.remove(&instance_id);
                    return;
                }
                drop(g);
            }
        });

        Ok(instance_id)
    }

    pub async fn lookup(&self, instance_id: i64) -> Option<(u16, Status)> {
        let g = self.instances.lock().await;
        g.get(&instance_id).map(|h| (h.port, h.status))
    }

    /// Gracefully stop one instance: send cmd:stop, wait up to SHUTDOWN_GRACE, then kill.
    pub async fn stop(&self, instance_id: i64, reason: &str) {
        let mut g = self.instances.lock().await;
        let Some(h) = g.get_mut(&instance_id) else { return; };
        let line = serde_json::to_string(&GameCommand::Stop { reason: reason.into() }).unwrap();
        let _ = h.stdin_tx.send(line).await;
        let child = h.child.clone();
        let room_id = h.room_id;
        h.status = Status::Stopped;
        drop(g);

        let deadline = Instant::now() + SHUTDOWN_GRACE;
        loop {
            let mut g = self.instances.lock().await;
            if !g.contains_key(&instance_id) {
                return;
            }
            if Instant::now() >= deadline {
                warn!(instance_id, "graceful shutdown timeout, force-killing");
                if let Some(mut c) = child.lock().await.take() {
                    let _ = c.start_kill();
                    let _ = c.wait().await;
                }
                g.remove(&instance_id);
                let _ = sqlx::query("UPDATE game_instances SET status='stopped', end_time=datetime('now') WHERE instance_id=?")
                    .bind(instance_id).execute(&self.db).await;
                let _ = sqlx::query("UPDATE rooms SET status='Destroyed' WHERE room_id=?")
                    .bind(room_id).execute(&self.db).await;
                return;
            }
            drop(g);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    #[allow(dead_code)]
    pub async fn send_start(&self, instance_id: i64) {
        let g = self.instances.lock().await;
        if let Some(h) = g.get(&instance_id) {
            let line = serde_json::to_string(&GameCommand::Start).unwrap();
            let _ = h.stdin_tx.send(line).await;
        }
    }

    /// Mark abnormal any instance without heartbeat for >15s.
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
                let room = h.room_id;
                let _ = sqlx::query("UPDATE game_instances SET status='abnormal', end_time=datetime('now') WHERE instance_id=?")
                    .bind(id).execute(&self.db).await;
                let _ = sqlx::query("UPDATE rooms SET status='Waiting' WHERE room_id=?")
                    .bind(room).execute(&self.db).await;
                timed_out.push(*id);
            }
        }
        timed_out
    }

    /// Push a freshly-issued session token to every running game instance.
    /// Each game adds it to its in-memory session registry so users who re-logged
    /// in after game start can still authenticate.
    pub async fn broadcast_session(&self, uid: i64, session: &str) -> usize {
        let line = format!(
            "{{\"event\":\"add_session\",\"uid\":{uid},\"session\":\"{session}\"}}\n"
        );
        let g = self.instances.lock().await;
        let mut pushed = 0usize;
        for (id, h) in g.iter() {
            if matches!(h.status, Status::Stopped | Status::Finished | Status::Abnormal) {
                continue;
            }
            match h.stdin_tx.send(line.clone()).await {
                Ok(()) => {
                    info!(instance_id = id, uid, "session pushed to game stdin");
                    pushed += 1;
                }
                Err(_) => {
                    warn!(instance_id = id, "failed to push session (stdin closed)");
                }
            }
        }
        pushed
    }

    /// Iterate active instances; stop everything (called on SIGTERM).
    pub async fn shutdown_all(&self) {
        let ids: Vec<i64> = self.instances.lock().await.keys().copied().collect();
        for id in ids {
            self.stop(id, "lobby_shutdown").await;
        }
    }
}

fn h_room_id(map: &HashMap<i64, ActiveInstance>, instance_id: i64) -> i64 {
    map.get(&instance_id).map(|h| h.room_id).unwrap_or(0)
}

async fn allocate_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}