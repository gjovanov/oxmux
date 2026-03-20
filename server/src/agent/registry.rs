//! Agent registry — tracks online oxmux-agents.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub hostname: String,
    pub host: String,
    pub quic_port: u16,
    pub version: String,
    pub last_seen: u64,
    /// SHA-256 hash of the agent's self-signed TLS cert (hex-encoded, lowercase).
    /// Used for WebTransport cert pinning via serverCertificateHashes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_hash: Option<String>,
}

pub struct AgentRegistry {
    agents: DashMap<String, AgentInfo>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
        }
    }

    pub fn register(&self, info: AgentInfo) {
        info!(id = %info.id, host = %info.host, "agent registered");
        self.agents.insert(info.id.clone(), info);
    }

    pub fn unregister(&self, id: &str) {
        self.agents.remove(id);
        info!(id = %id, "agent unregistered");
    }

    pub fn list(&self) -> Vec<AgentInfo> {
        self.agents.iter().map(|e| e.value().clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<AgentInfo> {
        self.agents.get(id).map(|e| e.value().clone())
    }

    pub fn find_by_host(&self, host: &str) -> Option<AgentInfo> {
        self.agents
            .iter()
            .find(|e| e.value().host == host)
            .map(|e| e.value().clone())
    }

    pub fn is_online(&self, id: &str) -> bool {
        self.agents.contains_key(id)
    }

    pub fn update_heartbeat(&self, id: &str, timestamp: u64) {
        if let Some(mut agent) = self.agents.get_mut(id) {
            agent.last_seen = timestamp;
        }
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
