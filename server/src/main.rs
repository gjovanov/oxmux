mod agent;
mod auth;
mod claude;
mod config;
mod db;
mod pty;
mod quic;
mod session;
mod ssh;
mod state;
mod tmux;
mod webrtc;
mod ws;

use anyhow::Result;
use axum::{Router, routing::{get, post}};
use secrecy::ExposeSecret;
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tower_http::services::{ServeDir, ServeFile};
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

    // Database
    let db_url = config.database.url.expose_secret();
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(db_url)
        .await?;
    db::init(&pool).await?;

    let state = Arc::new(AppState::new(config.clone(), pool).await?);

    // Spawn QUIC listener (server ↔ agent transport)
    let quic_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = quic::server::run(quic_state).await {
            tracing::error!("QUIC agent server error: {}", e);
        }
    });

    // Spawn WebTransport server (browser QUIC transport)
    let wt_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = ws::webtransport::run(wt_state).await {
            tracing::error!("WebTransport server error: {}", e);
        }
    });

    let spa = ServeDir::new("static").not_found_service(ServeFile::new("static/index.html"));

    let app = Router::new()
        // Auth
        .route("/api/auth/register", post(auth::handler::register))
        .route("/api/auth/login", post(auth::handler::login))
        .route("/api/auth/me", get(auth::handler::me))
        // WebSocket (authenticated via ?token=)
        .route("/ws", get(ws::handler::ws_handler))
        // Public
        .route("/api/ice-config", get(ws::handler::ice_config_handler))
        .route("/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
        .fallback_service(spa);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
