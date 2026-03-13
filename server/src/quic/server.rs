use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};

use crate::state::AppState;

/// QUIC listener for oxmux-agent connections.
/// Each connected agent registers its available panes and
/// proxies PTY output over QUIC streams instead of SSH.
///
/// Transport: Quinn (QUIC) with rustls TLS 1.3
/// Frame format: same MessagePack protocol as WebSocket
pub async fn run(state: Arc<AppState>) -> Result<()> {
    let port = state.config.quic.listen_port;
    let cert_path = &state.config.quic.cert_path;
    let key_path = &state.config.quic.key_path;

    // Load TLS certificate
    let cert_chain = load_certs(cert_path)?;
    let key = load_private_key(key_path)?;

    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)?
    ));

    let addr = format!("0.0.0.0:{}", port).parse()?;
    let endpoint = quinn::Endpoint::server(server_config, addr)?;

    info!("QUIC listener on :{}", port);

    while let Some(incoming) = endpoint.accept().await {
        let state = state.clone();
        tokio::spawn(async move {
            match incoming.await {
                Ok(conn) => {
                    info!("Agent connected via QUIC: {}", conn.remote_address());
                    if let Err(e) = handle_agent_connection(conn, state).await {
                        warn!("Agent connection error: {}", e);
                    }
                }
                Err(e) => warn!("QUIC connection error: {}", e),
            }
        });
    }

    Ok(())
}

async fn handle_agent_connection(
    conn: quinn::Connection,
    _state: Arc<AppState>,
) -> Result<()> {
    // Each QUIC stream = one pane subscription
    // Stream 0: control (register panes, resize, commands)
    // Stream N: pane output (binary PTY bytes)
    loop {
        match conn.accept_bi().await {
            Ok((_send, _recv)) => {
                // TODO: handle bidirectional stream per pane
            }
            Err(quinn::ConnectionError::ApplicationClosed { .. }) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn load_certs(path: &str) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let cert_file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(cert_file);
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(certs)
}

fn load_private_key(path: &str) -> Result<rustls::pki_types::PrivateKeyDer<'static>> {
    let key_file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut reader)?
        .ok_or_else(|| anyhow::anyhow!("no private key found in {}", path))?;
    Ok(key)
}
