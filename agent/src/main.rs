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

    // Generate self-signed cert for QUIC
    let (cert, key) = quic::tls::self_signed_cert()?;

    // Start QUIC listener
    quic::server::run(quic_port, cert, key, tmux_mgr, agent_secret).await?;

    Ok(())
}
