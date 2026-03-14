//! Integration tests for the oxmux-agent QUIC protocol.
//!
//! These tests connect directly to an agent running on localhost:4434
//! and verify the MessagePack protocol works end-to-end.

use anyhow::Result;
use std::sync::Arc;

#[tokio::test]
async fn test_agent_quic_connect_and_list() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok(); // May already be installed

    let agent_port: u16 = std::env::var("AGENT_TEST_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4434);

    let agent_addr = format!("127.0.0.1:{}", agent_port);

    // Configure QUIC client (skip cert verification — self-signed)
    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
            .expect("failed to create QUIC crypto config"),
    ));

    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    // Connect to agent
    let conn = match endpoint.connect(agent_addr.parse()?, "oxmux-agent") {
        Ok(connecting) => match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connecting,
        ).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                eprintln!("Agent not available at {}: {} (skipping test)", agent_addr, e);
                return Ok(());
            }
            Err(_) => {
                eprintln!("Agent connection timeout (skipping test)");
                return Ok(());
            }
        },
        Err(e) => {
            eprintln!("Failed to initiate connection: {} (skipping test)", e);
            return Ok(());
        }
    };

    println!("Connected to agent at {}", agent_addr);

    // Open bidirectional stream
    let (mut send, mut recv) = conn.open_bi().await?;

    // Send auth message (no secret configured = accepts any token)
    let auth_msg = rmp_serde::to_vec_named(&serde_json::json!({
        "t": "auth",
        "token": "test-token"
    }))?;
    send.write_all(&auth_msg).await?;

    // Read auth response
    let mut buf = vec![0u8; 4096];
    let n = recv.read(&mut buf).await?.unwrap_or(0);
    assert!(n > 0, "Expected auth response");
    let auth_resp: serde_json::Value = rmp_serde::from_slice(&buf[..n])?;
    assert_eq!(auth_resp.get("t").and_then(|v| v.as_str()), Some("auth_ok"));
    println!("Authenticated with agent");

    // Send sess_connect to create/attach a tmux session
    let connect_msg = rmp_serde::to_vec_named(&serde_json::json!({
        "t": "sess_connect",
        "name": "agent-test"
    }))?;
    send.write_all(&connect_msg).await?;

    // Read session connected response
    let n = recv.read(&mut buf).await?.unwrap_or(0);
    assert!(n > 0, "Expected sess_connected response");
    let resp: serde_json::Value = rmp_serde::from_slice(&buf[..n])?;
    assert_eq!(resp.get("t").and_then(|v| v.as_str()), Some("sess_connected"));
    println!("Session connected: {:?}", resp.get("session").and_then(|s| s.get("name")));

    // Send ping
    let ping_msg = rmp_serde::to_vec_named(&serde_json::json!({
        "t": "ping",
        "ts": 1234567890u64
    }))?;
    send.write_all(&ping_msg).await?;

    let n = recv.read(&mut buf).await?.unwrap_or(0);
    assert!(n > 0, "Expected pong response");
    let pong: serde_json::Value = rmp_serde::from_slice(&buf[..n])?;
    assert_eq!(pong.get("t").and_then(|v| v.as_str()), Some("pong"));
    assert_eq!(pong.get("ts").and_then(|v| v.as_u64()), Some(1234567890));
    println!("Ping/pong works");

    // Clean up: kill the test tmux session
    let _ = std::process::Command::new("tmux")
        .args(["kill-session", "-t", "agent-test"])
        .output();

    conn.close(0u32.into(), b"test done");
    println!("Agent protocol test passed");

    Ok(())
}

#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
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
