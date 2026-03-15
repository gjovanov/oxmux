//! Shared session message handling logic used by both WebSocket and WebTransport handlers.

use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use anyhow::Context;
use secrecy::ExposeSecret;
use tracing::{debug, info, warn};

use crate::state::AppState;
use super::protocol::{ClientMsg, ServerMsg, decode_client_msg, encode_server_msg};

/// Per-connection state shared across transport handlers.
pub struct ConnectionState {
    pub user_id: String,
    pub pane_subs: HashMap<String, broadcast::Receiver<Bytes>>,
    /// Channel for background tasks to send async messages to the client.
    pub async_sender: tokio::sync::mpsc::Sender<ServerMsg>,
}

impl ConnectionState {
    pub fn new(user_id: String, async_sender: tokio::sync::mpsc::Sender<ServerMsg>) -> Self {
        Self {
            user_id,
            pane_subs: HashMap::new(),
            async_sender,
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

        // ── Agent management ────────────────────────────────────────

        ClientMsg::AgentStatusRequest { host } => {
            // Check registry first
            if let Some(agent) = state.agent_registry.find_by_host(&host) {
                return Some(ServerMsg::AgentStatus {
                    session_id: String::new(),
                    host,
                    status: "online".to_string(),
                    agent_id: Some(agent.id),
                    version: Some(agent.version),
                    quic_port: Some(agent.quic_port),
                });
            }

            // Not in registry — return not_installed (agent will be detected on install)
            return Some(ServerMsg::AgentStatus {
                session_id: String::new(),
                host,
                status: "not_installed".to_string(),
                agent_id: None,
                version: None,
                quic_port: None,
            });

            // unreachable — returned above
            None
        }

        ClientMsg::InstallAgent { session_id } => {
            // Look up session to get SSH credentials and host
            let session = match state.session_manager.get(&session_id) {
                Some(s) => s,
                None => return Some(ServerMsg::Error {
                    code: "session_not_found".to_string(),
                    message: format!("session '{}' not found", session_id),
                }),
            };

            let (host, port, user, auth) = match &session.transport.backend {
                crate::session::types::BackendTransport::Ssh { host, port, user, auth } => {
                    (host.clone(), *port, user.clone(), auth.clone())
                }
                _ => return Some(ServerMsg::Error {
                    code: "not_ssh_session".to_string(),
                    message: "agent install requires an SSH session".to_string(),
                }),
            };

            let async_tx = conn.async_sender.clone();
            let session_id_clone = session_id.clone();
            let host_clone = host.clone();
            let state_clone = state.clone();

            // Spawn background deploy task
            tokio::spawn(async move {
                info!(host = %host_clone, "starting agent deployment");

                // Send "installing" status
                let _ = async_tx.send(ServerMsg::AgentStatus {
                    session_id: session_id_clone.clone(),
                    host: host_clone.clone(),
                    status: "installing".to_string(),
                    agent_id: None, version: None, quic_port: Some(4433),
                }).await;

                // Connect via SSH and deploy
                let config = std::sync::Arc::new(russh::client::Config::default());
                let handler = crate::agent::SshDeployHandler;
                let addr = format!("{}:{}", host_clone, port);

                let deploy_result = async {
                    let mut ssh = russh::client::connect(config, &addr, handler).await?;

                    // Authenticate
                    match &auth {
                        crate::session::types::SshAuthConfig::PrivateKey { path, passphrase } => {
                            let expanded = if path.starts_with("~/") {
                                let home = std::env::var("HOME").unwrap_or_else(|_| "/home/oxmux".to_string());
                                format!("{}/{}", home, &path[2..])
                            } else {
                                path.clone()
                            };
                            let key_data = tokio::fs::read_to_string(&expanded).await?;

                            // Handle encrypted keys
                            let key_data = if key_data.contains("DES-EDE3-CBC") || key_data.contains("DES-CBC") {
                                let pass = passphrase.as_deref().unwrap_or("");
                                let output = tokio::process::Command::new("openssl")
                                    .args(["rsa", "-in", &expanded, "-passin", &format!("pass:{}", pass), "-traditional"])
                                    .output().await?;
                                if !output.status.success() {
                                    anyhow::bail!("openssl key conversion failed");
                                }
                                String::from_utf8(output.stdout)?
                            } else {
                                key_data
                            };

                            let decode_pass = if key_data.starts_with("-----BEGIN PRIVATE KEY-----") { None } else { passphrase.as_deref() };
                            let key_pair = russh_keys::decode_secret_key(&key_data, decode_pass)?;
                            ssh.authenticate_publickey(&user, std::sync::Arc::new(key_pair)).await?;
                        }
                        crate::session::types::SshAuthConfig::Password { password } => {
                            ssh.authenticate_password(&user, password).await?;
                        }
                        crate::session::types::SshAuthConfig::Agent => {
                            ssh.authenticate_none(&user).await?;
                        }
                    }

                    let jwt_secret = state_clone.config.server.jwt_secret.expose_secret().to_string();
                    let deploy_res = crate::agent::deployer::deploy_via_ssh(&ssh, &host_clone, port, &user, &auth, &jwt_secret, 4433).await;
                    deploy_res.map(|r| (r, ssh))
                }.await;

                match deploy_result {
                    Ok((result, ssh_handle)) => {
                        info!(host = %host_clone, "agent deployed, checking status...");
                        let _ = async_tx.send(ServerMsg::AgentStatus {
                            session_id: session_id_clone.clone(),
                            host: host_clone.clone(),
                            status: "starting".to_string(),
                            agent_id: None,
                            version: Some("0.1.0".to_string()),
                            quic_port: Some(result.quic_port),
                        }).await;

                        // Check agent via SSH (more reliable than QUIC probe through K8s network)
                        let mut online = false;
                        for attempt in 1..=5 {
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            let check = crate::agent::deployer::exec_ssh_command(
                                &ssh_handle, "systemctl is-active oxmux-agent 2>/dev/null"
                            ).await;
                            match check {
                                Ok(output) if output.trim() == "active" => {
                                    online = true;
                                    info!(host = %host_clone, attempt, "agent is active (systemctl)");
                                    break;
                                }
                                _ => {
                                    debug!(host = %host_clone, attempt, "agent not ready yet");
                                }
                            }
                        }

                        if online {
                            let agent_id = uuid::Uuid::new_v4().to_string();

                            // Register in agent registry
                            state_clone.agent_registry.register(crate::agent::registry::AgentInfo {
                                id: agent_id.clone(),
                                hostname: host_clone.clone(),
                                host: host_clone.clone(),
                                quic_port: result.quic_port,
                                version: "0.1.0".to_string(),
                                last_seen: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs(),
                            });

                            let _ = async_tx.send(ServerMsg::AgentStatus {
                                session_id: session_id_clone,
                                host: host_clone,
                                status: "online".to_string(),
                                agent_id: Some(agent_id),
                                version: Some("0.1.0".to_string()),
                                quic_port: Some(result.quic_port),
                            }).await;
                        } else {
                            warn!(host = %host_clone, "agent started but not responding to QUIC probe");
                            let _ = async_tx.send(ServerMsg::AgentStatus {
                                session_id: session_id_clone,
                                host: host_clone,
                                status: "error".to_string(),
                                agent_id: None,
                                version: None,
                                quic_port: Some(result.quic_port),
                            }).await;
                        }
                    }
                    Err(e) => {
                        warn!(host = %host_clone, error = %e, "agent deploy failed");
                        let _ = async_tx.send(ServerMsg::AgentStatus {
                            session_id: session_id_clone,
                            host: host_clone,
                            status: "error".to_string(),
                            agent_id: None,
                            version: None,
                            quic_port: None,
                        }).await;
                    }
                }
            });

            // Return immediate acknowledgment
            Some(ServerMsg::AgentStatus {
                session_id,
                host,
                status: "installing".to_string(),
                agent_id: None,
                version: None,
                quic_port: Some(4433),
            })
        }

        ClientMsg::TransportUpgrade { session_id, target } => {
            let session = match state.session_manager.get(&session_id) {
                Some(s) => s,
                None => return Some(ServerMsg::TransportUpgradeFailed {
                    session_id,
                    error: "session not found".to_string(),
                }),
            };

            // Get the host from the session's backend transport
            let host = match &session.transport.backend {
                crate::session::types::BackendTransport::Ssh { host, .. } => host.clone(),
                crate::session::types::BackendTransport::Agent { host, .. } => host.clone(),
                _ => return Some(ServerMsg::TransportUpgradeFailed {
                    session_id,
                    error: "local sessions don't support P2P".to_string(),
                }),
            };

            // Find agent for this host
            let agent = match state.agent_registry.find_by_host(&host) {
                Some(a) => a,
                None => return Some(ServerMsg::TransportUpgradeFailed {
                    session_id,
                    error: format!("no agent online for host {}", host),
                }),
            };

            // Issue short-lived token for browser→agent auth
            use secrecy::ExposeSecret;
            let secret = state.config.server.jwt_secret.expose_secret();
            match crate::auth::jwt::create_agent_token(&agent.id, secret) {
                Ok(token) => {
                    info!(
                        session_id = %session_id,
                        target = %target,
                        agent_id = %agent.id,
                        "transport upgrade ready"
                    );
                    let agent_hostname = "agent.oxmux.app".to_string();
                    Some(ServerMsg::TransportUpgradeReady {
                        session_id,
                        agent_host: agent_hostname,
                        agent_port: agent.quic_port,
                        agent_token: token,
                        target: Some(target),
                    })
                }
                Err(e) => Some(ServerMsg::TransportUpgradeFailed {
                    session_id,
                    error: e.to_string(),
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
