mod pty;
mod quic;
mod claude;

use anyhow::Result;
use tracing::info;

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

    info!("oxmux-agent v{} starting, QUIC port {}", env!("CARGO_PKG_VERSION"), quic_port);

    // Generate self-signed cert for QUIC (or load from env)
    let (cert, key) = quic::tls::self_signed_cert()?;

    // Start QUIC listener — relay server connects here
    quic::server::run(quic_port, cert, key).await?;

    Ok(())
}
