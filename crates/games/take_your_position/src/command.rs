use serde::{Deserialize, Serialize};

/// Player action envelope. Identical wire format on stdin → SDK → `handle_action`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Prior prediction: which final rank (1..=5) the player guesses they'll finish at.
    /// `rank: None` means "pass / no prediction".
    Predict { rank: Option<u8> },
    /// Play one card from own hand by index (0-based).
    PlayCard { card_index: u8 },
    /// Posterior prediction (only `start_player` may submit): list of uids in
    /// predicted best-to-worst order. Length must equal the number of players.
    PosteriorPredict { rank_list: Vec<i64> },
    /// Vote on starting a new round after `phase == Ended`. `yes = true` → vote yes.
    RestartVote { yes: bool },
}
