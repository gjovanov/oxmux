use anyhow::Result;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub quic: QuicConfig,
    pub coturn: CoturnConfig,
    pub claude: ClaudeConfig,
    pub database: DatabaseConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub log_level: String,
    pub jwt_secret: SecretString,
}

#[derive(Debug, Deserialize, Clone)]
pub struct QuicConfig {
    pub listen_port: u16,
    pub cert_path: String,
    pub key_path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CoturnConfig {
    pub auth_secret: SecretString,
    pub realm: String,
    pub ttl: u64,
    /// Comma-separated list of host:port
    pub servers: String,
    pub tls_servers: String,
}

impl CoturnConfig {
    pub fn turn_urls(&self) -> Vec<String> {
        self.servers
            .split(',')
            .map(|s| format!("turn:{}", s.trim()))
            .collect()
    }

    pub fn turns_urls(&self) -> Vec<String> {
        self.tls_servers
            .split(',')
            .map(|s| format!("turns:{}", s.trim()))
            .collect()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ClaudeConfig {
    pub default_flags: String,
    pub cost_alert_usd: f64,
    pub context_warn_threshold: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: SecretString,
}

impl Config {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok();

        let cfg = config::Config::builder()
            .add_source(config::Environment::default().separator("_"))
            .build()?;

        let config = Config {
            server: ServerConfig {
                host: cfg.get_string("OXMUX_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
                port: cfg.get_int("OXMUX_PORT").unwrap_or(8080) as u16,
                log_level: cfg.get_string("OXMUX_LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
                jwt_secret: SecretString::new(cfg.get_string("OXMUX_JWT_SECRET")?.into()),
            },
            quic: QuicConfig {
                listen_port: cfg.get_int("QUIC_LISTEN_PORT").unwrap_or(4433) as u16,
                cert_path: cfg.get_string("QUIC_CERT_PATH").unwrap_or_else(|_| "/etc/oxmux/tls/fullchain.pem".to_string()),
                key_path: cfg.get_string("QUIC_KEY_PATH").unwrap_or_else(|_| "/etc/oxmux/tls/key.pem".to_string()),
            },
            coturn: CoturnConfig {
                auth_secret: SecretString::new(cfg.get_string("COTURN_AUTH_SECRET")?.into()),
                realm: cfg.get_string("COTURN_REALM").unwrap_or_else(|_| "coturn.roomler.live".to_string()),
                ttl: cfg.get_int("COTURN_TTL").unwrap_or(86400) as u64,
                servers: cfg.get_string("COTURN_SERVERS")
                    .unwrap_or_else(|_| "198.51.100.10:3478,198.51.100.20:3478,198.51.100.30:3478".to_string()),
                tls_servers: cfg.get_string("COTURN_TLS_SERVERS")
                    .unwrap_or_else(|_| "198.51.100.10:5349,198.51.100.20:5349,198.51.100.30:5349".to_string()),
            },
            claude: ClaudeConfig {
                default_flags: cfg.get_string("CLAUDE_DEFAULT_FLAGS")
                    .unwrap_or_else(|_| "--output-format stream-json".to_string()),
                cost_alert_usd: cfg.get_float("CLAUDE_COST_ALERT_USD").unwrap_or(1.0),
                context_warn_threshold: cfg.get_float("CLAUDE_CONTEXT_WARN_THRESHOLD").unwrap_or(0.8),
            },
            database: DatabaseConfig {
                url: SecretString::new(
                    cfg.get_string("DATABASE_URL")
                        .unwrap_or_else(|_| "sqlite:./oxmux.db".to_string())
                        .into(),
                ),
            },
        };

        Ok(config)
    }
}
