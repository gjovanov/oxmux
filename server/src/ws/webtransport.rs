//! WebTransport (QUIC) handler for browser connections.
//!
//! Provides Transport #2 (QUIC → SSH) and #4 (QUIC → Agent).
//! Uses the same MessagePack protocol as WebSocket, carried over QUIC bidirectional streams.

use anyhow::{Context, Result};
use secrecy::ExposeSecret;
use std::sync::Arc;

use tracing::{info, warn};
use wtransport::endpoint::IncomingSession;
use wtransport::{Endpoint, Identity, ServerConfig};

use crate::auth::jwt;
use crate::state::AppState;
use crate::ws::protocol::{decode_client_msg, encode_server_msg, ServerMsg};
use crate::ws::session_handler::{self, ConnectionState};

/// Start the WebTransport server on the configured QUIC port.
pub async fn run(state: Arc<AppState>) -> Result<()> {
    let port = state.config.quic.listen_port;
    let cert_path = &state.config.quic.cert_path;
    let key_path = &state.config.quic.key_path;

    // Load TLS identity
    let identity = Identity::load_pemfiles(cert_path, key_path)
        .await
        .context(format!(
            "failed to load TLS certs from {} and {}",
            cert_path, key_path
        ))?;

    let config = ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(identity)
        .build();

    let endpoint = Endpoint::server(config)?;

    info!(port, "WebTransport server listening");

    loop {
        let incoming = endpoint.accept().await;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_incoming(incoming, state).await {
                warn!(error = %e, "WebTransport session error");
            }
        });
    }
}

async fn handle_incoming(incoming: IncomingSession, state: Arc<AppState>) -> Result<()> {
    let session_request = incoming.await?;

    info!(
        authority = %session_request.authority(),
        path = %session_request.path(),
        "WebTransport session request"
    );

    let session = session_request.accept().await?;

    // Open a bidirectional stream for the control channel
    let (mut send, mut recv) = session
        .accept_bi()
        .await
        .context("failed to accept bidirectional stream")?;

    // First message must be auth: {"t": "auth", "token": "<jwt>"}
    let mut auth_buf = vec![0u8; 4096];
    let n = recv
        .read(&mut auth_buf)
        .await
        .context("failed to read auth message")?
        .ok_or_else(|| anyhow::anyhow!("stream closed before auth"))?;

    let auth_msg: serde_json::Value = rmp_serde::from_slice(&auth_buf[..n])
        .or_else(|_| serde_json::from_slice(&auth_buf[..n]))
        .context("failed to decode auth message")?;

    let token = auth_msg
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing token in auth message"))?;

    let jwt_secret = state.config.server.jwt_secret.expose_secret();
    let claims =
        jwt::validate_token(token, jwt_secret).context("WebTransport auth failed")?;

    let user_id = claims.sub;
    info!(user_id = %user_id, "WebTransport client authenticated");

    // Send auth OK
    let auth_ok = rmp_serde::to_vec_named(&serde_json::json!({"t": "auth_ok"}))?;
    send.write_all(&(auth_ok.len() as u32).to_be_bytes())
        .await?;
    send.write_all(&auth_ok).await?;

    // Now handle messages like WebSocket — same shared handler
    let mut conn = ConnectionState::new(user_id.clone());

    // Load user sessions
    match state.session_manager.load_user_sessions(&user_id).await {
        Ok(sessions) => {
            let msg = ServerMsg::SessionList {
                sessions: sessions.iter().map(|s| s.sanitized()).collect(),
            };
            if let Ok(encoded) = encode_server_msg(&msg) {
                let len = (encoded.len() as u32).to_be_bytes();
                let _ = send.write_all(&len).await;
                let _ = send.write_all(&encoded).await;
            }
        }
        Err(e) => warn!(error = %e, "failed to load user sessions"),
    }

    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(5));
    let mut msg_buf = vec![0u8; 65536]; // 64KB message buffer

    loop {
        // Drain pane output
        for frame in session_handler::drain_pane_outputs(&mut conn) {
            let len = (frame.len() as u32).to_be_bytes();
            if send.write_all(&len).await.is_err() || send.write_all(&frame).await.is_err() {
                info!(user_id = %user_id, "WebTransport client disconnected (write error)");
                return Ok(());
            }
        }

        tokio::select! {
            biased;

            result = recv.read(&mut msg_buf) => {
                match result {
                    Ok(Some(n)) if n > 0 => {
                        match decode_client_msg(&msg_buf[..n]) {
                            Ok(client_msg) => {
                                if let Some(reply) = session_handler::handle_client_msg(
                                    client_msg, &state, &mut conn
                                ).await {
                                    if let Ok(encoded) = encode_server_msg(&reply) {
                                        let len = (encoded.len() as u32).to_be_bytes();
                                        let _ = send.write_all(&len).await;
                                        let _ = send.write_all(&encoded).await;
                                    }
                                }
                            }
                            Err(e) => warn!("Failed to decode WT message: {}", e),
                        }
                    }
                    Ok(_) => {
                        info!(user_id = %user_id, "WebTransport client disconnected");
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, "WebTransport read error");
                        break;
                    }
                }
            }

            _ = interval.tick() => {}
        }
    }

    Ok(())
}
