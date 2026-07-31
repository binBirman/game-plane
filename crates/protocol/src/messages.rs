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
}