mod claude;
mod config;
mod pty;
mod quic;
mod ssh;
mod state;
mod tmux;
mod webrtc;
mod ws;

use anyhow::Result;
use axum::{Router, routing::get};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load()?;

    // Tracing
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.server.log_level)),
        )
        .json()
        .init();

    info!("Starting Oxmux server v{}", env!("CARGO_PKG_VERSION"));

    let state = Arc::new(AppState::new(config.clone()).await?);

    // Spawn QUIC listener (server ↔ agent transport)
    let quic_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = quic::server::run(quic_state).await {
            tracing::error!("QUIC server error: {}", e);
        }
    });

    let app = Router::new()
        .route("/ws", get(ws::handler::ws_handler))
        .route("/api/ice-config", get(ws::handler::ice_config_handler))
        .route("/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
