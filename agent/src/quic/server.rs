//! QUIC/WebTransport server for the agent.
//! Accepts connections from browsers (P2P via WebTransport) or the relay server (raw QUIC).

use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::{info, warn};
use wtransport::{Endpoint, Identity, ServerConfig};

use crate::tmux_manager::TmuxManager;

pub async fn run(
    port: u16,
    cert_path: &str,
    key_path: &str,
    tmux_mgr: Arc<TmuxManager>,
    agent_secret: String,
) -> Result<()> {
    let identity = Identity::load_pemfiles(cert_path, key_path).await
        .context("failed to load TLS identity")?;

    let config = ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(identity)
        .build();

    let endpoint = Endpoint::server(config)?;

    info!("WebTransport agent listener on :{}", port);

    loop {
        let incoming = endpoint.accept().await;
        let mgr = tmux_mgr.clone();
        let secret = agent_secret.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_session(incoming, mgr, secret).await {
                warn!("Session error: {}", e);
            }
        });
    }
}

async fn handle_session(
    incoming: wtransport::endpoint::IncomingSession,
    tmux_mgr: Arc<TmuxManager>,
    agent_secret: String,
) -> Result<()> {
    let session_request = incoming.await?;

    info!(
        authority = %session_request.authority(),
        path = %session_request.path(),
        "WebTransport session request"
    );

    let session = session_request.accept().await?;

    // Accept bidirectional stream
    let (mut send, mut recv) = session
        .accept_bi()
        .await
        .context("failed to accept bi stream")?;

    // Auth: first message must contain a JWT token
    let mut auth_buf = vec![0u8; 4096];
    let n = recv
        .read(&mut auth_buf)
        .await?
        .ok_or_else(|| anyhow::anyhow!("stream closed before auth"))?;

    let auth_msg: serde_json::Value = rmp_serde::from_slice(&auth_buf[..n])
        .or_else(|_| serde_json::from_slice(&auth_buf[..n]))
        .context("failed to decode auth message")?;

    let token = auth_msg
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing token"))?;

    // Verify JWT if secret is configured
    if !agent_secret.is_empty() {
        let validation = jsonwebtoken::Validation::default();
        let key = jsonwebtoken::DecodingKey::from_secret(agent_secret.as_bytes());
        let _claims = jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation)
            .context("JWT verification failed")?;
    }

    info!("Client authenticated");

    // Send auth OK (raw msgpack, no length prefix for auth handshake)
    let auth_ok = rmp_serde::to_vec_named(&serde_json::json!({"t": "auth_ok"}))?;
    send.write_all(&auth_ok).await?;

    // After auth, all messages are length-prefixed (client → agent and agent → client)

    // Message loop — read length-prefixed messages (4-byte big-endian + msgpack)
    let mut stream_buf = Vec::new();
    let mut read_buf = vec![0u8; 65536];

    loop {
        // Process any complete messages in the buffer
        while stream_buf.len() >= 4 {
            let msg_len = u32::from_be_bytes([stream_buf[0], stream_buf[1], stream_buf[2], stream_buf[3]]) as usize;
            if stream_buf.len() < 4 + msg_len {
                break; // Wait for more data
            }
            let msg_data = stream_buf[4..4 + msg_len].to_vec();
            stream_buf.drain(..4 + msg_len);

            if let Err(e) = handle_message(&msg_data, &tmux_mgr, &mut send).await {
                warn!("Message error: {}", e);
            }
        }

        // Read more data from the stream
        match recv.read(&mut read_buf).await {
            Ok(Some(n)) if n > 0 => {
                stream_buf.extend_from_slice(&read_buf[..n]);
            }
            Ok(_) => {
                info!("Client disconnected");
                break;
            }
            Err(e) => {
                warn!("Read error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Send a length-prefixed msgpack message.
async fn send_response(send: &mut wtransport::SendStream, msg: &serde_json::Value) -> Result<()> {
    let encoded = rmp_serde::to_vec_named(msg)?;
    let len = (encoded.len() as u32).to_be_bytes();
    send.write_all(&len).await?;
    send.write_all(&encoded).await?;
    Ok(())
}

async fn handle_message(
    data: &[u8],
    tmux_mgr: &TmuxManager,
    send: &mut wtransport::SendStream,
) -> Result<()> {
    let msg: serde_json::Value = rmp_serde::from_slice(data)
        .with_context(|| format!(
            "failed to decode message ({} bytes, first: {:?})",
            data.len(), &data[..data.len().min(20)]
        ))?;

    let t = msg.get("t").and_then(|v| v.as_str()).unwrap_or("");

    match t {
        "sess_connect" | "sess_create" => {
            let name = msg.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("default");

            tmux_mgr.ensure_session(name)?;
            let panes = tmux_mgr.list_panes(name)?;

            let reply = serde_json::json!({
                "t": "sess_connected",
                "session": {
                    "id": uuid::Uuid::new_v4().to_string(),
                    "name": name,
                    "status": "connected",
                    "tmux_sessions": [{
                        "id": panes.first().map(|p| &p.session_id).unwrap_or(&String::new()),
                        "name": name,
                        "windows": build_window_tree(&panes),
                    }],
                }
            });
            send_response(send, &reply).await?;
        }

        "sub" => {
            let pane = msg.get("pane").and_then(|v| v.as_str()).unwrap_or("").to_string();
            info!(pane = %pane, "client subscribed to pane");

            // Start capturing pane output via tmux capture-pane polling
            // (control mode would be better but this is simpler for P2P)
            let pane_clone = pane.clone();
            let send_clone = send as *mut wtransport::SendStream;
            // We can't easily share the send stream, so for now just send initial capture
            let capture = std::process::Command::new("tmux")
                .args(["capture-pane", "-t", &pane, "-p", "-e"])
                .output();
            if let Ok(output) = capture {
                if output.status.success() && !output.stdout.is_empty() {
                    let reply = serde_json::json!({
                        "t": "o",
                        "pane": pane,
                        "data": output.stdout,
                    });
                    let _ = send_response(send, &reply).await;
                }
            }
        }

        "i" => {
            let pane = msg.get("pane").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(data) = msg.get("data") {
                let bytes: Vec<u8> = if let Some(arr) = data.as_array() {
                    arr.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect()
                } else if let Some(s) = data.as_str() {
                    s.as_bytes().to_vec()
                } else if data.is_object() {
                    // MessagePack Binary type decoded as {"type": "Buffer", "data": [...]}
                    if let Some(arr) = data.get("data").and_then(|d| d.as_array()) {
                        arr.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect()
                    } else {
                        vec![]
                    }
                } else {
                    // Try to get raw bytes from serde_json Value
                    serde_json::to_vec(data).unwrap_or_default()
                };
                if !bytes.is_empty() {
                    info!(pane = %pane, bytes = bytes.len(), "sending input");
                    let _ = tmux_mgr.send_input(pane, &bytes);

                    // After input, capture and send updated output
                    let capture = std::process::Command::new("tmux")
                        .args(["capture-pane", "-t", pane, "-p", "-e"])
                        .output();
                    if let Ok(output) = capture {
                        if output.status.success() && !output.stdout.is_empty() {
                            let reply = serde_json::json!({
                                "t": "o",
                                "pane": pane,
                                "data": output.stdout,
                            });
                            let _ = send_response(send, &reply).await;
                        }
                    }
                }
            }
        }

        "r" => {
            let pane = msg.get("pane").and_then(|v| v.as_str()).unwrap_or("");
            let cols = msg.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;
            let rows = msg.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
            let _ = tmux_mgr.resize_pane(pane, cols, rows);
        }

        "ping" => {
            let ts = msg.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
            let reply = serde_json::json!({"t": "pong", "ts": ts});
            send_response(send, &reply).await?;
        }

        "sess_list" => {
            let reply = serde_json::json!({"t": "sess_list", "sessions": []});
            send_response(send, &reply).await?;
        }

        other => {
            warn!(msg_type = %other, "unhandled message type");
        }
    }

    Ok(())
}

fn build_window_tree(panes: &[crate::tmux_manager::PaneInfo]) -> Vec<serde_json::Value> {
    use std::collections::BTreeMap;

    let mut windows: BTreeMap<String, Vec<&crate::tmux_manager::PaneInfo>> = BTreeMap::new();
    for pane in panes {
        windows.entry(pane.window_id.clone()).or_default().push(pane);
    }

    windows
        .into_iter()
        .map(|(wid, panes)| {
            let first = panes.first().unwrap();
            serde_json::json!({
                "id": wid,
                "name": first.window_name,
                "index": first.window_index,
                "layout": first.layout,
                "panes": panes.iter().map(|p| serde_json::json!({
                    "id": p.pane_id,
                    "index": p.pane_index,
                    "cols": p.cols,
                    "rows": p.rows,
                    "current_command": p.current_command,
                    "is_active": p.is_active,
                    "is_claude": p.is_claude,
                })).collect::<Vec<_>>(),
            })
        })
        .collect()
}
