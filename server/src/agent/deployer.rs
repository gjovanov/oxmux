//! Agent deployer — installs oxmux-agent on remote hosts via SSH.

use anyhow::{Context, Result};
use tracing::info;

use crate::session::types::SshAuthConfig;

/// Deploy the oxmux-agent binary to a remote host via SSH.
///
/// Steps:
/// 1. SCP the agent binary to the host
/// 2. Create systemd service unit
/// 3. Enable and start the service
pub async fn deploy_agent(
    host: &str,
    port: u16,
    user: &str,
    auth: &SshAuthConfig,
    agent_binary_path: &str,
    agent_secret: &str,
    server_url: &str,
) -> Result<()> {
    info!(host, user, "deploying oxmux-agent");

    // This would use russh to:
    // 1. Connect to the host
    // 2. Upload the binary
    // 3. Create systemd unit
    // 4. Start the service

    // For now, generate the deployment commands
    let commands = format!(
        r#"
        # Upload agent binary
        scp -P {} {} {}@{}:/usr/local/bin/oxmux-agent

        # Create config
        ssh -p {} {}@{} 'cat > /etc/oxmux-agent.env << EOF
AGENT_QUIC_PORT=4433
OXMUX_SERVER={}
OXMUX_AGENT_SECRET={}
EOF'

        # Create systemd service
        ssh -p {} {}@{} 'cat > /etc/systemd/system/oxmux-agent.service << EOF
[Unit]
Description=Oxmux Agent
After=network.target

[Service]
Type=simple
EnvironmentFile=/etc/oxmux-agent.env
ExecStart=/usr/local/bin/oxmux-agent
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF'

        # Enable and start
        ssh -p {} {}@{} 'systemctl daemon-reload && systemctl enable --now oxmux-agent'
        "#,
        port, agent_binary_path, user, host,
        port, user, host, server_url, agent_secret,
        port, user, host,
        port, user, host,
    );

    info!("Agent deployment commands generated (manual execution required for now)");
    info!("{}", commands);

    // TODO: Execute via russh when fully automated
    Ok(())
}
