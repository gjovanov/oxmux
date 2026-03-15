//! Agent probe — verify an agent is online by connecting to its QUIC port.

use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::{debug, info};

/// Probe an agent's QUIC port to verify it's online.
/// Returns true if the agent responds to a QUIC connection.
pub async fn probe_agent(host: &str, port: u16) -> Result<bool> {
    let addr = format!("{}:{}", host, port);

    // Skip cert verification (agent uses self-signed certs)
    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipVerify))
        .with_no_client_auth();

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
            .context("failed to create QUIC config")?,
    ));

    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    let connect = endpoint.connect(addr.parse()?, "oxmux-agent");
    let conn = match connect {
        Ok(connecting) => {
            match tokio::time::timeout(std::time::Duration::from_secs(5), connecting).await {
                Ok(Ok(conn)) => conn,
                Ok(Err(_)) | Err(_) => return Ok(false),
            }
        }
        Err(_) => return Ok(false),
    };

    debug!(host, port, "QUIC probe connected");

    // Try to open a stream — if agent accepts, it's alive
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        conn.open_bi(),
    ).await {
        Ok(Ok(_)) => {
            info!(host, port, "agent probe: online");
            conn.close(0u32.into(), b"probe");
            endpoint.wait_idle().await;
            Ok(true)
        }
        _ => {
            conn.close(0u32.into(), b"probe-fail");
            Ok(false)
        }
    }
}

#[derive(Debug)]
struct SkipVerify;

impl rustls::client::danger::ServerCertVerifier for SkipVerify {
    fn verify_server_cert(
        &self, _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>, _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, _: &[u8], _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(&self, _: &[u8], _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }
}
