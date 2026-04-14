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

/// Pane output message — uses Bytes for data so rmp_serde encodes it as
/// MessagePack binary (bin type), matching the server's ServerMsg::Output.
#[derive(serde::Serialize)]
struct PaneOutput {
    t: &'static str,
    pane: String,
    data: Bytes,
}

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

    // Set window-size to 'manual' so tmux never auto-sizes windows based on
    // attached clients. Without this, the control mode client at 80x24 caps
    // all pane sizes. With 'manual', resize-pane commands work unconditionally.
    {
        let socket = find_tmux_socket();
        let mut cmd = std::process::Command::new("tmux");
        if let Some(ref s) = socket { cmd.arg("-S").arg(s); }
        cmd.args(["set-option", "-g", "window-size", "manual"]);
        let _ = cmd.output();
        let mut cmd2 = std::process::Command::new("tmux");
        if let Some(ref s) = socket { cmd2.arg("-S").arg(s); }
        cmd2.args(["set-option", "-g", "aggressive-resize", "on"]);
        let _ = cmd2.output();
    }

    // Spawn tmux -CC attach for live output streaming
    let (output_tx, mut output_rx) = mpsc::channel::<(String, Bytes)>(512);
    let ctrl_session_name = session_name.to_string();

    // Store WebRTC peer connection + pending candidates for ICE handling
    let webrtc_pc: Arc<tokio::sync::Mutex<Option<Arc<webrtc::peer_connection::RTCPeerConnection>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let webrtc_remote_set: Arc<std::sync::atomic::AtomicBool> =
        Arc::new(std::sync::atomic::AtomicBool::new(false));
    let webrtc_pending_candidates: Arc<tokio::sync::Mutex<Vec<webrtc::ice_transport::ice_candidate::RTCIceCandidateInit>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    // Shared DataChannel for sending output when WebRTC is active
    let webrtc_dc: Arc<tokio::sync::Mutex<Option<Arc<webrtc::data_channel::RTCDataChannel>>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    // Channel for DataChannel messages → main event loop (so all msg types are handled)
    let (dc_msg_tx, mut dc_msg_rx) = mpsc::channel::<Vec<u8>>(256);

    // Shared control mode stdin — for sending tmux commands (resize etc.)
    // through the control mode channel instead of spawning external tmux processes.
    // This is the same approach the server's SSH transport uses (write_control_command).
    let ctrl_stdin: Arc<tokio::sync::Mutex<Option<tokio::process::ChildStdin>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    // Wrap control mode in a restart loop
    let ctrl_stdin_clone = ctrl_stdin.clone();
    let ctrl_handle = tokio::spawn(async move {
        loop {
            info!(session = %ctrl_session_name, "starting control mode");
            match run_control_mode(&ctrl_session_name, output_tx.clone(), ctrl_stdin_clone.clone()).await {
                Ok(()) => {
                    info!(session = %ctrl_session_name, "control mode exited, restarting in 100ms");
                }
                Err(e) => {
                    warn!(session = %ctrl_session_name, error = %e, "control mode error, restarting in 100ms");
                }
            }
            // Fast restart — resize-window causes brief control mode detach,
            // need to reattach quickly to catch SIGWINCH redraw output
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });

    // Message loop
    let mut stream_buf = Vec::new();
    let mut read_buf = vec![0u8; 65536];
    // Track subscribed panes — only forward output for panes the client subscribed to.
    // This matches the server's behavior (pane_subs HashMap). Without this, the agent
    // sends ALL control mode output immediately (including stale buffered TUI renders
    // from before the client connected), which overwrites the terminal with old content
    // at the wrong scroll position.
    let mut subscribed_panes = std::collections::HashSet::<String>::new();
    // Track last-sent dimensions per pane to avoid redundant SIGWINCH.
    // When dimensions change, tmux naturally sends SIGWINCH (one signal).
    // When dimensions DON'T change (reconnect/view switch at same size),
    // we force SIGWINCH via run-shell. Without tracking, every resize
    // triggers run-shell SIGWINCH ON TOP of the natural one — double
    // SIGWINCHs during rapid sidebar drag corrupt Claude's scroll state.
    let mut pane_dimensions = std::collections::HashMap::<String, (u16, u16)>::new();

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

            if let Err(e) = handle_message(&msg_data, &tmux_mgr, &mut send, &webrtc_pc, &webrtc_remote_set, &webrtc_pending_candidates, &dc_msg_tx, &webrtc_dc, &ctrl_stdin, &mut subscribed_panes, &mut pane_dimensions).await {
                warn!("Message error: {}", e);
            }
        }

        tokio::select! {
            biased;

            // Forward control mode output to client (QUIC or WebRTC DataChannel)
            output = output_rx.recv() => {
                match output {
                    Some((pane_id, data)) if subscribed_panes.contains(&pane_id) => {
                        // CRITICAL: encode data as MessagePack binary (bin type), NOT as
                        // an array of integers. Using serde_json::json! with data.to_vec()
                        // serializes Vec<u8> as a JSON array → rmp_serde encodes as MsgPack
                        // array → client gets Array<number> instead of Uint8Array → xterm.js
                        // cursor positioning breaks. PaneOutput uses Bytes which serializes
                        // as MsgPack bin via serde_bytes, matching the server's encoding.
                        let reply = PaneOutput { t: "o", pane: pane_id, data };
                        let encoded = rmp_serde::to_vec_named(&reply).unwrap_or_default();

                        // Try DataChannel first (WebRTC P2P), fall back to QUIC stream
                        let sent_via_dc = if let Some(dc) = webrtc_dc.lock().await.as_ref() {
                            if dc.ready_state() == webrtc::data_channel::data_channel_state::RTCDataChannelState::Open {
                                dc.send(&Bytes::from(encoded.clone())).await.is_ok()
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if !sent_via_dc {
                            if send_raw(&mut send, &encoded).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some((_pane_id, _data)) => {
                        // Output for unsubscribed pane — discard silently.
                        // Client hasn't sent 'sub' for this pane yet.
                    }
                    None => {
                        // Control mode exited (e.g., Claude Code exit restores terminal).
                        // Don't break — control mode restarts automatically.
                        // Recreate the channel to receive from the restarted control mode.
                        warn!("control mode channel closed, waiting for restart");
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }

            // Handle messages from WebRTC DataChannel (forwarded via channel)
            dc_data = dc_msg_rx.recv() => {
                if let Some(data) = dc_data {
                    if let Err(e) = handle_message(&data, &tmux_mgr, &mut send, &webrtc_pc, &webrtc_remote_set, &webrtc_pending_candidates, &dc_msg_tx, &webrtc_dc, &ctrl_stdin, &mut subscribed_panes, &mut pane_dimensions).await {
                        warn!("DC message error: {}", e);
                    }
                }
            }

            // Read from client (QUIC stream)
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

    // Close WebRTC peer connection if active (prevent resource leak)
    if let Some(pc) = webrtc_pc.lock().await.take() {
        let _ = pc.close().await;
    }

    Ok(())
}

/// Run tmux -CC attach and parse %output events into the channel.
/// Returns the control mode stdin writer for sending tmux commands.
async fn run_control_mode(
    session_name: &str,
    tx: mpsc::Sender<(String, Bytes)>,
    ctrl_stdin: Arc<tokio::sync::Mutex<Option<tokio::process::ChildStdin>>>,
) -> Result<()> {
    use tokio::io::AsyncBufReadExt;

    // Use user's tmux socket
    let tmux_socket = find_tmux_socket();
    let tmux_cmd = if let Some(ref sock) = tmux_socket {
        format!("tmux -S {} -CC attach -t {}", sock, session_name)
    } else {
        format!("tmux -CC attach -t {}", session_name)
    };
    // Use `script` to provide a PTY (tmux -CC requires a terminal).
    // trap '' WINCH prevents the shell from exiting on SIGWINCH when
    // resize-window changes the tmux window size.
    let wrapper = format!("trap '' WINCH; {}", tmux_cmd);
    info!(cmd = %tmux_cmd, "starting control mode");
    let mut child = tokio::process::Command::new("script")
        .args(["-q", "-c", &wrapper, "/dev/null"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn tmux -CC via script")?;

    // Share stdin with the main loop for sending control commands (resize etc.)
    let stdin = child.stdin.take().context("no stdin")?;
    *ctrl_stdin.lock().await = Some(stdin);

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
    send_raw(send, &encoded).await
}

/// Send pre-encoded length-prefixed bytes.
async fn send_raw(send: &mut wtransport::SendStream, encoded: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let len = (encoded.len() as u32).to_be_bytes();
    send.write_all(&len).await?;
    send.write_all(encoded).await?;
    Ok(())
}

async fn handle_message(
    data: &[u8],
    tmux_mgr: &TmuxManager,
    send: &mut wtransport::SendStream,
    webrtc_pc: &Arc<tokio::sync::Mutex<Option<Arc<webrtc::peer_connection::RTCPeerConnection>>>>,
    webrtc_remote_set: &Arc<std::sync::atomic::AtomicBool>,
    webrtc_pending_candidates: &Arc<tokio::sync::Mutex<Vec<webrtc::ice_transport::ice_candidate::RTCIceCandidateInit>>>,
    dc_msg_tx: &mpsc::Sender<Vec<u8>>,
    webrtc_dc: &Arc<tokio::sync::Mutex<Option<Arc<webrtc::data_channel::RTCDataChannel>>>>,
    ctrl_stdin: &Arc<tokio::sync::Mutex<Option<tokio::process::ChildStdin>>>,
    subscribed_panes: &mut std::collections::HashSet<String>,
    pane_dimensions: &mut std::collections::HashMap<String, (u16, u16)>,
) -> Result<()> {
    let msg: rmpv::Value = rmpv::decode::read_value(&mut &data[..])?;
    let t = get_str(&msg, "t");

    match t {
        "sub" => {
            let pane = get_str(&msg, "pane");
            if is_valid_pane_id(pane) {
                info!(pane = %pane, "client subscribed to pane");
                subscribed_panes.insert(pane.to_string());
                // No SIGWINCH here — the client sends resize after subscribe,
                // and the resize handler forces SIGWINCH at the CORRECT size.
                // Sending SIGWINCH here would cause Claude to redraw at the OLD
                // tmux dimensions (before resize arrives), producing a conflicting
                // double-redraw that shows wrong content.
            }
        }

        "unsub" => {
            let pane = get_str(&msg, "pane");
            if is_valid_pane_id(pane) {
                info!(pane = %pane, "client unsubscribed from pane");
                subscribed_panes.remove(pane);
            }
        }

        "i" => {
            let pane = get_str(&msg, "pane");
            let bytes = get_bytes(&msg, "data");
            send_tmux_input(pane, &bytes, ctrl_stdin).await;
        }

        "r" => {
            let pane = get_str(&msg, "pane");
            let cols = get_u64(&msg, "cols") as u16;
            let rows = get_u64(&msg, "rows") as u16;
            if is_valid_pane_id(pane) && cols > 0 && rows > 0 {
                let dims_changed = pane_dimensions.get(pane) != Some(&(cols, rows));
                pane_dimensions.insert(pane.to_string(), (cols, rows));

                // When dimensions CHANGE: resize-window + resize-pane only.
                // tmux sends exactly ONE natural SIGWINCH. No forced SIGWINCH,
                // because double-SIGWINCH during rapid sidebar drag corrupts
                // Claude Code's internal scroll state (renders from middle).
                //
                // When dimensions DON'T change (reconnect/view switch at same
                // size): force SIGWINCH via run-shell so Claude redraws. Without
                // this, tmux sends no SIGWINCH and the terminal stays empty.
                let cmd = if dims_changed {
                    info!(pane = %pane, cols, rows, "resize (dims changed, natural SIGWINCH)");
                    format!(
                        "resize-window -t {} -x {} -y {}\nresize-pane -t {} -x {} -y {}\n",
                        pane, cols, rows, pane, cols, rows
                    )
                } else {
                    info!(pane = %pane, cols, rows, "resize (same dims, forcing SIGWINCH)");
                    format!(
                        "resize-window -t {} -x {} -y {}\nresize-pane -t {} -x {} -y {}\nrun-shell -t {} \"kill -WINCH #{{pane_pid}}\"\n",
                        pane, cols, rows, pane, cols, rows, pane
                    )
                };
                let mut stdin_lock = ctrl_stdin.lock().await;
                if let Some(ref mut stdin) = *stdin_lock {
                    use tokio::io::AsyncWriteExt;
                    if let Err(e) = stdin.write_all(cmd.as_bytes()).await {
                        warn!(error = %e, "failed to write resize to control mode stdin");
                    } else if let Err(e) = stdin.flush().await {
                        warn!(error = %e, "failed to flush resize to control mode stdin");
                    }
                } else {
                    warn!("control mode stdin not available for resize");
                }
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
            // Browser-as-offerer pattern (like parakeet-rs):
            // Browser sends offer with DataChannel + ICE candidates,
            // agent creates answer with its own candidates.
            let offer_sdp = get_str(&msg, "sdp").to_string();
            let offer_candidates = offer_sdp.matches("a=candidate").count();
            info!(candidates = offer_candidates, "Received WebRTC offer from browser");

            // Clean up previous PC if retrying
            if let Some(old_pc) = webrtc_pc.lock().await.take() {
                info!("Closing previous WebRTC PC for retry");
                let _ = old_pc.close().await;
            }
            *webrtc_dc.lock().await = None;
            webrtc_remote_set.store(false, std::sync::atomic::Ordering::SeqCst);
            webrtc_pending_candidates.lock().await.clear();

            let dc_store = webrtc_dc.clone();
            let dc_tx = dc_msg_tx.clone();
            match create_webrtc_answer(&offer_sdp, dc_store, dc_tx).await {
                Ok((pc, answer_sdp, candidate_count)) => {
                    *webrtc_pc.lock().await = Some(pc);

                    send_msg(send, &serde_json::json!({
                        "t": "webrtc_answer",
                        "sdp": answer_sdp,
                    })).await?;

                    info!(candidates = candidate_count, "WebRTC answer sent (vanilla ICE)");
                }
                Err(e) => {
                    warn!(error = %e, "WebRTC answer creation failed");
                    send_msg(send, &serde_json::json!({
                        "t": "webrtc_error",
                        "error": e.to_string(),
                    })).await?;
                }
            }
        }

        "webrtc_ready" => {
            // Legacy: browser signals readiness. With browser-as-offerer, this is a no-op.
            // The browser will send the offer directly.
            info!("Browser sent webrtc_ready (browser-as-offerer mode — waiting for offer)");
        }

        "webrtc_ice" => {
            // Browser sends candidate as object: {candidate: "...", sdpMid: "0", sdpMLineIndex: 0}
            let candidate_val = msg.as_map()
                .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some("candidate")))
                .map(|(_, v)| v);

            let (candidate_str, sdp_mid) = if let Some(val) = candidate_val {
                if let Some(map) = val.as_map() {
                    // Nested object: extract candidate and sdpMid fields
                    let c = map.iter().find(|(k, _)| k.as_str() == Some("candidate"))
                        .and_then(|(_, v)| v.as_str()).unwrap_or("").to_string();
                    let m = map.iter().find(|(k, _)| k.as_str() == Some("sdpMid"))
                        .and_then(|(_, v)| v.as_str()).unwrap_or("0").to_string();
                    (c, m)
                } else {
                    // Plain string
                    (val.as_str().unwrap_or("").to_string(), "0".to_string())
                }
            } else {
                (String::new(), "0".to_string())
            };

            if !candidate_str.is_empty() {
                let candidate = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
                    candidate: candidate_str.clone(),
                    sdp_mid: Some(sdp_mid),
                    sdp_mline_index: Some(0),
                    ..Default::default()
                };

                if webrtc_remote_set.load(std::sync::atomic::Ordering::SeqCst) {
                    // Remote description set — add immediately
                    info!(candidate = %candidate_str.get(..50).unwrap_or(&candidate_str), "adding ICE candidate");
                    if let Some(pc) = webrtc_pc.lock().await.as_ref() {
                        if let Err(e) = pc.add_ice_candidate(candidate).await {
                            warn!(error = %e, "failed to add ICE candidate");
                        }
                    }
                } else {
                    // Queue until remote description is set
                    info!(candidate = %candidate_str.get(..50).unwrap_or(&candidate_str), "queuing ICE candidate (no remote desc yet)");
                    webrtc_pending_candidates.lock().await.push(candidate);
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

/// Create a WebRTC peer connection as answerer.
/// Browser sends the offer (with DataChannel + candidates),
/// agent creates answer with its own candidates (vanilla ICE).
async fn create_webrtc_answer(
    offer_sdp: &str,
    dc_store: Arc<tokio::sync::Mutex<Option<Arc<webrtc::data_channel::RTCDataChannel>>>>,
    dc_msg_tx: mpsc::Sender<Vec<u8>>,
) -> Result<(Arc<webrtc::peer_connection::RTCPeerConnection>, String, usize)> {
    use webrtc::api::interceptor_registry::register_default_interceptors;
    use webrtc::api::media_engine::MediaEngine;
    use webrtc::api::APIBuilder;
    use webrtc::data_channel::data_channel_message::DataChannelMessage;
    use webrtc::ice_transport::ice_server::RTCIceServer;
    use webrtc::interceptor::registry::Registry;
    use webrtc::peer_connection::configuration::RTCConfiguration;
    use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

    use webrtc::api::setting_engine::SettingEngine;
    use webrtc::ice::network_type::NetworkType;
    use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;

    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)?;

    let mut setting_engine = SettingEngine::default();

    if let Ok(public_ip) = std::env::var("PUBLIC_IP") {
        info!(ip = %public_ip, "setting NAT 1:1 IP mapping (srflx)");
        setting_engine.set_nat_1to1_ips(vec![public_ip], RTCIceCandidateType::Srflx);
    }

    setting_engine.set_network_types(vec![
        NetworkType::Udp4,
        NetworkType::Udp6,
        NetworkType::Tcp4,
        NetworkType::Tcp6,
    ]);

    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting_engine)
        .build();

    // TURN credentials for agent's own STUN/TURN gathering
    let coturn_secret = std::env::var("COTURN_AUTH_SECRET").unwrap_or_default();
    let mut ice_servers = vec![
        RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_string()],
            ..Default::default()
        },
    ];

    if !coturn_secret.is_empty() {
        let ttl = 86400u64;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() + ttl;
        let username = format!("{}:oxmux-agent", timestamp);

        use hmac::{Hmac, Mac};
        use sha1::Sha1;
        type HmacSha1 = Hmac<Sha1>;

        let mut mac = HmacSha1::new_from_slice(coturn_secret.as_bytes())
            .expect("HMAC key");
        mac.update(username.as_bytes());
        let credential = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            mac.finalize().into_bytes(),
        );

        let turn_servers = std::env::var("COTURN_SERVERS")
            .unwrap_or_default();
        let turn_urls: Vec<String> = turn_servers.split(',')
            .map(|s| format!("turn:{}", s.trim()))
            .collect();

        ice_servers.push(RTCIceServer {
            urls: turn_urls,
            username,
            credential,
        });
    }

    let config = RTCConfiguration {
        ice_servers,
        ..Default::default()
    };

    let pc = Arc::new(api.new_peer_connection(config).await?);

    // Wait for ICE gathering complete
    let ice_done = Arc::new(tokio::sync::Notify::new());
    let ice_done_clone = ice_done.clone();

    pc.on_ice_candidate(Box::new(move |candidate| {
        let done = ice_done_clone.clone();
        Box::pin(async move {
            if candidate.is_none() {
                done.notify_one();
            }
        })
    }));

    // Handle DataChannel from browser (browser is offerer, creates DC)
    let dc_store_clone = dc_store.clone();
    pc.on_data_channel(Box::new(move |dc: Arc<webrtc::data_channel::RTCDataChannel>| {
        let store = dc_store_clone.clone();
        let tx = dc_msg_tx.clone();
        Box::pin(async move {
            info!(label = %dc.label(), "DataChannel received from browser");

            // Store DC when it opens
            let store_for_open = store.clone();
            let dc_for_open = dc.clone();
            dc.on_open(Box::new(move || {
                let s = store_for_open.clone();
                let d = dc_for_open.clone();
                Box::pin(async move {
                    info!("WebRTC DataChannel opened (agent side)");
                    *s.lock().await = Some(d);
                })
            }));

            // Clear DC from store on close (stops output routing via DC)
            let store_for_close = store.clone();
            dc.on_close(Box::new(move || {
                let s = store_for_close.clone();
                Box::pin(async move {
                    info!("WebRTC DataChannel closed (agent side)");
                    *s.lock().await = None;
                })
            }));

            // Forward messages to main event loop
            dc.on_message(Box::new(move |msg: DataChannelMessage| {
                let data = msg.data.to_vec();
                let tx = tx.clone();
                Box::pin(async move {
                    if tx.send(data).await.is_err() {
                        debug!("DC message channel closed");
                    }
                })
            }));
        })
    }));

    // Set browser's offer as remote description
    pc.set_remote_description(RTCSessionDescription::offer(offer_sdp.to_string())?).await?;

    // Create answer
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer).await?;

    // Wait for ICE gathering to complete (vanilla ICE on agent side)
    tokio::select! {
        _ = ice_done.notified() => {}
        _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
            warn!("ICE gathering timed out after 10s");
        }
    }

    let local_desc = pc.local_description().await
        .ok_or_else(|| anyhow::anyhow!("no local description after gathering"))?;
    let final_sdp = local_desc.sdp;
    let candidate_count = final_sdp.matches("a=candidate").count();

    info!(candidates = candidate_count, "WebRTC answer created (vanilla ICE)");
    Ok((pc, final_sdp, candidate_count))
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Validate tmux pane ID format (%N where N is a number).
fn is_valid_pane_id(pane: &str) -> bool {
    pane.starts_with('%') && pane.len() > 1 && pane[1..].chars().all(|c| c.is_ascii_digit())
}

/// Send input to a tmux pane via the control-mode stdin pipe.
/// Writes `send-keys -t <pane> -H <hex hex ...>\n` — identical wire format to
/// the server's SSH transport (server/src/session/ssh_transport.rs send_input).
/// Async + non-blocking: never spawns a subprocess. Critical for performance
/// because the previous `std::process::Command::new("tmux").output()` call
/// synchronously blocked the tokio event loop for 5-10ms per keystroke,
/// starving the output forwarder and causing typing lag + scroll corruption.
async fn send_tmux_input(
    pane: &str,
    data: &[u8],
    ctrl_stdin: &Arc<tokio::sync::Mutex<Option<tokio::process::ChildStdin>>>,
) {
    if data.is_empty() || !is_valid_pane_id(pane) { return; }
    let mut cmd = String::with_capacity(24 + data.len() * 3);
    cmd.push_str("send-keys -t ");
    cmd.push_str(pane);
    cmd.push_str(" -H");
    for byte in data {
        use std::fmt::Write;
        let _ = write!(cmd, " {:02x}", byte);
    }
    cmd.push('\n');

    let mut stdin_lock = ctrl_stdin.lock().await;
    if let Some(ref mut stdin) = *stdin_lock {
        use tokio::io::AsyncWriteExt;
        if let Err(e) = stdin.write_all(cmd.as_bytes()).await {
            warn!(error = %e, "failed to write input to control mode stdin");
        } else if let Err(e) = stdin.flush().await {
            warn!(error = %e, "failed to flush input to control mode stdin");
        }
    } else {
        warn!("control mode stdin not available for input");
    }
}

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
