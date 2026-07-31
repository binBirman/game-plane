use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::state::SharedState;

#[derive(Serialize)]
pub struct ChallengeResp {
    pub challenge: String,
    pub difficulty: u32,
    pub ttl_seconds: u64,
}

pub async fn issue(State(state): State<SharedState>) -> Json<ChallengeResp> {
    let (challenge, difficulty) = crate::auth::pow::issue(state.pow_difficulty);
    Json(ChallengeResp {
        challenge,
        difficulty,
        ttl_seconds: 300,
    })
}