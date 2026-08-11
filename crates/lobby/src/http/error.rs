use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(thiserror::Error, Debug)]
pub enum ApiError {
    #[error("invalid params: {0}")]
    InvalidParams(String),
    #[error("username taken")]
    UsernameTaken,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("captcha required")]
    CaptchaRequired,
    #[error("captcha invalid")]
    CaptchaInvalid,
    #[error("{0}")]
    WeakPassword(&'static str),
    #[error("room not found")]
    RoomNotFound,
    #[error("room full")]
    RoomFull,
    #[error("already in room")]
    AlreadyInRoom,
    #[error("not in room")]
    NotInRoom,
    #[error("only host can perform this action")]
    NotHost,
    #[error("room not in waiting state")]
    RoomNotWaiting,
    #[error("not enough players")]
    NotEnoughPlayers,
    #[error("game type not supported: {0}")]
    GameTypeUnsupported(String),
    #[error("instance not found")]
    InstanceNotFound,
    #[error("instance not ready")]
    InstanceNotReady,
    #[error("instance start failed: {0}")]
    InstanceStartFailed(String),
    #[error("game binary not found: {0}")]
    GameBinaryNotFound(String),
    #[error("rate limited")]
    RateLimited,
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::InvalidParams(_) => (StatusCode::BAD_REQUEST, "INVALID_PARAMS"),
            ApiError::UsernameTaken => (StatusCode::CONFLICT, "USERNAME_TAKEN"),
            ApiError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS"),
            ApiError::CaptchaRequired => (StatusCode::BAD_REQUEST, "CAPTCHA_REQUIRED"),
            ApiError::CaptchaInvalid => (StatusCode::BAD_REQUEST, "CAPTCHA_INVALID"),
            ApiError::WeakPassword(_) => (StatusCode::BAD_REQUEST, "WEAK_PASSWORD"),
            ApiError::RoomNotFound => (StatusCode::NOT_FOUND, "ROOM_NOT_FOUND"),
            ApiError::RoomFull => (StatusCode::CONFLICT, "ROOM_FULL"),
            ApiError::AlreadyInRoom => (StatusCode::CONFLICT, "ALREADY_IN_ROOM"),
            ApiError::NotInRoom => (StatusCode::FORBIDDEN, "NOT_IN_ROOM"),
            ApiError::NotHost => (StatusCode::FORBIDDEN, "NOT_HOST"),
            ApiError::RoomNotWaiting => (StatusCode::CONFLICT, "ROOM_NOT_WAITING"),
            ApiError::NotEnoughPlayers => (StatusCode::CONFLICT, "NOT_ENOUGH_PLAYERS"),
            ApiError::GameTypeUnsupported(_) => (StatusCode::BAD_REQUEST, "GAME_TYPE_UNSUPPORTED"),
            ApiError::InstanceNotFound => (StatusCode::NOT_FOUND, "INSTANCE_NOT_FOUND"),
            ApiError::InstanceNotReady => (StatusCode::CONFLICT, "INSTANCE_NOT_READY"),
            ApiError::InstanceStartFailed(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INSTANCE_START_FAILED"),
            ApiError::GameBinaryNotFound(_) => (StatusCode::SERVICE_UNAVAILABLE, "GAME_BINARY_NOT_FOUND"),
            ApiError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };
        let body = json!({
            "error": {
                "code": code,
                "message": self.to_string(),
            }
        });
        (status, Json(body)).into_response()
    }
}