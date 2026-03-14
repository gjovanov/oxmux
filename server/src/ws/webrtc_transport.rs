//! WebRTC DataChannel handler for browser connections.
//!
//! Provides Transport #3 (WebRTC → SSH) and #5 (WebRTC → Agent).
//! Signaling happens over the existing WebSocket connection.
//! Once the DataChannel is established, it carries the same MessagePack protocol.
//!
//! Flow:
//! 1. Browser sends SDP offer via WS Signal message
//! 2. Server creates RTCPeerConnection, generates answer
//! 3. ICE candidates exchanged via WS Signal messages
//! 4. DataChannel opens → MessagePack I/O

use anyhow::{Context, Result};
use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info, warn};

use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

use crate::state::AppState;
use crate::ws::protocol::{decode_client_msg, encode_server_msg};
use crate::ws::session_handler::{self, ConnectionState};

/// Manages WebRTC peer connections for browser clients.
/// Each browser that chooses WebRTC transport gets a peer connection here.
pub struct WebRtcManager {
    /// peer_id → peer connection
    peers: DashMap<String, PeerState>,
}

struct PeerState {
    pc: Arc<RTCPeerConnection>,
    /// Channel to send outgoing messages to the DataChannel
    outgoing_tx: mpsc::Sender<Vec<u8>>,
}

impl WebRtcManager {
    pub fn new() -> Self {
        Self {
            peers: DashMap::new(),
        }
    }

    /// Handle an SDP offer from a browser client. Returns the SDP answer.
    pub async fn handle_offer(
        &self,
        peer_id: &str,
        user_id: &str,
        sdp_offer: &str,
        state: &Arc<AppState>,
    ) -> Result<String> {
        use crate::webrtc::turn::generate_turn_credentials;

        // Build ICE servers config with TURN credentials
        let mut ice_servers = vec![RTCIceServer {
            urls: vec![
                "stun:stun.l.google.com:19302".to_string(),
            ],
            ..Default::default()
        }];

        // Add TURN servers if available
        if let Ok(creds) = generate_turn_credentials(&state.config.coturn, peer_id) {
            for uri in &creds.uris {
                ice_servers.push(RTCIceServer {
                    urls: vec![uri.clone()],
                    username: creds.username.clone(),
                    credential: creds.credential.clone(),
                    ..Default::default()
                });
            }
        }

        let config = RTCConfiguration {
            ice_servers,
            ..Default::default()
        };

        // Create peer connection
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut m)?;
        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .build();

        let pc = Arc::new(api.new_peer_connection(config).await?);

        // Create outgoing message channel
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Vec<u8>>(256);

        // Set up DataChannel handler
        let state = state.clone();
        let user_id_owned = user_id.to_string();
        let peer_id_owned = peer_id.to_string();

        pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            let state = state.clone();
            let user_id = user_id_owned.clone();
            let peer_id = peer_id_owned.clone();

            Box::pin(async move {
                info!(peer_id = %peer_id, label = %dc.label(), "DataChannel opened");

                let dc_send = dc.clone();
                let state_msg = state.clone();
                let user_id_msg = user_id.clone();

                // Create connection state for this WebRTC peer
                let conn = Arc::new(Mutex::new(ConnectionState::new(user_id.clone())));

                // Handle incoming DataChannel messages
                let conn_clone = conn.clone();
                dc.on_message(Box::new(move |msg: DataChannelMessage| {
                    let state = state_msg.clone();
                    let dc = dc_send.clone();
                    let conn = conn_clone.clone();

                    Box::pin(async move {
                        match decode_client_msg(&msg.data) {
                            Ok(client_msg) => {
                                let mut conn = conn.lock().await;
                                if let Some(reply) = session_handler::handle_client_msg(
                                    client_msg, &state, &mut conn
                                ).await {
                                    if let Ok(encoded) = encode_server_msg(&reply) {
                                        let _ = dc.send(&Bytes::from(encoded)).await;
                                    }
                                }

                                // Drain pane output
                                for frame in session_handler::drain_pane_outputs(&mut conn) {
                                    let _ = dc.send(&Bytes::from(frame)).await;
                                }
                            }
                            Err(e) => {
                                warn!("Failed to decode DataChannel message: {}", e);
                            }
                        }
                    })
                }));

                // Start periodic pane output drain
                let conn_drain = conn.clone();
                let dc_drain = dc.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(10));
                    loop {
                        interval.tick().await;
                        let mut conn = conn_drain.lock().await;
                        for frame in session_handler::drain_pane_outputs(&mut conn) {
                            if dc_drain.send(&Bytes::from(frame)).await.is_err() {
                                return; // DataChannel closed
                            }
                        }
                    }
                });
            })
        }));

        // Set remote description (browser's offer)
        let offer = RTCSessionDescription::offer(sdp_offer.to_string())?;
        pc.set_remote_description(offer).await?;

        // Create answer
        let answer = pc.create_answer(None).await?;
        let sdp_answer = answer.sdp.clone();

        // Set local description
        pc.set_local_description(answer).await?;

        // Store peer
        self.peers.insert(peer_id.to_string(), PeerState {
            pc: pc.clone(),
            outgoing_tx,
        });

        info!(peer_id = %peer_id, "WebRTC peer connection created");
        Ok(sdp_answer)
    }

    /// Add a remote ICE candidate.
    pub async fn add_ice_candidate(
        &self,
        peer_id: &str,
        candidate: &str,
        sdp_mid: Option<&str>,
        sdp_mline_index: Option<u16>,
    ) -> Result<()> {
        let peer = self.peers.get(peer_id)
            .ok_or_else(|| anyhow::anyhow!("peer '{}' not found", peer_id))?;

        let candidate = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
            candidate: candidate.to_string(),
            sdp_mid: sdp_mid.map(|s| s.to_string()),
            sdp_mline_index: sdp_mline_index,
            ..Default::default()
        };

        peer.pc.add_ice_candidate(candidate).await?;
        debug!(peer_id = %peer_id, "ICE candidate added");
        Ok(())
    }

    /// Remove a peer connection.
    pub async fn remove_peer(&self, peer_id: &str) {
        if let Some((_, peer)) = self.peers.remove(peer_id) {
            let _ = peer.pc.close().await;
            info!(peer_id = %peer_id, "WebRTC peer removed");
        }
    }
}
