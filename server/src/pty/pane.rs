use anyhow::Result;
use bytes::Bytes;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use tokio::sync::broadcast;
use tracing::{debug, warn};

pub struct PaneSession {
    pub id: String,
    pub cols: u16,
    pub rows: u16,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
}

impl PaneSession {
    /// Spawn a PTY attached to a tmux session/pane.
    pub fn spawn(
        pane_id: String,
        tmux_session: &str,
        cols: u16,
        rows: u16,
        tx: broadcast::Sender<Bytes>,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new("tmux");
        cmd.args([
            "attach-session",
            "-t", tmux_session,
            "-x", &cols.to_string(),
            "-y", &rows.to_string(),
        ]);
        cmd.env("TERM", "xterm-256color");

        pair.slave.spawn_command(cmd)?;

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        // Background reader task → broadcast channel
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = Bytes::copy_from_slice(&buf[..n]);
                        if tx.send(data).is_err() {
                            break; // no receivers
                        }
                    }
                    Err(e) => {
                        warn!("PTY read error: {}", e);
                        break;
                    }
                }
            }
            debug!("PTY reader exited");
        });

        Ok(Self {
            id: pane_id,
            cols,
            rows,
            writer,
            master: pair.master,
        })
    }

    pub fn write_input(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }
}
