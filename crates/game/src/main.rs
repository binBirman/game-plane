use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use protocol::LobbyInit;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ServerMsg {
    #[serde(rename = "login_ok")]
    LoginOk,
    #[serde(rename = "snapshot")]
    Snapshot { state: serde_json::Value },
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "game")]
    Game { data: serde_json::Value },
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
    Game { data: serde_json::Value },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event")]
enum LobbyEvent {
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "stop")]
    Stop { #[serde(default)] reason: String },
}

#[derive(Default)]
struct GameState {
    board: [Option<i64>; 9],
    turn: i64,
    players: Vec<i64>,
    // (uid, session_token) passed in by Lobby at spawn; used to validate login frames.
    player_sessions: Vec<(i64, String)>,
    phase: String,
    winner: Option<i64>,
}

impl GameState {
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "board": self.board.iter().map(|c| c.unwrap_or(0)).collect::<Vec<_>>(),
            "turn": self.turn,
            "players": self.players,
            "phase": self.phase,
            "winner": self.winner,
        })
    }

    fn check_winner(&self) -> Option<i64> {
        const LINES: &[(usize, usize, usize)] = &[
            (0, 1, 2), (3, 4, 5), (6, 7, 8),
            (0, 3, 6), (1, 4, 7), (2, 5, 8),
            (0, 4, 8), (2, 4, 6),
        ];
        for &(a, b, c) in LINES {
            if let (Some(x), Some(y), Some(z)) = (self.board[a], self.board[b], self.board[c]) {
                if x == y && y == z {
                    return Some(x);
                }
            }
        }
        None
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let init_line = reader
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("no init line"))?;
    let init: LobbyInit = serde_json::from_str(&init_line)?;
    info!(room_id = init.room_id, "game starting");

    let app = Router::new().route("/ws", get(ws_handler));
    let listener = tokio::net::TcpListener::bind(&init.listen).await?;
    let local_port = listener.local_addr()?.port();
    info!(port = local_port, "ws listening");

    println!("{{\"event\":\"ready\",\"port\":{}}}", local_port);

    // Heartbeat task
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            println!("{{\"event\":\"heartbeat\"}}");
        }
    });

    // Stdin cmd reader
    let mut cmd_reader = reader;
    tokio::spawn(async move {
        while let Ok(Some(line)) = cmd_reader.next_line().await {
            if let Ok(evt) = serde_json::from_str::<LobbyEvent>(&line) {
                info!(?evt, "lobby cmd");
                match evt {
                    LobbyEvent::Start => println!("{{\"event\":\"running\"}}"),
                    LobbyEvent::Stop { reason } => {
                        println!("{{\"event\":\"shutdown\"}}");
                        info!(reason, "stopping");
                        std::process::exit(0);
                    }
                }
            }
        }
    });

    let state = Arc::new(Mutex::new(GameState {
        players: init.players.iter().map(|p| p.uid).collect(),
        player_sessions: init.players.iter().map(|p| (p.uid, p.session.clone())).collect(),
        ..Default::default()
    }));
    let state_for_handler = state.clone();

    axum::serve(listener, app.with_state(state_for_handler)).await?;
    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<Arc<Mutex<GameState>>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn send_error(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>, code: &str, msg: &str) {
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&ServerMsg::Error {
                code: code.into(),
                message: msg.into(),
            })
            .unwrap(),
        ))
        .await;
}

async fn handle_socket(socket: WebSocket, state: Arc<Mutex<GameState>>) {
    let (mut sender, mut receiver) = socket.split();

    // First frame must be login or reconnect; validate session against state.player_sessions.
    let login_uid = loop {
        match receiver.next().await {
            Some(Ok(Message::Text(t))) => {
                match serde_json::from_str::<ClientMsg>(&t) {
                    Ok(ClientMsg::Login { uid, session }) => {
                        let valid = {
                            let s = state.lock().await;
                            s.player_sessions.iter().any(|(u, t)| *u == uid && *t == session)
                        };
                        if valid {
                            break uid;
                        } else {
                            send_error(&mut sender, "INVALID_SESSION", "session not valid").await;
                            return;
                        }
                    }
                    Ok(ClientMsg::Reconnect { uid, session }) => {
                        let valid = {
                            let s = state.lock().await;
                            s.player_sessions.iter().any(|(u, t)| *u == uid && *t == session)
                        };
                        if valid {
                            break uid;
                        } else {
                            send_error(&mut sender, "INVALID_SESSION", "session not valid").await;
                            return;
                        }
                    }
                    _ => {
                        send_error(&mut sender, "BAD_FRAME", "first frame must be login or reconnect").await;
                        return;
                    }
                }
            }
            _ => return,
        }
    };

    let _ = sender
        .send(Message::Text(serde_json::to_string(&ServerMsg::LoginOk).unwrap()))
        .await;

    let snap = state.lock().await.snapshot();
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&ServerMsg::Snapshot { state: snap }).unwrap(),
        ))
        .await;

    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(t) => match serde_json::from_str::<ClientMsg>(&t) {
                Ok(ClientMsg::Ping) => {
                    let _ = sender
                        .send(Message::Text(serde_json::to_string(&ServerMsg::Pong).unwrap()))
                        .await;
                }
                Ok(ClientMsg::Game { data }) => {
                    let mut s = state.lock().await;
                    let action = data.get("action").and_then(|v| v.as_str()).unwrap_or("");
                    if action == "move" {
                        let cell = data.get("cell").and_then(|v| v.as_u64()).unwrap_or(99) as usize;
                        if s.phase == "playing" && login_uid == s.turn && cell < 9 && s.board[cell].is_none() {
                            s.board[cell] = Some(login_uid);
                            s.turn = s.players.iter().find(|&&u| u != login_uid).copied().unwrap_or(login_uid);
                            if let Some(w) = s.check_winner() {
                                s.winner = Some(w);
                                s.phase = "finished".into();
                                println!("{{\"event\":\"finished\"}}");
                            } else if s.board.iter().all(|c| c.is_some()) {
                                s.phase = "finished".into();
                                println!("{{\"event\":\"finished\"}}");
                            }
                            let snap = s.snapshot();
                            drop(s);
                            let _ = sender
                                .send(Message::Text(
                                    serde_json::to_string(&ServerMsg::Game {
                                        data: serde_json::json!({"kind":"update","state": snap}),
                                    }).unwrap(),
                                ))
                                .await;
                        } else {
                            let _ = sender
                                .send(Message::Text(
                                    serde_json::to_string(&ServerMsg::GameError {
                                        code: "INVALID_MOVE".into(),
                                        message: "not your turn / bad cell".into(),
                                    }).unwrap(),
                                ))
                                .await;
                        }
                    } else {
                        let _ = sender
                            .send(Message::Text(
                                serde_json::to_string(&ServerMsg::GameError {
                                    code: "UNKNOWN_ACTION".into(),
                                    message: format!("unknown action: {}", action),
                                }).unwrap(),
                            ))
                            .await;
                    }
                }
                Ok(ClientMsg::Login { .. } | ClientMsg::Reconnect { .. }) => {
                    let _ = sender
                        .send(Message::Text(
                            serde_json::to_string(&ServerMsg::Error {
                                code: "ALREADY_LOGGED_IN".into(),
                                message: "session already established".into(),
                            }).unwrap(),
                        ))
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
}