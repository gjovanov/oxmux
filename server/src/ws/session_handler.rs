//! Shared session message handling logic used by both WebSocket and WebTransport handlers.

use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::state::AppState;
use super::protocol::{ClientMsg, ServerMsg, decode_client_msg, encode_server_msg};

/// Per-connection state shared across transport handlers.
pub struct ConnectionState {
    pub user_id: String,
    pub pane_subs: HashMap<String, broadcast::Receiver<Bytes>>,
}

impl ConnectionState {
    pub fn new(user_id: String) -> Self {
        Self {
            user_id,
            pane_subs: HashMap::new(),
        }
    }
}

/// Handle a decoded client message. Returns an optional reply.
pub async fn handle_client_msg(
    msg: ClientMsg,
    state: &Arc<AppState>,
    conn: &mut ConnectionState,
) -> Option<ServerMsg> {
    use secrecy::ExposeSecret;
    use crate::auth::jwt;
    use crate::webrtc::turn::{build_ice_config, generate_turn_credentials};

    match msg {
        ClientMsg::Subscribe { pane } => {
            info!(pane = %pane, "client subscribing to pane");
            let sender = state.get_or_create_pane_channel(&pane);
            conn.pane_subs.insert(pane, sender.subscribe());
            None
        }

        ClientMsg::Unsubscribe { pane } => {
            conn.pane_subs.remove(&pane);
            None
        }

        ClientMsg::Resize { pane, cols, rows } => {
            debug!("Resize pane {} to {}x{}", pane, cols, rows);
            if let Err(e) = state.session_manager.resize_pane(&pane, cols, rows).await {
                warn!("Failed to resize pane {}: {}", pane, e);
            }
            None
        }

        ClientMsg::Ping { ts } => Some(ServerMsg::Pong { ts }),

        ClientMsg::IceRequest { peer_id } => {
            match generate_turn_credentials(&state.config.coturn, &peer_id) {
                Ok(creds) => {
                    let config = build_ice_config(&creds);
                    Some(ServerMsg::IceConfig { peer_id, config })
                }
                Err(e) => {
                    warn!("Failed to generate TURN credentials: {}", e);
                    Some(ServerMsg::Error {
                        code: "turn_error".to_string(),
                        message: e.to_string(),
                    })
                }
            }
        }

        ClientMsg::Input { pane, data } => {
            if let Err(e) = state.session_manager.send_input_to_pane(&pane, &data).await {
                warn!("Failed to send input to pane {}: {}", pane, e);
            }
            None
        }

        ClientMsg::TmuxCommand { command: _ } => None,

        ClientMsg::Signal { peer_id, payload } => {
            Some(ServerMsg::Signal { peer_id, payload })
        }

        ClientMsg::ClaudeInput { session_id: _, prompt: _ } => None,

        // ── Session management ──────────────────────────────────────

        ClientMsg::CreateSession(req) => {
            match state.session_manager.create(&conn.user_id, req).await {
                Ok(session) => Some(ServerMsg::SessionCreated { session: session.sanitized() }),
                Err(e) => Some(ServerMsg::Error {
                    code: "session_create_error".to_string(),
                    message: e.to_string(),
                }),
            }
        }

        ClientMsg::ListSessions => {
            match state.session_manager.load_user_sessions(&conn.user_id).await {
                Ok(sessions) => Some(ServerMsg::SessionList {
                    sessions: sessions.iter().map(|s| s.sanitized()).collect(),
                }),
                Err(e) => Some(ServerMsg::Error {
                    code: "session_list_error".to_string(),
                    message: e.to_string(),
                }),
            }
        }

        ClientMsg::ConnectSession { session_id } => {
            match state.session_manager.connect(&session_id).await {
                Ok(session) => Some(ServerMsg::SessionConnected { session: session.sanitized() }),
                Err(e) => Some(ServerMsg::Error {
                    code: "session_connect_error".to_string(),
                    message: e.to_string(),
                }),
            }
        }

        ClientMsg::DisconnectSession { session_id } => {
            match state.session_manager.disconnect(&session_id).await {
                Ok(session) => Some(ServerMsg::SessionDisconnected { session: session.sanitized() }),
                Err(e) => Some(ServerMsg::Error {
                    code: "session_disconnect_error".to_string(),
                    message: e.to_string(),
                }),
            }
        }

        ClientMsg::UpdateSession { session_id, request } => {
            match state.session_manager.update(&session_id, request).await {
                Ok(session) => Some(ServerMsg::SessionUpdated { session: session.sanitized() }),
                Err(e) => Some(ServerMsg::Error {
                    code: "session_update_error".to_string(),
                    message: e.to_string(),
                }),
            }
        }

        ClientMsg::DeleteSession { session_id } => {
            match state.session_manager.delete(&session_id).await {
                Ok(_) => Some(ServerMsg::SessionDeleted { session_id }),
                Err(e) => Some(ServerMsg::Error {
                    code: "session_delete_error".to_string(),
                    message: e.to_string(),
                }),
            }
        }

        ClientMsg::RefreshSession { session_id } => {
            match state.session_manager.refresh_tmux_state(&session_id).await {
                Ok(session) => Some(ServerMsg::SessionConnected { session: session.sanitized() }),
                Err(e) => Some(ServerMsg::Error {
                    code: "session_refresh_error".to_string(),
                    message: e.to_string(),
                }),
            }
        }
    }
}

/// Drain all pending pane output from broadcast receivers.
/// Returns encoded ServerMsg::Output frames ready to send.
pub fn drain_pane_outputs(conn: &mut ConnectionState) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    for (pane_id, sub_rx) in conn.pane_subs.iter_mut() {
        loop {
            match sub_rx.try_recv() {
                Ok(data) => {
                    let msg = ServerMsg::Output {
                        pane: pane_id.clone(),
                        data,
                    };
                    if let Ok(encoded) = encode_server_msg(&msg) {
                        frames.push(encoded);
                    }
                }
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    warn!("Client lagged {} messages on pane {}", n, pane_id);
                }
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Closed) => break,
            }
        }
    }
    frames
}
