use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::broadcast;

use crate::ws::protocol::TmuxSessionInfo;

/// Status reported by a transport implementation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// Abstraction over the transport layer (Local tmux, SSH, QUIC, WebRTC).
///
/// Each transport ultimately provides access to a tmux instance
/// (local or remote) and relays PTY I/O.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Establish the connection and start streaming.
    async fn connect(&mut self) -> Result<()>;

    /// Gracefully disconnect.
    async fn disconnect(&mut self) -> Result<()>;

    /// List tmux sessions available through this transport.
    async fn list_tmux_sessions(&self) -> Result<Vec<TmuxSessionInfo>>;

    /// Send raw input bytes to a specific pane.
    async fn send_input(&self, pane_id: &str, data: &[u8]) -> Result<()>;

    /// Resize a pane.
    async fn resize_pane(&self, pane_id: &str, cols: u16, rows: u16) -> Result<()>;

    /// Run a raw tmux command and return the output.
    async fn run_tmux_command(&self, cmd: &str) -> Result<String>;

    /// Subscribe to output from a specific pane.
    /// Returns a broadcast receiver that yields PTY output bytes.
    fn subscribe_output(&self, pane_id: &str) -> Option<broadcast::Receiver<Bytes>>;

    /// Current transport status.
    fn status(&self) -> TransportStatus;
}
