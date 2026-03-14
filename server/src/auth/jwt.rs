use anyhow::{Context, Result};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// JWT claims payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// User ID
    pub sub: String,
    /// Username
    pub username: String,
    /// Expiration timestamp (Unix seconds)
    pub exp: u64,
}

/// Create a JWT token for a user.
pub fn create_token(user_id: &str, username: &str, secret: &str) -> Result<String> {
    let exp = jsonwebtoken::get_current_timestamp() + 7 * 24 * 3600; // 7 days
    let claims = Claims {
        sub: user_id.to_string(),
        username: username.to_string(),
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .context("failed to create JWT")
}

/// Create a short-lived JWT token for browser→agent auth.
pub fn create_agent_token(agent_id: &str, secret: &str) -> Result<String> {
    let exp = jsonwebtoken::get_current_timestamp() + 300; // 5 minutes
    let claims = Claims {
        sub: agent_id.to_string(),
        username: "agent".to_string(),
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .context("failed to create agent JWT")
}

/// Validate a JWT token and extract claims.
pub fn validate_token(token: &str, secret: &str) -> Result<Claims> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .context("invalid or expired token")?;
    Ok(data.claims)
}
