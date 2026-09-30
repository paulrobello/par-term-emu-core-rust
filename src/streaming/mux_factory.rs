//! Streaming sessions backed by par-mux panes.
//!
//! [`MuxSessionFactory`] is a [`SessionFactory`] whose sessions mirror a pane
//! the par-mux daemon owns instead of spawning a PTY of their own. The pane
//! keeps ONE terminal, in the daemon; each streaming session holds a mirror
//! `Terminal` fed by the daemon's screen replay plus its `%output` deltas, and
//! is never a second owner (par-mux.md D2/R7). Several streaming sessions may
//! mirror the same pane; each has its own daemon connection.
//!
//! One connection per session, read in order by one reader thread whose
//! lines feed first the handshake in `create_session`, then the drain thread.
//! The seed (`refresh-client -t %N`) is read in `create_session` against a
//! reply deadline, and `%output` pushed before its reply block is discarded
//! (the replay already reflects it); everything after is applied. Ordering is
//! the reason this does not use [`crate::mux::MuxClient`], which splits
//! replies and notifications into separate channels.
//!
//! `create_session` blocks on daemon I/O, so the streaming server calls it
//! off the async runtime (`spawn_blocking`).
//!
//! Size policy (owner decision 2026-09-24): latest-resize-wins. The mirror is
//! seeded at the pane's current size (`pane-info`), never the viewer's, and
//! re-fits only when a `%layout-change` changes that pane's rectangle. A
//! streaming client's `Resize` becomes `refresh-client -t %N -C WxH`.

use crate::mux::ipc::{connect_local_stream, LocalStream};
use crate::streaming::error::StreamingError;
use crate::streaming::protocol::ServerMessage;
use crate::streaming::server::{SessionFactory, SessionFactoryResult, StreamingServer};
use crate::streaming::session::StreamSessionState;
use crate::terminal::Terminal;
use crate::tmux_control::{TmuxControlParser, TmuxNotification};
use interprocess::TryClone as _;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Bytes of input per `send-keys -H` command. Hex triples the size on the
/// wire, so a 256 KiB paste becomes many bounded lines instead of one.
const INPUT_CHUNK: usize = 1024;

/// How long `create_session` waits for each handshake reply block. Same
/// value as [`crate::mux::MuxClient`]'s reply timeout.
const MUX_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// Daemon lines buffered between the reader thread and their consumer. When
/// full, the reader stops reading and the socket pushes back on the daemon,
/// as it did when the drain read the socket directly.
const MUX_LINE_BACKLOG: usize = 256;

/// Which daemon pane a streaming session mirrors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxPaneSelector {
    /// The same pane for every session.
    Pane(u32),
    /// Parse the streaming session id: `pane-N` mirrors pane N; any other id
    /// falls back to the daemon's first pane (`list-panes`).
    FromSessionId,
}

/// A [`SessionFactory`] whose sessions mirror par-mux panes.
pub struct MuxSessionFactory {
    socket: PathBuf,
    selector: MuxPaneSelector,
    scrollback: usize,
    reply_timeout: Duration,
    sessions: RwLock<HashMap<String, Arc<MirrorLink>>>,
    // Weak on purpose: the server holds this factory strongly
    // (`with_factory`), so a strong back-reference is a reference cycle
    // that keeps every session — and each session's daemon-connection
    // drain thread holds the server strongly in turn — alive after the
    // last real owner drops. Measured as the stress test hanging at
    // runtime teardown: the enqueue drain's blocking task never sees the
    // channel close because the cycle keeps the session state alive.
    streaming_server: RwLock<Option<std::sync::Weak<StreamingServer>>>,
}

/// One streaming session's link to its pane.
struct MirrorLink {
    pane: u32,
    /// The pane's window (`@W`), which its layout changes and close name.
    window: String,
    terminal: Arc<RwLock<Terminal>>,
    writer: Arc<Mutex<LocalStream>>,
    /// False once the daemon connection closed or the pane went away.
    alive: AtomicBool,
    /// Set by teardown so the drain thread stops forwarding.
    closed: AtomicBool,
    /// Output buffered until `setup_session` connects the broadcaster
    /// (`Buffering`), then the broadcaster's sender (`Live`).
    sink: Mutex<Sink>,
}

enum Sink {
    Buffering(Vec<u8>),
    Live(tokio::sync::mpsc::Sender<String>),
    Closed,
}

impl MuxSessionFactory {
    /// A factory mirroring panes of the daemon on `socket`.
    pub fn new(socket: impl Into<PathBuf>, selector: MuxPaneSelector) -> Self {
        Self {
            socket: socket.into(),
            selector,
            scrollback: 10_000,
            reply_timeout: MUX_REPLY_TIMEOUT,
            sessions: RwLock::new(HashMap::new()),
            streaming_server: RwLock::new(None),
        }
    }

    /// Scrollback lines kept by each mirror terminal (default 10 000).
    pub fn with_scrollback(mut self, lines: usize) -> Self {
        self.scrollback = lines;
        self
    }

    /// A shorter handshake reply deadline, so a timeout test runs in
    /// milliseconds instead of [`MUX_REPLY_TIMEOUT`].
    #[cfg(test)]
    fn with_reply_timeout(mut self, timeout: Duration) -> Self {
        self.reply_timeout = timeout;
        self
    }

    /// The server whose sessions this factory closes when a pane goes away
    /// and to which it announces mirror re-fits.
    pub fn set_streaming_server(&self, server: Arc<StreamingServer>) {
        *self.streaming_server.write() = Some(Arc::downgrade(&server));
    }

    fn pane_for(
        &self,
        session_id: &str,
        writer: &mut LocalStream,
        reader: &MuxLines,
    ) -> io::Result<u32> {
        match self.selector {
            MuxPaneSelector::Pane(n) => Ok(n),
            MuxPaneSelector::FromSessionId => {
                if let Some(n) = session_id
                    .strip_prefix("pane-")
                    .and_then(|n| n.parse::<u32>().ok())
                {
                    return Ok(n);
                }
                let body = command(writer, reader, "list-panes", self.reply_timeout)?;
                body.iter()
                    .find_map(|l| l.trim().strip_prefix('%').and_then(|n| n.parse().ok()))
                    .ok_or_else(|| io::Error::other("the daemon has no panes"))
            }
        }
    }
}

/// The daemon connection's lines, read in order by one thread.
///
/// Named pipes reject I/O timeouts, so a reply deadline cannot be a socket
/// read timeout; it is a `recv_timeout` on this channel instead. The same
/// receiver moves to the drain thread after the handshake, so no line is
/// lost at the hand-off. The reader thread exits at EOF, on a read error,
/// or at its first send after the receiver drops.
struct MuxLines {
    rx: Receiver<io::Result<String>>,
}

impl MuxLines {
    /// Start the reader thread over `stream` (the connection's read half).
    fn spawn(stream: LocalStream) -> io::Result<Self> {
        let (tx, rx): (SyncSender<io::Result<String>>, _) =
            std::sync::mpsc::sync_channel(MUX_LINE_BACKLOG);
        std::thread::Builder::new()
            .name("mux-mirror-reader".to_string())
            .spawn(move || {
                for line in BufReader::new(stream).lines() {
                    let failed = line.is_err();
                    if tx.send(line).is_err() || failed {
                        break;
                    }
                }
            })?;
        Ok(Self { rx })
    }
}

/// Send one command and read its reply block, skipping pushed lines read
/// along the way. `%error` becomes an `Err`, and so does a reply block that
/// has not closed within `timeout` of the send: pushed `%output` interleaved
/// with the reply cannot extend that deadline.
fn command(
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
fn parse_pane_info(line: &str) -> Option<(String, usize, usize)> {
    let mut fields = line.split_whitespace().skip(1);
    let window = fields.next()?.to_string();
    let (cols, rows) = fields.next()?.split_once('x')?;
    Some((window, cols.parse().ok()?, rows.parse().ok()?))
}

/// The size a tmux layout string gives `pane`, as (cols, rows).
///
/// A leaf is `WxH,X,Y,ID`; a container is `WxH,X,Y` followed by `{` or `[`.
/// Splitting on the brackets leaves runs of comma-separated fields in which
/// every leaf is four fields and a container header (three fields) sits last.
fn pane_size_in_layout(layout: &str, pane: u32) -> Option<(usize, usize)> {
    let body = layout
        .split_once(',')
        .map_or(layout, |(_checksum, rest)| rest);
    body.split(['{', '}', '[', ']'])
        .flat_map(|run| {
            let fields: Vec<&str> = run.split(',').filter(|f| !f.is_empty()).collect();
            fields
                .chunks(4)
                .filter_map(|leaf| match leaf {
                    [size, _x, _y, id] => {
                        let (w, h) = size.split_once('x')?;
                        Some((id.parse::<u32>().ok()?, w.parse().ok()?, h.parse().ok()?))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .find(|(id, _, _)| *id == pane)
        .map(|(_, w, h)| (w, h))
}

/// One `send-keys -H` command line carrying `chunk` for `pane`.
///
/// Encodes into one pre-sized buffer — this runs per input byte on the
/// paste path, and a per-byte `format!` allocated one String each
/// (ENH-031). Output must stay byte-identical to the daemon's
/// `send-keys -H` grammar.
fn send_keys_line(pane: u32, chunk: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut line = String::with_capacity(24 + 3 * chunk.len());
    use std::fmt::Write as _;
    let _ = write!(line, "send-keys -t %{pane} -H");
    for &b in chunk {
        line.push(' ');
        line.push(HEX[(b >> 4) as usize] as char);
        line.push(HEX[(b & 0x0f) as usize] as char);
    }
    line
}

/// The pre-ENH-031 per-byte `format!` encoder, kept as the equivalence
/// reference for `send_keys_line`.
#[cfg(test)]
fn send_keys_line_reference(pane: u32, chunk: &[u8]) -> String {
    let mut line = format!("send-keys -t %{pane} -H");
    for byte in chunk {
        line.push_str(&format!(" {byte:02x}"));
    }
    line
}

/// A `Write` that turns bytes into `send-keys -t %N -H <hex>` commands.
/// Hex bypasses the command grammar's quoting entirely.
struct SendKeysWriter {
    pane: u32,
    stream: Arc<Mutex<LocalStream>>,
}

impl Write for SendKeysWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut stream = self.stream.lock();
        for chunk in buf.chunks(INPUT_CHUNK) {
            writeln!(stream, "{}", send_keys_line(self.pane, chunk))?;
        }
        stream.flush()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.lock().flush()
    }
}

impl SessionFactory for MuxSessionFactory {
    fn create_session(
        &self,
        session_id: &str,
        _cols: u16,
        _rows: u16,
        _shell_command: Option<&str>,
    ) -> Result<SessionFactoryResult, StreamingError> {
        let fail = |what: &str, err: io::Error| {
            StreamingError::ServerError(format!(
                "mux session '{session_id}': {what} on {}: {err}",
                self.socket.display()
            ))
        };
        let stream = connect_local_stream(&self.socket).map_err(|e| fail("cannot connect", e))?;
        let mut writer = stream
            .try_clone()
            .map_err(|e| fail("cannot clone stream", e))?;
        let reader = MuxLines::spawn(stream).map_err(|e| fail("cannot start reader", e))?;
        let timeout = self.reply_timeout;

        // Any %output these queries race is superseded by the seed below.
        let pane = self
            .pane_for(session_id, &mut writer, &reader)
            .map_err(|e| fail("cannot select a pane", e))?;
        let info = command(
            &mut writer,
            &reader,
            &format!("pane-info -t %{pane}"),
            timeout,
        )
        .map_err(|e| fail("pane-info", e))?;
        let (window, cols, rows) = info
            .first()
            .and_then(|l| parse_pane_info(l))
            .ok_or_else(|| fail("pane-info", io::Error::other(format!("bad reply {info:?}"))))?;

        let mut terminal = Terminal::with_scrollback(cols, rows, self.scrollback);
        let seed = command(
            &mut writer,
            &reader,
            &format!("refresh-client -t %{pane}"),
            timeout,
        )
        .map_err(|e| fail("refresh-client -t", e))?;
        terminal.process(seed.join("\n").as_bytes());
        // The daemon's pane answers terminal queries itself; the mirror's
        // copies would double every reply.
        terminal.drain_responses();
        let terminal = Arc::new(RwLock::new(terminal));

        let writer = Arc::new(Mutex::new(writer));
        let link = Arc::new(MirrorLink {
            pane,
            window,
            terminal: Arc::clone(&terminal),
            writer: Arc::clone(&writer),
            alive: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            sink: Mutex::new(Sink::Buffering(Vec::new())),
        });
        spawn_drain(
            Arc::clone(&link),
            reader,
            session_id.to_string(),
            self.streaming_server.read().clone(),
        );
        self.sessions
            .write()
            .insert(session_id.to_string(), Arc::clone(&link));

        let input: Box<dyn Write + Send> = Box::new(SendKeysWriter {
            pane,
            stream: writer,
        });
        Ok(SessionFactoryResult {
            terminal,
            pty_writer: Some(Arc::new(Mutex::new(input))),
        })
    }

    fn setup_session(
        &self,
        session_id: &str,
        session: &Arc<StreamSessionState>,
    ) -> Result<(), StreamingError> {
        let Some(link) = self.sessions.read().get(session_id).cloned() else {
            return Ok(());
        };
        let sender = session.get_output_sender();
        {
            let mut sink = link.sink.lock();
            if let Sink::Buffering(bytes) = &*sink {
                if !bytes.is_empty() {
                    let _ = sender.try_send(String::from_utf8_lossy(bytes).into_owned());
                }
            }
            *sink = Sink::Live(sender);
        }

        // A viewer's resize is a deliberate request for the pane's size
        // (latest-resize-wins); the mirror follows via %layout-change.
        let resize_rx = session.get_resize_receiver();
        let writer = Arc::clone(&link.writer);
        let pane = link.pane;
        tokio::spawn(async move {
            let mut rx = resize_rx.lock().await;
            while let Some((cols, rows)) = rx.recv().await {
                // The writer mutex is shared with the input path and the
                // socket write can block on the daemon: both stay off the
                // runtime worker.
                let writer = Arc::clone(&writer);
                let sent = tokio::task::spawn_blocking(move || {
                    let mut stream = writer.lock();
                    writeln!(stream, "refresh-client -t %{pane} -C {cols}x{rows}")
                        .and_then(|()| stream.flush())
                })
                .await;
                if !matches!(sent, Ok(Ok(()))) {
                    break;
                }
            }
        });
        Ok(())
    }

    fn teardown_session(&self, session_id: &str) {
        // The daemon owns the pane: teardown only detaches this mirror.
        if let Some(link) = self.sessions.write().remove(session_id) {
            link.closed.store(true, Ordering::Relaxed);
            *link.sink.lock() = Sink::Closed;
        }
    }

    fn is_session_alive(&self, session_id: &str) -> bool {
        self.sessions
            .read()
            .get(session_id)
            .is_some_and(|link| link.alive.load(Ordering::Relaxed))
    }
}

/// Read the daemon connection in order: apply the pane's `%output` to the
/// mirror and forward it to viewers, re-fit on `%layout-change`, and mark the
/// link dead when the pane or the connection goes away.
///
/// The server is held weakly: this thread's own lifetime is tied to the
/// mirror link the factory owns, and the factory is owned by the server —
/// a strong capture here would close the loop that kept the server (and
/// every session's state) alive after the last real owner dropped.
fn spawn_drain(
    link: Arc<MirrorLink>,
    reader: MuxLines,
    session_id: String,
    server: Option<std::sync::Weak<StreamingServer>>,
) {
    std::thread::spawn(move || {
        let pane_id = format!("%{}", link.pane);
        let mut parser = TmuxControlParser::new(true);
        let mut in_reply = false;
        let mut reply_body = Vec::new();
        for line in reader.rx.iter() {
            let Ok(line) = line else { break };
            if link.closed.load(Ordering::Relaxed) {
                break;
            }
            // Reply blocks (input and resize acks) carry nothing to mirror,
            // but an %error-terminated one means the daemon rejected a
            // command this mirror sent (bad send-keys payload, unknown
            // pane): without this log the rejection is invisible on both
            // sides of the socket — no PTY_WRITE, no log — and looks
            // identical to input lost inside the streamer.
            if line.starts_with("%begin") {
                in_reply = true;
                reply_body.clear();
                continue;
            }
            if in_reply {
                if line.starts_with("%end") {
                    in_reply = false;
                } else if line.starts_with("%error") {
                    in_reply = false;
                    crate::debug_error!(
                        "STREAMING",
                        "mux daemon rejected a command for pane %{}: {}",
                        link.pane,
                        reply_body.join(" | ")
                    );
                } else {
                    reply_body.push(line);
                }
                continue;
            }
            let mut framed = line.into_bytes();
            framed.push(b'\n');
            for notification in parser.parse(&framed) {
                match notification {
                    TmuxNotification::Output { pane_id: id, data } if id == pane_id => {
                        {
                            let mut term = link.terminal.write();
                            term.process(&data);
                            term.drain_responses();
                        }
                        forward(&link, &data);
                    }
                    TmuxNotification::LayoutChange {
                        window_id,
                        window_layout,
                        ..
                    } if window_id == link.window => {
                        let Some((cols, rows)) = pane_size_in_layout(&window_layout, link.pane)
                        else {
                            // The window's new layout no longer holds the
                            // pane: it was killed or reaped.
                            link.alive.store(false, Ordering::Relaxed);
                            continue;
                        };
                        let changed = {
                            let mut term = link.terminal.write();
                            let changed = term.size() != (cols, rows);
                            if changed {
                                term.resize(cols, rows);
                            }
                            changed
                        };
                        if changed {
                            if let Some(server) = server.as_ref().and_then(std::sync::Weak::upgrade)
                            {
                                server.send_to_session(
                                    &session_id,
                                    ServerMessage::resize(cols as u16, rows as u16),
                                );
                            }
                        }
                    }
                    TmuxNotification::WindowClose { window_id } if window_id == link.window => {
                        link.alive.store(false, Ordering::Relaxed);
                    }
                    TmuxNotification::Exit => {
                        link.alive.store(false, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
            if !link.alive.load(Ordering::Relaxed) {
                break;
            }
        }
        link.alive.store(false, Ordering::Relaxed);
        if !link.closed.load(Ordering::Relaxed) {
            if let Some(server) = server.and_then(|weak| weak.upgrade()) {
                server.close_session(&session_id, "mux pane closed".to_string());
            }
        }
    });
}

/// Hand pane output to the session broadcaster, or buffer it until
/// `setup_session` connects one.
fn forward(link: &MirrorLink, data: &[u8]) {
    match &mut *link.sink.lock() {
        Sink::Buffering(buffer) => buffer.extend_from_slice(data),
        Sink::Live(sender) => {
            let _ = sender.try_send(String::from_utf8_lossy(data).into_owned());
        }
        Sink::Closed => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_keys_line_matches_reference() {
        // Every byte value on its own...
        for b in 0..=u8::MAX {
            assert_eq!(
                send_keys_line(7, &[b]),
                send_keys_line_reference(7, &[b]),
                "byte {b}"
            );
        }
        // ...and mixed chunks, including INPUT_CHUNK-sized ones (the
        // paste path splits there) and the empty chunk.
        let mixed: Vec<u8> = (0..=u8::MAX).cycle().take(3 * 1024).collect();
        for chunk in [&mixed[..0], &mixed[..1], &mixed, &mixed[..INPUT_CHUNK]] {
            assert_eq!(
                send_keys_line(42, chunk),
                send_keys_line_reference(42, chunk),
                "chunk len {}",
                chunk.len()
            );
        }
        assert_eq!(
            send_keys_line(0, &[0xde, 0xad, 0xbe, 0xef]),
            "send-keys -t %0 -H de ad be ef"
        );
    }

    /// ENH-031 measurement: 256 KiB encode, new vs the per-byte
    /// `format!` reference. `#[ignore]`d — a timing loop, not a gate.
    #[test]
    #[ignore = "timing measurement, run explicitly"]
    fn send_keys_line_256kib_timing() {
        let chunk: Vec<u8> = (0..=u8::MAX).cycle().take(256 * 1024).collect();
        let rounds = 20;

        let start = std::time::Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(send_keys_line(3, &chunk));
        }
        let new_elapsed = start.elapsed();

        let start = std::time::Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(send_keys_line_reference(3, &chunk));
        }
        let reference_elapsed = start.elapsed();

        println!(
            "256 KiB encode x{rounds}: nibble-table {new_elapsed:?}, reference {reference_elapsed:?}, speedup {:.1}x",
            reference_elapsed.as_secs_f64() / new_elapsed.as_secs_f64()
        );
    }

    #[test]
    fn pane_info_parses() {
        assert_eq!(
            parse_pane_info("%3 @1 120x40"),
            Some(("@1".to_string(), 120, 40))
        );
        assert_eq!(parse_pane_info("%3 @1"), None);
    }

    #[test]
    fn layout_leaf_sizes_are_found_by_pane_id() {
        let single = "b25d,80x24,0,0,0";
        assert_eq!(pane_size_in_layout(single, 0), Some((80, 24)));
        let split = "0000,81x24,0,0{40x24,0,0,1,40x24,41,0,2}";
        assert_eq!(pane_size_in_layout(split, 1), Some((40, 24)));
        assert_eq!(pane_size_in_layout(split, 2), Some((40, 24)));
        let nested = "0000,80x24,0,0[80x12,0,0,3,80x11,0,13{40x11,0,13,4,39x11,41,13,5}]";
        assert_eq!(pane_size_in_layout(nested, 3), Some((80, 12)));
        assert_eq!(pane_size_in_layout(nested, 5), Some((39, 11)));
        assert_eq!(pane_size_in_layout(nested, 9), None);
    }

    // --- Against a real in-process daemon -------------------------------

    use crate::mux::{MuxClient, MuxServer};
    use crate::streaming::{ConnectionParams, StreamingConfig};
    use std::time::{Duration, Instant};

    /// Unix echoes the typed line with the quotes; cmd.exe with the caret.
    /// Only the EXECUTED output holds the joined marker.
    #[cfg(unix)]
    const TYPED_BEFORE: &str = r#"echo SEED""-BEFORE"#;
    #[cfg(windows)]
    const TYPED_BEFORE: &str = "echo SEED^-BEFORE";
    #[cfg(unix)]
    const TYPED_AFTER: &str = r#"echo DELTA""-AFTER"#;
    #[cfg(windows)]
    const TYPED_AFTER: &str = "echo DELTA^-AFTER";

    /// Bound on the graceful exit of [`daemon`]'s serving thread. Only a
    /// wedged `run()` exceeds it, and by then the test is over anyway.
    const SERVING_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

    /// Owns [`daemon`]'s serving thread: Drop raises the shutdown flag and
    /// joins — bounded — so a finished test cannot leak a detached daemon
    /// thread holding the socket into whatever runs next (QA-185).
    struct ServingGuard {
        shutdown: std::sync::Arc<AtomicBool>,
        serving: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for ServingGuard {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::Relaxed);
            if let Some(serving) = self.serving.take() {
                let (done_tx, done_rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = serving.join();
                    let _ = done_tx.send(());
                });
                if done_rx.recv_timeout(SERVING_EXIT_TIMEOUT).is_err() {
                    eprintln!(
                        "mux_factory daemon still serving {SERVING_EXIT_TIMEOUT:?} after shutdown"
                    );
                }
            }
        }
    }

    /// A served daemon with one session; returns its socket dir (keep it
    /// alive), socket path, the serving-thread guard, a control client, and
    /// the first pane id `%N`.
    fn daemon() -> (tempfile::TempDir, PathBuf, ServingGuard, MuxClient, String) {
        let dir = tempfile::Builder::new()
            .prefix("par-mux-str-")
            .tempdir()
            .expect("temp dir");
        let socket = dir.path().join("s");
        let server = MuxServer::bind(&socket).expect("bind");
        let shutdown = server.shutdown_handle();
        let serving = std::thread::spawn(move || server.run());
        let guard = ServingGuard {
            shutdown,
            serving: Some(serving),
        };
        let mut control = MuxClient::connect(&socket).expect("control client");
        control.send("new-session -s stream").expect("new-session");
        let pane = control
            .send("list-panes")
            .expect("list-panes")
            .into_iter()
            .map(|l| l.trim().to_string())
            .find(|l| l.starts_with('%'))
            .expect("a pane");
        (dir, socket, guard, control, pane)
    }

    /// The session machinery spawns tasks (`resolve_session` starts the
    /// broadcaster, `setup_session` the resize forwarder), so tests enter a
    /// multi-thread runtime. The test body stays on the plain test thread:
    /// its blocking waits and writes never occupy a runtime worker.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    }

    /// A streaming server whose sessions mirror panes of the daemon.
    fn streaming(socket: &std::path::Path) -> Arc<StreamingServer> {
        let factory = Arc::new(MuxSessionFactory::new(
            socket,
            MuxPaneSelector::FromSessionId,
        ));
        let server = Arc::new(StreamingServer::with_factory(
            "127.0.0.1:0".to_string(),
            StreamingConfig::default(),
            Arc::clone(&factory) as Arc<dyn SessionFactory>,
        ));
        factory.set_streaming_server(Arc::clone(&server));
        server
    }

    /// Open the streaming session mirroring `pane` (`%N`).
    fn mirror(server: &Arc<StreamingServer>, pane: &str) -> Arc<StreamSessionState> {
        let params = ConnectionParams {
            session_id: format!("pane-{}", pane.trim_start_matches('%')),
            readonly: false,
            preset: None,
        };
        server.resolve_session(&params).expect("mux-backed session")
    }

    fn has_line(text: &str, marker: &str) -> bool {
        text.lines().any(|l| l.trim() == marker)
    }

    fn wait(what: &str, mut ready: impl FnMut() -> Option<String>) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match ready() {
                None => return,
                Some(state) => assert!(
                    Instant::now() < deadline,
                    "never saw {what}; last state:\n{state}"
                ),
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_mirror(session: &StreamSessionState, marker: &str) {
        wait(marker, || {
            let text = session.terminal.read().content();
            (!has_line(&text, marker)).then_some(text)
        });
    }

    /// Type `line` + Enter through the session's input path, the way the
    /// streaming server forwards a client's Input message.
    fn type_line(session: &StreamSessionState, line: &str) {
        let writer = session
            .pty_writer
            .read()
            .as_ref()
            .cloned()
            .expect("a mux-backed session accepts input");
        let mut w = writer.lock();
        w.write_all(line.as_bytes()).expect("input write");
        w.write_all(b"\r").expect("enter");
        w.flush().expect("flush");
    }

    fn pane_size(control: &mut MuxClient, pane: &str) -> (usize, usize) {
        let info = control
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info")
            .join("");
        let (_, cols, rows) = parse_pane_info(&info).expect("pane-info reply");
        (cols, rows)
    }

    #[test]
    fn a_mirror_is_seeded_from_the_pane_then_follows_its_output() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane) = daemon();

        // Output that exists BEFORE the viewer attaches arrives via the seed.
        control
            .send(&format!("send-keys -t {pane} '{TYPED_BEFORE}' Enter"))
            .expect("send-keys");
        wait("SEED-BEFORE in the pane", || {
            let text = control
                .send(&format!("capture-pane -t {pane}"))
                .expect("capture")
                .join("\n");
            (!has_line(&text, "SEED-BEFORE")).then_some(text)
        });

        let server = streaming(&socket);
        let session = mirror(&server, &pane);
        let seeded = session.terminal.read().content();
        assert!(
            has_line(&seeded, "SEED-BEFORE"),
            "the seed replays the pane's existing screen:\n{seeded}"
        );

        // Output produced AFTER the attach arrives as applied %output, and it
        // is driven by the viewer's own input (send-keys -H).
        type_line(&session, TYPED_AFTER);
        wait_mirror(&session, "DELTA-AFTER");
    }

    #[test]
    fn the_mirror_starts_at_the_pane_size_and_a_viewer_resize_refits_the_pane() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane) = daemon();
        control
            .send(&format!("refresh-client -t {pane} -C 100x30"))
            .expect("the desktop's size");

        let server = streaming(&socket);
        let session = mirror(&server, &pane);
        assert_eq!(
            session.terminal.read().size(),
            (100, 30),
            "the mirror seeds at the PANE's size, not the viewer's"
        );
        assert_eq!(
            pane_size(&mut control, &pane),
            (100, 30),
            "connecting a viewer must not resize the pane"
        );

        // A viewer resize is a deliberate request: latest-resize-wins. The
        // pane re-fits and the mirror follows via %layout-change.
        session.resize_tx.send((70, 20)).expect("resize request");
        wait("the pane and mirror at 70x20", || {
            let pane_now = pane_size(&mut control, &pane);
            let mirror_now = session.terminal.read().size();
            (pane_now != (70, 20) || mirror_now != (70, 20))
                .then(|| format!("pane {pane_now:?}, mirror {mirror_now:?}"))
        });

        // A resize from elsewhere (the desktop) is followed too.
        control
            .send(&format!("refresh-client -t {pane} -C 90x25"))
            .expect("desktop resize");
        wait("the mirror at 90x25", || {
            let now = session.terminal.read().size();
            (now != (90, 25)).then(|| format!("{now:?}"))
        });
    }

    /// Mouse reports: the server's Mouse arm encodes with the session
    /// terminal's `report_mouse` and writes the bytes to `pty_writer`. For a
    /// mirror that only works if the pane's mouse mode reached it as a delta.
    /// Unix-only: the pane runs `cat -v` to make the received report visible.
    #[cfg(unix)]
    #[test]
    fn a_mouse_report_encoded_by_the_mirror_reaches_the_pane() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane) = daemon();
        let server = streaming(&socket);
        let session = mirror(&server, &pane);

        type_line(&session, r#"printf '\033[?1000h'; echo MOUSE""-ON; cat -v"#);
        wait_mirror(&session, "MOUSE-ON");
        wait("the mirror in mouse mode", || {
            let mode = session.terminal.read().mouse_mode();
            (mode == crate::mouse::MouseMode::Off).then(|| format!("{mode:?}"))
        });

        let report = session
            .terminal
            .write()
            .report_mouse(crate::mouse::MouseEvent::new(0, 4, 2, true, 0));
        assert_eq!(report, b"\x1b[M %#", "an X10 press at col 4 row 2");
        let writer = session
            .pty_writer
            .read()
            .as_ref()
            .cloned()
            .expect("input path");
        {
            let mut w = writer.lock();
            w.write_all(&report).expect("mouse write");
            w.write_all(b"\r").expect("flush the line to cat");
            w.flush().expect("flush");
        }
        wait("the report echoed by cat -v in the pane", || {
            let text = control
                .send(&format!("capture-pane -t {pane}"))
                .expect("capture")
                .join("\n");
            (!text.contains("^[[M %#")).then_some(text)
        });
    }

    #[test]
    fn two_viewers_of_one_pane_both_stay_live() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, _control, pane) = daemon();

        // Two streaming servers stand for two independent mobile viewers:
        // each holds its own daemon connection to the same pane.
        let viewer_a = streaming(&socket);
        let viewer_b = streaming(&socket);
        let a = mirror(&viewer_a, &pane);
        let b = mirror(&viewer_b, &pane);

        type_line(&a, TYPED_AFTER);
        wait_mirror(&a, "DELTA-AFTER");
        wait_mirror(&b, "DELTA-AFTER");
    }

    /// Every typed frame, alternating the queued path and the direct writer
    /// across resizes and a second viewer, lands exactly once in the
    /// pane-side file.
    #[test]
    fn stress_input_frames_of_varied_sizes_all_land_exactly_once() {
        let rt = runtime();
        let _entered = rt.enter();
        let (dir, socket, _serving, _control, pane) = daemon();
        let viewer_a = streaming(&socket);
        // A second viewer doubles the pane's %output fanout and the daemon
        // connections racing the input socket.
        let viewer_b = streaming(&socket);
        let a = mirror(&viewer_a, &pane);
        let _b = mirror(&viewer_b, &pane);

        // Keystroke-sized frames (the original observation was ~6-16 B
        // sends): everything typed behind a running `cat` is one file line
        // per marker, so a 4-char marker + CR is a true 5 B input frame.
        #[cfg(unix)]
        let mut expected_cat: Vec<String> = Vec::new();
        #[cfg(unix)]
        {
            let cat_file = dir.path().join("keystrokes.txt");
            type_line(&a, &format!("cat >> {}", cat_file.display()));
            // The shell opens the redirect target before exec, so the file
            // existing means `cat` owns the terminal's input from here on.
            let deadline = Instant::now() + Duration::from_secs(10);
            while !cat_file.exists() {
                assert!(Instant::now() < deadline, "cat never started");
                std::thread::sleep(Duration::from_millis(20));
            }
            for i in 0..30u32 {
                let marker = format!("T{i:02}");
                expected_cat.push(marker.clone());
                let frame = format!("{marker}\r");
                if i % 2 == 0 {
                    a.enqueue_pty_input(frame.into_bytes());
                } else {
                    let writer = a.pty_writer.read().as_ref().cloned().expect("input path");
                    let mut w = writer.lock();
                    w.write_all(frame.as_bytes()).expect("keystroke write");
                    w.flush().expect("flush");
                }
            }
            // EOF ends `cat`: the pane is a shell again for phase 2. It
            // must take the same queue as the frames above it — a direct
            // writer write races the drain and lands first, so `cat`
            // exits and the queued frames arrive at the shell instead of
            // the file.
            a.enqueue_pty_input(b"\x04".to_vec());
            let deadline = Instant::now() + Duration::from_secs(15);
            while expected_cat.len()
                != std::fs::read_to_string(&cat_file)
                    .map(|t| t.lines().count())
                    .unwrap_or(0)
            {
                assert!(
                    Instant::now() < deadline,
                    "keystroke frames never fully appended; rerun with DEBUG_LEVEL=1 — \
                     the debug log names the dropping side"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            let mut want = expected_cat;
            let mut got: Vec<String> = std::fs::read_to_string(&cat_file)
                .expect("keystrokes file")
                .lines()
                .map(str::trim)
                .map(str::to_string)
                .collect();
            want.sort();
            got.sort();
            assert_eq!(got, want, "keystroke frames lost or duplicated");
        }

        let file = dir.path().join("frames.txt");
        // Marker byte lengths spanning the observed small-frame range; the
        // full typed line (echo + redirect + temp path) runs ~60-330 B.
        const SIZES: [usize; 6] = [12, 24, 64, 120, 180, 240];
        let mut expected: Vec<String> = Vec::new();
        let have = || {
            std::fs::read_to_string(&file)
                .map(|t| t.lines().count())
                .unwrap_or(0)
        };
        for batch in 0..3 {
            for cycle in 0..5 {
                for (i, &size) in SIZES.iter().enumerate() {
                    let index = batch * 5 * SIZES.len() + cycle * SIZES.len() + i;
                    let mut marker = format!("F{batch}_{cycle:02}_{size}");
                    marker.extend(std::iter::repeat_n('x', size - marker.len()));
                    expected.push(marker.clone());
                    let cmd = format!("echo {marker} >> {}", file.display());
                    if index.is_multiple_of(2) {
                        let mut bytes = cmd.into_bytes();
                        bytes.push(b'\r');
                        a.enqueue_pty_input(bytes);
                    } else {
                        type_line(&a, &cmd);
                    }
                    // Resizes interleaved with typing: the original
                    // observation sat next to resize traffic.
                    if index % 10 == 9 {
                        let to = if (index / 10).is_multiple_of(2) {
                            (120, 40)
                        } else {
                            (80, 24)
                        };
                        a.resize_tx.send(to).expect("resize request");
                    }
                    // Wait for each line before the next: a PTY's kernel
                    // input queue (~1 KB, canonical while the shell runs a
                    // child) drops bytes typed faster than the pane drains
                    // — plain bash + script(1) loses 24/30 the same way, so
                    // that is tty semantics, not this input path. The
                    // barrier keeps the send rate within what a pane can
                    // consume while preserving the size coverage.
                    let want = expected.len();
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while have() < want {
                        assert!(
                            Instant::now() < deadline,
                            "frame {index}: {}/{} lines reached the pane; rerun \
                             with DEBUG_LEVEL=1 — the debug log names the side",
                            have(),
                            want
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        }

        let got: Vec<String> = std::fs::read_to_string(&file)
            .expect("frames file")
            .lines()
            .map(str::trim)
            .map(str::to_string)
            .collect();
        let mut want = expected;
        let mut got_sorted = got.clone();
        want.sort();
        got_sorted.sort();
        let missing: Vec<&String> = want.iter().filter(|m| !got.contains(*m)).collect();
        let extra: Vec<&String> = got.iter().filter(|m| !want.contains(*m)).collect();
        assert_eq!(
            got_sorted, want,
            "frames lost = {missing:?}, extra/duplicated = {extra:?}"
        );

        // Teardown: the drain task `enqueue_pty_input` spawned lives until
        // every channel sender drops, and this session outlives the test in
        // the streaming server's registry — so drop the sender here or
        // tokio's blocking-pool shutdown blocks past the test's end.
        *a.pty_input_tx.write() = None;
    }

    #[test]
    fn a_killed_pane_marks_its_mirror_dead() {
        let (_dir, socket, _serving, mut control, pane) = daemon();
        let second = control
            .send(&format!("split-window -h -t {pane}"))
            .expect("split")
            .join("")
            .trim()
            .to_string();
        let factory = Arc::new(MuxSessionFactory::new(
            &socket,
            MuxPaneSelector::FromSessionId,
        ));
        let session_id = format!("pane-{}", second.trim_start_matches('%'));
        factory
            .create_session(&session_id, 80, 24, None)
            .expect("mirror the split pane");
        assert!(factory.is_session_alive(&session_id));

        control
            .send(&format!("kill-pane -t {second}"))
            .expect("kill-pane");
        wait("the mirror marked dead", || {
            factory
                .is_session_alive(&session_id)
                .then(|| "still alive".to_string())
        });
    }

    /// A daemon that accepts and never replies must fail `create_session`
    /// by its reply deadline instead of blocking the caller forever.
    #[test]
    fn create_session_times_out_against_a_silent_daemon() {
        let dir = tempfile::Builder::new()
            .prefix("par-mux-silent-")
            .tempdir()
            .expect("temp dir");
        let socket = dir.path().join("s");
        let listener = crate::mux::ipc::bind_local_listener(&socket).expect("bind");
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let silent = std::thread::spawn(move || {
            let (stream, _abort) = crate::mux::ipc::accept_connection(&listener).expect("accept");
            // Hold the connection open, unanswered, until the test is done.
            let _ = release_rx.recv_timeout(Duration::from_secs(30));
            drop(stream);
        });

        let timeout = Duration::from_millis(300);
        let factory = MuxSessionFactory::new(&socket, MuxPaneSelector::FromSessionId)
            .with_reply_timeout(timeout);
        let started = Instant::now();
        // "main" is not `pane-N`, so the factory must ask with list-panes.
        let result = factory.create_session("main", 80, 24, None);
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        silent.join().expect("silent daemon thread");

        let err = match result {
            Ok(_) => panic!("a silent daemon cannot produce a session"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("no reply"), "unexpected error: {err}");
        assert!(
            elapsed >= timeout && elapsed < timeout + Duration::from_secs(5),
            "create_session returned after {elapsed:?}, deadline {timeout:?}"
        );
    }

    #[test]
    fn input_becomes_hex_send_keys_the_daemon_parses() {
        let line = send_keys_line(7, b"a\x1b\"'\n");
        assert_eq!(
            crate::mux::parse_command(&line).expect("the daemon accepts the line"),
            crate::mux::MuxCommand::SendKeys {
                pane: crate::mux::ids::Target::Id(crate::mux::PaneId(7)),
                keys: crate::mux::command::SendKeysPayload(vec![
                    crate::mux::command::SendKeysPart::Bytes(b"a\x1b\"'\n".to_vec())
                ]),
            }
        );
    }
}
