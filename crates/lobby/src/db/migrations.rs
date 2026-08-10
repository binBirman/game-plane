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

    Ok(())
}