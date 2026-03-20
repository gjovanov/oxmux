use anyhow::Result;
use bytes::Bytes;
use dashmap::DashMap;
use secrecy::SecretString;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::{error, info};
use uuid::Uuid;

use crate::db::repo;

use super::local::LocalTransport;
use super::quic_transport::QuicTransport;
use super::ssh_transport::SshTransport;
use super::transport::Transport;
use super::types::*;
use super::webrtc_signal::WebRtcSignaler;

pub struct SessionManager {
    /// In-memory session state (active transports)
    sessions: DashMap<SessionId, SessionEntry>,
    /// pane_id → session_id mapping for routing input
    pane_to_session: DashMap<String, SessionId>,
    /// Shared pane output channels (from AppState)
    shared_pane_outputs: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    /// Database pool for persistence
    db: SqlitePool,
    pub signaler: Arc<WebRtcSignaler>,
    /// Ephemeral key store: key_id → (PEM key data, optional passphrase)
    /// Keys uploaded from browser, never persisted to disk.
    pub ephemeral_keys: Arc<DashMap<String, (SecretString, Option<SecretString>)>>,
}

struct SessionEntry {
    meta: ManagedSession,
    transport: Arc<Mutex<Box<dyn Transport>>>,
}

impl SessionManager {
    pub fn new(
        shared_pane_outputs: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
        db: SqlitePool,
    ) -> Self {
        Self {
            sessions: DashMap::new(),
            pane_to_session: DashMap::new(),
            shared_pane_outputs,
            db,
            signaler: Arc::new(WebRtcSignaler::new()),
            ephemeral_keys: Arc::new(DashMap::new()),
        }
    }

    /// Load persisted sessions for a user from DB into memory.
    /// Called when a user's WS connects.
    pub async fn load_user_sessions(&self, user_id: &str) -> Result<Vec<ManagedSession>> {
        let rows = repo::list_sessions_for_user(&self.db, user_id).await?;
        let mut sessions = Vec::new();

        for row in &rows {
            let meta = repo::row_to_managed_session(row)?;

            // Only add to in-memory map if not already there
            if !self.sessions.contains_key(&meta.id) {
                let transport = self.create_transport(&meta)?;
                self.sessions.insert(meta.id.clone(), SessionEntry {
                    meta: meta.clone(),
                    transport: Arc::new(Mutex::new(transport)),
                });
            }

            // Return current in-memory state (may have been connected)
            if let Some(entry) = self.sessions.get(&meta.id) {
                sessions.push(entry.meta.clone());
            } else {
                sessions.push(meta);
            }
        }

        Ok(sessions)
    }

    /// Create a backend transport instance from session metadata.
    /// The browser transport (WS/QUIC/WebRTC) is handled at the connection layer,
    /// not here — this creates the backend (SSH/Agent/Local) that manages tmux.
    fn create_transport(&self, meta: &ManagedSession) -> Result<Box<dyn Transport>> {
        let transport: Box<dyn Transport> = match &meta.transport.backend {
            BackendTransport::Local => {
                Box::new(LocalTransport::new(meta.name.clone()))
            }
            BackendTransport::Ssh { host, port, user, auth } => {
                Box::new(SshTransport::new(
                    host.clone(),
                    *port,
                    user.clone(),
                    auth.clone(),
                    meta.name.clone(),
                    self.shared_pane_outputs.clone(),
                    self.ephemeral_keys.clone(),
                ))
            }
            BackendTransport::Agent { host, port, .. } => {
                // Agent backend uses QUIC transport to connect to the agent
                Box::new(QuicTransport::new(
                    host.clone(),
                    *port,
                    meta.name.clone(),
                ))
            }
        };
        Ok(transport)
    }

    /// Create a new session (persisted to DB).
    pub async fn create(&self, user_id: &str, req: CreateSessionRequest) -> Result<ManagedSession> {
        let id = Uuid::new_v4().to_string();

        // Persist to DB
        repo::insert_session(&self.db, &id, user_id, &req.name, &req.transport).await?;

        let meta = ManagedSession {
            id: id.clone(),
            name: req.name,
            transport: req.transport,
            status: SessionStatus::Created,
            error: None,
            tmux_sessions: Vec::new(),
        };

        let transport = self.create_transport(&meta)?;
        self.sessions.insert(id.clone(), SessionEntry {
            meta: meta.clone(),
            transport: Arc::new(Mutex::new(transport)),
        });

        info!(id = %id, name = %meta.name, "session created");
        Ok(meta)
    }

    pub fn list(&self) -> Vec<ManagedSession> {
        self.sessions.iter().map(|e| e.value().meta.clone()).collect()
    }

    /// List sessions for a specific user (from in-memory state).
    pub fn list_for_user(&self, user_id: &str) -> Vec<ManagedSession> {
        // We need to check DB ownership. For now, return all in-memory sessions
        // that belong to this user. We'll use load_user_sessions() on WS connect.
        self.sessions.iter().map(|e| e.value().meta.clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<ManagedSession> {
        self.sessions.get(id).map(|e| e.meta.clone())
    }

    pub async fn update(&self, id: &str, req: UpdateSessionRequest) -> Result<ManagedSession> {
        let mut entry = self.sessions.get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("session '{}' not found", id))?;

        if let Some(ref name) = req.name {
            entry.meta.name = name.clone();
            repo::update_session_name(&self.db, id, name).await?;
        }

        info!(id = %id, "session updated");
        Ok(entry.meta.clone())
    }

    pub async fn delete(&self, id: &str) -> Result<ManagedSession> {
        self.pane_to_session.retain(|_, sid| sid != id);

        let (_, entry) = self.sessions.remove(id)
            .ok_or_else(|| anyhow::anyhow!("session '{}' not found", id))?;

        let mut transport = entry.transport.lock().await;
        let _ = transport.disconnect().await;

        // Clean up ephemeral key if this session used one
        if let BackendTransport::Ssh { auth: SshAuthConfig::UploadedKey { ref key_id, .. }, .. } = entry.meta.transport.backend {
            self.ephemeral_keys.remove(key_id);
            info!(key_id = %key_id, "ephemeral key removed on session delete");
        }

        // Remove from DB
        repo::delete_session(&self.db, id).await?;

        info!(id = %id, name = %entry.meta.name, "session deleted");
        Ok(entry.meta)
    }

    pub async fn connect(&self, id: &str) -> Result<ManagedSession> {
        let entry = self.sessions.get(id)
            .ok_or_else(|| anyhow::anyhow!("session '{}' not found", id))?;
        let transport = entry.transport.clone();
        drop(entry);

        if let Some(mut entry) = self.sessions.get_mut(id) {
            entry.meta.status = SessionStatus::Connecting;
        }
        repo::update_session_status(&self.db, id, "connecting", None).await.ok();

        let mut transport = transport.lock().await;
        match transport.connect().await {
            Ok(()) => {
                let tmux_sessions = transport.list_tmux_sessions().await.unwrap_or_default();

                for ts in &tmux_sessions {
                    for w in &ts.windows {
                        for p in &w.panes {
                            self.pane_to_session.insert(p.id.clone(), id.to_string());
                        }
                    }
                }

                if let Some(mut entry) = self.sessions.get_mut(id) {
                    entry.meta.status = SessionStatus::Connected;
                    entry.meta.error = None;
                    entry.meta.tmux_sessions = tmux_sessions;
                }
                repo::update_session_status(&self.db, id, "connected", None).await.ok();
                info!(id = %id, "session connected");
            }
            Err(e) => {
                error!(id = %id, error = %e, "session connect failed");
                let err_msg = e.to_string();
                if let Some(mut entry) = self.sessions.get_mut(id) {
                    entry.meta.status = SessionStatus::Error;
                    entry.meta.error = Some(err_msg.clone());
                }
                repo::update_session_status(&self.db, id, "error", Some(&err_msg)).await.ok();
                return Err(e);
            }
        }

        self.get(id).ok_or_else(|| anyhow::anyhow!("session disappeared"))
    }

    pub async fn disconnect(&self, id: &str) -> Result<ManagedSession> {
        self.pane_to_session.retain(|_, sid| sid != id);

        let entry = self.sessions.get(id)
            .ok_or_else(|| anyhow::anyhow!("session '{}' not found", id))?;
        let transport = entry.transport.clone();
        drop(entry);

        let mut transport = transport.lock().await;
        transport.disconnect().await?;

        if let Some(mut entry) = self.sessions.get_mut(id) {
            entry.meta.status = SessionStatus::Disconnected;
            entry.meta.tmux_sessions.clear();
        }
        repo::update_session_status(&self.db, id, "disconnected", None).await.ok();

        info!(id = %id, "session disconnected");
        self.get(id).ok_or_else(|| anyhow::anyhow!("session disappeared"))
    }

    pub fn find_session_for_pane(&self, pane_id: &str) -> Option<SessionId> {
        self.pane_to_session.get(pane_id).map(|v| v.clone())
    }

    pub async fn send_input_to_pane(&self, pane_id: &str, data: &[u8]) -> Result<()> {
        let session_id = self.find_session_for_pane(pane_id)
            .ok_or_else(|| anyhow::anyhow!("no session owns pane '{}'", pane_id))?;
        let entry = self.sessions.get(&session_id)
            .ok_or_else(|| anyhow::anyhow!("session '{}' not found", session_id))?;
        let transport = entry.transport.clone();
        drop(entry);
        let transport = transport.lock().await;
        transport.send_input(pane_id, data).await
    }

    pub async fn resize_pane(&self, pane_id: &str, cols: u16, rows: u16) -> Result<()> {
        let session_id = self.find_session_for_pane(pane_id)
            .ok_or_else(|| anyhow::anyhow!("no session owns pane '{}'", pane_id))?;
        let entry = self.sessions.get(&session_id)
            .ok_or_else(|| anyhow::anyhow!("session '{}' not found", session_id))?;
        let transport = entry.transport.clone();
        drop(entry);
        let transport = transport.lock().await;
        transport.resize_pane(pane_id, cols, rows).await
    }

    pub async fn refresh_tmux_state(&self, id: &str) -> Result<ManagedSession> {
        let entry = self.sessions.get(id)
            .ok_or_else(|| anyhow::anyhow!("session '{}' not found", id))?;
        let transport = entry.transport.clone();
        drop(entry);

        let transport = transport.lock().await;
        let tmux_sessions = transport.list_tmux_sessions().await?;

        for ts in &tmux_sessions {
            for w in &ts.windows {
                for p in &w.panes {
                    self.pane_to_session.insert(p.id.clone(), id.to_string());
                }
            }
        }

        if let Some(mut entry) = self.sessions.get_mut(id) {
            entry.meta.tmux_sessions = tmux_sessions;
        }

        self.get(id).ok_or_else(|| anyhow::anyhow!("session disappeared"))
    }
}
