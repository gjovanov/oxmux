//! Local tmux management for the agent.

use anyhow::{Context, Result};
use bytes::Bytes;
use dashmap::DashMap;
use std::process::{Command, Stdio};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PaneInfo {
    pub session_id: String,
    pub session_name: String,
    pub window_id: String,
    pub window_name: String,
    pub window_index: usize,
    pub layout: String,
    pub pane_id: String,
    pub pane_index: usize,
    pub cols: u16,
    pub rows: u16,
    pub current_command: String,
    pub is_active: bool,
    pub is_claude: bool,
}

pub struct TmuxManager {
    pub pane_outputs: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
}

impl TmuxManager {
    pub fn new() -> Self {
        Self {
            pane_outputs: Arc::new(DashMap::new()),
        }
    }

    fn tmux_cmd() -> Command {
        let mut cmd = Command::new("tmux");
        if let Some(sock) = find_socket() {
            cmd.arg("-S").arg(sock);
        }
        cmd
    }

    pub fn ensure_session(&self, name: &str) -> Result<()> {
        let status = Self::tmux_cmd()
            .args(["has-session", "-t", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        match status {
            Ok(s) if s.success() => {
                info!(session = name, "tmux session already exists");
                Ok(())
            }
            _ => {
                info!(session = name, "creating new tmux session");
                let status = Self::tmux_cmd()
                    .args(["new-session", "-d", "-s", name, "-x", "80", "-y", "24"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .status()
                    .context("failed to spawn tmux new-session")?;
                if !status.success() {
                    anyhow::bail!("tmux new-session exited with {}", status);
                }
                Ok(())
            }
        }
    }

    pub fn list_panes(&self, session_name: &str) -> Result<Vec<PaneInfo>> {
        let output = Self::tmux_cmd()
            .args([
                "list-panes", "-t", session_name, "-s",
                "-F",
                "#{session_id}|||#{session_name}|||#{window_id}|||#{window_name}|||#{window_index}|||#{window_layout}|||#{pane_id}|||#{pane_index}|||#{pane_width}|||#{pane_height}|||#{pane_current_command}|||#{pane_active}",
            ])
            .output()
            .context("failed to run tmux list-panes")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("tmux list-panes failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut panes = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split("|||").collect();
            if parts.len() < 12 { continue; }
            let cmd = parts[10].to_string();
            panes.push(PaneInfo {
                session_id: parts[0].to_string(),
                session_name: parts[1].to_string(),
                window_id: parts[2].to_string(),
                window_name: parts[3].to_string(),
                window_index: parts[4].parse().unwrap_or(0),
                layout: parts[5].to_string(),
                pane_id: parts[6].to_string(),
                pane_index: parts[7].parse().unwrap_or(0),
                cols: parts[8].parse().unwrap_or(80),
                rows: parts[9].parse().unwrap_or(24),
                is_claude: cmd.contains("claude"),
                current_command: cmd,
                is_active: parts[11] == "1",
            });
        }
        Ok(panes)
    }

    pub fn send_input(&self, pane_id: &str, data: &[u8]) -> Result<()> {
        let hex: String = data.iter().map(|b| format!("{:02x} ", b)).collect();
        Self::tmux_cmd()
            .args(["send-keys", "-t", pane_id, "-H", hex.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to run tmux send-keys")?;
        Ok(())
    }

    pub fn resize_pane(&self, pane_id: &str, cols: u16, rows: u16) -> Result<()> {
        Self::tmux_cmd()
            .args(["resize-pane", "-t", pane_id, "-x", &cols.to_string(), "-y", &rows.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to run tmux resize-pane")?;
        Ok(())
    }

    pub fn get_or_create_pane_channel(&self, pane_id: &str) -> broadcast::Sender<Bytes> {
        self.pane_outputs
            .entry(pane_id.to_string())
            .or_insert_with(|| broadcast::channel(4096).0)
            .clone()
    }
}

fn find_socket() -> Option<String> {
    if let Ok(entries) = std::fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("tmux-") {
                let sock = format!("/tmp/{}/default", name);
                if std::path::Path::new(&sock).exists() {
                    return Some(sock);
                }
            }
        }
    }
    None
}
