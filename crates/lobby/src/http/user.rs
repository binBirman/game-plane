use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tracing::Instrument;

use crate::auth::{password, pow, session};
use crate::http::error::ApiError;
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct CaptchaPayload {
    pub challenge: String,
    pub nonce: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterReq {
    pub username: String,
    pub password: String,
    pub nickname: String,
    #[serde(default)]
    pub captcha: Option<CaptchaPayload>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResp {
    pub uid: i64,
}

pub async fn register(
    State(state): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<RegisterReq>,
) -> Result<impl IntoResponse, ApiError> {
    let span = tracing::info_span!("register", username = %req.username, nickname = %req.nickname);

    async move {
        if !state.rl_register.check(addr.ip()).await {
            tracing::warn!("rate limited");
            return Err(ApiError::RateLimited);
        }
        let cap = req.captcha.as_ref().ok_or(ApiError::CaptchaRequired)?;
        if !pow::verify(&cap.challenge, &cap.nonce, state.pow_difficulty) {
            tracing::warn!("captcha invalid");
            return Err(ApiError::CaptchaInvalid);
        }

        if req.username.is_empty() || req.password.is_empty() || req.nickname.is_empty() {
            tracing::warn!("missing required field");
            return Err(ApiError::InvalidParams("username/password/nickname required".into()));
        }

        if let Err(reason) = password::validate_strength(&req.password) {
            tracing::warn!(reason, "weak password rejected");
            return Err(ApiError::WeakPassword(reason));
        }

        let hash = password::hash_password(&req.password).map_err(|e| {
            tracing::error!(error = %e, "hash_password failed");
            ApiError::Internal(e)
        })?;

        let res = sqlx::query("INSERT INTO users (username, password_hash, nickname) VALUES (?, ?, ?)")
            .bind(&req.username)
            .bind(&hash)
            .bind(&req.nickname)
            .execute(&state.db)
            .await;

        match res {
            Ok(r) => {
                let uid = r.last_insert_rowid();
                tracing::info!(uid, "user registered");
                Ok((StatusCode::OK, Json(RegisterResp { uid })))
            }
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                tracing::warn!("username taken");
                Err(ApiError::UsernameTaken)
            }
            Err(e) => {
                tracing::error!(error = %e, "insert user failed");
                Err(ApiError::Internal(anyhow::Error::from(e)))
            }
        }
    }
    .instrument(span)
    .await
}

#[derive(Debug, Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub captcha: Option<CaptchaPayload>,
}

#[derive(Debug, Serialize)]
pub struct LoginResp {
    pub uid: i64,
    pub token: String,
}

pub async fn login(
    State(state): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginReq>,
) -> Result<Json<LoginResp>, ApiError> {
    let span = tracing::info_span!("login", username = %req.username);

    async move {
        if !state.rl_login.check(addr.ip()).await {
            tracing::warn!("rate limited");
            return Err(ApiError::RateLimited);
        }
        let cap = req.captcha.as_ref().ok_or(ApiError::CaptchaRequired)?;
        if !pow::verify(&cap.challenge, &cap.nonce, state.pow_difficulty) {
            tracing::warn!("captcha invalid");
            return Err(ApiError::CaptchaInvalid);
        }

        if req.username.is_empty() || req.password.is_empty() {
            tracing::warn!("missing required field");
            return Err(ApiError::InvalidParams("username/password required".into()));
        }

        let row: Option<(i64, String)> =
            sqlx::query_as("SELECT id, password_hash FROM users WHERE username = ?")
                .bind(&req.username)
                .fetch_optional(&state.db)
                .await
                .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))?;

        let Some((uid, hash)) = row else {
            tracing::warn!("user not found");
            return Err(ApiError::InvalidCredentials);
        };

        let ok = password::verify_password(&req.password, &hash).map_err(|e| {
            tracing::error!(error = %e, "verify_password failed");
            ApiError::Internal(e)
        })?;
        if !ok {
            tracing::warn!("password mismatch");
            return Err(ApiError::InvalidCredentials);
        }

        let token = session::generate_token();
        let expires = session::expires_at(state.session_ttl_days).map_err(|e| {
            tracing::error!(error = %e, "expires_at failed");
            ApiError::Internal(e)
        })?;

        sqlx::query("INSERT INTO sessions (token, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(&token)
            .bind(uid)
            .bind(&expires)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))?;

        tracing::info!(uid, "login ok");
        Ok(Json(LoginResp { uid, token }))
    }
    .instrument(span)
    .await
}

pub async fn logout(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    // No CurrentUser here — we want to revoke the *current* token even if it's
    // already expired/invalid, so that subsequent /api/* calls (which would
    // re-use the same browser storage) get 401 and the frontend clears state.
    // We also accept a missing header as "already gone" -> 200 (idempotent).
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    let Some(token) = token else {
        return Ok(Json(serde_json::json!({"ok": true})));
    };

    let res = sqlx::query("DELETE FROM sessions WHERE token = ?")
        .bind(&token)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))?;
    tracing::info!(deleted = res.rows_affected(), "logout");
    Ok(Json(serde_json::json!({"ok": true})))
}