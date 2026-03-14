//! Agent binary management — locate, cache, and serve the oxmux-agent binary.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::info;

/// Find the agent binary path.
/// Checks in order:
/// 1. AGENT_BINARY_PATH env var
/// 2. /app/oxmux-agent (Docker)
/// 3. ./target/release/oxmux-agent (dev)
pub fn find_agent_binary() -> Result<PathBuf> {
    // 1. Env var
    if let Ok(path) = std::env::var("AGENT_BINARY_PATH") {
        let p = PathBuf::from(&path);
        if p.exists() {
            info!(path = %path, "using agent binary from env");
            return Ok(p);
        }
    }

    // 2. Docker path
    let docker_path = PathBuf::from("/app/oxmux-agent");
    if docker_path.exists() {
        info!("using agent binary from /app/oxmux-agent");
        return Ok(docker_path);
    }

    // 3. Dev build
    let dev_path = PathBuf::from("target/release/oxmux-agent");
    if dev_path.exists() {
        info!("using agent binary from target/release/");
        return Ok(dev_path);
    }

    anyhow::bail!(
        "oxmux-agent binary not found. Set AGENT_BINARY_PATH or build with `cargo build --release -p oxmux-agent`"
    )
}

/// Get the size of the agent binary in bytes (for progress tracking).
pub fn binary_size(path: &Path) -> Result<u64> {
    let meta = std::fs::metadata(path).context("failed to stat agent binary")?;
    Ok(meta.len())
}
