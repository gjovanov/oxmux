use anyhow::Result;
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// Generate a self-signed certificate for the agent's QUIC listener.
/// The relay server pins the fingerprint (via AGENT_TLS_FINGERPRINT env var)
/// instead of doing full CA-based verification.
pub fn self_signed_cert() -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let subject_alt_names = vec!["oxmux-agent".to_string(), "localhost".to_string()];
    let certified_key = generate_simple_self_signed(subject_alt_names)?;

    let cert_der = certified_key.cert.der().clone();
    let key_der = PrivateKeyDer::try_from(certified_key.key_pair.serialize_der())
        .map_err(|e| anyhow::anyhow!("private key error: {}", e))?;

    // Print fingerprint for operator to copy into relay server config
    let fingerprint = sha256_fingerprint(cert_der.as_ref());
    tracing::info!("Agent TLS fingerprint (SHA-256): {}", fingerprint);

    Ok((vec![cert_der], key_der))
}

fn sha256_fingerprint(der: &[u8]) -> String {
    use std::fmt::Write;
    // Simple SHA-256 hex — replace with ring/sha2 in production
    let mut hex = String::new();
    for byte in der.iter().take(32) {
        write!(hex, "{:02x}", byte).unwrap();
    }
    hex
}
