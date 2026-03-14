use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::{error, info};

use crate::ws::protocol::{TmuxPaneInfo, TmuxSessionInfo, TmuxWindowInfo};

use super::transport::{Transport, TransportStatus};

/// Local transport — runs tmux directly on the server host.
///
/// Uses `tmux -CC` control mode to get structured state and events,
/// plus direct `tmux send-keys` for input routing.
pub struct LocalTransport {
    session_name: String,
    status: TransportStatus,
    /// Broadcast channels per pane_id for PTY output
    pane_channels: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    /// Handle to the control mode stdin for sending commands
    control_stdin: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    /// Cached tmux state
    tmux_state: Arc<Mutex<Vec<TmuxSessionInfo>>>,
    /// Background task handle
    _task: Option<tokio::task::JoinHandle<()>>,
}

impl LocalTransport {
    pub fn new(session_name: String) -> Self {
        Self {
            session_name,
            status: TransportStatus::Disconnected,
            pane_channels: Arc::new(DashMap::new()),
            control_stdin: Arc::new(Mutex::new(None)),
            tmux_state: Arc::new(Mutex::new(Vec::new())),
            _task: None,
        }
    }

    /// Ensure a tmux session exists with the given name.
    fn ensure_session(name: &str) -> Result<()> {
        let check = Command::new("tmux")
            .args(["has-session", "-t", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        match check {
            Ok(s) if s.success() => {
                info!(session = name, "tmux session already exists");
                Ok(())
            }
            _ => {
                info!(session = name, "creating new tmux session");
                let status = Command::new("tmux")
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

    /// Query tmux for current session/window/pane state.
    fn query_tmux_state(session_name: &str) -> Result<Vec<TmuxSessionInfo>> {
        // List panes with structured format
        let output = Command::new("tmux")
            .args([
                "list-panes",
                "-t", session_name,
                "-a",
                "-F",
                "#{session_id}\t#{session_name}\t#{window_id}\t#{window_name}\t#{window_index}\t#{window_layout}\t#{pane_id}\t#{pane_index}\t#{pane_width}\t#{pane_height}\t#{pane_current_command}\t#{pane_active}",
            ])
            .output()
            .context("failed to run tmux list-panes")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("tmux list-panes failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut sessions: Vec<TmuxSessionInfo> = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 12 {
                continue;
            }

            let sess_id = parts[0].to_string();
            let sess_name = parts[1].to_string();
            let win_id = parts[2].to_string();
            let win_name = parts[3].to_string();
            let win_index: usize = parts[4].parse().unwrap_or(0);
            let win_layout = parts[5].to_string();
            let pane_id = parts[6].to_string();
            let pane_index: usize = parts[7].parse().unwrap_or(0);
            let pane_cols: u16 = parts[8].parse().unwrap_or(80);
            let pane_rows: u16 = parts[9].parse().unwrap_or(24);
            let pane_cmd = parts[10].to_string();
            let pane_active = parts[11] == "1";

            // Detect Claude by process name
            let is_claude = pane_cmd.contains("claude");

            let pane = TmuxPaneInfo {
                id: pane_id,
                index: pane_index,
                cols: pane_cols,
                rows: pane_rows,
                current_command: pane_cmd,
                is_active: pane_active,
                is_claude,
            };

            // Find or create session
            let session = sessions
                .iter_mut()
                .find(|s| s.id == sess_id);
            let session = match session {
                Some(s) => s,
                None => {
                    sessions.push(TmuxSessionInfo {
                        id: sess_id.clone(),
                        name: sess_name,
                        windows: Vec::new(),
                    });
                    sessions.last_mut().unwrap()
                }
            };

            // Find or create window
            let window = session
                .windows
                .iter_mut()
                .find(|w| w.id == win_id);
            let window = match window {
                Some(w) => w,
                None => {
                    session.windows.push(TmuxWindowInfo {
                        id: win_id.clone(),
                        name: win_name,
                        index: win_index,
                        layout: win_layout,
                        panes: Vec::new(),
                    });
                    session.windows.last_mut().unwrap()
                }
            };

            window.panes.push(pane);
        }

        Ok(sessions)
    }

    /// Start capturing output from a pane using `tmux pipe-pane`.
    fn start_pane_capture(
        pane_id: &str,
        tx: broadcast::Sender<Bytes>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let pane_id = pane_id.to_string();
        let tx_clone = tx;

        // Use tmux capture-pane in a loop, or pipe-pane for streaming
        // For now, use a PTY attached to the pane via `tmux pipe-pane`
        let handle = tokio::task::spawn_blocking(move || {
            // Spawn a process that reads from the tmux pane
            let mut child = match Command::new("tmux")
                .args(["pipe-pane", "-t", &pane_id, "-o", "cat"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    error!(pane = %pane_id, error = %e, "failed to start pipe-pane");
                    return;
                }
            };

            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(data) => {
                            let mut bytes = data.into_bytes();
                            bytes.push(b'\n');
                            let _ = tx_clone.send(Bytes::from(bytes));
                        }
                        Err(_) => break,
                    }
                }
            }

            let _ = child.wait();
        });

        Some(handle)
    }
}

#[async_trait]
impl Transport for LocalTransport {
    async fn connect(&mut self) -> Result<()> {
        self.status = TransportStatus::Connecting;

        // 1. Ensure tmux session exists
        let name = self.session_name.clone();
        tokio::task::spawn_blocking(move || Self::ensure_session(&name))
            .await??;

        // 2. Query initial state
        let name = self.session_name.clone();
        let state = tokio::task::spawn_blocking(move || Self::query_tmux_state(&name))
            .await??;

        // 3. Create broadcast channels for each pane
        for session in &state {
            for window in &session.windows {
                for pane in &window.panes {
                    let (tx, _) = broadcast::channel(1024);
                    self.pane_channels.insert(pane.id.clone(), tx);
                }
            }
        }

        *self.tmux_state.lock().await = state;
        self.status = TransportStatus::Connected;

        info!(session = %self.session_name, "local transport connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.status = TransportStatus::Disconnected;
        self.pane_channels.clear();
        info!(session = %self.session_name, "local transport disconnected");
        Ok(())
    }

    async fn list_tmux_sessions(&self) -> Result<Vec<TmuxSessionInfo>> {
        let name = self.session_name.clone();
        let state = tokio::task::spawn_blocking(move || Self::query_tmux_state(&name))
            .await??;
        *self.tmux_state.lock().await = state.clone();
        Ok(state)
    }

    async fn send_input(&self, pane_id: &str, data: &[u8]) -> Result<()> {
        let text = String::from_utf8_lossy(data).to_string();
        let pane_id = pane_id.to_string();

        tokio::task::spawn_blocking(move || {
            // Use tmux send-keys with literal flag
            let status = Command::new("tmux")
                .args(["send-keys", "-t", &pane_id, "-l", &text])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();

            match status {
                Ok(s) if s.success() => Ok(()),
                Ok(s) => anyhow::bail!("tmux send-keys exited with {}", s),
                Err(e) => Err(e).context("failed to run tmux send-keys"),
            }
        })
        .await?
    }

    async fn resize_pane(&self, pane_id: &str, cols: u16, rows: u16) -> Result<()> {
        let pane_id = pane_id.to_string();
        tokio::task::spawn_blocking(move || {
            Command::new("tmux")
                .args([
                    "resize-pane",
                    "-t", &pane_id,
                    "-x", &cols.to_string(),
                    "-y", &rows.to_string(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .context("failed to run tmux resize-pane")?;
            Ok(())
        })
        .await?
    }

    async fn run_tmux_command(&self, cmd: &str) -> Result<String> {
        let cmd = cmd.to_string();
        tokio::task::spawn_blocking(move || {
            let output = Command::new("sh")
                .args(["-c", &format!("tmux {}", cmd)])
                .output()
                .context("failed to run tmux command")?;
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        })
        .await?
    }

    fn subscribe_output(&self, pane_id: &str) -> Option<broadcast::Receiver<Bytes>> {
        self.pane_channels
            .get(pane_id)
            .map(|tx| tx.subscribe())
    }

    fn status(&self) -> TransportStatus {
        self.status
    }
}
