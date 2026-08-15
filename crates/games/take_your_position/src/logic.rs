//! `GameLogic` adapter wrapping the pure-rule `GameState`.

use std::collections::HashSet;

use async_trait::async_trait;
use game_sdk::{ActionOutcome, GameLogic, PhaseInfo};
use protocol::PlayerInit;
use rand::seq::SliceRandom;
use rand::rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::event::Event;
use crate::rules::{build_players, RuleError};
use crate::state::{GameState, Phase};

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct TakeYourPositionConfig;

pub struct TakeYourPosition {
    state: GameState,
    player_sessions: Vec<(i64, Vec<String>)>,
    /// Events from the most recent action, drained by `snapshot`.
    pending_events: Vec<Event>,
}

#[async_trait]
impl GameLogic for TakeYourPosition {
    type Config = TakeYourPositionConfig;

    fn new(players: &[PlayerInit], _config: &Self::Config) -> Self {
        // Shuffle the seating so the round is random each game.
        let mut uids: Vec<i64> = players.iter().map(|p| p.uid).collect();
        uids.shuffle(&mut rng());
        let player_sessions = players
            .iter()
            .map(|p| (p.uid, p.sessions.clone()))
            .collect::<Vec<_>>();
        let mut state = GameState::new(build_players(&uids));
        state.deal();
        Self {
            state,
            player_sessions,
            pending_events: vec![
                Event::PhaseChanged { phase: Phase::PriorPrediction.name().into() },
            ],
        }
    }

    fn validate_session(&self, uid: i64, session: &str) -> bool {
        self.player_sessions
            .iter()
            .any(|(u, ss)| *u == uid && ss.iter().any(|s| s == session))
    }

    fn is_over(&self) -> bool {
        self.state.phase == Phase::End
    }

    fn phase(&self) -> PhaseInfo {
        let (name, active, awaiting) = match self.state.phase {
            Phase::PriorPrediction => {
                let cur = self.state.current_player.unwrap_or(self.state.start_player);
                let active = self.state.uid_of_seat(cur);
                let awaiting = self.unacted_uids();
                ("prior_prediction", active, awaiting)
            }
            Phase::Play => {
                // Simultaneous — no active player; awaiting = all uncommitted
                let awaiting = self.unacted_uids();
                ("play", None, awaiting)
            }
            Phase::PosteriorPrediction => {
                let sp = self.state.start_player;
                let active = self.state.uid_of_seat(sp);
                let awaiting = if self.state.players[sp].posterior_prediction.is_none() {
                    vec![active.unwrap_or(0)]
                } else {
                    vec![]
                };
                ("posterior_prediction", active, awaiting)
            }
            Phase::End => ("ended", None, vec![]),
        };
        PhaseInfo {
            name: name.to_string(),
            active_player: active,
            awaiting,
            time_limit_ms: None,
        }
    }

    fn snapshot(&self, viewer: Option<i64>) -> Value {
        let players = &self.state.players;
        let scores: Vec<(i64, i32)> = players.iter().map(|p| (p.uid, p.score)).collect();
        let predictions: Vec<(i64, Option<u8>, bool)> = players
            .iter()
            .map(|p| (p.uid, p.prediction, p.has_predicted))
            .collect();
        let committed: Vec<(i64, Option<Value>)> = players
            .iter()
            .map(|p| {
                let card_json = match &p.committed_card {
                    // During Play phase, all committed cards are face-down to
                    // everyone else — but the OWNER sees their own card
                    // face-up so they can confirm what they played.
                    Some(c) => {
                        if viewer == Some(p.uid) {
                            Some(json!({
                                "s": c.suit.as_code(),
                                "r": c.rank.as_code(),
                                "hidden": false,
                            }))
                        } else {
                            Some(json!({ "hidden": true }))
                        }
                    }
                    None => None,
                };
                (p.uid, card_json)
            })
            .collect();
        let posterior: Vec<(i64, Option<Vec<i64>>, bool)> = players
            .iter()
            .map(|p| {
                let v = p.posterior_prediction.clone();
                let committed = v.is_some();
                (p.uid, v, committed)
            })
            .collect();
        // Per-viewer table:
        //  - PriorPrediction: empty
        //  - Play: empty (cards are in `committed`, face-down)
        //  - PosteriorPrediction / End: revealed (face-up to all, owner sees {s,r,hidden:false})
        let table: Vec<(i64, Option<Value>)> = match self.state.phase {
            Phase::Play | Phase::PriorPrediction => vec![],
            Phase::PosteriorPrediction | Phase::End => self
                .state
                .table
                .iter()
                .map(|(uid, card)| {
                    (uid.clone(), Some(json!({
                        "s": card.suit.as_code(),
                        "r": card.rank.as_code(),
                        "hidden": false,
                    })))
                })
                .collect(),
        };
        // Hand: only owner's hand, face-up to themselves.
        let hand = viewer.and_then(|uid| {
            self.state.seat_of(uid).map(|seat| {
                self.state.players[seat]
                    .hand
                    .iter()
                    .map(|c| json!({ "s": c.suit.as_code(), "r": c.rank.as_code() }))
                    .collect::<Vec<_>>()
            })
        });
        // Played-history per player (suit + rank only).
        let history: Vec<(i64, Vec<Value>)> = players
            .iter()
            .map(|p| {
                let cards = p
                    .played_history
                    .iter()
                    .map(|c| json!({ "s": c.suit.as_code(), "r": c.rank.as_code() }))
                    .collect();
                (p.uid, cards)
            })
            .collect();

        json!({
            "phase": self.state.phase.name(),
            "round": self.state.round,
            "current_player": self.state.current_player.and_then(|s| self.state.uid_of_seat(s)),
            "start_player": self.state.uid_of_seat(self.state.start_player),
            "players": players.iter().map(|p| p.uid).collect::<Vec<_>>(),
            "seats": players.iter().map(|p| p.id).collect::<Vec<_>>(),
            "scores": scores,
            "predictions": predictions,
            "committed": committed,
            "posterior": posterior,
            "table": table,
            "hand": hand,
            "history": history,
            "pending_events": self.pending_events_for_snapshot(),
            "is_over": self.is_over(),
        })
    }

    fn handle_action(&mut self, uid: i64, action: Value) -> ActionOutcome {
        let kind = action.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let result = match kind {
            "predict" => self.state.apply_predict(
                uid,
                action.get("rank").and_then(|v| v.as_u64()).map(|n| n as u8),
            ),
            "play_card" => self.state.apply_play_card(
                uid,
                action.get("card_index").and_then(|v| v.as_u64()).unwrap_or(99) as u8,
            ),
            "posterior_predict" => self.state.apply_posterior(
                uid,
                action
                    .get("rank_list")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
                    .unwrap_or_default(),
            ),
            "restart_vote" => {
                if let Some(seat) = self.state.seat_of(uid) {
                    self.state.players[seat].restart_yes = action
                        .get("yes")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let yes_count = self.state.players.iter().filter(|p| p.restart_yes).count();
                    if yes_count == self.state.players.len() && self.is_over() {
                        self.state.deal();
                        self.pending_events.push(Event::PhaseChanged {
                            phase: self.state.phase.name().into(),
                        });
                        return ActionOutcome::Ok;
                    }
                }
                Ok(vec![])
            }
            _ => Err(RuleError::Unknown),
        };

        match result {
            Ok(mut events) => {
                self.advance_phase(&mut events);
                self.pending_events.extend(events);
                if self.is_over() {
                    self.pending_events.push(self.state.end_game());
                    ActionOutcome::GameOver
                } else {
                    ActionOutcome::Ok
                }
            }
            Err(e) => ActionOutcome::Reject(e.to_string()),
        }
    }

    fn min_players(&self) -> usize { 5 }
    fn max_players(&self) -> usize { 5 }
    fn game_name(&self) -> &'static str { "TYP · Take Your Position" }
}

impl TakeYourPosition {
    fn unacted_uids(&self) -> Vec<i64> {
        let n = self.state.players.len();
        let mut out = Vec::new();
        let mut seen: HashSet<usize> = HashSet::new();
        match self.state.phase {
            Phase::PriorPrediction => {
                let start = self.state.current_player.unwrap_or(self.state.start_player);
                for off in 0..n {
                    let idx = (start + off) % n;
                    if seen.contains(&idx) { continue; }
                    seen.insert(idx);
                    if !self.state.players[idx].has_predicted {
                        out.push(self.state.players[idx].uid);
                    }
                }
            }
            Phase::Play => {
                for (i, p) in self.state.players.iter().enumerate() {
                    if p.committed_card.is_none() {
                        out.push(p.uid);
                    }
                }
            }
            Phase::PosteriorPrediction => {
                let sp = self.state.start_player;
                if self.state.players[sp].posterior_prediction.is_none() {
                    out.push(self.state.players[sp].uid);
                }
            }
            Phase::End => {}
        }
        out
    }

    fn advance_phase(&mut self, events: &mut Vec<Event>) {
        match self.state.phase {
            Phase::PriorPrediction => {
                // Move current_player to next unacted; if all acted → Play.
                if let Some(next) = self.state.next_unacted() {
                    self.state.current_player = Some(next);
                    return;
                }
                self.state.phase = Phase::Play;
                self.state.current_player = None; // simultaneous
                events.push(Event::PhaseChanged { phase: Phase::Play.name().into() });
            }
            Phase::Play => {
                // Wait until all 5 committed.
                if self.state.players.iter().any(|p| p.committed_card.is_none()) {
                    return;
                }
                // Enter posterior WITHOUT revealing — cards stay face-down on the
                // table and history stays stale until the posterior is committed.
                self.state.phase = Phase::PosteriorPrediction;
                self.state.current_player = Some(self.state.start_player);
                events.push(Event::PhaseChanged { phase: Phase::PosteriorPrediction.name().into() });
            }
            Phase::PosteriorPrediction => {
                let sp = self.state.start_player;
                if self.state.players[sp].posterior_prediction.is_none() {
                    return;
                }
                // NOW reveal: move committed_card → table, append to history,
                // then score the round.
                self.state.reveal_plays();
                events.push(self.state.finish_round());
                self.state.round += 1;
                if self.state.round >= 5 {
                    self.state.phase = Phase::End;
                    events.push(Event::PhaseChanged { phase: Phase::End.name().into() });
                } else {
                    self.state.begin_next_round();
                    events.push(Event::PhaseChanged { phase: Phase::PriorPrediction.name().into() });
                }
            }
            Phase::End => {}
        }
    }

    fn pending_events_for_snapshot(&self) -> Vec<Value> {
        self.pending_events
            .iter()
            .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
            .collect()
    }
}

// Helper trait method on GameState so the snapshot can be safely emitted
// even for the post-game Empty seat.
trait GameStateExt {
    fn uid_of_seat_safe(&self, seat: usize) -> Option<i64>;
}
impl GameStateExt for GameState {
    fn uid_of_seat_safe(&self, seat: usize) -> Option<i64> {
        self.players.get(seat).map(|p| p.uid)
    }
}