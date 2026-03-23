//! Agent deployer — installs oxmux-agent on remote hosts via SSH.

use anyhow::{Context, Result};
use russh::client;
use russh::ChannelMsg;
use secrecy::ExposeSecret;
use tracing::{info, warn};

use super::binary;

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
    port: u16,
    user: &str,
    auth: &crate::session::types::SshAuthConfig,
    agent_secret: &str,
    coturn_secret: &str,
    coturn_servers: &str,
    quic_port: u16,
) -> Result<DeployResult> {
    info!(host, "deploying oxmux-agent");

    let binary_path = binary::find_agent_binary()?;
    let binary_data = tokio::fs::read(&binary_path)
        .await
        .context("failed to read agent binary")?;

    info!(host, size = binary_data.len(), "uploading agent binary");

    // Upload via scp command (most reliable for large binaries)
    // Write binary to a temp file, then scp it
    let tmp_path = format!("/tmp/oxmux-agent-{}", std::process::id());
    tokio::fs::write(&tmp_path, &binary_data).await
        .context("failed to write temp agent binary")?;

    info!(host, tmp = %tmp_path, "uploading via scp command");

    // Build scp command with the session's SSH key.
    // If the key is encrypted, convert it to a temp unencrypted key first.
    let (key_path, tmp_key_path) = match auth {
        crate::session::types::SshAuthConfig::PrivateKey { path, passphrase } => {
            let expanded = if path.starts_with("~/") {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/home/oxmux".to_string());
                format!("{}/{}", home, &path[2..])
            } else {
                path.clone()
            };

            // Check if key is encrypted and needs conversion
            let key_content = tokio::fs::read_to_string(&expanded).await.unwrap_or_default();
            if key_content.contains("DES-EDE3-CBC") || key_content.contains("DES-CBC") || key_content.contains("Proc-Type: 4,ENCRYPTED") {
                let pass = passphrase.as_ref().map(|p| p.expose_secret().to_string()).unwrap_or_default();
                let pass = pass.as_str();
                let tmp_key = format!("/tmp/oxmux-scp-key-{}", std::process::id());
                let output = tokio::process::Command::new("openssl")
                    .args(["rsa", "-in", &expanded, "-out", &tmp_key, "-passin", &format!("pass:{}", pass), "-traditional"])
                    .output().await?;
                if !output.status.success() {
                    anyhow::bail!("failed to convert encrypted key for scp");
                }
                // Set permissions
                let _ = tokio::process::Command::new("chmod").args(["600", &tmp_key]).output().await;
                (tmp_key.clone(), Some(tmp_key))
            } else {
                (expanded, None)
            }
        }
        _ => (String::new(), None),
    };

    let mut scp_cmd = tokio::process::Command::new("scp");
    scp_cmd
        .arg("-o").arg("StrictHostKeyChecking=no")
        .arg("-o").arg("UserKnownHostsFile=/dev/null")
        .arg("-P").arg(port.to_string());

    if !key_path.is_empty() {
        scp_cmd.arg("-i").arg(&key_path);
    }

    // SCP to /tmp first (no root needed), then move via sudo
    scp_cmd
        .arg(&tmp_path)
        .arg(format!("{}@{}:/tmp/oxmux-agent-upload", user, host));

    let scp_output = scp_cmd.output().await.context("scp command failed")?;

    // Clean up temp files
    let _ = tokio::fs::remove_file(&tmp_path).await;
    if let Some(ref tmp_key) = tmp_key_path {
        let _ = tokio::fs::remove_file(tmp_key).await;
    }

    if !scp_output.status.success() {
        let stderr = String::from_utf8_lossy(&scp_output.stderr);
        anyhow::bail!("scp failed: {}", stderr.trim());
    }

    info!(host, "binary uploaded to /tmp via scp");

    // Move to /usr/local/bin and make executable (try sudo, fall back to user-local)
    let install_result = exec_ssh_command(handle,
        "sudo mv /tmp/oxmux-agent-upload /usr/local/bin/oxmux-agent && sudo chmod +x /usr/local/bin/oxmux-agent && echo INSTALL_OK || \
         (mv /tmp/oxmux-agent-upload ~/oxmux-agent && chmod +x ~/oxmux-agent && echo INSTALL_OK_HOME)"
    ).await?;

    if install_result.contains("INSTALL_OK_HOME") {
        info!(host, "binary installed to ~/oxmux-agent (no sudo)");
    } else if install_result.contains("INSTALL_OK") {
        info!(host, "binary installed to /usr/local/bin/oxmux-agent");
    } else {
        anyhow::bail!("install failed: {}", install_result.trim());
    }

    info!(host, "binary uploaded successfully");

    // Determine binary path based on install result
    let agent_bin = if install_result.contains("INSTALL_OK_HOME") {
        format!("/home/{}/oxmux-agent", user)
    } else {
        "/usr/local/bin/oxmux-agent".to_string()
    };

    // Create systemd service (needs sudo)
    info!(host, bin = %agent_bin, "creating systemd service");
    let service_cmd = format!(
        r#"sudo bash -c 'cat > /etc/systemd/system/oxmux-agent.service << SVCEOF
[Unit]
Description=Oxmux Agent
After=network.target

[Service]
Type=simple
User={user}
Environment=AGENT_QUIC_PORT={quic_port}
Environment=OXMUX_AGENT_SECRET={agent_secret}
Environment=COTURN_AUTH_SECRET={coturn_secret}
Environment=COTURN_SERVERS={coturn_servers}
Environment=PUBLIC_IP={host}
Environment=RUST_LOG=oxmux_agent=info
ExecStart={agent_bin}
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
SVCEOF
systemctl daemon-reload && systemctl enable --now oxmux-agent' && echo SERVICE_OK"#,
        quic_port = quic_port, agent_secret = agent_secret, agent_bin = agent_bin
    );

    let result = exec_ssh_command(handle, &service_cmd).await?;
    if !result.contains("SERVICE_OK") {
        anyhow::bail!("service creation failed: {}", result.trim());
    }

    info!(host, port = quic_port, "agent deployed and started");

    Ok(DeployResult {
        host: host.to_string(),
        quic_port,
        status: "starting".to_string(),
    })
}

/// Execute a command via SSH and return stdout. Public for use by probe/status checks.
pub async fn exec_ssh_command(handle: &client::Handle<super::SshDeployHandler>, cmd: &str) -> Result<String> {
    let mut channel = handle.channel_open_session().await?;
    channel.exec(true, cmd.as_bytes()).await?;

    let mut output = String::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => output.push_str(&String::from_utf8_lossy(data)),
            ChannelMsg::Eof | ChannelMsg::Close => break,
            ChannelMsg::ExitStatus { .. } => {}
            _ => {}
        }
    }
    Ok(output)
}
