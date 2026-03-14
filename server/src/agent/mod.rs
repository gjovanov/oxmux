pub mod binary;
pub mod deployer;
pub mod handler;
pub mod registry;

/// SSH handler for agent deployment (accepts all host keys).
pub(crate) struct SshDeployHandler;

#[async_trait::async_trait]
impl russh::client::Handler for SshDeployHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}
