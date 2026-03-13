use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::config::Config;

pub type PaneId = String;
pub type SessionId = String;

/// Global application state — cloned into every Axum handler via Arc
pub struct AppState {
    pub config: Config,
    /// pane_id → broadcast channel for PTY output
    pub pane_outputs: DashMap<PaneId, broadcast::Sender<bytes::Bytes>>,
    /// claude session_id → broadcast channel for structured events
    pub claude_sessions: DashMap<SessionId, broadcast::Sender<crate::claude::parser::ClaudeEvent>>,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        Ok(Self {
            config,
            pane_outputs: DashMap::new(),
            claude_sessions: DashMap::new(),
        })
    }

    pub fn get_or_create_pane_channel(
        &self,
        pane_id: &str,
    ) -> broadcast::Sender<bytes::Bytes> {
        self.pane_outputs
            .entry(pane_id.to_string())
            .or_insert_with(|| broadcast::channel(1024).0)
            .clone()
    }
}
