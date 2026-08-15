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
    /// Waiting for all players to connect (no timer running yet).
    WaitingAll,
    PriorPrediction,
    Play,
    PosteriorPrediction,
    End,
}

impl Phase {
    pub fn name(&self) -> &'static str {
        match self {
            Phase::WaitingAll => "waiting_all",
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
    // ── per-player time budget ───────────────────────────────
    /// Remaining "refresh" pool (ms). Refills to `refresh_ms` after each action.
    pub time_a_ms: u64,
    /// Remaining "reserve" pool (ms). Never resets; drains after A is empty.
    pub time_b_ms: u64,
    /// When this player's current thinking interval began (None = not thinking).
    pub thinking_since: Option<std::time::Instant>,
}

/// Player time budget. `A` refills on every action; `B` is a one-time reserve
/// for the whole game. A player's total available time = A + B.
#[derive(Debug, Clone, Copy)]
pub struct StepTimers {
    /// Refresh pool per action (ms).
    pub refresh_ms: u64,
    /// One-time reserve pool per player (ms), never resets.
    pub reserve_ms: u64,
}

impl StepTimers {
    /// Parse "A+B" (ms from seconds). A = per-action refresh, B = whole-game reserve.
    pub fn from_preset(preset: Option<&str>) -> Self {
        let (a, b) = match preset.unwrap_or("30+60").split('+').collect::<Vec<_>>()[..] {
            [a, b] => {
                let a: u64 = a.trim().parse().unwrap_or(30);
                let b: u64 = b.trim().parse().unwrap_or(60);
                (a, b)
            }
            _ => (30, 60),
        };
        Self { refresh_ms: a * 1000, reserve_ms: b * 1000 }
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

    /// Player time budget config (None = no time limit).
    pub timers: Option<StepTimers>,
    /// uids that have successfully authenticated (login/reconnect) — used to
    /// delay the first round until everyone is in.
    pub joined: std::collections::HashSet<i64>,
    /// In-progress posterior prediction draft (best→worst uids). First player
    /// edits this live via `draft_posterior`; the frontend sees it in
    /// snapshots so everyone watches the selection in real time. Only the
    /// committed posterior (via `posterior_predict`) counts for scoring.
    pub posterior_draft: Vec<i64>,
}

impl GameState {
    pub fn new(players: Vec<PlayerState>) -> Self {
        let start_player = players.first().map(|p| p.id).unwrap_or(0);
        // joined starts empty — WaitingAll until every player authenticates.
        let joined = std::collections::HashSet::new();
        Self {
            players,
            round: 0,
            start_player,
            phase: Phase::WaitingAll,  // wait for everyone to connect
            current_player: Some(start_player),
            table: vec![],
            pending_events: Vec::new(),
            timers: None,
            joined,
            posterior_draft: Vec::new(),
        }
    }

    /// Mark a player as authenticated. Returns true when the last player just
    /// joined (transition to PriorPrediction + start timer).
    pub fn mark_joined(&mut self, uid: i64) -> bool {
        self.joined.insert(uid);
        self.joined.len() == self.players.len()
    }

    pub fn all_joined(&self) -> bool {
        self.joined.len() >= self.players.len()
    }

    /// Initialize each player's time pools from the timer config.
    /// Called after `timers` is set.
    pub fn apply_timer_config(&mut self) {
        if let Some(t) = self.timers {
            for p in &mut self.players {
                p.time_a_ms = t.refresh_ms;
                p.time_b_ms = t.reserve_ms;
            }
        }
    }

    /// Start counting thinking time for players who still need to act.
    /// Players already thinking keep their interval.
    pub fn start_thinking(&mut self) {
        let now = std::time::Instant::now();
        for (seat, p) in self.players.iter_mut().enumerate() {
            let needs_act = match self.phase {
                Phase::WaitingAll | Phase::End => false,
                Phase::PriorPrediction => !p.has_predicted,
                Phase::Play => p.committed_card.is_none(),
                Phase::PosteriorPrediction => seat == self.start_player && p.posterior_prediction.is_none(),
            };
            if needs_act && p.thinking_since.is_none() {
                p.thinking_since = Some(now);
            }
            if !needs_act {
                p.thinking_since = None;
            }
        }
    }

    /// Elapsed ms while thinking for a player (0 if not thinking).
    pub fn thinking_elapsed_ms(&self, seat: usize) -> u64 {
        match self.players[seat].thinking_since {
            Some(t0) => t0.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            None => 0,
        }
    }

    /// Remaining total time (ms) for a player: A + B, accounting for the
    /// in-progress thinking interval.
    pub fn remaining_ms(&self, seat: usize) -> u64 {
        let a = self.players[seat].time_a_ms;
        let b = self.players[seat].time_b_ms;
        let elapsed = self.thinking_elapsed_ms(seat);
        let total = a.saturating_add(b).saturating_sub(elapsed);
        total
    }

    /// Settle the thinking interval for a player after an action:
    /// deduct elapsed from A first, then from B, then refill A.
    pub fn settle_action(&mut self, seat: usize) {
        let elapsed = self.thinking_elapsed_ms(seat);
        if elapsed > 0 {
            let a = &mut self.players[seat].time_a_ms;
            let take_from_a = (*a).min(elapsed);
            *a -= take_from_a;
            let rest = elapsed - take_from_a;
            if rest > 0 {
                let b = &mut self.players[seat].time_b_ms;
                *b = b.saturating_sub(rest);
            }
        }
        self.players[seat].thinking_since = None;
        // Refill A after the action.
        if let Some(t) = self.timers {
            self.players[seat].time_a_ms = t.refresh_ms;
        }
    }

    /// True if a player has run out of time (A + B both consumed → will proxy).
    pub fn out_of_time(&self, seat: usize) -> bool {
        self.timers.is_some() && self.remaining_ms(seat) == 0
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
        // deal() is called both at initial construction (game should wait for
        // everyone to join) and on restart (everyone is already present). So
        // keep the current phase; callers decide WaitingAll vs PriorPrediction.
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
            Phase::End | Phase::WaitingAll => None,
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
        self.posterior_draft.clear();
        self.table.clear();
        self.pending_events.clear();
        self.phase = Phase::PriorPrediction;
        self.start_thinking();
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