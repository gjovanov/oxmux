use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use russh::client;
use russh::ChannelMsg;
use russh_keys::key;
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{debug, error, info, warn};

use crate::tmux::control::{ControlModeParser, TmuxEvent};
use crate::ws::protocol::{TmuxPaneInfo, TmuxSessionInfo, TmuxWindowInfo};

use super::transport::{Transport, TransportStatus};
use super::types::SshAuthConfig;

/// SSH transport — connects to a remote host via russh,
/// attaches to tmux in control mode (`-CC`), and streams PTY I/O.
pub struct SshTransport {
    host: String,
    port: u16,
    user: String,
    auth: SshAuthConfig,
    session_name: String,
    status: TransportStatus,
    /// SSH handle for sending data to channels
    ssh_handle: Arc<Mutex<Option<client::Handle<SshHandler>>>>,
    /// Channel ID of the control mode session (for writing commands)
    control_channel_id: Arc<Mutex<Option<russh::ChannelId>>>,
    /// Shared pane output map (from AppState) — transport writes directly here
    shared_pane_outputs: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    /// Background control mode reader task
    reader_task: Option<tokio::task::JoinHandle<()>>,
    /// Cached tmux state
    tmux_state: Arc<Mutex<Vec<TmuxSessionInfo>>>,
    /// Ephemeral key store for uploaded keys (shared with SessionManager)
    ephemeral_keys: Arc<DashMap<String, (SecretString, Option<SecretString>)>>,
}

struct SshHandler;

#[async_trait]
impl client::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

impl SshTransport {
    pub fn new(
        host: String,
        port: u16,
        user: String,
        auth: SshAuthConfig,
        session_name: String,
        shared_pane_outputs: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
        ephemeral_keys: Arc<DashMap<String, (SecretString, Option<SecretString>)>>,
    ) -> Self {
        Self {
            host,
            port,
            user,
            auth,
            session_name,
            status: TransportStatus::Disconnected,
            ssh_handle: Arc::new(Mutex::new(None)),
            control_channel_id: Arc::new(Mutex::new(None)),
            shared_pane_outputs,
            reader_task: None,
            tmux_state: Arc::new(Mutex::new(Vec::new())),
            ephemeral_keys,
        }
    }

    /// Execute a one-shot command over SSH and return stdout.
    async fn exec_command(handle: &client::Handle<SshHandler>, cmd: &str) -> Result<String> {
        let mut channel = handle
            .channel_open_session()
            .await
            .context("failed to open SSH channel")?;

        channel
            .exec(true, cmd.as_bytes())
            .await
            .context("failed to exec SSH command")?;

        let mut stdout = Vec::new();
        let mut got_eof = false;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => {
                    stdout.extend_from_slice(data);
                }
                ChannelMsg::ExtendedData { ref data, .. } => {
                    // stderr — log but don't add to stdout
                    debug!(stderr = %String::from_utf8_lossy(data), "SSH exec stderr");
                }
                ChannelMsg::Eof => {
                    got_eof = true;
                }
                ChannelMsg::Close => break,
                ChannelMsg::ExitStatus { exit_status } => {
                    debug!(exit_status, "SSH exec exit status");
                    if got_eof { break; }
                }
                _ => {}
            }
        }

        let output = String::from_utf8_lossy(&stdout).to_string();
        debug!(cmd = %cmd, output_len = output.len(), "SSH exec_command result");
        Ok(output)
    }

    /// Parse `tmux list-panes` output into structured state.
    fn parse_pane_listing(output: &str) -> Vec<TmuxSessionInfo> {
        let mut sessions: Vec<TmuxSessionInfo> = Vec::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split("|||").collect();
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

            let session = sessions.iter_mut().find(|s| s.id == sess_id);
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

            let window = session.windows.iter_mut().find(|w| w.id == win_id);
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

        sessions
    }

    /// Write a command to the control mode channel.
    async fn write_control_command(&self, cmd: &str) -> Result<()> {
        let handle_lock = self.ssh_handle.lock().await;
        let handle = handle_lock
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("SSH not connected"))?;

        let channel_id_lock = self.control_channel_id.lock().await;
        let channel_id = channel_id_lock
            .ok_or_else(|| anyhow::anyhow!("control mode channel not open"))?;

        let mut data = cmd.as_bytes().to_vec();
        if !cmd.ends_with('\n') {
            data.push(b'\n');
        }

        handle
            .data(channel_id, russh::CryptoVec::from_slice(&data))
            .await
            .map_err(|_| anyhow::anyhow!("failed to write to control mode channel"))?;

        Ok(())
    }
}

#[async_trait]
impl Transport for SshTransport {
    async fn connect(&mut self) -> Result<()> {
        self.status = TransportStatus::Connecting;

        let config = Arc::new(client::Config::default());
        let handler = SshHandler;

        let addr = format!("{}:{}", self.host, self.port);
        info!(addr = %addr, user = %self.user, "connecting via SSH");

        let mut handle = client::connect(config, &addr, handler)
            .await
            .context(format!("SSH connect to {} failed", addr))?;

        // Authenticate
        match &self.auth {
            SshAuthConfig::Agent => {
                warn!("SSH agent auth not fully implemented, trying none auth");
                let _accepted = handle
                    .authenticate_none(&self.user)
                    .await
                    .context("SSH none auth failed")?;
            }
            SshAuthConfig::Password { password } => {
                let accepted = handle
                    .authenticate_password(&self.user, password.expose_secret())
                    .await
                    .context("SSH password auth failed")?;
                if !accepted {
                    anyhow::bail!("SSH password rejected");
                }
            }
            SshAuthConfig::PrivateKey { path, passphrase } => {
                let expanded_path = if path.starts_with("~/") {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/oxmux".to_string());
                    format!("{}/{}", home, &path[2..])
                } else {
                    path.clone()
                };
                let key_data = tokio::fs::read_to_string(&expanded_path)
                    .await
                    .context(format!("failed to read SSH key at {}", expanded_path))?;

                let passphrase_str = passphrase.as_ref().map(|p| p.expose_secret().to_string());
                info!(
                    path = %expanded_path,
                    has_passphrase = passphrase.is_some(),
                    contains_des3 = key_data.contains("DES-EDE3-CBC"),
                    "decoding SSH private key"
                );

                // russh_keys doesn't support DES-EDE3-CBC — convert via openssl
                let key_data = if key_data.contains("DES-EDE3-CBC") || key_data.contains("DES-CBC") {
                    let pass = passphrase_str.as_deref().unwrap_or("");
                    info!("converting encrypted key via openssl");
                    let output = tokio::process::Command::new("openssl")
                        .args(["rsa", "-in", &expanded_path, "-passin", &format!("pass:{}", pass), "-traditional"])
                        .output()
                        .await
                        .context("failed to run openssl to convert key")?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        anyhow::bail!("openssl key conversion failed: {}", stderr.trim());
                    }
                    String::from_utf8(output.stdout)
                        .context("openssl output is not valid UTF-8")?
                } else {
                    key_data
                };

                let decode_pass = if key_data.starts_with("-----BEGIN PRIVATE KEY-----") {
                    None // PKCS#8 unencrypted
                } else {
                    passphrase_str.as_deref()
                };
                let key_pair = russh_keys::decode_secret_key(&key_data, decode_pass)
                    .context(format!("failed to decode SSH key at '{}'", expanded_path))?;
                let accepted = handle
                    .authenticate_publickey(&self.user, Arc::new(key_pair))
                    .await
                    .context("SSH pubkey auth failed")?;
                if !accepted {
                    anyhow::bail!("SSH public key rejected");
                }
            }
            SshAuthConfig::UploadedKey { key_id, passphrase } => {
                // Clone data out of DashMap guard to avoid holding it across await
                let (key_str, pp) = {
                    let entry = self.ephemeral_keys
                        .get(key_id)
                        .ok_or_else(|| anyhow::anyhow!(
                            "uploaded key '{}' not found in memory (server may have restarted — re-upload the key)", key_id
                        ))?;
                    let (k, sp) = entry.value();
                    (
                        k.expose_secret().to_string(),
                        sp.as_ref().or(passphrase.as_ref()).map(|p| p.expose_secret().to_string()),
                    )
                }; // guard dropped here

                let decode_pass = if key_str.starts_with("-----BEGIN PRIVATE KEY-----") {
                    None
                } else {
                    pp.as_deref()
                };
                let key_pair = russh_keys::decode_secret_key(&key_str, decode_pass)
                    .context(format!("failed to decode uploaded SSH key '{}'", key_id))?;
                let accepted = handle
                    .authenticate_publickey(&self.user, Arc::new(key_pair))
                    .await
                    .context("SSH pubkey auth (uploaded key) failed")?;
                if !accepted {
                    anyhow::bail!("SSH uploaded key rejected");
                }
            }
        }

        info!(addr = %addr, "SSH authenticated, setting up tmux");

        // 1. Ensure tmux session exists + set window-size manual
        // window-size=manual disables auto-sizing from attached clients,
        // allowing resize-pane to work independently of control mode PTY size
        let create_cmd = format!(
            "tmux has-session -t {name} 2>/dev/null || tmux new-session -d -s {name} -x 80 -y 24; \
             tmux set-option -g window-size manual 2>/dev/null; \
             tmux set-option -g aggressive-resize on 2>/dev/null",
            name = self.session_name
        );
        Self::exec_command(&handle, &create_cmd).await?;

        // 2. Query initial state
        // Use ||| as separator since \t is not interpreted over SSH exec
        let list_cmd = format!(
            "tmux list-panes -t {} -s -F '#{{session_id}}|||#{{session_name}}|||#{{window_id}}|||#{{window_name}}|||#{{window_index}}|||#{{window_layout}}|||#{{pane_id}}|||#{{pane_index}}|||#{{pane_width}}|||#{{pane_height}}|||#{{pane_current_command}}|||#{{pane_active}}'",
            self.session_name
        );
        let output = Self::exec_command(&handle, &list_cmd).await?;
        let state = Self::parse_pane_listing(&output);
        *self.tmux_state.lock().await = state.clone();

        // 3. Open persistent control mode channel for live streaming
        let mut ctrl_channel = handle
            .channel_open_session()
            .await
            .context("failed to open control mode channel")?;

        let channel_id = ctrl_channel.id();
        *self.control_channel_id.lock().await = Some(channel_id);

        // Request PTY at standard 80x24 for control mode.
        // With window-size=manual, the PTY size doesn't constrain panes —
        // resize-window commands set the actual size independently.
        ctrl_channel
            .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
            .await
            .context("failed to request PTY")?;

        // Start tmux in control mode
        let attach_cmd = format!("tmux -CC attach -t {}", self.session_name);
        ctrl_channel
            .exec(true, attach_cmd.as_bytes())
            .await
            .context("failed to exec tmux -CC")?;

        info!(addr = %addr, session = %self.session_name, "control mode channel opened");

        // 4. Abort any existing reader task before spawning a new one
        // (prevents duplicate output from multiple control mode attachments)
        if let Some(old_task) = self.reader_task.take() {
            info!(session = %self.session_name, "aborting old reader task");
            old_task.abort();
        }
        // Also close old SSH handle if any
        if let Some(old_handle) = self.ssh_handle.lock().await.take() {
            let _ = old_handle.disconnect(russh::Disconnect::ByApplication, "reconnecting", "").await;
        }

        // Spawn background task: read control mode output, parse, broadcast
        let shared_outputs = self.shared_pane_outputs.clone();
        let tmux_state = self.tmux_state.clone();
        let session_name = self.session_name.clone();

        let reader_task = tokio::spawn(async move {
            let (event_tx, mut event_rx) = mpsc::channel::<TmuxEvent>(512);
            let mut parser = ControlModeParser::new(event_tx);
            let mut line_buf = String::new();

            loop {
                tokio::select! {
                    msg = ctrl_channel.wait() => {
                        match msg {
                            Some(ChannelMsg::Data { ref data }) => {
                                let text = String::from_utf8_lossy(data);
                                line_buf.push_str(&text);

                                // Process complete lines
                                while let Some(newline_pos) = line_buf.find('\n') {
                                    let line: String = line_buf[..newline_pos].to_string();
                                    line_buf.drain(..=newline_pos);
                                    if let Err(e) = parser.feed_line(&line).await {
                                        warn!(error = %e, line = %line, "control mode parse error");
                                    }
                                }
                            }
                            Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                                info!(session = %session_name, "control mode channel closed");
                                break;
                            }
                            Some(other) => {
                                info!(session = %session_name, msg = ?other, "control mode other msg");
                            }
                        }
                    }

                    event = event_rx.recv() => {
                        match event {
                            Some(TmuxEvent::Output { pane_id, data }) => {
                                let tx = shared_outputs
                                    .entry(pane_id.clone())
                                    .or_insert_with(|| broadcast::channel(4096).0);
                                let _ = tx.send(data);
                            }
                            Some(TmuxEvent::Exit { reason }) => {
                                info!(session = %session_name, ?reason, "tmux control mode exited");
                                break;
                            }
                            Some(_other_event) => {
                                // State change events — could refresh tmux_state here
                                debug!(session = %session_name, "tmux event received");
                            }
                            None => break,
                        }
                    }
                }
            }
        });

        self.reader_task = Some(reader_task);
        *self.ssh_handle.lock().await = Some(handle);
        self.status = TransportStatus::Connected;

        info!(addr = %addr, session = %self.session_name, panes = state.iter().map(|s| s.windows.iter().map(|w| w.panes.len()).sum::<usize>()).sum::<usize>(), "SSH transport connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        // Stop reader task
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }

        // Close SSH connection
        let mut handle_lock = self.ssh_handle.lock().await;
        if let Some(handle) = handle_lock.take() {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "closing", "en")
                .await;
        }
        *self.control_channel_id.lock().await = None;
        self.status = TransportStatus::Disconnected;
        info!(host = %self.host, "SSH transport disconnected");
        Ok(())
    }

    async fn list_tmux_sessions(&self) -> Result<Vec<TmuxSessionInfo>> {
        let handle_lock = self.ssh_handle.lock().await;
        let handle = handle_lock
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("SSH not connected"))?;

        let list_cmd = format!(
            "tmux list-panes -t {} -s -F '#{{session_id}}|||#{{session_name}}|||#{{window_id}}|||#{{window_name}}|||#{{window_index}}|||#{{window_layout}}|||#{{pane_id}}|||#{{pane_index}}|||#{{pane_width}}|||#{{pane_height}}|||#{{pane_current_command}}|||#{{pane_active}}'",
            self.session_name
        );
        let output = Self::exec_command(handle, &list_cmd).await?;
        let state = Self::parse_pane_listing(&output);
        *self.tmux_state.lock().await = state.clone();
        Ok(state)
    }

    async fn send_input(&self, pane_id: &str, data: &[u8]) -> Result<()> {
        // Send raw bytes via control mode: send-keys -t <pane> -H <hex>
        let hex: String = data.iter().map(|b| format!("{:02x} ", b)).collect();
        let cmd = format!("send-keys -t {} -H {}", pane_id, hex.trim());
        self.write_control_command(&cmd).await
    }

    async fn resize_pane(&self, pane_id: &str, cols: u16, rows: u16) -> Result<()> {
        info!(pane = %pane_id, cols, rows, "resizing window+pane");
        // resize-window via control mode command (safe — doesn't kill control mode
        // because it's sent WITHIN the control mode session, not as external tmux cmd)
        let win_cmd = format!("resize-window -t {} -x {} -y {}", pane_id, cols, rows);
        self.write_control_command(&win_cmd).await?;
        let pane_cmd = format!("resize-pane -t {} -x {} -y {}", pane_id, cols, rows);
        self.write_control_command(&pane_cmd).await
    }

    async fn run_tmux_command(&self, cmd: &str) -> Result<String> {
        let handle_lock = self.ssh_handle.lock().await;
        let handle = handle_lock
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("SSH not connected"))?;
        Self::exec_command(handle, &format!("tmux {}", cmd)).await
    }

    fn subscribe_output(&self, pane_id: &str) -> Option<broadcast::Receiver<Bytes>> {
        self.shared_pane_outputs
            .get(pane_id)
            .map(|tx| tx.subscribe())
    }

    fn status(&self) -> TransportStatus {
        self.status
    }
}
