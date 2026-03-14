use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use quinn::{ClientConfig, Endpoint};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, error, info};

use crate::ws::protocol::TmuxSessionInfo;

use super::transport::{Transport, TransportStatus};

/// QUIC transport — connects to an oxmux-agent running on a remote machine.
///
/// Protocol over QUIC:
/// - Stream 0 (bidirectional): control channel — JSON commands/responses
/// - Stream N (bidirectional): per-pane PTY I/O — raw binary bytes
///
/// Control commands:
/// - `{"cmd":"list-sessions"}`  → `{"sessions":[...]}`
/// - `{"cmd":"send-input","pane":"...","data":"..."}`
/// - `{"cmd":"resize","pane":"...","cols":N,"rows":N}`
/// - `{"cmd":"tmux","args":"..."}`  → `{"output":"..."}`
pub struct QuicTransport {
    host: String,
    port: u16,
    session_name: String,
    status: TransportStatus,
    /// Quinn connection to the agent
    connection: Arc<Mutex<Option<quinn::Connection>>>,
    /// Control stream for sending commands
    control_send: Arc<Mutex<Option<quinn::SendStream>>>,
    control_recv: Arc<Mutex<Option<quinn::RecvStream>>>,
    /// Per-pane broadcast channels
    pane_channels: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    /// Cached tmux state
    tmux_state: Arc<Mutex<Vec<TmuxSessionInfo>>>,
}

impl QuicTransport {
    pub fn new(host: String, port: u16, session_name: String) -> Self {
        Self {
            host,
            port,
            session_name,
            status: TransportStatus::Disconnected,
            connection: Arc::new(Mutex::new(None)),
            control_send: Arc::new(Mutex::new(None)),
            control_recv: Arc::new(Mutex::new(None)),
            pane_channels: Arc::new(DashMap::new()),
            tmux_state: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Send a control command and read the response.
    async fn send_control_command(&self, cmd: &serde_json::Value) -> Result<serde_json::Value> {
        let mut send_lock = self.control_send.lock().await;
        let send = send_lock
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("QUIC control stream not open"))?;

        // Write length-prefixed JSON
        let payload = serde_json::to_vec(cmd)?;
        let len = (payload.len() as u32).to_be_bytes();
        send.write_all(&len).await?;
        send.write_all(&payload).await?;

        // Read response
        let mut recv_lock = self.control_recv.lock().await;
        let recv = recv_lock
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("QUIC control recv not open"))?;

        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;

        let mut resp_buf = vec![0u8; resp_len];
        recv.read_exact(&mut resp_buf).await?;

        let resp: serde_json::Value = serde_json::from_slice(&resp_buf)?;
        Ok(resp)
    }

    /// Spawn a background task to read pane output from a dedicated QUIC stream.
    fn spawn_pane_reader(
        mut recv: quinn::RecvStream,
        pane_id: String,
        tx: broadcast::Sender<Bytes>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match recv.read(&mut buf).await {
                    Ok(Some(n)) => {
                        let _ = tx.send(Bytes::copy_from_slice(&buf[..n]));
                    }
                    Ok(None) => {
                        debug!(pane = %pane_id, "QUIC pane stream ended");
                        break;
                    }
                    Err(e) => {
                        error!(pane = %pane_id, error = %e, "QUIC pane read error");
                        break;
                    }
                }
            }
        })
    }
}

#[async_trait]
impl Transport for QuicTransport {
    async fn connect(&mut self) -> Result<()> {
        self.status = TransportStatus::Connecting;

        let addr: SocketAddr = format!("{}:{}", self.host, self.port)
            .parse()
            .context("invalid QUIC address")?;

        info!(addr = %addr, "connecting via QUIC to oxmux-agent");

        // Configure QUIC client with self-signed cert support
        let crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_no_client_auth();

        let client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
                .context("failed to create QUIC crypto config")?,
        ));

        let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
        endpoint.set_default_client_config(client_config);

        let connection = endpoint
            .connect(addr, &self.host)?
            .await
            .context("QUIC connect failed")?;

        info!(addr = %addr, "QUIC connection established");

        // Open control stream (stream 0)
        let (send, recv) = connection
            .open_bi()
            .await
            .context("failed to open QUIC control stream")?;

        *self.connection.lock().await = Some(connection);
        *self.control_send.lock().await = Some(send);
        *self.control_recv.lock().await = Some(recv);

        // Create remote tmux session
        let create_cmd = serde_json::json!({
            "cmd": "ensure-session",
            "name": self.session_name,
        });
        let _ = self.send_control_command(&create_cmd).await;

        // Query initial state
        let state = self.list_tmux_sessions().await?;

        // Set up pane output streams
        for session in &state {
            for window in &session.windows {
                for pane in &window.panes {
                    let (tx, _) = broadcast::channel(1024);
                    self.pane_channels.insert(pane.id.clone(), tx);
                }
            }
        }

        self.status = TransportStatus::Connected;
        info!(addr = %addr, session = %self.session_name, "QUIC transport connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(conn) = self.connection.lock().await.take() {
            conn.close(0u32.into(), b"closing");
        }
        *self.control_send.lock().await = None;
        *self.control_recv.lock().await = None;
        self.pane_channels.clear();
        self.status = TransportStatus::Disconnected;
        info!(host = %self.host, "QUIC transport disconnected");
        Ok(())
    }

    async fn list_tmux_sessions(&self) -> Result<Vec<TmuxSessionInfo>> {
        let cmd = serde_json::json!({
            "cmd": "list-sessions",
            "name": self.session_name,
        });
        let resp = self.send_control_command(&cmd).await?;

        let sessions: Vec<TmuxSessionInfo> = serde_json::from_value(
            resp.get("sessions")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![])),
        )?;

        *self.tmux_state.lock().await = sessions.clone();
        Ok(sessions)
    }

    async fn send_input(&self, pane_id: &str, data: &[u8]) -> Result<()> {
        let cmd = serde_json::json!({
            "cmd": "send-input",
            "pane": pane_id,
            "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data),
        });
        self.send_control_command(&cmd).await?;
        Ok(())
    }

    async fn resize_pane(&self, pane_id: &str, cols: u16, rows: u16) -> Result<()> {
        let cmd = serde_json::json!({
            "cmd": "resize",
            "pane": pane_id,
            "cols": cols,
            "rows": rows,
        });
        self.send_control_command(&cmd).await?;
        Ok(())
    }

    async fn run_tmux_command(&self, cmd: &str) -> Result<String> {
        let req = serde_json::json!({
            "cmd": "tmux",
            "args": cmd,
        });
        let resp = self.send_control_command(&req).await?;
        Ok(resp
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    fn subscribe_output(&self, pane_id: &str) -> Option<broadcast::Receiver<Bytes>> {
        self.pane_channels.get(pane_id).map(|tx| tx.subscribe())
    }

    fn status(&self) -> TransportStatus {
        self.status
    }
}

/// Accept any server certificate (for self-signed agent certs).
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
        ]
    }
}
