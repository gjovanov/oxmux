use axum::{
    extract::{Query, State, WebSocketUpgrade},
    response::{IntoResponse, Json, Response},
};
use secrecy::ExposeSecret;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::{
    auth::jwt,
    state::AppState,
    webrtc::turn::{build_ice_config, generate_turn_credentials},
    ws::protocol::{decode_client_msg, encode_server_msg, ServerMsg},
    ws::session_handler::{self, ConnectionState},
};

/// WebSocket upgrade endpoint: GET /ws?token=<jwt>
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let token = match params.get("token") {
        Some(t) => t.clone(),
        None => return (axum::http::StatusCode::UNAUTHORIZED, "missing token").into_response(),
    };

    let jwt_secret = state.config.server.jwt_secret.expose_secret();
    let claims = match jwt::validate_token(&token, jwt_secret) {
        Ok(c) => c,
        Err(_) => return (axum::http::StatusCode::UNAUTHORIZED, "invalid token").into_response(),
    };

    let user_id = claims.sub;

    ws.max_message_size(2 * 1024 * 1024)
        .on_upgrade(move |socket| handle_socket(socket, state, user_id))
}

async fn handle_socket(socket: axum::extract::ws::WebSocket, state: Arc<AppState>, user_id: String) {
    use axum::extract::ws::Message;
    use futures_util::{SinkExt, StreamExt};

    let (mut tx, mut rx) = socket.split();

    let mut conn = ConnectionState::new(user_id.clone());

    info!(user_id = %user_id, "WebSocket client connected");

    // Load user's persisted sessions and send them
    match state.session_manager.load_user_sessions(&user_id).await {
        Ok(sessions) => {
            let msg = ServerMsg::SessionList {
                sessions: sessions.iter().map(|s| s.sanitized()).collect(),
            };
            if let Ok(encoded) = encode_server_msg(&msg) {
                let _ = tx.send(Message::Binary(encoded.into())).await;
            }
        }
        Err(e) => warn!(error = %e, "failed to load user sessions"),
    }

    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(5));

    loop {
        // Drain pending pane output
        for frame in session_handler::drain_pane_outputs(&mut conn) {
            let _ = tx.send(Message::Binary(frame.into())).await;
        }

        tokio::select! {
            biased;

            msg = rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        match decode_client_msg(&data) {
                            Ok(client_msg) => {
                                if let Some(reply) = session_handler::handle_client_msg(
                                    client_msg, &state, &mut conn
                                ).await {
                                    if let Ok(encoded) = encode_server_msg(&reply) {
                                        let _ = tx.send(Message::Binary(encoded.into())).await;
                                    }
                                }
                            }
                            Err(e) => warn!("Failed to decode client message: {}", e),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!(user_id = %user_id, "WebSocket client disconnected");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            _ = interval.tick() => {}
        }
    }
}

/// REST endpoint: GET /api/ice-config?user=<id>
pub async fn ice_config_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let user_id = params.get("user").cloned().unwrap_or_else(|| "anonymous".to_string());

    match generate_turn_credentials(&state.config.coturn, &user_id) {
        Ok(creds) => Json(build_ice_config(&creds)).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
