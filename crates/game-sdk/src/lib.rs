//! Game SDK: 通信骨架（无游戏逻辑）。
//!
//! 各游戏 crate 通过实现 [`GameLogic`] trait 并调用 [`run`] 启动。
//! SDK 负责：stdin init 解析、WS 服务、session 校验、心跳、lifecycle 事件、
//! 按 viewer 的 `snapshot()` 推送给各玩家。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use protocol::LobbyInit;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

/// Initialize tracing for a game binary. Output goes to stderr so stdout
/// stays clean for the JSON line protocol that Lobby reads.
pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

/// 当前阶段信息（推送给客户端，供 UI 展示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_player: Option<i64>,
    #[serde(default)]
    pub awaiting: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_limit_ms: Option<u64>,
}

/// 单次 action 的处理结果。
#[derive(Debug)]
pub enum ActionOutcome {
    Ok,
    Reject(String),
    GameOver,
}

/// 游戏逻辑 trait。每个游戏 crate 实现一次。
#[async_trait]
pub trait GameLogic: Send + Sync + 'static {
    type Config: Default
        + Serialize
        + for<'de> Deserialize<'de>
        + Clone
        + Send
        + Sync;

    /// 用玩家列表与配置构造。
    fn new(players: &[protocol::PlayerInit], config: &Self::Config) -> Self;

    /// 给指定玩家的快照视图。`None` = 全公开（旁观者 / 结算）。
    fn snapshot(&self, viewer: Option<i64>) -> Value;

    /// 处理一个 action；返回 [`ActionOutcome`]。
    fn handle_action(&mut self, uid: i64, action: Value) -> ActionOutcome;

    /// 游戏是否结束。
    fn is_over(&self) -> bool;

    /// 当前阶段。
    fn phase(&self) -> PhaseInfo;

    /// 验证 `(uid, session)`。SDK 在 login / reconnect 时调用。
    fn validate_session(&self, uid: i64, session: &str) -> bool;

    /// 游戏结束时的全局结果（写 DB / UI 展示）。
    fn result(&self) -> Value {
        Value::Null
    }

    /// 玩家数下限（Lobby `start` 前校验）。
    fn min_players(&self) -> usize {
        2
    }

    /// 玩家数上限。
    fn max_players(&self) -> usize {
        8
    }

    /// 显示名（注册表 / UI）。
    fn game_name(&self) -> &'static str {
        "Game"
    }
}

/// 启动游戏：读 stdin init → 解析 config → bind WS → 发 ready → 跑事件循环。
///
/// 调用方需要在 main 中先初始化 tracing。`init.config` 若有则作为 `L::Config` 解析，缺省用 `L::Config::default()`。
pub async fn run<L: GameLogic>(init: LobbyInit) -> anyhow::Result<()> {
    let config: L::Config = match init.config.as_ref() {
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| anyhow::anyhow!("parse game config: {e}"))?,
        None => L::Config::default(),
    };

    info!(
        room_id = init.room_id,
        players = init.players.len(),
        "game-sdk starting"
    );

    let logic = Arc::new(Mutex::new(L::new(&init.players, &config)));
    let registry: ConnRegistry = Arc::new(Mutex::new(HashMap::new()));
    let sessions = Arc::new(SessionRegistry::new());

    let app = Router::new()
        .route("/ws", get(ws_handler::<L>))
        .with_state((logic.clone(), registry.clone(), sessions.clone()));
    let listener = tokio::net::TcpListener::bind(&init.listen).await?;
    let port = listener.local_addr()?.port();
    info!(port, "ws listening");

    println!("{{\"event\":\"ready\",\"port\":{port}}}");

    // Heartbeat task
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            println!("{{\"event\":\"heartbeat\"}}");
        }
    });

    // Stdin cmd reader
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if let Ok(evt) = serde_json::from_str::<LobbyEvent>(&line) {
                match evt {
                    LobbyEvent::Start => println!("{{\"event\":\"running\"}}"),
                    LobbyEvent::Stop { reason } => {
                        println!("{{\"event\":\"shutdown\"}}");
                        info!(reason, "stopping");
                        std::process::exit(0);
                    }
                    LobbyEvent::AddSession { uid, session } => {
                        sessions.add(uid, &session);
                        info!(uid, "session added (pushed from lobby)");
                    }
                }
            }
        }
    });

    axum::serve(listener, app).await?;
    Ok(())
}

// ─── internal ─────────────────────────────────────────────────────────

/// Session registry that lobby can populate mid-game via stdin commands.
/// Login is accepted if EITHER the game's `validate_session` says yes
/// OR this registry contains the token (lobby-pushed sessions).
#[derive(Default)]
pub struct SessionRegistry {
    sessions: std::sync::Mutex<HashMap<i64, std::collections::HashSet<String>>>,
}

impl SessionRegistry {
    pub fn new() -> Self { Self::default() }
    pub fn add(&self, uid: i64, session: &str) {
        let mut m = self.sessions.lock().expect("session registry poisoned");
        m.entry(uid).or_default().insert(session.to_string());
    }
    pub fn contains(&self, uid: i64, session: &str) -> bool {
        let m = self.sessions.lock().expect("session registry poisoned");
        m.get(&uid).is_some_and(|s| s.contains(session))
    }
}

type ConnRegistry = Arc<Mutex<HashMap<i64, mpsc::Sender<ServerMsg>>>>;

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ServerMsg {
    #[serde(rename = "login_ok")]
    LoginOk,
    #[serde(rename = "snapshot")]
    Snapshot { state: Value },
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "game")]
    #[allow(dead_code)]
    Game { data: Value },
    #[serde(rename = "game_error")]
    GameError { code: String, message: String },
    #[serde(rename = "error")]
    Error { code: String, message: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClientMsg {
    #[serde(rename = "login")]
    Login { uid: i64, session: String },
    #[serde(rename = "reconnect")]
    Reconnect { uid: i64, session: String },
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "game")]
    Game { data: Value },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event")]
enum LobbyEvent {
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "stop")]
    Stop {
        #[serde(default)]
        reason: String,
    },
    /// Lobby pushes a freshly-created session token (e.g. user re-logged in
    /// after the game started). Adds to the session registry; subsequent
    /// login/reconnect with this token succeeds.
    #[serde(rename = "add_session")]
    AddSession { uid: i64, session: String },
}

async fn ws_handler<L: GameLogic>(
    ws: WebSocketUpgrade,
    axum::extract::State((logic, registry, sessions)): axum::extract::State<(
        Arc<Mutex<L>>,
        ConnRegistry,
        Arc<SessionRegistry>,
    )>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket::<L>(socket, logic, registry, sessions))
}

async fn handle_socket<L: GameLogic>(
    socket: WebSocket,
    logic: Arc<Mutex<L>>,
    registry: ConnRegistry,
    sessions: Arc<SessionRegistry>,
) {
    let (mut sender, mut receiver) = socket.split();

    // First frame: login or reconnect
    let mut uid: i64 = 0;
    let mut authed = false;
    while !authed {
        match receiver.next().await {
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientMsg>(&t) {
                Ok(ClientMsg::Login { uid: u, session }) => {
                    let from_logic = logic.lock().await.validate_session(u, &session);
                    let from_registry = sessions.contains(u, &session);
                    if from_logic || from_registry {
                        uid = u;
                        authed = true;
                    } else {
                        let _ = send_err(
                            &mut sender,
                            "INVALID_SESSION",
                            "session not valid",
                        )
                        .await;
                        return;
                    }
                }
                Ok(ClientMsg::Reconnect { uid: u, session }) => {
                    let from_logic = logic.lock().await.validate_session(u, &session);
                    let from_registry = sessions.contains(u, &session);
                    if from_logic || from_registry {
                        uid = u;
                        authed = true;
                    } else {
                        let _ = send_err(
                            &mut sender,
                            "INVALID_SESSION",
                            "session not valid",
                        )
                        .await;
                        return;
                    }
                }
                _ => {
                    let _ = send_err(
                        &mut sender,
                        "BAD_FRAME",
                        "first frame must be login or reconnect",
                    )
                    .await;
                    return;
                }
            },
            _ => return,
        }
    }

    // Register this connection's outbound channel
    let (tx, mut rx) = mpsc::channel::<ServerMsg>(16);
    {
        let mut r = registry.lock().await;
        if let Some(old) = r.insert(uid, tx.clone()) {
            // Close previous connection by dropping its sender; old connection's send will fail.
            drop(old);
        }
    }

    if send_msg(&mut sender, &ServerMsg::LoginOk).await.is_err() {
        cleanup(&registry, uid).await;
        return;
    }

    // Initial snapshot (per-viewer)
    let initial = logic.lock().await.snapshot(Some(uid));
    if send_msg(&mut sender, &ServerMsg::Snapshot { state: initial })
        .await
        .is_err()
    {
        cleanup(&registry, uid).await;
        return;
    }

    // Outbound pump: forward queued ServerMsg to WS
    let mut sender_for_outbound = sender; // rename for clarity
    let outbound = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if send_msg(&mut sender_for_outbound, &m).await.is_err() {
                break;
            }
        }
    });

    // Inbound loop
    let game_over = false;
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(t) => match serde_json::from_str::<ClientMsg>(&t) {
                Ok(ClientMsg::Ping) => {
                    let _ = tx.send(ServerMsg::Pong).await;
                }
                Ok(ClientMsg::Game { data }) => {
                    let outcome = {
                        let mut g = logic.lock().await;
                        g.handle_action(uid, data)
                    };
                    match outcome {
                        ActionOutcome::Ok => {
                            broadcast_snapshot(&logic, &registry).await;
                        }
                        ActionOutcome::Reject(reason) => {
                            let _ = tx
                                .send(ServerMsg::GameError {
                                    code: "INVALID_MOVE".into(),
                                    message: reason,
                                })
                                .await;
                        }
                        ActionOutcome::GameOver => {
                            let _ = game_over; // suppress unused_assignments (loop exits via `break`)
                            println!("{{\"event\":\"finished\"}}");
                            broadcast_snapshot(&logic, &registry).await;
                            break;
                        }
                    }
                    // Check game-over signal from logic itself
                    let over = logic.lock().await.is_over();
                    if over {
                        let _ = game_over;
                        println!("{{\"event\":\"finished\"}}");
                        broadcast_snapshot(&logic, &registry).await;
                        break;
                    }
                }
                Ok(ClientMsg::Login { .. } | ClientMsg::Reconnect { .. }) => {
                    let _ = tx
                        .send(ServerMsg::Error {
                            code: "ALREADY_LOGGED_IN".into(),
                            message: "session already established".into(),
                        })
                        .await;
                }
                Err(e) => {
                    warn!(error=%e, "bad frame");
                }
            },
            Message::Close(_) => break,
            _ => {}
        }
    }

    // Inbound loop ended. Drop the channel's sender halves so the outbound
    // pump's `rx.recv()` returns `None` and it drains naturally. Aborting
    // the pump here would race with `broadcast_snapshot` — the last
    // game-over snapshot is queued via `tx.send()` and can be lost if the
    // pump is killed before flushing it onto the wire.
    drop(tx);
    cleanup(&registry, uid).await;
    let _ = outbound.await;
}

async fn cleanup(registry: &ConnRegistry, uid: i64) {
    let mut r = registry.lock().await;
    r.remove(&uid);
}

async fn broadcast_snapshot<L: GameLogic>(
    logic: &Arc<Mutex<L>>,
    registry: &ConnRegistry,
) {
    // Build per-player snapshots under game lock, then send under registry lock (drop game lock first).
    let pending: Vec<(i64, Value)> = {
        let g = logic.lock().await;
        let r = registry.lock().await;
        r.keys()
            .map(|uid| (*uid, g.snapshot(Some(*uid))))
            .collect()
    };
    for (uid, snap) in pending {
        if let Some(tx) = registry.lock().await.get(&uid) {
            let _ = tx.send(ServerMsg::Snapshot { state: snap }).await;
        }
    }
}

async fn send_msg(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    msg: &ServerMsg,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(msg)
        .map_err(axum::Error::new)?;
    sender
        .send(Message::Text(text))
        .await
        .map_err(axum::Error::new)
}

async fn send_err(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    code: &str,
    msg: &str,
) -> Result<(), axum::Error> {
    send_msg(
        sender,
        &ServerMsg::Error {
            code: code.into(),
            message: msg.into(),
        },
    )
    .await
}