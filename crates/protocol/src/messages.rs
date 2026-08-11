use serde::{Deserialize, Serialize};

/// One player in the LobbyInit handed to a game process.
///
/// `sessions` carries **every non-expired session token Lobby currently has**
/// for this `uid` — a user may hold several (re-logins in another tab leave
/// the old row alive until GC). The game must accept ANY of them on
/// `login` / `reconnect`, not only the "latest" one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInit {
    pub uid: i64,
    pub sessions: Vec<String>,
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