use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::claude::parser::{ClaudeEvent, SessionAccumulator};
use crate::session::types::{CreateSessionRequest, ManagedSession, UpdateSessionRequest};
use crate::webrtc::turn::IceConfig;

/// Messages sent from server → browser
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Raw PTY output for a pane (binary terminal bytes)
    #[serde(rename = "o")]
    Output { pane: String, data: Bytes },

    /// Full tmux state dump (on connect or reconnect)
    #[serde(rename = "s")]
    State { sessions: Vec<TmuxSessionInfo> },

    /// Incremental tmux event (pane added/removed, layout changed etc.)
    #[serde(rename = "e")]
    TmuxEvent(TmuxEventMsg),

    /// Structured Claude Code event (replaces raw Output for claude panes)
    #[serde(rename = "c")]
    ClaudeEvent { session_id: String, event: ClaudeEvent },

    /// Claude session accumulator snapshot (cost, files changed, context usage)
    #[serde(rename = "ca")]
    ClaudeAccumulator { session_id: String, state: SessionAccumulator },

    /// ICE configuration for WebRTC (includes TURN credentials)
    #[serde(rename = "ice")]
    IceConfig { peer_id: String, config: IceConfig },

    /// WebRTC signaling: SDP offer/answer/ICE candidate relay
    #[serde(rename = "sig")]
    Signal { peer_id: String, payload: serde_json::Value },

    /// Error message
    #[serde(rename = "err")]
    Error { code: String, message: String },

    /// Pong response to client ping
    #[serde(rename = "pong")]
    Pong { ts: u64 },

    // ── Session management responses ────────────────────────────────────

    /// List of all managed sessions
    #[serde(rename = "sess_list")]
    SessionList { sessions: Vec<ManagedSession> },

    /// A session was created
    #[serde(rename = "sess_created")]
    SessionCreated { session: ManagedSession },

    /// A session was updated
    #[serde(rename = "sess_updated")]
    SessionUpdated { session: ManagedSession },

    /// A session was deleted
    #[serde(rename = "sess_deleted")]
    SessionDeleted { session_id: String },

    /// A session connected (includes tmux state)
    #[serde(rename = "sess_connected")]
    SessionConnected { session: ManagedSession },

    /// A session disconnected
    #[serde(rename = "sess_disconnected")]
    SessionDisconnected { session: ManagedSession },

    // ── Agent management responses ──────────────────────────────────────

    /// Agent status update (not_installed, installing, starting, online, error)
    #[serde(rename = "agent_status")]
    AgentStatus {
        session_id: String,
        host: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        quic_port: Option<u16>,
    },

    /// Transport upgrade ready — browser can now connect P2P
    #[serde(rename = "transport_upgrade_ready")]
    TransportUpgradeReady {
        session_id: String,
        agent_host: String,
        agent_port: u16,
        agent_token: String,
    },

    /// Transport upgrade failed
    #[serde(rename = "transport_upgrade_failed")]
    TransportUpgradeFailed {
        session_id: String,
        error: String,
    },
}

/// Messages sent from browser → server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Subscribe to a pane's output stream
    #[serde(rename = "sub")]
    Subscribe { pane: String },

    /// Unsubscribe from a pane
    #[serde(rename = "unsub")]
    Unsubscribe { pane: String },

    /// Keyboard/paste input for a pane
    #[serde(rename = "i")]
    Input { pane: String, data: Bytes },

    /// Resize notification (browser terminal resized)
    #[serde(rename = "r")]
    Resize { pane: String, cols: u16, rows: u16 },

    /// Raw tmux command (e.g. "new-session -s foo")
    #[serde(rename = "cmd")]
    TmuxCommand { command: String },

    /// Request ICE config for a new WebRTC peer connection
    #[serde(rename = "ice_req")]
    IceRequest { peer_id: String },

    /// WebRTC signaling passthrough (SDP offer/answer/ICE candidates)
    #[serde(rename = "sig")]
    Signal { peer_id: String, payload: serde_json::Value },

    /// Inject a prompt into a running Claude Code session
    #[serde(rename = "claude_in")]
    ClaudeInput { session_id: String, prompt: String },

    /// Ping (latency measurement)
    #[serde(rename = "ping")]
    Ping { ts: u64 },

    // ── Session management commands ─────────────────────────────────────

    /// Create a new session
    #[serde(rename = "sess_create")]
    CreateSession(CreateSessionRequest),

    /// List all sessions
    #[serde(rename = "sess_list")]
    ListSessions,

    /// Connect a session (start transport)
    #[serde(rename = "sess_connect")]
    ConnectSession { session_id: String },

    /// Disconnect a session (stop transport, keep metadata)
    #[serde(rename = "sess_disconnect")]
    DisconnectSession { session_id: String },

    /// Update session metadata (rename)
    #[serde(rename = "sess_update")]
    UpdateSession { session_id: String, #[serde(flatten)] request: UpdateSessionRequest },

    /// Delete a session entirely
    #[serde(rename = "sess_delete")]
    DeleteSession { session_id: String },

    /// Refresh tmux state for a session
    #[serde(rename = "sess_refresh")]
    RefreshSession { session_id: String },

    // ── Agent management ────────────────────────────────────────────────

    /// Install agent on the host of a connected SSH session
    #[serde(rename = "agent_install")]
    InstallAgent { session_id: String },

    /// Request transport upgrade from SSH to P2P
    #[serde(rename = "transport_upgrade")]
    TransportUpgrade { session_id: String, target: String },

    /// Check agent status for a host
    #[serde(rename = "agent_status")]
    AgentStatusRequest { host: String },
}

/// tmux session/window/pane tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmuxSessionInfo {
    pub id: String,
    pub name: String,
    pub windows: Vec<TmuxWindowInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmuxWindowInfo {
    pub id: String,
    pub name: String,
    pub index: usize,
    pub layout: String,
    pub panes: Vec<TmuxPaneInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmuxPaneInfo {
    pub id: String,
    pub index: usize,
    pub cols: u16,
    pub rows: u16,
    pub current_command: String,
    pub is_active: bool,
    pub is_claude: bool,
}

/// Incremental tmux event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum TmuxEventMsg {
    SessionCreated { id: String, name: String },
    SessionClosed { id: String },
    WindowCreated { session_id: String, window_id: String, name: String },
    WindowClosed { session_id: String, window_id: String },
    PaneCreated { window_id: String, pane: TmuxPaneInfo },
    PaneClosed { pane_id: String },
    LayoutChanged { window_id: String, layout: String },
    PaneTitleChanged { pane_id: String, title: String },
}

/// Encode a server message to MessagePack bytes
pub fn encode_server_msg(msg: &ServerMsg) -> anyhow::Result<Vec<u8>> {
    Ok(rmp_serde::to_vec_named(msg)?)
}

/// Decode a client message from MessagePack bytes
pub fn decode_client_msg(bytes: &[u8]) -> anyhow::Result<ClientMsg> {
    Ok(rmp_serde::from_slice(bytes)?)
}
