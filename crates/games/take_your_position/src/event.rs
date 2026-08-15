use serde::{Deserialize, Serialize};

use crate::card::Card;

/// Server-originated events emitted into `pending_events` inside snapshots,
/// and into the per-viewer hand field. These are *informational only* —
/// the SDK only ships `snapshot` frames to clients, so the frontend
/// reconstructs what happened by diffing snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Event {
    /// A prior prediction was accepted for `uid`.
    PredictionAccepted { uid: i64 },
    /// A card was played by `uid` (the actual card is on the `table`).
    CardPlayed { uid: i64 },
    /// A posterior prediction was accepted for `uid` (the first player).
    PosteriorPredictionAccepted { uid: i64 },
    /// Round N finished; carries per-player scores, ranking, predictions.
    RoundResult {
        round: u8,
        /// Cards as `(suit_code, rank_code)` pairs, aligned with `ranking`
        /// (index i ⇔ ranking[i]). Integers so the frontend can render them
        /// directly (the `Card` enum-name serialization is not useful to UI).
        cards: Vec<(u8, u8)>,
        ranking: Vec<i64>,               // best → worst
        prediction: Vec<Option<u8>>,     // per-seat prior predictions
        posterior_prediction: Vec<i64>,  // first player's posterior ranking
        score_delta: Vec<i32>,
        /// Per-player placement score (by final rank).
        rank_score: Vec<i32>,
        /// Per-player prior-prediction score (+2/-2/0).
        prediction_score: Vec<i32>,
        /// Per-player posterior-prediction score (first player only).
        posterior_score: Vec<i32>,
    },
    /// Phase changed (e.g. PriorPrediction → Play).
    PhaseChanged { phase: String },
    /// Final scores for the whole game.
    GameEnded { final_scores: Vec<(i64, i32)> },
}
