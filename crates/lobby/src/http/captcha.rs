use axum::extract::{ConnectInfo, State};
use axum::Json;
use serde::Serialize;
use std::net::SocketAddr;

use crate::http::error::ApiError;
use crate::state::SharedState;

#[derive(Serialize)]
pub struct ChallengeResp {
    pub challenge: String,
    pub difficulty: u32,
    pub ttl_seconds: u64,
}

pub async fn issue(
    State(state): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<Json<ChallengeResp>, ApiError> {
    if !state.rl_captcha.check(addr.ip()).await {
        return Err(ApiError::RateLimited);
    }
    let (challenge, difficulty) = crate::auth::pow::issue(state.pow_difficulty);
    Ok(Json(ChallengeResp {
        challenge,
        difficulty,
        ttl_seconds: 300,
    }))
}