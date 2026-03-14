use anyhow::Result;
use bytes::Bytes;
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::agent::registry::AgentRegistry;
use crate::config::Config;
use crate::session::manager::SessionManager;

pub type PaneId = String;
pub type SessionId = String;

pub struct AppState {
    pub config: Config,
    pub db: SqlitePool,
    pub pane_outputs: Arc<DashMap<PaneId, broadcast::Sender<Bytes>>>,
    pub claude_sessions: DashMap<SessionId, broadcast::Sender<crate::claude::parser::ClaudeEvent>>,
    pub session_manager: SessionManager,
    pub agent_registry: AgentRegistry,
}

impl AppState {
    pub async fn new(config: Config, db: SqlitePool) -> Result<Self> {
        let pane_outputs = Arc::new(DashMap::new());
        Ok(Self {
            config,
            db: db.clone(),
            pane_outputs: pane_outputs.clone(),
            claude_sessions: DashMap::new(),
            session_manager: SessionManager::new(pane_outputs, db),
            agent_registry: AgentRegistry::new(),
        })
    }

    pub fn get_or_create_pane_channel(&self, pane_id: &str) -> broadcast::Sender<Bytes> {
        self.pane_outputs
            .entry(pane_id.to_string())
            .or_insert_with(|| broadcast::channel(4096).0)
            .clone()
    }
}
