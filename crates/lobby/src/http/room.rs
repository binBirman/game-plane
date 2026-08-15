use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::Instrument;

use crate::auth::extractor::CurrentUser;
use crate::games::registry::public_view;
use crate::http::error::ApiError;
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct CreateReq {
    pub game_type: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    /// Per-round or global step timer: "30+60" | "40+120" | "60+180".
    #[serde(default)]
    pub timer_preset: Option<String>,
}

fn validate_timer_preset(p: &str) -> bool {
    // "N+M": two non-negative integers separated by '+'. The UI offers
    // 30+60 / 40+120 / 60+180 / 300+0, but any pair is valid (0 = unlimited
    // for that step).
    let parts: Vec<&str> = p.split('+').collect();
    parts.len() == 2
        && parts.iter().all(|x| !x.is_empty() && x.parse::<u64>().is_ok())
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub players: Vec<PlayerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_instance_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_players: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_players: Option<usize>,
    /// Per-round or global step timer preset: "30+60" / "40+120" / "60+180".
    /// `null` for non-TYP games (tictactoe has no time limit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timer_preset: Option<String>,
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
    let span = tracing::info_span!(
        "room.create",
        game_type = %req.game_type,
        uid = user.uid
    );

    async move {
        let entry = state
            .games
            .get(&req.game_type)
            .ok_or_else(|| ApiError::GameTypeUnsupported(req.game_type.clone()))?;

        if let Some(v) = &req.variant {
            if !entry.variants.is_empty() && !entry.variants.iter().any(|x| x == v) {
                return Err(ApiError::InvalidParams(format!(
                    "unknown variant '{}' for game_type '{}'",
                    v, req.game_type
                )));
            }
        }

        let config_str = match &req.config {
            Some(v) => Some(serde_json::to_string(v).map_err(|e| ApiError::Internal(e.into()))?),
            None => None,
        };

        let timer_preset = req.timer_preset.clone().unwrap_or_else(|| "30+60".to_string());
        if !validate_timer_preset(&timer_preset) {
            return Err(ApiError::InvalidParams(format!(
                "timer_preset must be one of: 30+60, 40+120, 60+180 (got '{timer_preset}')"
            )));
        }

        let mut tx = state.db.begin().await.map_err(|e| ApiError::Internal(e.into()))?;

        let row: (i64,) = sqlx::query_as(
            "INSERT INTO rooms (game_type, host_uid, status, variant, config, timer_preset) VALUES (?, ?, 'Waiting', ?, ?, ?) RETURNING room_id",
        )
        .bind(&req.game_type)
        .bind(user.uid)
        .bind(&req.variant)
        .bind(&config_str)
        .bind(&timer_preset)
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
    let db = state.db.clone();

    async move {
        let row: Option<(String, i64, String, Option<String>, String)> = sqlx::query_as(
            "SELECT game_type, host_uid, status, variant, timer_preset FROM rooms WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_optional(&db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

        let (game_type, host_uid, status, variant, timer_preset) =
            row.ok_or(ApiError::RoomNotFound)?;
        // Room-page heartbeat: whoever is viewing this room keeps it alive.
        let _ = sqlx::query("UPDATE rooms SET last_active_at = datetime('now') WHERE room_id = ?")
            .bind(room_id)
            .execute(&db)
            .await;
        let entry = state.games.get(&game_type);
        let timer_preset_opt: Option<String> =
            if game_type == "take_your_position" { Some(timer_preset) } else { None };
        let (min_players, max_players) = entry
            .map(|e| (Some(e.min_players), Some(e.max_players)))
            .unwrap_or((None, None));

        let players: Vec<(i64, String, i32)> = sqlx::query_as(
            "SELECT u.id, u.nickname, rp.seat FROM room_players rp JOIN users u ON u.id = rp.uid WHERE rp.room_id = ? ORDER BY rp.seat",
        )
        .bind(room_id)
        .fetch_all(&db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

        // Latest instance id (any active or most-recent finished one).
        let current_instance_id: Option<i64> = sqlx::query_scalar(
            "SELECT instance_id FROM game_instances WHERE room_id = ? \
             ORDER BY instance_id DESC LIMIT 1",
        )
        .bind(room_id)
        .fetch_optional(&db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

        Ok(Json(RoomInfo {
            room_id,
            game_type,
            host_uid,
            status,
            variant,
            players: players.into_iter().map(|(uid, nickname, seat)| PlayerInfo { uid, nickname, seat }).collect(),
            current_instance_id,
            min_players,
            max_players,
            timer_preset: timer_preset_opt,
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

        let row: Option<(String, i64, String, Option<String>, String)> = sqlx::query_as(
            "SELECT game_type, host_uid, status, variant, timer_preset FROM rooms WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
        let (game_type, host_uid, status, variant, timer_preset) = row.ok_or(ApiError::RoomNotFound)?;

        if status != "Waiting" {
            return Err(ApiError::RoomNotWaiting);
        }

        let max_p = state.games.get(&game_type).map(|e| e.max_players).unwrap_or(2);

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM room_players WHERE room_id = ?")
            .bind(room_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        if count.0 as usize >= max_p {
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

        // seat = next free index in [0..max_p)
        let taken: Vec<(i32,)> = sqlx::query_as(
            "SELECT seat FROM room_players WHERE room_id = ? ORDER BY seat",
        )
        .bind(room_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
        let taken: Vec<i32> = taken.into_iter().map(|(s,)| s).collect();
        let seat = (0..max_p as i32).find(|i| !taken.contains(i)).unwrap_or(0);

        sqlx::query("INSERT INTO room_players (room_id, uid, seat) VALUES (?, ?, ?)")
            .bind(room_id)
            .bind(user.uid)
            .bind(seat)
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

        let timer_preset_opt: Option<String> =
            if game_type == "take_your_position" { Some(timer_preset) } else { None };
        Ok(Json(RoomInfo {
            room_id,
            game_type,
            host_uid,
            status,
            variant,
            players: players.into_iter().map(|(uid, nickname, seat)| PlayerInfo { uid, nickname, seat }).collect(),
            current_instance_id: None,
            min_players: Some(2),
            max_players: Some(2),
            timer_preset: timer_preset_opt,
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
        // Pull host_uid up front — we need it to decide whether to promote.
        let host_uid: i64 = sqlx::query_scalar("SELECT host_uid FROM rooms WHERE room_id = ?")
            .bind(room_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

        let affected = sqlx::query("DELETE FROM room_players WHERE room_id = ? AND uid = ?")
            .bind(room_id)
            .bind(user.uid)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

        if affected.rows_affected() == 0 {
            return Err(ApiError::NotInRoom);
        }

        // Auto-close when empty; otherwise, if the host left, promote the
        // earliest-joined remaining player so every room always has a host.
        let remaining: Vec<(i64,)> = sqlx::query_as(
            "SELECT uid FROM room_players WHERE room_id = ? \
             ORDER BY joined_at ASC, seat ASC LIMIT 1",
        )
        .bind(room_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

        if remaining.is_empty() {
            sqlx::query("UPDATE rooms SET status='Destroyed' WHERE room_id = ?")
                .bind(room_id)
                .execute(&state.db)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
        } else if host_uid == user.uid {
            let new_host = remaining[0].0;
            sqlx::query("UPDATE rooms SET host_uid = ? WHERE room_id = ?")
                .bind(new_host)
                .bind(room_id)
                .execute(&state.db)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
            tracing::info!(room_id, old_host = user.uid, new_host, "host promoted");
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

#[derive(sqlx::FromRow)]
struct RoomRowStart {
    game_type: String,
    host_uid: i64,
    status: String,
    #[allow(dead_code)]
    variant: Option<String>,
    config: Option<String>,
    timer_preset: String,
}

pub async fn start(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(room_id): Path<i64>,
) -> Result<Json<StartResp>, ApiError> {
    let span = tracing::info_span!("room.start", room_id, uid = user.uid);

    async move {
        let row: Option<(String, i64, String, Option<String>, Option<String>, String)> = sqlx::query_as(
            "SELECT game_type, host_uid, status, variant, config, timer_preset FROM rooms WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
        let (game_type, host_uid, status, _variant, config_str, timer_preset) =
            row.ok_or(ApiError::RoomNotFound)?;

        if host_uid != user.uid {
            return Err(ApiError::NotHost);
        }

        // Allow start from Waiting or Finished (replay).
        if !matches!(status.as_str(), "Waiting" | "Finished") {
            return Err(ApiError::RoomNotWaiting);
        }

        let entry = state
            .games
            .get(&game_type)
            .ok_or_else(|| ApiError::GameTypeUnsupported(game_type.clone()))?;

        // Fail fast with a clear 503 instead of letting `Command::spawn` blow
        // up later — most "500" reports trace back to PATH/lobby.env mistakes.
        match entry.resolve_binary() {
            crate::games::registry::BinResolve::Ok => {}
            crate::games::registry::BinResolve::NotFound(why) => {
                tracing::error!(
                    game_type = %game_type,
                    bin = %entry.binary.display(),
                    why = %why,
                    "game binary missing"
                );
                return Err(ApiError::GameBinaryNotFound(format!(
                    "{} ({})",
                    entry.binary.display(),
                    why
                )));
            }
            crate::games::registry::BinResolve::NotExecutable => {
                tracing::error!(
                    game_type = %game_type,
                    bin = %entry.binary.display(),
                    "game binary not executable"
                );
                return Err(ApiError::GameBinaryNotFound(format!(
                    "{} (not executable — chmod +x)",
                    entry.binary.display()
                )));
            }
        }

        // Every non-expired session per player. A user can hold several active
        // sessions (re-logins in another tab leave old rows until GC); we pass
        // them all so the game accepts any of them on login/reconnect, not just
        // the "latest".
        let token_rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT rp.uid, s.token
             FROM room_players rp
             JOIN sessions s ON s.user_id = rp.uid
             WHERE rp.room_id = ?
               AND s.expires_at >= datetime('now')
             ORDER BY rp.uid, s.created_at",
        )
        .bind(room_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

        let mut by_uid: std::collections::BTreeMap<i64, Vec<String>> =
            std::collections::BTreeMap::new();
        for (uid, tok) in token_rows {
            by_uid.entry(uid).or_default().push(tok);
        }
        let rows: Vec<protocol::PlayerInit> = by_uid
            .into_iter()
            .map(|(uid, sessions)| protocol::PlayerInit { uid, sessions })
            .collect();

        if rows.len() < entry.min_players {
            return Err(ApiError::NotEnoughPlayers);
        }
        if rows.len() > entry.max_players {
            return Err(ApiError::RoomFull);
        }

        // Mark Starting
        sqlx::query("UPDATE rooms SET status='Starting' WHERE room_id = ?")
            .bind(room_id)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

        let mut init_config = config_str
            .as_deref()
            .map(|s| serde_json::from_str(s).unwrap_or(serde_json::Value::Null))
            .unwrap_or_else(|| serde_json::json!({}));
        // Inject timer preset so the game process knows the per-round/global budget.
        // Stored in config.timer_preset and parsed by the game SDK.
        if let serde_json::Value::Object(ref mut m) = init_config {
            m.insert("timer_preset".to_string(), serde_json::Value::String(timer_preset.clone()));
        }

        let instance_id = state
            .instances
            .spawn(room_id, &game_type, &entry.binary, Some(init_config), rows.clone())
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "spawn failed");
                // Rollback room status on failure (spec §1.1).
                let db = state.db.clone();
                let rid = room_id;
                tokio::spawn(async move {
                    let _ = sqlx::query("UPDATE rooms SET status='Waiting' WHERE room_id = ?")
                        .bind(rid)
                        .execute(&db)
                        .await;
                });
                ApiError::InstanceStartFailed(format!("{e:#}"))
            })?;

        let ws_url = format!("ws://{}:{}/ws/{}", state.public_host, state.public_port, instance_id);

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
    let rows: Vec<(i64, String, i64, String, Option<String>, String)> = sqlx::query_as(
        "SELECT room_id, game_type, host_uid, status, variant, timer_preset FROM rooms WHERE status IN ('Waiting', 'Running') ORDER BY room_id DESC LIMIT 50",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let mut rooms = Vec::with_capacity(rows.len());
    for (room_id, game_type, host_uid, status, variant, timer_preset) in rows {
        let players: Vec<(i64, String, i32)> = sqlx::query_as(
            "SELECT u.id, u.nickname, rp.seat FROM room_players rp JOIN users u ON u.id = rp.uid WHERE rp.room_id = ? ORDER BY rp.seat",
        )
        .bind(room_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
        let entry = state.games.get(&game_type);
        let timer_preset_opt: Option<String> =
            if game_type == "take_your_position" { Some(timer_preset) } else { None };
        let (min_players, max_players) = entry
            .map(|e| (Some(e.min_players), Some(e.max_players)))
            .unwrap_or((None, None));
        rooms.push(RoomInfo {
            room_id,
            game_type,
            host_uid,
            status,
            variant,
            players: players.into_iter().map(|(uid, nickname, seat)| PlayerInfo { uid, nickname, seat }).collect(),
            current_instance_id: None,
            min_players,
            max_players,
            timer_preset: timer_preset_opt,
        });
    }

    Ok(Json(ListResp { rooms }))
}

#[derive(Debug, Serialize)]
pub struct GamesResp {
    pub games: Vec<serde_json::Value>,
}

pub async fn games(State(state): State<SharedState>) -> Json<GamesResp> {
    let games = state.games.list_enabled().into_iter().map(public_view).collect();
    Json(GamesResp { games })
}