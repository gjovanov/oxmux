//! SSH key upload endpoint — accepts private keys from browser,
//! stores them in ephemeral memory only (never written to disk or DB).

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use crate::auth::{handler::extract_token, jwt};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct UploadKeyRequest {
    /// PEM-encoded SSH private key
    pub key_pem: String,
    /// Optional passphrase for encrypted keys
    #[serde(default)]
    pub passphrase: Option<String>,
}

#[derive(Serialize)]
pub struct UploadKeyResponse {
    pub key_id: String,
}

/// POST /api/ssh-keys — upload an SSH private key to ephemeral memory.
///
/// The key is validated, stored in-memory with a UUID reference,
/// and never persisted to disk or database. Lost on server restart.
pub async fn upload_ssh_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    Json(req): Json<UploadKeyRequest>,
) -> impl IntoResponse {
    // Authenticate
    let token = match extract_token(&headers, &params) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "missing token").into_response(),
    };
    let jwt_secret = state.config.server.jwt_secret.expose_secret();
    if let Err(_) = jwt::validate_token(&token, jwt_secret) {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }

    // Validate size (8KB max — SSH keys are typically 1-3KB)
    if req.key_pem.len() > 8192 {
        return (StatusCode::PAYLOAD_TOO_LARGE, "key too large (max 8KB)").into_response();
    }

    if req.key_pem.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty key").into_response();
    }

    // Validate it's a parseable SSH private key
    let passphrase_str = req.passphrase.as_deref();
    let decode_pass = if req.key_pem.starts_with("-----BEGIN PRIVATE KEY-----") {
        None
    } else {
        passphrase_str
    };
    if let Err(e) = russh_keys::decode_secret_key(&req.key_pem, decode_pass) {
        warn!(error = %e, "uploaded SSH key validation failed");
        return (StatusCode::BAD_REQUEST, "invalid or unsupported SSH key format").into_response();
    }

    // Cap total ephemeral keys to prevent memory exhaustion
    const MAX_EPHEMERAL_KEYS: usize = 100;
    if state.session_manager.ephemeral_keys.len() >= MAX_EPHEMERAL_KEYS {
        return (StatusCode::TOO_MANY_REQUESTS, "too many uploaded keys — delete unused sessions first").into_response();
    }

    // Store in ephemeral memory
    let key_id = uuid::Uuid::new_v4().to_string();
    let passphrase_secret = req.passphrase.map(|p| SecretString::new(p.into()));
    state.session_manager.ephemeral_keys.insert(
        key_id.clone(),
        (SecretString::new(req.key_pem.into()), passphrase_secret),
    );

    info!(key_id = %key_id, "SSH key uploaded to ephemeral store");

    Json(UploadKeyResponse { key_id }).into_response()
}
