use axum::routing::{get, post};
use axum::Router;

use super::captcha;
use super::room;
use super::static_files;
use super::user;
use crate::state::SharedState;
use crate::ws_proxy::handler::ws_handler;

pub fn build(state: SharedState) -> Router {
    Router::new()
        .route("/api/register", post(user::register))
        .route("/api/login", post(user::login))
        .route("/api/captcha/challenge", post(captcha::issue))
        .route("/api/rooms", get(room::list).post(room::create))
        .route("/api/rooms/:room_id", get(room::get))
        .route("/api/rooms/:room_id/join", post(room::join))
        .route("/api/rooms/:room_id/leave", post(room::leave))
        .route("/api/rooms/:room_id/start", post(room::start))
        .route("/ws/:instance_id", get(ws_handler))
        .fallback(static_files::fallback)
        .with_state(state)
}