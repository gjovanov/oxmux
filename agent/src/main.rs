mod claude;
mod pty;
mod quic;
mod tmux_manager;

use anyhow::Result;
use std::sync::Arc;
use tracing::info;

use tmux_manager::TmuxManager;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "oxmux_agent=info".to_string()),
        )
        .init();

    let quic_port: u16 = std::env::var("AGENT_QUIC_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4433);

    let agent_secret = std::env::var("OXMUX_AGENT_SECRET").unwrap_or_default();

    // Install rustls crypto provider
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    info!(
        "oxmux-agent v{} starting, QUIC port {}",
        env!("CARGO_PKG_VERSION"),
        quic_port
    );

    let tmux_mgr = Arc::new(TmuxManager::new());

    // TLS cert paths
    let cert_path = std::env::var("AGENT_CERT_PATH")
        .unwrap_or_else(|_| "/etc/oxmux-agent-cert.pem".to_string());
    let key_path = std::env::var("AGENT_KEY_PATH")
        .unwrap_or_else(|_| "/etc/oxmux-agent-key.pem".to_string());

    if !std::path::Path::new(&cert_path).exists() || !std::path::Path::new(&key_path).exists() {
        // Generate self-signed cert and write to default paths
        info!("No cert files found, generating self-signed");
        let (certs, key) = quic::tls::self_signed_cert()?;
        let cert_pem: String = certs.iter().map(|c| {
            let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, c.as_ref());
            format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", b64)
        }).collect();
        let key_der = match &key {
            rustls::pki_types::PrivateKeyDer::Pkcs8(k) => k.secret_pkcs8_der(),
            _ => anyhow::bail!("unsupported self-signed key format"),
        };
        let key_pem = format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key_der));
        std::fs::write(&cert_path, &cert_pem)?;
        std::fs::write(&key_path, &key_pem)?;
    }

    info!("Using TLS cert: {}, key: {}", cert_path, key_path);

    // Start WebTransport listener
    quic::server::run(quic_port, &cert_path, &key_path, tmux_mgr, agent_secret).await?;

    Ok(())
}
