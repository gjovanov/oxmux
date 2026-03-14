pub mod repo;

use anyhow::Result;
use sqlx::SqlitePool;
use tracing::info;

/// Initialize the database: create tables if they don't exist.
pub async fn init(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY NOT NULL,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY NOT NULL,
            user_id TEXT NOT NULL REFERENCES users(id),
            name TEXT NOT NULL,
            transport_config TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'created',
            error TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id)")
        .execute(pool)
        .await?;

    // Reset all sessions to disconnected on server startup
    // (transports are in-memory and lost on restart)
    sqlx::query("UPDATE sessions SET status = 'disconnected' WHERE status IN ('connected', 'connecting')")
        .execute(pool)
        .await?;

    info!("database initialized");
    Ok(())
}
