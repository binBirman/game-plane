use anyhow::Result;
use sqlx::SqlitePool;

const SCHEMA: &str = include_str!("schema.sql");

pub async fn run(pool: &SqlitePool) -> Result<()> {
    sqlx::query(SCHEMA).execute(pool).await?;

    // Idempotent additive migrations for pre-V1.1 schemas.
    let _ = sqlx::query("ALTER TABLE rooms ADD COLUMN variant TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE rooms ADD COLUMN config TEXT")
        .execute(pool)
        .await;

    // V1.2 — TYP support + cleanup task.
    let _ = sqlx::query("ALTER TABLE rooms ADD COLUMN timer_preset TEXT NOT NULL DEFAULT '30+60'")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE room_players ADD COLUMN online INTEGER NOT NULL DEFAULT 1")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE game_instances ADD COLUMN last_action_at TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE rooms ADD COLUMN last_active_at TEXT")
        .execute(pool)
        .await;

    // Indexes on the just-added columns (must run after ALTER).
    let _ = sqlx::query("CREATE INDEX IF NOT EXISTS idx_room_players_online ON room_players(online)")
        .execute(pool)
        .await;
    let _ = sqlx::query("CREATE INDEX IF NOT EXISTS idx_instances_action ON game_instances(last_action_at)")
        .execute(pool)
        .await;

    Ok(())
}