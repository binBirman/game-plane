//! Take Your Position — 5-player trick-prediction card game.
//!
//! Communication boilerplate (stdin init, WS, session check, heartbeat,
//! lifecycle events) lives in `game_sdk::run`. This binary only declares
//! the game crate and wires `TakeYourPosition` into the SDK.

mod card;
mod command;
mod event;
mod logic;
mod rules;
mod state;

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::logic::TakeYourPosition;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    game_sdk::init_tracing();

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let init_line = reader
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("no init line"))?;
    let init: protocol::LobbyInit = serde_json::from_str(&init_line)?;

    game_sdk::run::<TakeYourPosition>(init).await
}
