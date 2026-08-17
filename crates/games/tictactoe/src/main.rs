//! Tic-Tac-Toe game implementing `GameLogic` via game-sdk.
//!
//! All communication boilerplate (stdin init, WS server, heartbeat, session
//! validation, lifecycle events) lives in `game_sdk::run`. This binary only
//! contains the game rules.

use async_trait::async_trait;
use game_sdk::{ActionOutcome, GameLogic, PhaseInfo};
use protocol::PlayerInit;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Default, Serialize, Deserialize, Clone)]
pub struct TicTacToeConfig {
    /// Injected by lobby from rooms.timer_preset; tictactoe has no timers, ignored.
    #[serde(default)]
    pub timer_preset: Option<String>,
}

struct TicTacToe {
    board: [Option<i64>; 9],
    turn: i64,
    players: Vec<i64>,
    player_sessions: Vec<(i64, Vec<String>)>,
    phase: String, // "playing" | "finished"
    winner: Option<i64>,
}

#[async_trait]
impl GameLogic for TicTacToe {
    type Config = TicTacToeConfig;

    fn new(players: &[PlayerInit], _config: &Self::Config) -> Self {
        let player_sessions = players
            .iter()
            .map(|p| (p.uid, p.sessions.clone()))
            .collect();
        let player_uids: Vec<i64> = players.iter().map(|p| p.uid).collect();
        let turn = player_uids.first().copied().unwrap_or(0);
        Self {
            board: [None; 9],
            turn,
            players: player_uids,
            player_sessions,
            phase: "playing".into(),
            winner: None,
        }
    }

    fn snapshot(&self, _viewer: Option<i64>) -> Value {
        json!({
            "board": self.board.iter().map(|c| c.unwrap_or(0)).collect::<Vec<_>>(),
            "turn": self.turn,
            "players": self.players,
            "phase": self.phase,
            "winner": self.winner,
        })
    }

    fn handle_action(&mut self, uid: i64, action: Value) -> ActionOutcome {
        let kind = action.get("action").and_then(|v| v.as_str()).unwrap_or("");
        if kind != "move" {
            return ActionOutcome::Reject("unknown action".into());
        }
        let cell = action.get("cell").and_then(|v| v.as_u64()).unwrap_or(99) as usize;

        if self.phase != "playing" {
            return ActionOutcome::Reject("game not in progress".into());
        }
        if uid != self.turn {
            return ActionOutcome::Reject("not your turn".into());
        }
        if cell >= 9 || self.board[cell].is_some() {
            return ActionOutcome::Reject("invalid cell".into());
        }

        self.board[cell] = Some(uid);

        if let Some(w) = self.check_winner() {
            self.winner = Some(w);
            self.phase = "finished".into();
            return ActionOutcome::GameOver;
        }
        if self.board.iter().all(|c| c.is_some()) {
            self.phase = "finished".into();
            return ActionOutcome::GameOver;
        }

        // Switch turn to the other player
        self.turn = self
            .players
            .iter()
            .find(|&&u| u != uid)
            .copied()
            .unwrap_or(uid);
        ActionOutcome::Ok
    }

    fn is_over(&self) -> bool {
        self.phase == "finished"
    }

    fn phase(&self) -> PhaseInfo {
        PhaseInfo {
            name: self.phase.clone(),
            active_player: Some(self.turn),
            awaiting: vec![self.turn],
            time_limit_ms: None,
        }
    }

    fn validate_session(&self, uid: i64, session: &str) -> bool {
        self.player_sessions
            .iter()
            .any(|(u, ss)| *u == uid && ss.iter().any(|s| s == session))
    }

    fn result(&self) -> Value {
        json!({
            "winner": self.winner,
            "board": self.board.iter().map(|c| c.unwrap_or(0)).collect::<Vec<_>>(),
        })
    }

    fn min_players(&self) -> usize {
        2
    }
    fn max_players(&self) -> usize {
        2
    }
    fn game_name(&self) -> &'static str {
        "Tic-Tac-Toe"
    }
}

impl TicTacToe {
    fn check_winner(&self) -> Option<i64> {
        const LINES: &[(usize, usize, usize)] = &[
            (0, 1, 2),
            (3, 4, 5),
            (6, 7, 8),
            (0, 3, 6),
            (1, 4, 7),
            (2, 5, 8),
            (0, 4, 8),
            (2, 4, 6),
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
    game_sdk::init_tracing();

    use tokio::io::{AsyncBufReadExt, BufReader};
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let init_line = reader
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("no init line"))?;
    let init: protocol::LobbyInit = serde_json::from_str(&init_line)?;

    game_sdk::run::<TicTacToe>(init).await
}