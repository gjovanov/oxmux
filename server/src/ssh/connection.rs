use serde::{Deserialize, Serialize};
use secrecy::SecretString;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshHost {
    pub id: uuid::Uuid,
    pub alias: String,
    pub hostname: String,
    pub port: u16,
    pub user: String,
    #[serde(skip)]
    pub auth: SshAuth,
}

#[derive(Debug, Clone, Default)]
pub enum SshAuth {
    #[default]
    Agent,
    Password(SecretString),
    PrivateKey {
        path: std::path::PathBuf,
        passphrase: Option<SecretString>,
    },
}
