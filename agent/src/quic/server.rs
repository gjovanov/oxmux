//! QUIC/WebTransport server for the agent.
//! Accepts P2P connections from browsers via WebTransport.
//! Uses tmux control mode for live PTY streaming (same as server's SSH transport).

use anyhow::{Context, Result};
use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
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

    // Note: keep_alive and idle_timeout are configured at the quinn level
    // wtransport handles this internally

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
    info!(authority = %session_request.authority(), "WebTransport session request");

    let session = session_request.accept().await?;
    let (mut send, mut recv) = session.accept_bi().await.context("failed to accept bi stream")?;

    // Auth
    let mut auth_buf = vec![0u8; 4096];
    let n = recv.read(&mut auth_buf).await?.ok_or_else(|| anyhow::anyhow!("closed before auth"))?;
    let auth_msg: rmpv::Value = rmpv::decode::read_value(&mut &auth_buf[..n])?;
    let token = get_str(&auth_msg, "token");

    if !agent_secret.is_empty() {
        let key = jsonwebtoken::DecodingKey::from_secret(agent_secret.as_bytes());
        jsonwebtoken::decode::<serde_json::Value>(token, &key, &jsonwebtoken::Validation::default())
            .context("JWT verification failed")?;
    }

    info!("Client authenticated");
    let auth_ok = rmp_serde::to_vec_named(&serde_json::json!({"t": "auth_ok"}))?;
    send.write_all(&auth_ok).await?;

    // Wait for client to tell us which tmux session to use
    // Read the first message which should be sess_connect with a name
    let mut first_msg_buf = vec![0u8; 65536];
    let mut first_buf = Vec::new();

    // Read length-prefixed first message
    loop {
        match recv.read(&mut first_msg_buf).await {
            Ok(Some(n)) if n > 0 => {
                first_buf.extend_from_slice(&first_msg_buf[..n]);
                if first_buf.len() >= 4 {
                    let msg_len = u32::from_be_bytes([first_buf[0], first_buf[1], first_buf[2], first_buf[3]]) as usize;
                    if first_buf.len() >= 4 + msg_len {
                        break;
                    }
                }
            }
            _ => anyhow::bail!("connection closed before first message"),
        }
    }

    // Parse first message to get session name
    let msg_len = u32::from_be_bytes([first_buf[0], first_buf[1], first_buf[2], first_buf[3]]) as usize;
    let first_msg: rmpv::Value = rmpv::decode::read_value(&mut &first_buf[4..4 + msg_len])?;
    let session_name = get_str(&first_msg, "name");
    let session_name = if session_name.is_empty() { "default" } else { session_name };

    info!(session = %session_name, "attaching to tmux session");
    tmux_mgr.ensure_session(session_name)?;

    // Spawn tmux -CC attach for live output streaming
    let (output_tx, mut output_rx) = mpsc::channel::<(String, Bytes)>(512);
    let ctrl_session_name = session_name.to_string();

    // Store WebRTC peer connection for ICE candidate handling
    let webrtc_pc: Arc<tokio::sync::Mutex<Option<Arc<webrtc::peer_connection::RTCPeerConnection>>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    let ctrl_handle = tokio::spawn(async move {
        if let Err(e) = run_control_mode(&ctrl_session_name, output_tx).await {
            warn!(error = %e, "control mode ended");
        }
    });

    // Message loop
    let mut stream_buf = Vec::new();
    let mut read_buf = vec![0u8; 65536];

    // Send initial pane list
    let panes = tmux_mgr.list_panes(session_name).unwrap_or_default();
    let reply = serde_json::json!({
        "t": "sess_connected",
        "session": {
            "id": uuid::Uuid::new_v4().to_string(),
            "name": session_name,
            "status": "connected",
            "tmux_sessions": [{
                "id": panes.first().map(|p| p.session_id.as_str()).unwrap_or(""),
                "name": session_name,
                "windows": build_window_tree(&panes),
            }],
        }
    });
    send_msg(&mut send, &reply).await?;

    loop {
        // Process buffered messages
        while stream_buf.len() >= 4 {
            let msg_len = u32::from_be_bytes([stream_buf[0], stream_buf[1], stream_buf[2], stream_buf[3]]) as usize;
            if stream_buf.len() < 4 + msg_len { break; }
            let msg_data = stream_buf[4..4 + msg_len].to_vec();
            stream_buf.drain(..4 + msg_len);

            if let Err(e) = handle_message(&msg_data, &tmux_mgr, &mut send, &webrtc_pc).await {
                warn!("Message error: {}", e);
            }
        }

        tokio::select! {
            biased;

            // Forward control mode output to client
            output = output_rx.recv() => {
                match output {
                    Some((pane_id, data)) => {
                        let reply = serde_json::json!({
                            "t": "o",
                            "pane": pane_id,
                            "data": data.to_vec(),
                        });
                        if send_msg(&mut send, &reply).await.is_err() {
                            break;
                        }
                    }
                    None => {
                        warn!("control mode channel closed");
                        break;
                    }
                }
            }

            // Read from client
            result = recv.read(&mut read_buf) => {
                match result {
                    Ok(Some(n)) if n > 0 => {
                        stream_buf.extend_from_slice(&read_buf[..n]);
                    }
                    Ok(_) => { info!("Client disconnected"); break; }
                    Err(e) => { warn!("Read error: {}", e); break; }
                }
            }
        }
    }

    ctrl_handle.abort();
    Ok(())
}

/// Run tmux -CC attach and parse %output events into the channel.
async fn run_control_mode(
    session_name: &str,
    tx: mpsc::Sender<(String, Bytes)>,
) -> Result<()> {
    use tokio::io::AsyncBufReadExt;

    // Use user's tmux socket
    let tmux_socket = find_tmux_socket();
    let tmux_cmd = if let Some(ref sock) = tmux_socket {
        format!("tmux -S {} -CC attach -t {}", sock, session_name)
    } else {
        format!("tmux -CC attach -t {}", session_name)
    };
    info!(cmd = %tmux_cmd, "starting control mode");

    let mut child = tokio::process::Command::new("script")
        .args(["-q", "-c", &tmux_cmd, "/dev/null"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn tmux -CC via script")?;

    let stdout = child.stdout.take().context("no stdout")?;
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim_end_matches('\r');

        if let Some(rest) = line.strip_prefix("%output ") {
            if let Some((pane_id, data)) = rest.split_once(' ') {
                let decoded = decode_tmux_output(data);
                if tx.send((pane_id.to_string(), Bytes::from(decoded))).await.is_err() {
                    break;
                }
            }
        }
        // Ignore other control mode events for now
    }

    let _ = child.kill().await;
    Ok(())
}

/// Decode tmux octal-escaped output.
fn decode_tmux_output(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // Check for octal escape \ooo
            let mut octal = String::new();
            for _ in 0..3 {
                if let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() && c < '8' {
                        octal.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            if !octal.is_empty() {
                if let Ok(byte) = u8::from_str_radix(&octal, 8) {
                    out.push(byte);
                }
            } else if let Some(&next) = chars.peek() {
                match next {
                    '\\' => { out.push(b'\\'); chars.next(); }
                    _ => out.push(b'\\'),
                }
            }
        } else {
            let mut buf = [0u8; 4];
            let bytes = ch.encode_utf8(&mut buf);
            out.extend_from_slice(bytes.as_bytes());
        }
    }

    out
}

/// Send a length-prefixed msgpack message.
async fn send_msg(send: &mut wtransport::SendStream, msg: &serde_json::Value) -> Result<()> {
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
    webrtc_pc: &Arc<tokio::sync::Mutex<Option<Arc<webrtc::peer_connection::RTCPeerConnection>>>>,
) -> Result<()> {
    let msg: rmpv::Value = rmpv::decode::read_value(&mut &data[..])?;
    let t = get_str(&msg, "t");

    match t {
        "sub" => {
            let pane = get_str(&msg, "pane");
            info!(pane = %pane, "client subscribed to pane");
        }

        "i" => {
            let pane = get_str(&msg, "pane");
            let bytes = get_bytes(&msg, "data");
            if !bytes.is_empty() && !pane.is_empty() {
                let hex: String = bytes.iter().map(|b| format!("{:02x} ", b)).collect();
                let socket = find_tmux_socket();
                let mut cmd = std::process::Command::new("tmux");
                if let Some(ref s) = socket { cmd.arg("-S").arg(s); }
                cmd.args(["send-keys", "-t", pane, "-H", hex.trim()]);
                let _ = cmd.output();
            }
        }

        "r" => {
            let pane = get_str(&msg, "pane");
            let cols = get_u64(&msg, "cols") as u16;
            let rows = get_u64(&msg, "rows") as u16;
            if !pane.is_empty() && cols > 0 && rows > 0 {
                let _ = tmux_mgr.resize_pane(pane, cols, rows);
            }
        }

        "ping" => {
            let ts = get_u64(&msg, "ts");
            send_msg(send, &serde_json::json!({"t": "pong", "ts": ts})).await?;
        }

        "sess_list" => {
            send_msg(send, &serde_json::json!({"t": "sess_list", "sessions": []})).await?;
        }

        "webrtc_offer" => {
            // Browser wants to establish a WebRTC DataChannel P2P connection.
            // The SDP offer arrives over the existing WebTransport connection.
            let sdp = get_str(&msg, "sdp").to_string();
            let peer_id = get_str(&msg, "peer_id").to_string();

            info!(peer_id = %peer_id, "WebRTC offer received, creating peer connection");

            let tmux_clone = tmux_mgr.pane_outputs.clone();
            match create_webrtc_peer(&sdp, tmux_clone).await {
                Ok((pc, answer_sdp, ice_candidates)) => {
                    // Store the peer connection for ICE candidate handling
                    *webrtc_pc.lock().await = Some(pc);
                    // Send answer back over WebTransport
                    send_msg(send, &serde_json::json!({
                        "t": "webrtc_answer",
                        "peer_id": peer_id,
                        "sdp": answer_sdp,
                    })).await?;

                    // Send gathered ICE candidates
                    for candidate in ice_candidates {
                        send_msg(send, &serde_json::json!({
                            "t": "webrtc_ice",
                            "peer_id": peer_id,
                            "candidate": candidate,
                        })).await?;
                    }
                }
                Err(e) => {
                    warn!(error = %e, "WebRTC peer creation failed");
                    send_msg(send, &serde_json::json!({
                        "t": "webrtc_error",
                        "peer_id": peer_id,
                        "error": e.to_string(),
                    })).await?;
                }
            }
        }

        "webrtc_ice" => {
            let candidate_str = get_str(&msg, "candidate").to_string();
            let sdp_mid = get_str(&msg, "sdp_mid").to_string();
            info!(candidate = %candidate_str, "received ICE candidate from browser");

            if let Some(pc) = webrtc_pc.lock().await.as_ref() {
                let candidate = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
                    candidate: candidate_str,
                    sdp_mid: Some(if sdp_mid.is_empty() { "0".to_string() } else { sdp_mid }),
                    sdp_mline_index: Some(0),
                    ..Default::default()
                };
                if let Err(e) = pc.add_ice_candidate(candidate).await {
                    warn!(error = %e, "failed to add ICE candidate");
                }
            }
        }

        other => {
            if !other.is_empty() {
                debug!(msg_type = %other, "unhandled");
            }
        }
    }

    Ok(())
}

// ── WebRTC P2P ──────────────────────────────────────────────────────────

/// Create a WebRTC peer connection with DataChannel for terminal I/O.
async fn create_webrtc_peer(
    offer_sdp: &str,
    pane_outputs: Arc<dashmap::DashMap<String, tokio::sync::broadcast::Sender<Bytes>>>,
) -> Result<(Arc<webrtc::peer_connection::RTCPeerConnection>, String, Vec<String>)> {
    use webrtc::api::interceptor_registry::register_default_interceptors;
    use webrtc::api::media_engine::MediaEngine;
    use webrtc::api::APIBuilder;
    use webrtc::data_channel::data_channel_message::DataChannelMessage;
    use webrtc::data_channel::RTCDataChannel;
    use webrtc::ice_transport::ice_server::RTCIceServer;
    use webrtc::interceptor::registry::Registry;
    use webrtc::peer_connection::configuration::RTCConfiguration;
    use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)?;

    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();

    let config = RTCConfiguration {
        ice_servers: vec![
            RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let pc = Arc::new(api.new_peer_connection(config).await?);

    // Collect ICE candidates
    let (ice_tx, mut ice_rx) = mpsc::channel::<String>(32);
    let ice_done = Arc::new(tokio::sync::Notify::new());
    let ice_done_clone = ice_done.clone();

    pc.on_ice_candidate(Box::new(move |candidate| {
        let tx = ice_tx.clone();
        let done = ice_done_clone.clone();
        Box::pin(async move {
            match candidate {
                Some(c) => {
                    if let Ok(json) = c.to_json() {
                        let _ = tx.send(json.candidate).await;
                    }
                }
                None => done.notify_one(), // gathering complete
            }
        })
    }));

    // Handle DataChannel from browser
    pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
        let pane_outputs = pane_outputs.clone();
        Box::pin(async move {
            info!(label = %dc.label(), "WebRTC DataChannel opened");

            dc.on_message(Box::new(move |msg: DataChannelMessage| {
                let data = msg.data.to_vec();
                Box::pin(async move {
                    // Decode msgpack and handle (input, subscribe, etc.)
                    match rmpv::decode::read_value(&mut &data[..]) {
                        Ok(val) => {
                            let t = val.as_map()
                                .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some("t")))
                                .and_then(|(_, v)| v.as_str())
                                .unwrap_or("");

                            if t == "i" {
                                let pane = val.as_map()
                                    .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some("pane")))
                                    .and_then(|(_, v)| v.as_str())
                                    .unwrap_or("");
                                let input = val.as_map()
                                    .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some("data")))
                                    .map(|(_, v)| match v {
                                        rmpv::Value::Binary(b) => b.clone(),
                                        rmpv::Value::String(s) => s.as_bytes().to_vec(),
                                        _ => vec![],
                                    })
                                    .unwrap_or_default();

                                if !input.is_empty() {
                                    let hex: String = input.iter().map(|b| format!("{:02x} ", b)).collect();
                                    let _ = std::process::Command::new("tmux")
                                        .args(["send-keys", "-t", pane, "-H", hex.trim()])
                                        .output();
                                }
                            }
                        }
                        Err(e) => debug!("DataChannel decode error: {}", e),
                    }
                })
            }));
        })
    }));

    // Set remote description (browser's offer)
    let offer = RTCSessionDescription::offer(offer_sdp.to_string())?;
    pc.set_remote_description(offer).await?;

    // Create answer
    let answer = pc.create_answer(None).await?;
    let answer_sdp = answer.sdp.clone();
    pc.set_local_description(answer).await?;

    // Wait for ICE gathering to complete (with timeout)
    tokio::select! {
        _ = ice_done.notified() => {}
        _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
            warn!("ICE gathering timed out");
        }
    }

    // Collect candidates
    let mut candidates = Vec::new();
    while let Ok(c) = ice_rx.try_recv() {
        candidates.push(c);
    }

    info!(candidates = candidates.len(), "WebRTC peer created");
    Ok((pc, answer_sdp, candidates))
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn get_str<'a>(v: &'a rmpv::Value, key: &str) -> &'a str {
    v.as_map()
        .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some(key)))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("")
}

fn get_bytes(v: &rmpv::Value, key: &str) -> Vec<u8> {
    v.as_map()
        .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some(key)))
        .map(|(_, v)| match v {
            rmpv::Value::Binary(b) => b.clone(),
            rmpv::Value::String(s) => s.as_bytes().to_vec(),
            _ => vec![],
        })
        .unwrap_or_default()
}

fn get_u64(v: &rmpv::Value, key: &str) -> u64 {
    v.as_map()
        .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some(key)))
        .and_then(|(_, v)| v.as_u64())
        .unwrap_or(0)
}

/// Find the user's tmux socket (prefer non-root).
fn find_tmux_socket() -> Option<String> {
    let mut sockets = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("tmux-") {
                let sock = format!("/tmp/{}/default", name);
                if std::path::Path::new(&sock).exists() {
                    // Extract uid from tmux-<uid>
                    let uid: u32 = name.strip_prefix("tmux-")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    sockets.push((uid, sock));
                }
            }
        }
    }
    // Prefer non-root (uid > 0) socket
    sockets.sort_by_key(|(uid, _)| if *uid == 0 { u32::MAX } else { *uid });
    sockets.into_iter().next().map(|(_, s)| s)
}

fn build_window_tree(panes: &[crate::tmux_manager::PaneInfo]) -> Vec<serde_json::Value> {
    use std::collections::BTreeMap;
    let mut windows: BTreeMap<String, Vec<&crate::tmux_manager::PaneInfo>> = BTreeMap::new();
    for pane in panes { windows.entry(pane.window_id.clone()).or_default().push(pane); }
    windows.into_iter().map(|(wid, panes)| {
        let first = panes.first().unwrap();
        serde_json::json!({
            "id": wid, "name": first.window_name, "index": first.window_index,
            "layout": first.layout,
            "panes": panes.iter().map(|p| serde_json::json!({
                "id": p.pane_id, "index": p.pane_index, "cols": p.cols, "rows": p.rows,
                "current_command": p.current_command, "is_active": p.is_active, "is_claude": p.is_claude,
            })).collect::<Vec<_>>(),
        })
    }).collect()
}
