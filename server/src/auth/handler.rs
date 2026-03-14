use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

use crate::db::repo;
use crate::state::AppState;

use super::jwt;

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
}

/// POST /api/auth/register
pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AuthRequest>,
) -> impl IntoResponse {
    let username = req.username.trim();
    if username.is_empty() || req.password.len() < 4 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "username required, password must be at least 4 characters"
        }))).into_response();
    }

    // Check if user exists
    match repo::find_user_by_username(&state.db, username).await {
        Ok(Some(_)) => {
            return (StatusCode::CONFLICT, Json(serde_json::json!({
                "error": "username already taken"
            }))).into_response();
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
        Ok(None) => {}
    }

    // Hash password
    let password_hash = match hash_password(&req.password) {
        Ok(h) => h,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let user_id = uuid::Uuid::new_v4().to_string();

    if let Err(e) = repo::create_user(&state.db, &user_id, username, &password_hash).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    let jwt_secret = state.config.server.jwt_secret.expose_secret();
    let token = match jwt::create_token(&user_id, username, jwt_secret) {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    info!(username = %username, "user registered");

    Json(AuthResponse {
        token,
        user: UserInfo { id: user_id, username: username.to_string() },
    }).into_response()
}

/// POST /api/auth/login
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AuthRequest>,
) -> impl IntoResponse {
    let user = match repo::find_user_by_username(&state.db, &req.username).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
                "error": "invalid credentials"
            }))).into_response();
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    if !verify_password(&req.password, &user.password_hash) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "invalid credentials"
        }))).into_response();
    }

    let jwt_secret = state.config.server.jwt_secret.expose_secret();
    let token = match jwt::create_token(&user.id, &user.username, jwt_secret) {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    info!(username = %user.username, "user logged in");

    Json(AuthResponse {
        token,
        user: UserInfo { id: user.id, username: user.username },
    }).into_response()
}

/// GET /api/auth/me
pub async fn me(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let token = extract_token(&headers, &params);
    let token = match token {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "missing token").into_response(),
    };

    let jwt_secret = state.config.server.jwt_secret.expose_secret();
    let claims = match jwt::validate_token(&token, jwt_secret) {
        Ok(c) => c,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid token").into_response(),
    };

    Json(UserInfo { id: claims.sub, username: claims.username }).into_response()
}

/// Extract JWT from Authorization header or query param.
pub fn extract_token(
    headers: &axum::http::HeaderMap,
    params: &std::collections::HashMap<String, String>,
) -> Option<String> {
    // Try Authorization: Bearer <token>
    if let Some(auth) = headers.get("authorization") {
        if let Ok(val) = auth.to_str() {
            if let Some(token) = val.strip_prefix("Bearer ") {
                return Some(token.to_string());
            }
        }
    }
    // Try ?token=<jwt> query param
    params.get("token").cloned()
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::SaltString;
    use argon2::password_hash::rand_core::OsRng;

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash password: {}", e))?
        .to_string();
    Ok(hash)
}

fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::{Argon2, PasswordVerifier};
    use argon2::password_hash::PasswordHash;

    let parsed = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

use secrecy::ExposeSecret;
