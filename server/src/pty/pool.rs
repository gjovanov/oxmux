use dashmap::DashMap;
use std::sync::Arc;

use super::pane::PaneSession;

pub struct PtyPool {
    panes: DashMap<String, Arc<tokio::sync::Mutex<PaneSession>>>,
}

impl PtyPool {
    pub fn new() -> Self {
        Self { panes: DashMap::new() }
    }
}

impl Default for PtyPool {
    fn default() -> Self {
        Self::new()
    }
}
