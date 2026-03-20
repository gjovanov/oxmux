use serde::{Deserialize, Serialize};
use secrecy::{ExposeSecret, SecretString};
use std::fmt;

pub type SessionId = String;

// ── Browser Transport (how browser talks to server/agent) ───────────────

/// How the browser connects to the server or agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTransport {
    /// WebSocket over HTTPS (default, works everywhere)
    Websocket,
    /// QUIC via WebTransport API (low latency, 0-RTT)
    Quic,
    /// WebRTC DataChannel (NAT traversal via TURN/STUN)
    Webrtc,
}

impl Default for BrowserTransport {
    fn default() -> Self {
        BrowserTransport::Websocket
    }
}

impl fmt::Display for BrowserTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrowserTransport::Websocket => write!(f, "websocket"),
            BrowserTransport::Quic => write!(f, "quic"),
            BrowserTransport::Webrtc => write!(f, "webrtc"),
        }
    }
}

// ── Backend Transport (how server/agent reaches tmux) ───────────────────

/// How the server reaches the remote tmux session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackendTransport {
    /// SSH into a remote host, attach to tmux there.
    /// Used with server-relayed transports (1, 2, 3).
    Ssh {
        host: String,
        #[serde(default = "default_ssh_port")]
        port: u16,
        user: String,
        #[serde(default)]
        auth: SshAuthConfig,
    },
    /// Direct connection to an oxmux-agent running on the host.
    /// Agent manages tmux locally — no SSH needed.
    /// Used with P2P transports (4, 5).
    Agent {
        /// Agent's hostname/IP
        host: String,
        /// Agent's QUIC port
        #[serde(default = "default_agent_port")]
        port: u16,
        /// Agent registration ID (from server registry)
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    /// Local tmux on the server host (dev/testing only).
    Local,
}

fn default_ssh_port() -> u16 {
    22
}
fn default_agent_port() -> u16 {
    4433
}

// ── SSH Auth ────────────────────────────────────────────────────────────

/// Custom serde for SecretString — serializes the actual value (needed for DB persistence).
mod secret_string_serde {
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(val: &SecretString, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(val.expose_secret())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
        let raw = String::deserialize(d)?;
        Ok(SecretString::new(raw.into()))
    }
}

mod opt_secret_string_serde {
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(val: &Option<SecretString>, s: S) -> Result<S::Ok, S::Error> {
        match val {
            Some(v) => s.serialize_some(v.expose_secret()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SecretString>, D::Error> {
        let raw = Option::<String>::deserialize(d)?;
        Ok(raw.map(|r| SecretString::new(r.into())))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum SshAuthConfig {
    Agent,
    Password {
        #[serde(with = "secret_string_serde")]
        password: SecretString,
    },
    PrivateKey {
        path: String,
        #[serde(default, with = "opt_secret_string_serde")]
        passphrase: Option<SecretString>,
    },
    /// Key uploaded from browser — key material stored in ephemeral memory only.
    /// The key_id references an entry in SessionManager's ephemeral_keys DashMap.
    UploadedKey {
        key_id: String,
        #[serde(default, with = "opt_secret_string_serde")]
        passphrase: Option<SecretString>,
    },
}

impl Clone for SshAuthConfig {
    fn clone(&self) -> Self {
        match self {
            SshAuthConfig::Agent => SshAuthConfig::Agent,
            SshAuthConfig::Password { password } => SshAuthConfig::Password {
                password: SecretString::new(password.expose_secret().to_string().into()),
            },
            SshAuthConfig::PrivateKey { path, passphrase } => SshAuthConfig::PrivateKey {
                path: path.clone(),
                passphrase: passphrase.as_ref().map(|p| SecretString::new(p.expose_secret().to_string().into())),
            },
            SshAuthConfig::UploadedKey { key_id, passphrase } => SshAuthConfig::UploadedKey {
                key_id: key_id.clone(),
                passphrase: passphrase.as_ref().map(|p| SecretString::new(p.expose_secret().to_string().into())),
            },
        }
    }
}

impl fmt::Debug for SshAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SshAuthConfig::Agent => write!(f, "SshAuthConfig::Agent"),
            SshAuthConfig::Password { .. } => write!(f, "SshAuthConfig::Password {{ *** }}"),
            SshAuthConfig::PrivateKey { path, passphrase } => {
                write!(f, "SshAuthConfig::PrivateKey {{ path: {:?}, has_passphrase: {} }}", path, passphrase.is_some())
            }
            SshAuthConfig::UploadedKey { key_id, passphrase } => {
                write!(f, "SshAuthConfig::UploadedKey {{ key_id: {:?}, has_passphrase: {} }}", key_id, passphrase.is_some())
            }
        }
    }
}

impl Default for SshAuthConfig {
    fn default() -> Self {
        SshAuthConfig::Agent
    }
}

// ── Combined Transport Config ───────────────────────────────────────────

/// Full transport configuration combining browser and backend.
///
/// Valid combinations:
/// | # | Browser   | Backend | Description                    |
/// |---|-----------|---------|--------------------------------|
/// | 1 | Websocket | Ssh     | WS → Server → SSH → Host       |
/// | 2 | Quic      | Ssh     | QUIC → Server → SSH → Host     |
/// | 3 | Webrtc    | Ssh     | WebRTC → Server → SSH → Host   |
/// | 4 | Quic      | Agent   | QUIC → Agent (P2P)             |
/// | 5 | Webrtc    | Agent   | WebRTC → Agent (P2P)           |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// How the browser connects (WS/QUIC/WebRTC)
    #[serde(default)]
    pub browser: BrowserTransport,
    /// How the backend reaches tmux (SSH/Agent/Local)
    pub backend: BackendTransport,
}

// ── Session Status ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Created,
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
    Error,
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionStatus::Created => write!(f, "created"),
            SessionStatus::Connecting => write!(f, "connecting"),
            SessionStatus::Connected => write!(f, "connected"),
            SessionStatus::Reconnecting => write!(f, "reconnecting"),
            SessionStatus::Disconnected => write!(f, "disconnected"),
            SessionStatus::Error => write!(f, "error"),
        }
    }
}

// ── Managed Session ─────────────────────────────────────────────────────

/// A managed session visible to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSession {
    pub id: SessionId,
    pub name: String,
    pub transport: TransportConfig,
    pub status: SessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tmux_sessions: Vec<super::super::ws::protocol::TmuxSessionInfo>,
}

impl ManagedSession {
    /// Return a copy with secrets stripped (for sending to clients).
    pub fn sanitized(&self) -> Self {
        let mut s = self.clone();
        if let BackendTransport::Ssh { ref mut auth, .. } = s.transport.backend {
            *auth = match &*auth {
                SshAuthConfig::Password { .. } => SshAuthConfig::Password {
                    password: SecretString::new("***".to_string().into()),
                },
                SshAuthConfig::PrivateKey { path, .. } => SshAuthConfig::PrivateKey {
                    path: path.clone(),
                    passphrase: None,
                },
                SshAuthConfig::UploadedKey { key_id, .. } => SshAuthConfig::UploadedKey {
                    key_id: key_id.clone(),
                    passphrase: None,
                },
                other => other.clone(),
            };
        }
        s
    }

    /// Returns the transport number (1-5) for display.
    pub fn transport_number(&self) -> u8 {
        match (&self.transport.browser, &self.transport.backend) {
            (BrowserTransport::Websocket, BackendTransport::Ssh { .. }) => 1,
            (BrowserTransport::Quic, BackendTransport::Ssh { .. }) => 2,
            (BrowserTransport::Webrtc, BackendTransport::Ssh { .. }) => 3,
            (BrowserTransport::Quic, BackendTransport::Agent { .. }) => 4,
            (BrowserTransport::Webrtc, BackendTransport::Agent { .. }) => 5,
            _ => 0, // Local or invalid combination
        }
    }
}

// ── Requests ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub name: String,
    pub transport: TransportConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
