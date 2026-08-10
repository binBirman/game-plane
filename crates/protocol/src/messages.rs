use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInit {
    pub uid: i64,
    pub session: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LobbyInit {
    pub room_id: i64,
    pub game_type: String,
    pub listen: String,
    pub players: Vec<PlayerInit>,
    /// Optional game-specific config (forwarded as-is to the game).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}