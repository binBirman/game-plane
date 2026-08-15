//! Card types for "Take Your Position".
//!
//! `s ∈ 0..3` (Spade, Heart, Diamond, Club) and `r ∈ 0..12` (A, 2, …, K) form
//! the on-the-wire identifiers emitted by [`snapshot`](super::TakeYourPosition::snapshot).
//! See `crates/lobby/static/card-render.js` for the matching frontend renderer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Suit {
    Spade = 0,
    Heart = 1,
    Diamond = 2,
    Club = 3,
}

impl Suit {
    pub fn from_code(c: u8) -> Self {
        match c {
            0 => Suit::Spade,
            1 => Suit::Heart,
            2 => Suit::Diamond,
            3 => Suit::Club,
            _ => Suit::Spade,
        }
    }
    pub fn as_code(&self) -> u8 {
        *self as u8
    }
    pub fn glyph(&self) -> &'static str {
        match self {
            Suit::Spade => "♠",
            Suit::Heart => "♥",
            Suit::Diamond => "♦",
            Suit::Club => "♣",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Rank {
    A = 0,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    J,
    Q,
    K,
}

impl Rank {
    pub fn from_code(c: u8) -> Self {
        match c {
            0 => Rank::A,
            1 => Rank::Two,
            2 => Rank::Three,
            3 => Rank::Four,
            4 => Rank::Five,
            5 => Rank::Six,
            6 => Rank::Seven,
            7 => Rank::Eight,
            8 => Rank::Nine,
            9 => Rank::Ten,
            10 => Rank::J,
            11 => Rank::Q,
            12 => Rank::K,
            _ => Rank::A,
        }
    }
    pub fn as_code(&self) -> u8 {
        *self as u8
    }
    pub fn text(&self) -> &'static str {
        match self {
            Rank::A => "A",
            Rank::Two => "2",
            Rank::Three => "3",
            Rank::Four => "4",
            Rank::Five => "5",
            Rank::Six => "6",
            Rank::Seven => "7",
            Rank::Eight => "8",
            Rank::Nine => "9",
            Rank::Ten => "10",
            Rank::J => "J",
            Rank::Q => "Q",
            Rank::K => "K",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub rank: Rank,
    pub suit: Suit,
}

impl Card {
    pub fn cmp_table(&self, other: &Card, table: &[Card]) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        // A and K become trump when both are present on the table:
        // A > everything, K > everything except A.
        let has_a = table.iter().any(|c| c.rank == Rank::A);
        let has_k = table.iter().any(|c| c.rank == Rank::K);
        if has_a && has_k {
            match (self.rank, other.rank) {
                (Rank::A, Rank::A) | (Rank::K, Rank::K) => {}
                (Rank::A, _) => return Ordering::Greater,
                (_, Rank::A) => return Ordering::Less,
                (Rank::K, _) => return Ordering::Greater,
                (_, Rank::K) => return Ordering::Less,
                _ => {}
            }
        }

        match self.rank.cmp(&other.rank) {
            Ordering::Equal => suit_value(self.suit).cmp(&suit_value(other.suit)),
            other => other,
        }
    }
}

fn suit_value(s: Suit) -> u8 {
    match s {
        Suit::Spade => 4,
        Suit::Heart => 3,
        Suit::Diamond => 2,
        Suit::Club => 1,
    }
}
