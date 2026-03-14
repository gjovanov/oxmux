//! REST + WS handlers for agent management.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use secrecy::ExposeSecret;
use std::sync::Arc;
use tracing::{info, warn};

use crate::state::AppState;
use super::registry::AgentInfo;

/// GET /api/agents — list all registered agents.
pub async fn list_agents(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let agents = state.agent_registry.list();
    Json(serde_json::json!({ "agents": agents }))
}

/// GET /api/agents/:host/status — check if agent is online for a host.
pub async fn agent_status(
    State(state): State<Arc<AppState>>,
    Path(host): Path<String>,
) -> impl IntoResponse {
    match state.agent_registry.find_by_host(&host) {
        Some(agent) => Json(serde_json::json!({
            "status": "online",
            "agent": agent,
        })).into_response(),
        None => Json(serde_json::json!({
            "status": "not_installed",
        })).into_response(),
    }
}

/// POST /api/agents/:agent_id/token — issue short-lived JWT for browser→agent auth.
pub async fn agent_token(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let agent = match state.agent_registry.get(&agent_id) {
        Some(a) => a,
        None => return (StatusCode::NOT_FOUND, "agent not found").into_response(),
    };

    let agent_secret = state.config.server.jwt_secret.expose_secret();
    match crate::auth::jwt::create_agent_token(&agent_id, agent_secret) {
        Ok(token) => Json(serde_json::json!({
            "token": token,
            "host": agent.host,
            "quic_port": agent.quic_port,
            "expires_in": 300,
        })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
