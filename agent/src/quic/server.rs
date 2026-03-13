use anyhow::Result;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;
use tracing::{info, warn};

pub async fn run(
    port: u16,
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
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
        tokio::spawn(async move {
            match incoming.await {
                Ok(conn) => {
                    info!("Relay server connected: {}", conn.remote_address());
                    if let Err(e) = handle_relay(conn).await {
                        warn!("Relay connection error: {}", e);
                    }
                }
                Err(e) => warn!("Incoming QUIC error: {}", e),
            }
        });
    }

    Ok(())
}

async fn handle_relay(conn: quinn::Connection) -> Result<()> {
    // Stream 0: control channel (pane list, resize, commands)
    // Stream N+: one stream per pane subscription
    loop {
        match conn.accept_bi().await {
            Ok((_send, _recv)) => {
                // TODO: dispatch to PTY manager based on control messages
            }
            Err(quinn::ConnectionError::ApplicationClosed { .. }) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
