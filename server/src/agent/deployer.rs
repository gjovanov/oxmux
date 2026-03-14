//! Agent deployer — installs oxmux-agent on remote hosts via SSH.

use anyhow::{Context, Result};
use russh::client;
use russh::ChannelMsg;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tracing::{info, warn};

use crate::session::types::SshAuthConfig;

use super::binary;

/// Deploy result with agent info.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeployResult {
    pub host: String,
    pub quic_port: u16,
    pub status: String,
}

/// Deploy the oxmux-agent to a remote host using an existing SSH handle.
pub async fn deploy_via_ssh(
    handle: &client::Handle<super::SshDeployHandler>,
    host: &str,
    agent_secret: &str,
    quic_port: u16,
) -> Result<DeployResult> {
    info!(host, "deploying oxmux-agent");

    // 1. Find local agent binary
    let binary_path = binary::find_agent_binary()?;
    let binary_data = tokio::fs::read(&binary_path)
        .await
        .context("failed to read agent binary")?;
    let binary_size = binary_data.len();

    info!(host, size = binary_size, "uploading agent binary");

    // 2. Upload binary via SSH exec + stdin
    // Use base64 encoding for reliable transfer
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &binary_data);

    // Upload in chunks via echo + base64 decode
    let upload_cmd = "cat > /tmp/oxmux-agent.b64 && base64 -d /tmp/oxmux-agent.b64 > /usr/local/bin/oxmux-agent && chmod +x /usr/local/bin/oxmux-agent && rm /tmp/oxmux-agent.b64 && echo UPLOAD_OK";

    let mut channel = handle.channel_open_session().await?;
    channel.exec(true, upload_cmd.as_bytes()).await?;

    // Write base64 data to stdin
    let crypto_vec = russh::CryptoVec::from_slice(b64.as_bytes());
    handle.data(channel.id(), crypto_vec).await
        .map_err(|_| anyhow::anyhow!("failed to write binary data"))?;
    // Signal end of stdin
    let _ = handle.data(channel.id(), russh::CryptoVec::new()).await;

    // Wait for completion
    let mut output = String::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => {
                output.push_str(&String::from_utf8_lossy(data));
            }
            ChannelMsg::Eof | ChannelMsg::Close => break,
            ChannelMsg::ExitStatus { exit_status } => {
                if exit_status != 0 {
                    anyhow::bail!("upload failed with exit code {}: {}", exit_status, output.trim());
                }
            }
            _ => {}
        }
    }

    if !output.contains("UPLOAD_OK") {
        anyhow::bail!("upload did not complete successfully: {}", output.trim());
    }

    info!(host, "binary uploaded, creating systemd service");

    // 3. Create systemd service
    let service_cmd = format!(
        r#"cat > /etc/systemd/system/oxmux-agent.service << 'SVCEOF'
[Unit]
Description=Oxmux Agent
After=network.target

[Service]
Type=simple
Environment=AGENT_QUIC_PORT={}
Environment=OXMUX_AGENT_SECRET={}
Environment=RUST_LOG=oxmux_agent=info
ExecStart=/usr/local/bin/oxmux-agent
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
SVCEOF
systemctl daemon-reload && systemctl enable --now oxmux-agent && echo SERVICE_OK"#,
        quic_port, agent_secret
    );

    let service_output = exec_command(handle, &service_cmd).await?;
    if !service_output.contains("SERVICE_OK") {
        anyhow::bail!("service creation failed: {}", service_output.trim());
    }

    info!(host, port = quic_port, "agent deployed and started");

    Ok(DeployResult {
        host: host.to_string(),
        quic_port,
        status: "starting".to_string(),
    })
}

/// Execute a command via SSH and return stdout.
async fn exec_command(handle: &client::Handle<super::SshDeployHandler>, cmd: &str) -> Result<String> {
    let mut channel = handle.channel_open_session().await?;
    channel.exec(true, cmd.as_bytes()).await?;

    let mut output = String::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => output.push_str(&String::from_utf8_lossy(data)),
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    Ok(output)
}
