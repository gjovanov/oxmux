use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::session::types::{BackendTransport, BrowserTransport, ManagedSession, SessionStatus, TransportConfig};

/// User record from the database.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub password_hash: String,
}

// ── User operations ─────────────────────────────────────────────────────

pub async fn create_user(pool: &SqlitePool, id: &str, username: &str, password_hash: &str) -> Result<()> {
    sqlx::query("INSERT INTO users (id, username, password_hash) VALUES (?, ?, ?)")
        .bind(id)
        .bind(username)
        .bind(password_hash)
        .execute(pool)
        .await
        .context("failed to create user")?;
    Ok(())
}

pub async fn find_user_by_username(pool: &SqlitePool, username: &str) -> Result<Option<UserRow>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    .context("failed to query user")?;
    Ok(row)
}

pub async fn find_user_by_id(pool: &SqlitePool, id: &str) -> Result<Option<UserRow>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("failed to query user")?;
    Ok(row)
}

// ── Session operations ──────────────────────────────────────────────────

/// Persisted session row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionRow {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub transport_config: String,
    pub status: String,
    pub error: Option<String>,
}

pub async fn insert_session(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    name: &str,
    transport_config: &TransportConfig,
) -> Result<()> {
    let config_json = serde_json::to_string(transport_config)?;
    sqlx::query(
        "INSERT INTO sessions (id, user_id, name, transport_config, status) VALUES (?, ?, ?, ?, 'created')",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(&config_json)
    .execute(pool)
    .await
    .context("failed to insert session")?;
    Ok(())
}

pub async fn list_sessions_for_user(pool: &SqlitePool, user_id: &str) -> Result<Vec<SessionRow>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, user_id, name, transport_config, status, error FROM sessions WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("failed to list sessions")?;
    Ok(rows)
}

pub async fn update_session_status(pool: &SqlitePool, id: &str, status: &str, error: Option<&str>) -> Result<()> {
    sqlx::query(
        "UPDATE sessions SET status = ?, error = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(status)
    .bind(error)
    .bind(id)
    .execute(pool)
    .await
    .context("failed to update session status")?;
    Ok(())
}

pub async fn update_session_name(pool: &SqlitePool, id: &str, name: &str) -> Result<()> {
    sqlx::query("UPDATE sessions SET name = ?, updated_at = datetime('now') WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(pool)
        .await
        .context("failed to update session name")?;
    Ok(())
}

pub async fn delete_session(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .context("failed to delete session")?;
    Ok(())
}

/// Convert a SessionRow to a ManagedSession.
pub fn row_to_managed_session(row: &SessionRow) -> Result<ManagedSession> {
    // Try new format first, then fall back to legacy format
    let transport: TransportConfig = serde_json::from_str(&row.transport_config)
        .or_else(|_| -> Result<TransportConfig> {
            // Legacy format: {"type":"ssh","host":"..."} → convert to new format
            let legacy: serde_json::Value = serde_json::from_str(&row.transport_config)?;
            let backend: BackendTransport = serde_json::from_value(legacy)?;
            Ok(TransportConfig {
                browser: BrowserTransport::default(),
                backend,
            })
        })
        .context("failed to deserialize transport_config")?;
    let status = match row.status.as_str() {
        "created" => SessionStatus::Created,
        "connecting" => SessionStatus::Connecting,
        "connected" => SessionStatus::Disconnected, // on reload, connected → disconnected
        "disconnected" => SessionStatus::Disconnected,
        "error" => SessionStatus::Error,
        _ => SessionStatus::Created,
    };
    Ok(ManagedSession {
        id: row.id.clone(),
        name: row.name.clone(),
        transport,
        status,
        error: row.error.clone(),
        tmux_sessions: Vec::new(),
    })
}
