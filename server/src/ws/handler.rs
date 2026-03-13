use axum::{
    extract::{State, WebSocketUpgrade},
    response::{IntoResponse, Json, Response},
};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::{
    state::AppState,
    webrtc::turn::{build_ice_config, generate_turn_credentials},
    ws::protocol::{decode_client_msg, encode_server_msg, ClientMsg, ServerMsg},
};

/// WebSocket upgrade endpoint: GET /ws
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.max_message_size(2 * 1024 * 1024) // 2MB max frame
        .on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: axum::extract::ws::WebSocket, state: Arc<AppState>) {
    use axum::extract::ws::Message;
    use futures_util::{SinkExt, StreamExt};

    let (mut tx, mut rx) = socket.split();

    // Per-connection subscription map: pane_id → broadcast::Receiver
    let mut pane_subs: std::collections::HashMap<String, tokio::sync::broadcast::Receiver<bytes::Bytes>> =
        std::collections::HashMap::new();

    info!("WebSocket client connected");

    loop {
        tokio::select! {
            // Incoming client message
            msg = rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        match decode_client_msg(&data) {
                            Ok(client_msg) => {
                                if let Some(reply) = handle_client_msg(client_msg, &state, &mut pane_subs).await {
                                    if let Ok(encoded) = encode_server_msg(&reply) {
                                        let _ = tx.send(Message::Binary(encoded.into())).await;
                                    }
                                }
                            }
                            Err(e) => warn!("Failed to decode client message: {}", e),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!("WebSocket client disconnected");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            // Fan-out pane output to this client
            // Drain all active pane subscriptions
            _ = async {
                for (pane_id, sub_rx) in pane_subs.iter_mut() {
                    match sub_rx.try_recv() {
                        Ok(data) => {
                            let msg = ServerMsg::Output {
                                pane: pane_id.clone(),
                                data: data.clone(),
                            };
                            if let Ok(encoded) = encode_server_msg(&msg) {
                                let _ = tx.send(Message::Binary(encoded.into())).await;
                            }
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                            warn!("Client lagged {} messages on pane {}", n, pane_id);
                        }
                        _ => {}
                    }
                }
                // Yield to prevent busy-loop
                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
            } => {}
        }
    }
}

async fn handle_client_msg(
    msg: ClientMsg,
    state: &Arc<AppState>,
    pane_subs: &mut std::collections::HashMap<String, tokio::sync::broadcast::Receiver<bytes::Bytes>>,
) -> Option<ServerMsg> {
    match msg {
        ClientMsg::Subscribe { pane } => {
            debug!("Client subscribing to pane {}", pane);
            let sender = state.get_or_create_pane_channel(&pane);
            pane_subs.insert(pane, sender.subscribe());
            None
        }

        ClientMsg::Unsubscribe { pane } => {
            pane_subs.remove(&pane);
            None
        }

        ClientMsg::Resize { pane, cols, rows } => {
            debug!("Resize pane {} to {}x{}", pane, cols, rows);
            // TODO: propagate to PTY manager and tmux
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

        ClientMsg::Input { pane: _, data: _ } => {
            // TODO: route to PTY writer for the pane
            None
        }

        ClientMsg::TmuxCommand { command: _ } => {
            // TODO: send via tmux control mode client
            None
        }

        ClientMsg::Signal { peer_id, payload } => {
            // Forward WebRTC signaling to the target peer (agent or browser)
            Some(ServerMsg::Signal { peer_id, payload })
        }

        ClientMsg::ClaudeInput { session_id: _, prompt: _ } => {
            // TODO: inject prompt into running claude process
            None
        }
    }
}

/// REST endpoint: GET /api/ice-config?user=<id>
/// Returns TURN credentials for the requesting browser
pub async fn ice_config_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let user_id = params.get("user").cloned().unwrap_or_else(|| "anonymous".to_string());

    match generate_turn_credentials(&state.config.coturn, &user_id) {
        Ok(creds) => Json(build_ice_config(&creds)).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        )
            .into_response(),
    }
}
