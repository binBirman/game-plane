use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::Instrument;

use crate::auth::extractor::CurrentUser;
use crate::http::error::ApiError;
use crate::state::SharedState;

const SUPPORTED_GAMES: &[&str] = &["tictactoe"];

#[derive(Debug, Deserialize)]
pub struct CreateReq {
    pub game_type: String,
}

#[derive(Debug, Serialize)]
pub struct PlayerInfo {
    pub uid: i64,
    pub nickname: String,
    pub seat: i32,
}

#[derive(Debug, Serialize)]
pub struct RoomInfo {
    pub room_id: i64,
    pub game_type: String,
    pub host_uid: i64,
    pub status: String,
    pub players: Vec<PlayerInfo>,
}

#[derive(Debug, Serialize)]
pub struct CreateResp {
    pub room_id: i64,
    pub status: String,
}

pub async fn create(
    State(state): State<SharedState>,
    user: CurrentUser,
    Json(req): Json<CreateReq>,
) -> Result<impl IntoResponse, ApiError> {
    let span = tracing::info_span!("room.create", game_type = %req.game_type, uid = user.uid);

    async move {
        if !SUPPORTED_GAMES.contains(&req.game_type.as_str()) {
            tracing::warn!("unsupported game_type");
            return Err(ApiError::GameTypeUnsupported(req.game_type));
        }

        let mut tx = state.db.begin().await.map_err(|e| ApiError::Internal(e.into()))?;

        let row: (i64,) = sqlx::query_as(
            "INSERT INTO rooms (game_type, host_uid, status) VALUES (?, ?, 'Waiting') RETURNING room_id",
        )
        .bind(&req.game_type)
        .bind(user.uid)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
        let room_id = row.0;

        sqlx::query("INSERT INTO room_players (room_id, uid, seat) VALUES (?, ?, 0)")
            .bind(room_id)
            .bind(user.uid)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

        tx.commit().await.map_err(|e| ApiError::Internal(e.into()))?;

        tracing::info!(room_id, "room created");
        Ok((
            StatusCode::CREATED,
            Json(CreateResp { room_id, status: "Waiting".into() }),
        ))
    }
    .instrument(span)
    .await
}

pub async fn get(
    State(state): State<SharedState>,
    _user: CurrentUser,
    Path(room_id): Path<i64>,
) -> Result<Json<RoomInfo>, ApiError> {
    let span = tracing::info_span!("room.get", room_id);

    async move {
        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT game_type, host_uid, status FROM rooms WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

        let (game_type, host_uid, status) = row.ok_or(ApiError::RoomNotFound)?;

        let players: Vec<(i64, String, i32)> = sqlx::query_as(
            "SELECT u.id, u.nickname, rp.seat FROM room_players rp JOIN users u ON u.id = rp.uid WHERE rp.room_id = ? ORDER BY rp.seat",
        )
        .bind(room_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

        Ok(Json(RoomInfo {
            room_id,
            game_type,
            host_uid,
            status,
            players: players.into_iter().map(|(uid, nickname, seat)| PlayerInfo { uid, nickname, seat }).collect(),
        }))
    }
    .instrument(span)
    .await
}

pub async fn join(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(room_id): Path<i64>,
) -> Result<Json<RoomInfo>, ApiError> {
    let span = tracing::info_span!("room.join", room_id, uid = user.uid);

    async move {
        let mut tx = state.db.begin().await.map_err(|e| ApiError::Internal(e.into()))?;

        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT game_type, host_uid, status FROM rooms WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
        let (game_type, host_uid, status) = row.ok_or(ApiError::RoomNotFound)?;

        if status != "Waiting" {
            return Err(ApiError::RoomNotWaiting);
        }

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM room_players WHERE room_id = ?")
            .bind(room_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        if count.0 >= 2 {
            return Err(ApiError::RoomFull);
        }

        let existing: Option<(i64,)> = sqlx::query_as("SELECT uid FROM room_players WHERE room_id = ? AND uid = ?")
            .bind(room_id)
            .bind(user.uid)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        if existing.is_some() {
            return Err(ApiError::AlreadyInRoom);
        }

        sqlx::query("INSERT INTO room_players (room_id, uid, seat) VALUES (?, ?, 1)")
            .bind(room_id)
            .bind(user.uid)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

        tx.commit().await.map_err(|e| ApiError::Internal(e.into()))?;

        let players: Vec<(i64, String, i32)> = sqlx::query_as(
            "SELECT u.id, u.nickname, rp.seat FROM room_players rp JOIN users u ON u.id = rp.uid WHERE rp.room_id = ? ORDER BY rp.seat",
        )
        .bind(room_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

        Ok(Json(RoomInfo {
            room_id,
            game_type,
            host_uid,
            status,
            players: players.into_iter().map(|(uid, nickname, seat)| PlayerInfo { uid, nickname, seat }).collect(),
        }))
    }
    .instrument(span)
    .await
}

pub async fn leave(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(room_id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let span = tracing::info_span!("room.leave", room_id, uid = user.uid);

    async move {
        let affected = sqlx::query("DELETE FROM room_players WHERE room_id = ? AND uid = ?")
            .bind(room_id)
            .bind(user.uid)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

        if affected.rows_affected() == 0 {
            return Err(ApiError::NotInRoom);
        }

        // If room is empty, mark destroyed
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM room_players WHERE room_id = ?")
            .bind(room_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        if count.0 == 0 {
            let _ = sqlx::query("UPDATE rooms SET status='Destroyed' WHERE room_id = ?")
                .bind(room_id)
                .execute(&state.db)
                .await;
        }

        Ok(Json(serde_json::json!({"ok": true})))
    }
    .instrument(span)
    .await
}

#[derive(Debug, Serialize)]
pub struct StartResp {
    pub instance_id: i64,
    pub ws_url: String,
}

pub async fn start(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(room_id): Path<i64>,
) -> Result<Json<StartResp>, ApiError> {
    let span = tracing::info_span!("room.start", room_id, uid = user.uid);

    async move {
        // Verify room + host + status
        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT game_type, host_uid, status FROM rooms WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
        let (game_type, host_uid, status) = row.ok_or(ApiError::RoomNotFound)?;

        if host_uid != user.uid {
            return Err(ApiError::NotHost);
        }

        if status != "Waiting" {
            return Err(ApiError::RoomNotWaiting);
        }

        // Verify players count
        let players: Vec<(i64, String)> = sqlx::query_as(
            "SELECT rp.uid, s.token FROM room_players rp JOIN sessions s ON s.user_id = rp.uid WHERE rp.room_id = ?",
        )
        .bind(room_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

        if players.len() < 2 {
            return Err(ApiError::NotEnoughPlayers);
        }

        // Mark Starting
        sqlx::query("UPDATE rooms SET status='Starting' WHERE room_id = ?")
            .bind(room_id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

        // Spawn Game
        let instance_id = state
            .instances
            .spawn(room_id, &game_type, players.clone())
            .await
            .map_err(|e| {
                tracing::error!(error=%e, "spawn failed");
                ApiError::InstanceStartFailed
            })?;

        let ws_url = format!("ws://{}:8192/ws/{}", state.public_host, instance_id);

        tracing::info!(instance_id, ws_url = %ws_url, "game started");
        Ok(Json(StartResp { instance_id, ws_url }))
    }
    .instrument(span)
    .await
}

#[derive(Debug, Serialize)]
pub struct ListResp {
    pub rooms: Vec<RoomInfo>,
}

pub async fn list(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> Result<Json<ListResp>, ApiError> {
    let rows: Vec<(i64, String, i64, String)> = sqlx::query_as(
        "SELECT room_id, game_type, host_uid, status FROM rooms WHERE status IN ('Waiting', 'Running') ORDER BY room_id DESC LIMIT 50",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let mut rooms = Vec::with_capacity(rows.len());
    for (room_id, game_type, host_uid, status) in rows {
        let players: Vec<(i64, String, i32)> = sqlx::query_as(
            "SELECT u.id, u.nickname, rp.seat FROM room_players rp JOIN users u ON u.id = rp.uid WHERE rp.room_id = ? ORDER BY rp.seat",
        )
        .bind(room_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
        rooms.push(RoomInfo {
            room_id,
            game_type,
            host_uid,
            status,
            players: players.into_iter().map(|(uid, nickname, seat)| PlayerInfo { uid, nickname, seat }).collect(),
        });
    }

    Ok(Json(ListResp { rooms }))
}