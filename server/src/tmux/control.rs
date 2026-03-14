use anyhow::Result;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Events emitted by tmux control mode (`tmux -CC`)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TmuxEvent {
    /// %output <pane-id> <data>
    Output { pane_id: String, data: Bytes },
    /// %begin / %end block (response to a command)
    CommandResponse { timestamp: u64, flags: u32, lines: Vec<String> },
    /// %session-changed <session-id> <name>
    SessionChanged { id: String, name: String },
    /// %session-created <session-id>
    SessionCreated { id: String, name: String },
    /// %session-closed <session-id>
    SessionClosed { id: String },
    /// %window-add <window-id>
    WindowAdd { id: String },
    /// %window-close <window-id>
    WindowClose { id: String },
    /// %window-renamed <window-id> <name>
    WindowRenamed { id: String, name: String },
    /// %layout-change <window-id> <layout>
    LayoutChange { window_id: String, layout: String },
    /// %pane-mode-changed <pane-id>
    PaneModeChanged { pane_id: String },
    /// %exit [reason]
    Exit { reason: Option<String> },
}

/// State machine for parsing tmux control mode output line by line
pub struct ControlModeParser {
    /// Accumulates lines inside a %begin/%end block
    in_block: Option<BlockAccumulator>,
    tx: mpsc::Sender<TmuxEvent>,
}

struct BlockAccumulator {
    timestamp: u64,
    flags: u32,
    lines: Vec<String>,
}

impl ControlModeParser {
    pub fn new(tx: mpsc::Sender<TmuxEvent>) -> Self {
        Self { in_block: None, tx }
    }

    /// Feed a single line of control mode output.
    /// Should be called for every line received from the tmux process stdout.
    pub async fn feed_line(&mut self, line: &str) -> Result<()> {
        let line = line.trim_end_matches('\r');
        debug!("tmux ctrl: {}", line);

        // Inside a %begin/%end block
        if let Some(ref mut block) = self.in_block {
            if let Some(rest) = line.strip_prefix("%end ") {
                // Close block
                let _parts: Vec<&str> = rest.splitn(3, ' ').collect();
                let lines = block.lines.clone();
                let ts = block.timestamp;
                let fl = block.flags;
                self.in_block = None;
                self.emit(TmuxEvent::CommandResponse {
                    timestamp: ts,
                    flags: fl,
                    lines,
                }).await;
            } else if line.starts_with("%error ") {
                self.in_block = None;
                warn!("tmux control mode error block");
            } else {
                block.lines.push(line.to_string());
            }
            return Ok(());
        }

        // Notification lines
        if let Some(rest) = line.strip_prefix("%begin ") {
            let parts: Vec<&str> = rest.splitn(3, ' ').collect();
            let timestamp = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            let flags = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            self.in_block = Some(BlockAccumulator { timestamp, flags, lines: vec![] });
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%output ") {
            if let Some((pane_id, data)) = rest.split_once(' ') {
                // Data may contain octal escapes from tmux — decode them
                let decoded = decode_tmux_output(data);
                self.emit(TmuxEvent::Output {
                    pane_id: pane_id.to_string(),
                    data: Bytes::from(decoded),
                }).await;
            }
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%session-changed ") {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() == 2 {
                self.emit(TmuxEvent::SessionChanged {
                    id: parts[0].to_string(),
                    name: parts[1].to_string(),
                }).await;
            }
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%session-created ") {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            self.emit(TmuxEvent::SessionCreated {
                id: parts.first().unwrap_or(&"").to_string(),
                name: parts.get(1).unwrap_or(&"").to_string(),
            }).await;
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%session-closed ") {
            self.emit(TmuxEvent::SessionClosed { id: rest.to_string() }).await;
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%window-add ") {
            self.emit(TmuxEvent::WindowAdd { id: rest.to_string() }).await;
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%window-close ") {
            self.emit(TmuxEvent::WindowClose { id: rest.to_string() }).await;
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%window-renamed ") {
            if let Some((id, name)) = rest.split_once(' ') {
                self.emit(TmuxEvent::WindowRenamed {
                    id: id.to_string(),
                    name: name.to_string(),
                }).await;
            }
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%layout-change ") {
            if let Some((wid, layout)) = rest.split_once(' ') {
                self.emit(TmuxEvent::LayoutChange {
                    window_id: wid.to_string(),
                    layout: layout.to_string(),
                }).await;
            }
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%pane-mode-changed ") {
            self.emit(TmuxEvent::PaneModeChanged { pane_id: rest.to_string() }).await;
            return Ok(());
        }

        if line == "%exit" {
            self.emit(TmuxEvent::Exit { reason: None }).await;
            return Ok(());
        }

        if let Some(rest) = line.strip_prefix("%exit ") {
            self.emit(TmuxEvent::Exit { reason: Some(rest.to_string()) }).await;
            return Ok(());
        }

        Ok(())
    }

    async fn emit(&self, event: TmuxEvent) {
        if self.tx.send(event).await.is_err() {
            warn!("tmux event receiver dropped");
        }
    }
}

/// Decode tmux's output escaping:
/// Non-printable bytes are encoded as `\ooo` (octal) in control mode output.
fn decode_tmux_output(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // Check for octal escape: \NNN
            let d1 = chars.peek().copied().filter(|c| c.is_ascii_digit() && *c <= '7');
            if let Some(d1) = d1 {
                chars.next();
                let d2 = chars.next().filter(|c| c.is_ascii_digit() && *c <= '7');
                let d3 = chars.next().filter(|c| c.is_ascii_digit() && *c <= '7');
                if let (Some(d2), Some(d3)) = (d2, d3) {
                    let octal = format!("{}{}{}", d1, d2, d3);
                    if let Ok(byte) = u8::from_str_radix(&octal, 8) {
                        out.push(byte);
                        continue;
                    }
                }
            }
            // Not an octal escape — literal backslash
            out.push(b'\\');
        } else {
            // Encode char as UTF-8 bytes
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    async fn parse_lines(lines: &[&str]) -> Vec<TmuxEvent> {
        let (tx, mut rx) = mpsc::channel(64);
        let mut parser = ControlModeParser::new(tx);
        for line in lines {
            parser.feed_line(line).await.unwrap();
        }
        drop(parser);
        let mut events = vec![];
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        events
    }

    #[tokio::test]
    async fn parse_output_notification() {
        let events = parse_lines(&["%output %1 hello"]).await;
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], TmuxEvent::Output { pane_id, data }
            if pane_id == "%1" && data.as_ref() == b"hello"));
    }

    #[tokio::test]
    async fn parse_begin_end_block() {
        let events = parse_lines(&[
            "%begin 1700000000 1 0",
            "main",
            "dev",
            "%end 1700000000 1 0",
        ]).await;
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], TmuxEvent::CommandResponse { lines, .. }
            if lines == &["main", "dev"]));
    }

    #[tokio::test]
    async fn parse_session_events() {
        let events = parse_lines(&[
            "%session-created $1 main",
            "%session-changed $1 main",
            "%session-closed $1",
        ]).await;
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], TmuxEvent::SessionCreated { id, .. } if id == "$1"));
        assert!(matches!(&events[1], TmuxEvent::SessionChanged { name, .. } if name == "main"));
        assert!(matches!(&events[2], TmuxEvent::SessionClosed { id } if id == "$1"));
    }

    #[tokio::test]
    async fn decode_octal_escapes() {
        // \033 = ESC, \007 = BEL
        let decoded = decode_tmux_output("\\033[32mhello\\007");
        assert_eq!(decoded[0], 0x1b); // ESC
        assert_eq!(&decoded[1..8], b"[32mhel");
        assert_eq!(*decoded.last().unwrap(), 7u8); // BEL
    }

    #[tokio::test]
    async fn layout_change_event() {
        let events = parse_lines(&["%layout-change @1 24x80,0,0,0"]).await;
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], TmuxEvent::LayoutChange { window_id, layout }
            if window_id == "@1" && layout == "24x80,0,0,0"));
    }

    #[tokio::test]
    async fn error_block_clears_accumulator() {
        // %error should not crash the parser, just discard
        let events = parse_lines(&[
            "%begin 1700000000 1 0",
            "some output",
            "%error 1700000000 1 0",
            "%output %1 after_error",
        ]).await;
        // Only the output after the error block should emit
        assert!(events.iter().any(|e| matches!(e, TmuxEvent::Output { .. })));
    }
}
