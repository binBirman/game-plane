//! Stale-room cleanup task.
//!
//! Releases rooms that look abandoned:
//!   - A `Running`/`Starting` room whose game instance saw no player action
//!     for `LOBBY_CLEANUP_ACTION_SECS` (default 300s) → stop the instance,
//!     mark it abnormal, destroy the room.
//!   - A `Waiting` room whose room-page heartbeat (`last_active_at`) is older
//!     than `LOBBY_CLEANUP_WAITING_SECS` (default 300s) → no one is viewing it
//!     anymore, destroy it.
//!
//! Runs every `LOBBY_CLEANUP_INTERVAL_SECS` (default 10s).

use std::sync::Arc;
use std::time::Duration;

use crate::state::AppState;

const DEFAULT_ACTION_SECS: i64 = 300;
const DEFAULT_WAITING_SECS: i64 = 300;

pub fn spawn_cleanup_task(state: Arc<AppState>) {
    let interval_secs: u64 = std::env::var("LOBBY_CLEANUP_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let action_secs: i64 = std::env::var("LOBBY_CLEANUP_ACTION_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_ACTION_SECS);
    let waiting_secs: i64 = std::env::var("LOBBY_CLEANUP_WAITING_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_WAITING_SECS);

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(interval_secs));
        loop {
            tick.tick().await;
            if let Err(e) = sweep_once(&state, action_secs, waiting_secs).await {
                tracing::warn!(error = %e, "stale-room sweep failed");
            }
        }
    });
}

async fn sweep_once(state: &AppState, action_secs: i64, waiting_secs: i64) -> anyhow::Result<()> {
    sweep_running_stuck(state, action_secs).await?;
    sweep_waiting_stale(state, waiting_secs).await?;
    Ok(())
}

/// Destroy a room + its live instances.
async fn destroy_room(state: &AppState, room_id: i64) -> anyhow::Result<()> {
    // Stop any live game instances for this room.
    let inst_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT instance_id FROM game_instances WHERE room_id = ? AND status IN ('starting','ready','running')",
    )
    .bind(room_id)
    .fetch_all(&state.db)
    .await?;
    for id in inst_ids {
        if let Some(port) = state.instances.lookup(id).await.map(|(p, _)| p) {
            // Best-effort: ask the game to stop gracefully.
            let _ = state.instances.stop(id, "stale_room_cleanup").await;
            let _ = port; // unused otherwise
        }
        let _ = sqlx::query(
            "UPDATE game_instances SET status='abnormal', end_time=datetime('now') WHERE instance_id=?",
        )
        .bind(id)
        .execute(&state.db)
        .await;
    }

    // Remove players + destroy the room.
    let _ = sqlx::query("DELETE FROM room_players WHERE room_id=?")
        .bind(room_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("UPDATE rooms SET status='Destroyed' WHERE room_id=?")
        .bind(room_id)
        .execute(&state.db)
        .await;
    tracing::warn!(room_id, "stale room destroyed");
    Ok(())
}

/// A Running/Starting room whose game has had no player action for
/// `action_secs` is considered abandoned.
async fn sweep_running_stuck(state: &AppState, action_secs: i64) -> anyhow::Result<()> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT r.room_id, gi.instance_id
         FROM rooms r
         JOIN game_instances gi ON gi.room_id = r.room_id
         WHERE r.status IN ('Running','Starting')
           AND gi.status IN ('starting','ready','running')
           AND (
             gi.last_action_at IS NULL
             OR datetime(gi.last_action_at, ?) < datetime('now')
           )
         LIMIT 20",
    )
    // sqlite doesn't allow parameterizing the modifier string, so we inline.
    .bind(format!("+{action_secs} seconds"))
    .fetch_all(&state.db)
    .await?;
    for (room_id, _instance_id) in rows {
        destroy_room(state, room_id).await?;
    }
    Ok(())
}

/// A Waiting room with no viewer for `waiting_secs` is considered abandoned.
async fn sweep_waiting_stale(state: &AppState, waiting_secs: i64) -> anyhow::Result<()> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT room_id FROM rooms
         WHERE status = 'Waiting'
           AND (
             last_active_at IS NULL
             OR datetime(last_active_at, ?) < datetime('now')
           )
         LIMIT 20",
    )
    .bind(format!("+{waiting_secs} seconds"))
    .fetch_all(&state.db)
    .await?;
    for (room_id,) in rows {
        // Only destroy if nobody is actually on the room page right now.
        // last_active_at is updated on every GET /api/rooms/:id, so a stale
        // value genuinely means no one is looking.
        destroy_room(state, room_id).await?;
    }
    Ok(())
}
