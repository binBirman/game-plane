//! Game state for "Take Your Position".
//!
//! 5 players, 5 rounds. Each round:
//!   PriorPrediction → Play → PosteriorPrediction → score → next round
//! After 5 rounds the game ends.
//!
//! Seating: `state.players[i].id == i`. At game start the seats are assigned
//! randomly (uids are shuffled in `TakeYourPosition::new`). `start_player`
//! rotates counter-clockwise between rounds.

use rand::rng;
use rand::seq::SliceRandom;

use crate::card::{Card, Rank, Suit};
use crate::event::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    PriorPrediction,
    Play,
    PosteriorPrediction,
    End,
}

impl Phase {
    pub fn name(&self) -> &'static str {
        match self {
            Phase::PriorPrediction => "prior_prediction",
            Phase::Play => "play",
            Phase::PosteriorPrediction => "posterior_prediction",
            Phase::End => "ended",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlayerState {
    pub id: usize,                            // 0..=4 (seat around the table)
    pub uid: i64,                             // Lobby uid
    pub hand: Vec<Card>,                       // cards still held
    pub score: i32,
    /// Committed prior prediction (locked once set).
    /// `None` means "放弃/不预测" (only valid when `has_predicted == true`).
    pub prediction: Option<u8>,
    /// True once the player committed their prior prediction — distinguishes
    /// "haven't acted yet" (false) from "skipped / rank=None" (true).
    /// This is what `next_unacted` / `unacted_uids` must check, NOT
    /// `prediction.is_some()`, otherwise a skipped player blocks the round.
    pub has_predicted: bool,
    /// Card this player committed this round. `None` while they haven't
    /// played yet; moves to `GameState::table` when the round advances.
    pub committed_card: Option<Card>,
    /// Committed posterior prediction: `Some(vec)` with exactly 5 uids
    /// (best → worst) or `None` (skipped). Locked once set.
    pub posterior_prediction: Option<Vec<i64>>,
    pub restart_yes: bool,
    /// Every card this player has played across all rounds.
    pub played_history: Vec<Card>,
}

/// Step timer budget. `per_round` == true → reset each round; false → the
/// budget is shared across the whole game (global countdown).
#[derive(Debug, Clone, Copy)]
pub struct StepTimers {
    pub per_round: bool,
    pub predict_secs: u64,
    pub play_secs: u64,
}

impl StepTimers {
    /// Parse a timer preset like "30+60" | "40+120" | "60+180".
    /// First number = prior-prediction budget, second = play budget.
    /// Presets < 60s for play are treated as per-round; the 60+180 preset is
    /// the global-shared one.
    pub fn from_preset(preset: Option<&str>) -> Self {
        let (predict, play, per_round) = match preset.unwrap_or("30+60").split('+').collect::<Vec<_>>()[..] {
            [p, pl] => {
                let p: u64 = p.trim().parse().unwrap_or(30);
                let pl: u64 = pl.trim().parse().unwrap_or(60);
                // 60+180 => global shared; others => per-round reset.
                (p, pl, !(p == 60 && pl == 180))
            }
            _ => (30, 60, true),
        };
        Self { per_round, predict_secs: predict, play_secs: play }
    }
}

#[derive(Debug)]
pub struct GameState {
    pub players: Vec<PlayerState>,
    pub round: u8,                 // 0..=4; round N+1 finishes and increments
    pub start_player: usize,       // seat index of the round's first player
    pub phase: Phase,
    /// Seat index of the player whose turn it is during PriorPrediction.
    /// `None` during Play (simultaneous) and PosteriorPrediction (only the
    /// start player acts).
    pub current_player: Option<usize>,
    /// Cards revealed on the table, indexed by player uid (so the frontend
    /// can look them up by uid, not by seat index). Populated only after
    /// all 5 players have committed (Play phase end).
    pub table: Vec<(i64, Card)>,

    /// Pending events queued for the next snapshot. Drained (cleared) by
    /// `begin_next_round` so the per-snapshot event stream doesn't grow
    /// unbounded across rounds.
    pub pending_events: Vec<crate::event::Event>,

    /// Step timers (None = no time limit).
    pub timers: Option<StepTimers>,
    /// Monotonic deadline for the current phase, if a timer is active.
    pub deadline: Option<std::time::Instant>,
}

impl GameState {
    pub fn new(players: Vec<PlayerState>) -> Self {
        let start_player = players.first().map(|p| p.id).unwrap_or(0);
        Self {
            players,
            round: 0,
            start_player,
            phase: Phase::PriorPrediction,
            current_player: Some(start_player),
            table: vec![],
            pending_events: Vec::new(),
            timers: None,
            deadline: None,
        }
    }

    /// Hand out 5 cards per player: 2 small (A-7 ♥/♣/♦) + 2 big (8-K ♥/♣/♦) + 1 spade (A-K ♠).
    /// Idempotent: clears existing hands first.
    pub fn deal(&mut self) {
        let mut rng = rng();
        let mut small: Vec<Card> = vec![];
        let mut big: Vec<Card> = vec![];
        for &suit in &[Suit::Heart, Suit::Club, Suit::Diamond] {
            for r in 0..=6 {
                small.push(Card { rank: Rank::from_code(r), suit });
            }
            for r in 7..=12 {
                big.push(Card { rank: Rank::from_code(r), suit });
            }
        }
        let mut spades: Vec<Card> = (0..=12)
            .map(|r| Card { rank: Rank::from_code(r), suit: Suit::Spade })
            .collect();
        small.shuffle(&mut rng);
        big.shuffle(&mut rng);
        spades.shuffle(&mut rng);

        for p in &mut self.players {
            p.hand = vec![
                small.pop().unwrap(),
                small.pop().unwrap(),
                big.pop().unwrap(),
                big.pop().unwrap(),
                spades.pop().unwrap(),
            ];
            p.prediction = None;
            p.has_predicted = false;
            p.committed_card = None;
            p.posterior_prediction = None;
            p.restart_yes = false;
            p.played_history.clear();
        }
        self.table.clear();
        self.round = 0;
        self.start_player = self.players.first().map(|p| p.id).unwrap_or(0);
        self.current_player = Some(self.start_player);
        self.phase = Phase::PriorPrediction;
    }

    /// Seat index of the next player who hasn't acted yet in the current phase.
    /// For PriorPrediction: next player without `prediction`.
    /// For Play: next player without `committed_card`.
    /// For PosteriorPrediction: only the start_player if they haven't predicted.
    pub fn next_unacted(&self) -> Option<usize> {
        let n = self.players.len();
        match self.phase {
            Phase::PriorPrediction => {
                // Sequential, starting from current_player.
                // Use `has_predicted` (not `prediction.is_none()`) so a skipped
                // player (rank=None) still counts as acted.
                let start = self.current_player.unwrap_or(self.start_player);
                for off in 0..n {
                    let idx = (start + off) % n;
                    if !self.players[idx].has_predicted {
                        return Some(idx);
                    }
                }
                None
            }
            Phase::Play => {
                // Simultaneous — any player without committed_card
                self.players.iter().position(|p| p.committed_card.is_none())
            }
            Phase::PosteriorPrediction => {
                let sp = self.start_player;
                if self.players[sp].posterior_prediction.is_none() {
                    Some(sp)
                } else {
                    None
                }
            }
            Phase::End => None,
        }
    }

    pub fn seat_of(&self, uid: i64) -> Option<usize> {
        self.players.iter().position(|p| p.uid == uid)
    }

    pub fn uid_of_seat(&self, seat: usize) -> Option<i64> {
        self.players.get(seat).map(|p| p.uid)
    }

    /// Advance the round: rotate start_player counter-clockwise, reset
    /// per-round state, and clear accumulated pending_events so the
    /// per-snapshot event stream doesn't grow unbounded across rounds.
    pub fn begin_next_round(&mut self) {
        let n = self.players.len();
        self.start_player = if n == 0 { 0 } else { (self.start_player + n - 1) % n };
        self.current_player = Some(self.start_player);
        for p in &mut self.players {
            p.prediction = None;
            p.has_predicted = false;
            p.committed_card = None;
            p.posterior_prediction = None;
        }
        self.table.clear();
        self.pending_events.clear();
        self.phase = Phase::PriorPrediction;
        self.refresh_deadline();
    }

    /// Set / keep the deadline for the current phase based on `timers`.
    /// Per-round mode resets the deadline on every phase entry; global mode
    /// keeps the first-set deadline across rounds.
    pub fn refresh_deadline(&mut self) {
        let Some(t) = self.timers else {
            self.deadline = None;
            return;
        };
        if t.per_round {
            // Fresh budget each time we enter a phase.
            self.deadline = self.phase_deadline();
        } else {
            // Global shared: keep whatever deadline was first set for this phase.
            if self.deadline.is_none() {
                self.deadline = self.phase_deadline();
            }
        }
    }

    fn phase_deadline(&self) -> Option<std::time::Instant> {
        let t = self.timers?;
        let secs = match self.phase {
            Phase::PriorPrediction => t.predict_secs,
            Phase::Play => t.play_secs,
            Phase::PosteriorPrediction => t.play_secs, // reuse play budget for posterior
            Phase::End => return None,
        };
        Some(std::time::Instant::now() + std::time::Duration::from_secs(secs))
    }

    /// True if the current phase's deadline has passed.
    pub fn deadline_passed(&self) -> bool {
        match self.deadline {
            Some(d) => std::time::Instant::now() >= d,
            None => false,
        }
    }

    /// Move every player's `committed_card` into `table` (revealed) and append
    /// to each player's `played_history`. Call once all 5 players have committed
    /// during Play. History + table update happen together at reveal time.
    pub fn reveal_plays(&mut self) {
        self.table.clear();
        for p in &mut self.players {
            if let Some(card) = &p.committed_card {
                self.table.push((p.uid, card.clone()));
                p.played_history.push(card.clone());
            }
            p.committed_card = None;
        }
    }

    /// Build the GameEnded event with final scores.
    pub fn end_game(&self) -> Event {
        let mut final_scores: Vec<(i64, i32)> =
            self.players.iter().map(|p| (p.uid, p.score)).collect();
        final_scores.sort_by(|a, b| b.1.cmp(&a.1));
        Event::GameEnded { final_scores }
    }
}