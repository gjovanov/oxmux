use anyhow::Result;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// WebRTC signaling relay.
///
/// For WebRTC P2P transport, the server does NOT relay PTY data.
/// Instead, it:
/// 1. Maintains a registry of online oxmux-agents (registered via QUIC or REST)
/// 2. Relays SDP offers/answers and ICE candidates between browser and agent
/// 3. Once the DataChannel is established, all data flows browser ↔ agent directly
///
/// This is the preferred transport for lowest latency.
pub struct WebRtcSignaler {
    /// Registry of online agents: agent_id → agent metadata
    agents: Arc<DashMap<String, AgentInfo>>,
    /// Per-peer signaling channels: peer_id → sender
    signal_channels: Arc<DashMap<String, mpsc::Sender<SignalMessage>>>,
}

/// Metadata about a registered oxmux-agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub agent_id: String,
    pub hostname: String,
    /// IP:port for QUIC fallback
    pub quic_addr: Option<String>,
    /// Whether agent supports WebRTC DataChannel
    pub webrtc_capable: bool,
    /// Timestamp of last heartbeat
    pub last_seen: u64,
}

/// A signaling message relayed between browser and agent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalMessage {
    /// SDP offer from browser to agent
    Offer {
        from: String,
        to: String,
        sdp: String,
    },
    /// SDP answer from agent to browser
    Answer {
        from: String,
        to: String,
        sdp: String,
    },
    /// ICE candidate from either side
    IceCandidate {
        from: String,
        to: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u32>,
    },
    /// Agent connected and ready
    Ready {
        agent_id: String,
    },
    /// Agent disconnected
    Bye {
        from: String,
    },
}

impl WebRtcSignaler {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(DashMap::new()),
            signal_channels: Arc::new(DashMap::new()),
        }
    }

    /// Register an agent as online and capable.
    pub fn register_agent(&self, info: AgentInfo) {
        info!(agent_id = %info.agent_id, hostname = %info.hostname, "agent registered");
        self.agents.insert(info.agent_id.clone(), info);
    }

    /// Remove agent registration.
    pub fn unregister_agent(&self, agent_id: &str) {
        info!(agent_id = %agent_id, "agent unregistered");
        self.agents.remove(agent_id);
        self.signal_channels.remove(agent_id);
    }

    /// List all registered agents.
    pub fn list_agents(&self) -> Vec<AgentInfo> {
        self.agents
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Check if an agent is online.
    pub fn is_agent_online(&self, agent_id: &str) -> bool {
        self.agents.contains_key(agent_id)
    }

    /// Create a signaling channel for a peer (browser or agent).
    /// Returns a receiver to consume incoming signaling messages.
    pub fn create_signal_channel(&self, peer_id: &str) -> mpsc::Receiver<SignalMessage> {
        let (tx, rx) = mpsc::channel(64);
        self.signal_channels.insert(peer_id.to_string(), tx);
        rx
    }

    /// Relay a signaling message to the target peer.
    pub async fn relay_signal(&self, msg: SignalMessage) -> Result<()> {
        let target = match &msg {
            SignalMessage::Offer { to, .. } => to.clone(),
            SignalMessage::Answer { to, .. } => to.clone(),
            SignalMessage::IceCandidate { to, .. } => to.clone(),
            SignalMessage::Ready { agent_id } => agent_id.clone(),
            SignalMessage::Bye { from } => {
                // Notify all channels about disconnection
                debug!(from = %from, "peer disconnected");
                return Ok(());
            }
        };

        if let Some(tx) = self.signal_channels.get(&target) {
            tx.send(msg)
                .await
                .map_err(|_| anyhow::anyhow!("signaling channel closed for {}", target))?;
        } else {
            warn!(target = %target, "no signaling channel for target peer");
        }

        Ok(())
    }

    /// Remove a peer's signaling channel.
    pub fn remove_signal_channel(&self, peer_id: &str) {
        self.signal_channels.remove(peer_id);
    }
}

impl Default for WebRtcSignaler {
    fn default() -> Self {
        Self::new()
    }
}
