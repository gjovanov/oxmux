/// Stub — mirrors server/src/pty/pane.rs but runs locally on the agent machine.
/// Full implementation shares logic via a shared `oxmux-pty` crate (planned v1.1).
pub struct AgentPane {
    pub id: String,
}
