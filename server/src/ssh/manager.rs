/// SSH connection manager — pooling, auto-reconnect, agent forwarding.
/// Full implementation planned in v1.1.
pub struct SshManager;

impl SshManager {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SshManager {
    fn default() -> Self {
        Self::new()
    }
}
