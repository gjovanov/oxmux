use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmac::{Hmac, Mac};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::CoturnConfig;

type HmacSha1 = Hmac<Sha1>;

/// Time-limited TURN credentials using COTURN shared secret auth.
/// Compatible with coturn's `use-auth-secret` + `static-auth-secret` mode.
///
/// Spec: https://www.ietf.org/rfc/rfc5389.txt + coturn extension
/// Implementation matches iTerm2 / roomler-ai WebRTC credential generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCredentials {
    pub username: String,
    pub credential: String,
    pub ttl: u64,
    pub uris: Vec<String>,
}

pub fn generate_turn_credentials(
    config: &CoturnConfig,
    user_id: &str,
) -> Result<TurnCredentials> {
    let secret = config.auth_secret.expose_secret();
    let ttl = config.ttl;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs();
    let expiry = now + ttl;

    // COTURN username format: "<expiry_timestamp>:<user_id>"
    let username = format!("{}:{}", expiry, user_id);

    // HMAC-SHA1(secret, username) → base64
    let mut mac = HmacSha1::new_from_slice(secret.as_bytes())
        .map_err(|e| anyhow::anyhow!("HMAC init error: {}", e))?;
    mac.update(username.as_bytes());
    let result = mac.finalize();
    let credential = BASE64.encode(result.into_bytes());

    // Build ICE server URIs: STUN + TURN + TURNS for all 3 workers
    let mut uris: Vec<String> = config.turn_urls();
    uris.extend(config.turns_urls());

    Ok(TurnCredentials {
        username,
        credential,
        ttl,
        uris,
    })
}

/// Full ICE configuration for browser RTCPeerConnection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceConfig {
    pub ice_servers: Vec<IceServer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

pub fn build_ice_config(credentials: &TurnCredentials) -> IceConfig {
    // Plain TURN URLs (UDP relay)
    let turn_urls: Vec<String> = credentials.uris.iter()
        .filter(|u| u.starts_with("turn:"))
        .cloned()
        .collect();

    IceConfig {
        ice_servers: vec![
            // Google STUN (reliable, always reachable)
            IceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                username: None,
                credential: None,
            },
            // TURN over UDP (plain) + TURNS over TLS on port 443 via domain name
            // Using domain name for TURNS since it has a valid TLS certificate
            IceServer {
                urls: {
                    let mut urls = turn_urls;
                    urls.push("turns:coturn.roomler.live:443?transport=tcp".to_string());
                    urls
                },
                username: Some(credentials.username.clone()),
                credential: Some(credentials.credential.clone()),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn test_config() -> CoturnConfig {
        CoturnConfig {
            auth_secret: SecretString::new("test_secret_abc123".to_string().into()),
            realm: "coturn.roomler.live".to_string(),
            ttl: 86400,
            servers: "198.51.100.10:3478,198.51.100.20:3478,198.51.100.30:3478".to_string(),
            tls_servers: "198.51.100.10:5349,198.51.100.20:5349,198.51.100.30:5349".to_string(),
        }
    }

    #[test]
    fn credentials_have_correct_format() {
        let creds = generate_turn_credentials(&test_config(), "user-123").unwrap();

        // Username: "<timestamp>:user-123"
        let parts: Vec<&str> = creds.username.splitn(2, ':').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1], "user-123");
        let timestamp: u64 = parts[0].parse().expect("timestamp should be u64");
        assert!(timestamp > 0);

        // Credential is non-empty base64
        assert!(!creds.credential.is_empty());
        BASE64.decode(&creds.credential).expect("credential should be valid base64");
    }

    #[test]
    fn credentials_include_all_turn_servers() {
        let creds = generate_turn_credentials(&test_config(), "u").unwrap();
        assert_eq!(creds.uris.len(), 6); // 3 TURN + 3 TURNS
        assert!(creds.uris.iter().any(|u| u.starts_with("turn:")));
        assert!(creds.uris.iter().any(|u| u.starts_with("turns:")));
    }

    #[test]
    fn different_users_get_different_credentials() {
        let cfg = test_config();
        let c1 = generate_turn_credentials(&cfg, "alice").unwrap();
        let c2 = generate_turn_credentials(&cfg, "bob").unwrap();
        assert_ne!(c1.username, c2.username);
        assert_ne!(c1.credential, c2.credential);
    }

    #[test]
    fn ice_config_has_stun_and_turn_entries() {
        let creds = generate_turn_credentials(&test_config(), "u").unwrap();
        let ice = build_ice_config(&creds);
        assert_eq!(ice.ice_servers.len(), 2);
        // First entry: Google STUN (no credentials)
        assert!(ice.ice_servers[0].username.is_none());
        assert!(ice.ice_servers[0].urls[0].contains("stun.l.google.com"));
        // Second entry: TURN + TURNS with credentials
        assert!(ice.ice_servers[1].username.is_some());
        assert!(ice.ice_servers[1].urls.iter().any(|u| u.starts_with("turn:")));
        assert!(ice.ice_servers[1].urls.iter().any(|u| u.contains("coturn.roomler.live")));
    }
}
