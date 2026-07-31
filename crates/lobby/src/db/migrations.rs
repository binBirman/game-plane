use anyhow::Result;
use sqlx::SqlitePool;

const SCHEMA: &str = include_str!("schema.sql");

pub async fn run(pool: &SqlitePool) -> Result<()> {
    sqlx::query(SCHEMA).execute(pool).await?;
    Ok(())
}