//! The mirror's daemon-connection plumbing: the ordered line reader,
//! the request/reply `command` helper, and the `pane-info` reply parser.

use crate::mux::LocalStream;
use interprocess::TryClone as _;
use std::io::{self, BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Daemon lines buffered between the reader thread and their consumer. When
/// full, the reader stops reading and the socket pushes back on the daemon,
/// as it did when the drain read the socket directly.
const MUX_LINE_BACKLOG: usize = 256;

/// The daemon connection's lines, read in order by one thread.
///
/// Named pipes reject I/O timeouts, so a reply deadline cannot be a socket
/// read timeout; it is a `recv_timeout` on this channel instead. The same
/// receiver moves to the drain thread after the handshake, so no line is
/// lost at the hand-off. The reader thread exits at EOF, on a read error,
/// or at its first send after the receiver drops.
pub(super) struct MuxLines {
    pub(super) rx: Receiver<io::Result<String>>,
    stopped: Arc<AtomicBool>,
    wake: LocalStream,
}

impl Drop for MuxLines {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        // A quiet pane otherwise leaves the reader blocked in read forever.
        let _ = writeln!(self.wake, "list-panes");
        let _ = self.wake.flush();
    }
}

impl MuxLines {
    /// Start the reader thread over `stream` (the connection's read half).
    pub(super) fn spawn(stream: LocalStream) -> io::Result<Self> {
        let (tx, rx): (SyncSender<io::Result<String>>, _) =
            std::sync::mpsc::sync_channel(MUX_LINE_BACKLOG);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let wake = stream.try_clone()?;
        std::thread::Builder::new()
            .name("mux-mirror-reader".to_string())
            .spawn(move || {
                for line in BufReader::new(stream).lines() {
                    let failed = line.is_err();
                    if stop.load(Ordering::Relaxed) || tx.send(line).is_err() || failed {
                        break;
                    }
                }
            })?;
        Ok(Self { rx, stopped, wake })
    }
}

/// Send one command and read its reply block, skipping pushed lines read
/// along the way. `%error` becomes an `Err`, and so does a reply block that
/// has not closed within `timeout` of the send: pushed `%output` interleaved
/// with the reply cannot extend that deadline.
pub(super) fn command(
    writer: &mut LocalStream,
    reader: &MuxLines,
    line: &str,
    timeout: Duration,
) -> io::Result<Vec<String>> {
    writeln!(writer, "{line}")?;
    writer.flush()?;
    let deadline = Instant::now() + timeout;
    let mut body: Option<Vec<String>> = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let next = match reader.rx.recv_timeout(remaining) {
            Ok(next) => next?,
            Err(RecvTimeoutError::Timeout) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("no reply to {line:?} within {timeout:?}"),
                ));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "the daemon closed the connection",
                ));
            }
        };
        if next.starts_with("%begin") {
            body = Some(Vec::new());
        } else if body.is_some() && next.starts_with("%end") {
            return Ok(body.unwrap_or_default());
        } else if body.is_some() && next.starts_with("%error") {
            return Err(io::Error::other(format!(
                "{line:?} failed: {}",
                body.unwrap_or_default().join(" ")
            )));
        } else if let Some(body) = body.as_mut() {
            body.push(next);
        }
    }
}

/// Parse `%N @W COLSxROWS` (the `pane-info` reply) into window and size.
pub(super) fn parse_pane_info(line: &str) -> Option<(String, usize, usize)> {
    let mut fields = line.split_whitespace().skip(1);
    let window = fields.next()?.to_string();
    let (cols, rows) = fields.next()?.split_once('x')?;
    Some((window, cols.parse().ok()?, rows.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_info_parses() {
        assert_eq!(
            parse_pane_info("%3 @1 120x40"),
            Some(("@1".to_string(), 120, 40))
        );
        assert_eq!(parse_pane_info("%3 @1"), None);
    }
}
