//! QUIC server for the agent — accepts connections from browsers (P2P) or the relay server.
//! Speaks the same MessagePack protocol as the server's WebSocket handler.

use anyhow::{Context, Result};
use bytes::Bytes;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;
use tracing::{info, warn};

use crate::tmux_manager::TmuxManager;

pub async fn run(
    port: u16,
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    tmux_mgr: Arc<TmuxManager>,
    agent_secret: String,
) -> Result<()> {
    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)?,
    ));

    let addr = format!("0.0.0.0:{}", port).parse()?;
    let endpoint = quinn::Endpoint::server(server_config, addr)?;

    info!("QUIC agent listener on :{}", port);

    while let Some(incoming) = endpoint.accept().await {
        let mgr = tmux_mgr.clone();
        let secret = agent_secret.clone();
        tokio::spawn(async move {
            match incoming.await {
                Ok(conn) => {
                    info!("Client connected: {}", conn.remote_address());
                    if let Err(e) = handle_connection(conn, mgr, secret).await {
                        warn!("Connection error: {}", e);
                    }
                }
                Err(e) => warn!("Incoming QUIC error: {}", e),
            }
        });
    }

    Ok(())
}

async fn handle_connection(
    conn: quinn::Connection,
    tmux_mgr: Arc<TmuxManager>,
    agent_secret: String,
) -> Result<()> {
    // Accept bidirectional stream for control
    let (mut send, mut recv) = conn
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

    // Verify JWT if agent secret is configured
    if !agent_secret.is_empty() {
        let validation = jsonwebtoken::Validation::default();
        let key = jsonwebtoken::DecodingKey::from_secret(agent_secret.as_bytes());
        let _claims = jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation)
            .context("JWT verification failed")?;
    }

    info!("Client authenticated");

    // Send auth OK
    let auth_ok = rmp_serde::to_vec_named(&serde_json::json!({"t": "auth_ok"}))?;
    send.write_all(&auth_ok).await?;

    // Message loop
    let mut msg_buf = vec![0u8; 65536];

    loop {
        // Drain pane outputs
        for entry in tmux_mgr.pane_outputs.iter() {
            let pane_id = entry.key().clone();
            let tx = entry.value();
            let mut rx = tx.subscribe();
            while let Ok(data) = rx.try_recv() {
                let msg = serde_json::json!({
                    "t": "o",
                    "pane": pane_id,
                    "data": data.to_vec(),
                });
                if let Ok(encoded) = rmp_serde::to_vec_named(&msg) {
                    let _ = send.write_all(&encoded).await;
                }
            }
        }

        tokio::select! {
            biased;

            result = recv.read(&mut msg_buf) => {
                match result {
                    Ok(Some(n)) if n > 0 => {
                        if let Err(e) = handle_message(&msg_buf[..n], &tmux_mgr, &mut send).await {
                            warn!("Message handling error: {}", e);
                        }
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

            _ = tokio::time::sleep(tokio::time::Duration::from_millis(10)) => {}
        }
    }

    Ok(())
}

async fn handle_message(
    data: &[u8],
    tmux_mgr: &TmuxManager,
    send: &mut quinn::SendStream,
) -> Result<()> {
    let msg: serde_json::Value = rmp_serde::from_slice(data)
        .context("failed to decode message")?;

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
            let encoded = rmp_serde::to_vec_named(&reply)?;
            send.write_all(&encoded).await?;
        }

        "sub" => {
            let pane = msg.get("pane").and_then(|v| v.as_str()).unwrap_or("");
            let _ = tmux_mgr.get_or_create_pane_channel(pane);
            info!(pane = %pane, "client subscribed to pane");
        }

        "i" => {
            let pane = msg.get("pane").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(data) = msg.get("data") {
                let bytes: Vec<u8> = if let Some(arr) = data.as_array() {
                    arr.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect()
                } else if let Some(s) = data.as_str() {
                    s.as_bytes().to_vec()
                } else {
                    vec![]
                };
                let _ = tmux_mgr.send_input(pane, &bytes);
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
            let encoded = rmp_serde::to_vec_named(&reply)?;
            send.write_all(&encoded).await?;
        }

        "sess_list" => {
            let reply = serde_json::json!({
                "t": "sess_list",
                "sessions": []
            });
            let encoded = rmp_serde::to_vec_named(&reply)?;
            send.write_all(&encoded).await?;
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
