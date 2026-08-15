//! Game rules: action validation, phase advancement, round scoring.

use crate::card::Card;
use crate::event::Event;
use crate::state::{GameState, Phase, PlayerState};

/// Errors surfaced to the offending client as `game_error`.
#[derive(Debug)]
pub enum RuleError {
    NotYourTurn { expected: i64, got: i64 },
    WrongPhase { expected: &'static str, got: &'static str },
    OutOfRange,
    NotFirstPlayer,
    AlreadyActed,
    NotEnoughPlayers,
    Unknown,
}

impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleError::NotYourTurn { expected, got } => {
                write!(f, "not your turn (expected uid={expected}, got {got})")
            }
            RuleError::WrongPhase { expected, got } => {
                write!(f, "wrong phase: expected {expected}, current {got}")
            }
            RuleError::OutOfRange => write!(f, "value out of range"),
            RuleError::NotFirstPlayer => write!(f, "only the first player may submit the posterior prediction"),
            RuleError::AlreadyActed => write!(f, "you have already acted in this phase; cannot change"),
            RuleError::NotEnoughPlayers => write!(f, "not enough players"),
            RuleError::Unknown => write!(f, "unknown error"),
        }
    }
}

impl GameState {
    /// Apply a prior prediction. Locks the prediction once set (no re-prediction).
    pub fn apply_predict(&mut self, uid: i64, rank: Option<u8>) -> Result<Vec<Event>, RuleError> {
        if self.phase != Phase::PriorPrediction {
            return Err(RuleError::WrongPhase { expected: "prior_prediction", got: self.phase.name() });
        }
        let seat = self.seat_of(uid).ok_or(RuleError::Unknown)?;
        // Lock: cannot change once prediction is committed (rank or skip).
        if self.players[seat].has_predicted {
            return Err(RuleError::AlreadyActed);
        }
        if let Some(r) = rank {
            if !(1..=self.players.len() as u8).contains(&r) {
                return Err(RuleError::OutOfRange);
            }
        }
        self.players[seat].prediction = rank;
        self.players[seat].has_predicted = true;
        Ok(vec![Event::PredictionAccepted { uid }])
    }

    /// Apply a play. Removes the card from the hand and stores it in
    /// `committed_card` (face-down to all). Locked once set.
    pub fn apply_play_card(&mut self, uid: i64, card_index: u8) -> Result<Vec<Event>, RuleError> {
        if self.phase != Phase::Play {
            return Err(RuleError::WrongPhase { expected: "play", got: self.phase.name() });
        }
        let seat = self.seat_of(uid).ok_or(RuleError::Unknown)?;
        // Lock: cannot play twice
        if self.players[seat].committed_card.is_some() {
            return Err(RuleError::AlreadyActed);
        }
        let idx = card_index as usize;
        if idx >= self.players[seat].hand.len() {
            return Err(RuleError::OutOfRange);
        }
        let card = self.players[seat].hand.remove(idx);
        // NOTE: played_history is appended later in `reveal_plays` — the card
        // record should appear only when the table is revealed (posterior phase),
        // per user requirement. Kept in `committed_card` until then.
        self.players[seat].committed_card = Some(card);
        Ok(vec![Event::CardPlayed { uid }])
    }

    /// Apply a posterior prediction. Must be either empty (skip) or a full
    /// ranking of all 5 player uids in best→worst order. Locked once set.
    pub fn apply_posterior(&mut self, uid: i64, mut rank_list: Vec<i64>) -> Result<Vec<Event>, RuleError> {
        if self.phase != Phase::PosteriorPrediction {
            return Err(RuleError::WrongPhase { expected: "posterior_prediction", got: self.phase.name() });
        }
        let seat = self.seat_of(uid).ok_or(RuleError::Unknown)?;
        if seat != self.start_player {
            return Err(RuleError::NotFirstPlayer);
        }
        // Lock
        if self.players[seat].posterior_prediction.is_some() {
            return Err(RuleError::AlreadyActed);
        }
        let n = self.players.len();
        // Empty = skip. Otherwise must contain exactly all n uids, no duplicates.
        if rank_list.is_empty() {
            self.players[seat].posterior_prediction = Some(Vec::new());
            return Ok(vec![Event::PosteriorPredictionAccepted { uid }]);
        }
        if rank_list.len() != n {
            return Err(RuleError::OutOfRange);
        }
        let mut seen = vec![false; n];
        for &u in &rank_list {
            let s = self.seat_of(u).ok_or(RuleError::OutOfRange)?;
            if seen[s] { return Err(RuleError::OutOfRange); }
            seen[s] = true;
        }
        // All seats must appear
        if seen.iter().any(|&x| !x) {
            return Err(RuleError::OutOfRange);
        }
        // No further mutation needed — store as-is
        rank_list.shrink_to_fit();
        self.players[seat].posterior_prediction = Some(rank_list);
        Ok(vec![Event::PosteriorPredictionAccepted { uid }])
    }

    /// Compute scores for the just-revealed table and return the RoundResult
    /// event. Caller must have already called `reveal_plays()`.
    pub fn finish_round(&mut self) -> Event {
        let n = self.players.len();
        let rank_scores = [2i32, 1, 0, -1, -2]; // 1st..5th
        let first_seat = self.start_player;

        // Sort table by card strength (worst → best), then reverse for best → worst.
        // `table` is `(uid, Card)`; we sort by index (positional), then map to uid.
        let mut ranking_idx: Vec<usize> = (0..self.table.len()).collect();
        let cards_at: Vec<Card> = self.table.iter().map(|(_, c)| c.clone()).collect();
        ranking_idx.sort_by(|a, b| {
            let ca = &cards_at[*a];
            let cb = &cards_at[*b];
            ca.cmp_table(cb, &cards_at)
        });
        ranking_idx.reverse();

        let mut delta = vec![0i32; n];
        let mut rank_score = vec![0i32; n];          // placement score
        let mut prediction_score = vec![0i32; n];     // prior-prediction score
        let mut posterior_score = vec![0i32; n];      // posterior-prediction score
        let mut prediction_snapshot: Vec<Option<u8>> = Vec::with_capacity(n);
        for p in &self.players {
            prediction_snapshot.push(p.prediction);
        }

        // Posterior scoring for the first player.
        let mut posterior_snapshot: Vec<i64> = Vec::with_capacity(n);
        if let Some(pred) = self.players[first_seat].posterior_prediction.clone() {
            // pred == empty vec means they skipped
            if !pred.is_empty() {
                // ranking_idx[i] is the original table-index of the i-th-best player.
                // Map it back to uid for accuracy comparison.
                let ranking_uids: Vec<i64> = ranking_idx
                    .iter()
                    .map(|&i| self.table[i].0)
                    .collect();
                let mut accurate = 0usize;
                for (i, &uid) in pred.iter().enumerate() {
                    if ranking_uids.get(i) == Some(&uid) {
                        accurate += 1;
                    }
                }
                // Per game rules accurate is guaranteed to be one of {5, 3, 2, 1, 0}.
                // (`4` is impossible: a 5-player ranking has no 1-off-by-one derangement.)
                // Anything else (incl. 4) means upstream logic broke — assert so
                // the bug is loud rather than silently mis-scoring.
                let posterior_pts = match accurate {
                    5 => 2,
                    3 => 1,
                    2 => 0,
                    1 => -1,
                    _ => -2, // 0 (or any unexpected value below)
                };
                assert!(
                    matches!(accurate, 5 | 3 | 2 | 1 | 0),
                    "posterior accurate={} not in {{5,3,2,1,0}}; check upstream logic",
                    accurate
                );
                posterior_score[first_seat] += posterior_pts;
                delta[first_seat] += posterior_pts;
                posterior_snapshot = pred;
            }
        }

        // Per-player rank + prior-prediction scoring.
        for (rank_pos, &idx) in ranking_idx.iter().enumerate() {
            let uid = self.table[idx].0;
            let seat = self.seat_of(uid).unwrap_or(0);
            let score_idx = rank_pos.min(4);
            rank_score[seat] += rank_scores[score_idx];
            delta[seat] += rank_scores[score_idx];
            if let Some(rank_guess) = self.players[seat].prediction {
                if rank_guess as usize == rank_pos + 1 {
                    prediction_score[seat] += 2;
                    delta[seat] += 2;
                } else {
                    prediction_score[seat] -= 2;
                    delta[seat] -= 2;
                }
            }
        }

        // Apply deltas.
        for (i, p) in self.players.iter_mut().enumerate() {
            p.score += delta[i];
        }

        // `cards` must align with `ranking` (index i ⇔ ranking[i]) so the
        // frontend can show each placed player's card correctly. Emitted as
        // `(suit_code, rank_code)` integers for the UI.
        let cards: Vec<(u8, u8)> = ranking_idx
            .iter()
            .map(|&i| {
                let c = &self.table[i].1;
                (c.suit.as_code(), c.rank.as_code())
            })
            .collect();
        let ranking_uids: Vec<i64> = ranking_idx.iter().map(|&i| self.table[i].0).collect();

        Event::RoundResult {
            round: self.round,
            cards,
            ranking: ranking_uids,
            prediction: prediction_snapshot,
            posterior_prediction: posterior_snapshot,
            score_delta: delta,
            rank_score,
            prediction_score,
            posterior_score,
        }
    }
}

/// Build the per-seat PlayerState list from a uid ordering (already shuffled
/// at game start). Seat index = position in the slice.
pub fn build_players(uid_in_order: &[i64]) -> Vec<PlayerState> {
    uid_in_order
        .iter()
        .enumerate()
        .map(|(id, &uid)| PlayerState {
            id,
            uid,
            hand: vec![],
            score: 0,
            prediction: None,
            has_predicted: false,
            committed_card: None,
            posterior_prediction: None,
            restart_yes: false,
            played_history: vec![],
        })
        .collect()
}