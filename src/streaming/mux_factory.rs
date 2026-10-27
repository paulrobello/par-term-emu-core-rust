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
//! The seed (`refresh-client -t %N -G` — the screen replay plus the pane's
//! graphics placements, with a plain-form retry when an older daemon
//! rejects the flag) is read in `create_session` against a reply deadline,
//! and `%output` pushed before its reply block is discarded (the replay
//! already reflects it); everything after is applied. Ordering is
//! the reason this does not use [`crate::mux::MuxClient`], which splits
//! replies and notifications into separate channels.
//!
//! `create_session` blocks on daemon I/O, so the streaming server calls it
//! off the async runtime (`spawn_blocking`).
//!
//! Size policy: the daemon's smallest-attached-client rule — a window's
//! extent is the componentwise minimum over the render clients displaying
//! it, and a streaming viewer's `Resize` becomes `refresh-client -t %N -C
//! WxH`, one sizing contribution per streaming server connection. The
//! mirror is seeded at the pane's current size (`pane-info`), never the
//! viewer's, and re-fits only when a `%layout-change` changes that pane's
//! rectangle.

use crate::mux::{connect_local_stream, LocalStream};
use crate::streaming::error::StreamingError;
use crate::streaming::protocol::{ServerMessage, MAX_INLINE_IMAGE_BYTES};
use crate::streaming::server::{
    SessionFactory, SessionFactoryResult, StreamingServer, UploadForwarder,
};
use crate::streaming::session::StreamSessionState;
use crate::streaming::{apply_inline_image, inline_image_frame};
use crate::terminal::{Terminal, TerminalEvent};
use crate::tmux_control::{TmuxControlParser, TmuxNotification};
use base64::Engine as _;
use interprocess::TryClone as _;
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
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

/// The session id that makes a [`MuxPaneSelector::FromSessionId`] mirror
/// follow the daemon's active workspace: each `%workspaces-changed` the
/// daemon broadcasts re-resolves the mirror to the active workspace's
/// active pane and re-seeds the viewers in place, no reconnect.
pub const ACTIVE_WORKSPACE_SESSION_ID: &str = "workspace-active";

/// Which daemon pane a streaming session mirrors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxPaneSelector {
    /// The same pane for every session.
    Pane(u32),
    /// Parse the streaming session id: `pane-N` mirrors pane N;
    /// [`ACTIVE_WORKSPACE_SESSION_ID`] follows the daemon's active
    /// workspace; any other id falls back to the daemon's first pane
    /// (`list-panes`).
    FromSessionId,
}

/// A [`SessionFactory`] whose sessions mirror par-mux panes.
pub struct MuxSessionFactory {
    socket: PathBuf,
    selector: MuxPaneSelector,
    scrollback: usize,
    reply_timeout: Duration,
    sessions: RwLock<HashMap<String, Arc<MirrorLink>>>,
    /// One client-upload pipe per session (the upload verbs' own daemon
    /// connection); registered at session creation, removed at teardown.
    pipes: RwLock<HashMap<String, Arc<MuxUploadPipe>>>,
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
    /// The pane this mirror serves. `workspace-active` mirrors retarget it
    /// when the daemon's active workspace moves; input and size
    /// contributions read it at call time so they follow.
    pane: Mutex<u32>,
    /// The pane's window (`@W`), which its layout changes and close name.
    /// Retargeted together with `pane` so the pair stays consistent.
    window: Mutex<String>,
    /// Set at creation: only [`ACTIVE_WORKSPACE_SESSION_ID`] mirrors
    /// re-resolve on `%workspaces-changed`; `pane-N` mirrors stay pinned.
    follows_active: bool,
    terminal: Arc<RwLock<Terminal>>,
    writer: Arc<Mutex<Option<LocalStream>>>,
    /// False once the daemon connection closed or the pane went away.
    alive: AtomicBool,
    /// Set by teardown so the drain thread stops forwarding.
    closed: AtomicBool,
    /// Output buffered until `setup_session` connects the broadcaster
    /// (`Buffering`), then the broadcaster's sender (`Live`).
    sink: Mutex<Sink>,
    /// Graphics ids already streamed as InlineImage frames. A placement
    /// raises `GraphicsAdded` exactly once per mirror, but re-fits re-seed
    /// and a future seed carrying graphics could re-raise an id — this is
    /// the once-only guard, so a graphic is sent once, never on every poll.
    sent_images: Mutex<HashSet<u64>>,
    status: Mutex<Option<(ServerMessage, Vec<String>)>>,
    session: RwLock<Option<std::sync::Weak<StreamSessionState>>>,
}

impl MirrorLink {
    fn pane(&self) -> u32 {
        *self.pane.lock()
    }

    fn window(&self) -> String {
        self.window.lock().clone()
    }

    /// Claim a graphics id for streaming; `false` when this mirror already
    /// sent it, so a re-raised announce never re-streams the frame.
    fn claim_image(&self, image_id: u64) -> bool {
        self.sent_images.lock().insert(image_id)
    }

    /// Move the mirror to a new pane, window pair. The drain thread is the
    /// only writer, so the pair is never observed half-applied.
    fn set_target(&self, pane: u32, window: String) {
        *self.pane.lock() = pane;
        *self.window.lock() = window;
    }
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
            pipes: RwLock::new(HashMap::new()),
            streaming_server: RwLock::new(None),
        }
    }

    /// Scrollback lines kept by each mirror terminal (default 10 000).
    pub fn with_scrollback(mut self, lines: usize) -> Self {
        self.scrollback = lines;
        self
    }

    /// A test's handshake reply deadline in place of [`MUX_REPLY_TIMEOUT`]:
    /// shorter so a timeout test runs in milliseconds, or the hang ceiling
    /// so a starved host cannot fail a healthy handshake.
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

    /// Test-only observable: how many mirror links the factory holds.
    /// The dead-upstream close path's teardown leg removes the link; the
    /// pre-fix panic at the spawn left it parked forever (the production
    /// leak). `is_session_alive` cannot discriminate — it flips on the
    /// `alive` flag well before the close.
    #[cfg(test)]
    fn mirror_link_count(&self) -> usize {
        self.sessions.read().len()
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
                if session_id == ACTIVE_WORKSPACE_SESSION_ID {
                    let (session, pane) =
                        query_active_workspace_pane(writer, reader, self.reply_timeout)?;
                    debug_log_active_workspace(&session, pane);
                    return Ok(pane);
                }
                let body = command(writer, reader, "list-panes", self.reply_timeout)?;
                body.iter()
                    .find_map(|l| l.trim().strip_prefix('%').and_then(|n| n.parse().ok()))
                    .ok_or_else(|| io::Error::other("the daemon has no panes"))
            }
        }
    }
}

/// Issue `switch-client` bare (the daemon answers the active workspace's
/// active session) and `list-panes -t $N` (its active window's panes, the
/// active one marked `*`), and pair the two into the pane to mirror. Both
/// commands are read-only daemon queries.
fn query_active_workspace_pane(
    writer: &mut LocalStream,
    reader: &MuxLines,
    timeout: Duration,
) -> io::Result<(String, u32)> {
    let session = command(writer, reader, "switch-client", timeout)?;
    let session = active_session_from_reply(&session)
        .ok_or_else(|| io::Error::other("the daemon has no active workspace session"))?;
    let panes = command(writer, reader, &format!("list-panes -t {session}"), timeout)?;
    let pane = active_pane_from_listing(&panes).ok_or_else(|| {
        io::Error::other(format!("the active session {session} has no active pane"))
    })?;
    Ok((session, pane))
}

/// The `$N` of a bare `switch-client` reply, `None` when the daemon has no
/// active session (an empty reply body).
fn active_session_from_reply(reply: &[String]) -> Option<String> {
    reply
        .first()
        .map(|l| l.trim())
        .filter(|l| l.starts_with('$'))
        .map(str::to_string)
}

/// The `*`-marked pane of a `list-panes -t <session>` reply (`%N leaf
/// marker` rows).
fn active_pane_from_listing(reply: &[String]) -> Option<u32> {
    reply.iter().find_map(
        |line| match line.split_whitespace().collect::<Vec<_>>().as_slice() {
            [pane, _leaf, "*"] => pane.strip_prefix('%').and_then(|n| n.parse().ok()),
            _ => None,
        },
    )
}

/// Why a workspace-active mirror resolved where it did: one log line per
/// resolution, so a phone showing the "wrong" pane is diagnosable from the
/// streamer log alone.
fn debug_log_active_workspace(session: &str, pane: u32) {
    crate::debug_error!(
        "STREAMING",
        "workspace-active mirror resolves to session {session} pane %{pane}"
    )
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
    fn spawn(stream: LocalStream) -> io::Result<Self> {
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

fn parse_viewer_status(line: &str) -> Option<(ServerMessage, Vec<String>)> {
    let (_, cols, rows) = parse_pane_info(line)?;
    let tokens: HashMap<_, _> = line
        .split_whitespace()
        .skip(3)
        .filter_map(|token| token.split_once('='))
        .collect();
    let owner = |key| {
        tokens
            .get(key)
            .filter(|v| **v != "-")
            .map(|v| (*v).to_string())
    };
    let viewers = tokens
        .get("viewers")?
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    Some((
        ServerMessage::PaneSizeStatus {
            cols: cols as u16,
            rows: rows as u16,
            column_viewer: owner("constraint_cols"),
            row_viewer: owner("constraint_rows"),
        },
        viewers,
    ))
}

fn apply_viewer_status(link: &MirrorLink, line: &str) {
    if let Some((size, viewers)) = parse_viewer_status(line) {
        let mut status = link.status.lock();
        *status = Some((size.clone(), viewers.clone()));
        if let Some(session) = link
            .session
            .read()
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        {
            session.update_mux_status(size, viewers);
        }
    }
}

fn query_viewer_status(link: &MirrorLink) -> io::Result<()> {
    let mut writer = link.writer.lock();
    let writer = writer
        .as_mut()
        .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?;
    writeln!(writer, "pane-info -t %{} -M", link.pane())?;
    writer.flush()
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
/// Hex bypasses the command grammar's quoting entirely. The pane is read
/// at write time so a `workspace-active` mirror's input follows its
/// retarget; one write() call stays on one pane even across a mid-paste
/// switch.
struct SendKeysWriter {
    link: Arc<MirrorLink>,
    stream: Arc<Mutex<Option<LocalStream>>>,
}

impl Write for SendKeysWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let pane = self.link.pane();
        let mut guard = self.stream.lock();
        let stream = guard
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?;
        for chunk in buf.chunks(INPUT_CHUNK) {
            writeln!(stream, "{}", send_keys_line(pane, chunk))?;
        }
        stream.flush()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream
            .lock()
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
            .flush()
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
            &format!("pane-info -t %{pane} -M"),
            timeout,
        )
        .map_err(|e| fail("pane-info", e))?;
        let (window, cols, rows) = info
            .first()
            .and_then(|l| parse_pane_info(l))
            .ok_or_else(|| fail("pane-info", io::Error::other(format!("bad reply {info:?}"))))?;

        let mut terminal = Terminal::with_scrollback(cols, rows, self.scrollback);
        let seed = seed_reply(&mut writer, &reader, pane, timeout)
            .map_err(|e| fail("refresh-client -t", e))?;
        let (screen, seed_graphics) = split_graphics_seed(&seed);
        terminal.process(screen.as_bytes());
        // The daemon's pane answers terminal queries itself; the mirror's
        // copies would double every reply.
        terminal.drain_responses();
        ingest_graphics_seed(&mut terminal, &seed_graphics);
        let terminal = Arc::new(RwLock::new(terminal));

        let writer = Arc::new(Mutex::new(Some(writer)));
        let link = Arc::new(MirrorLink {
            follows_active: session_id == ACTIVE_WORKSPACE_SESSION_ID,
            pane: Mutex::new(pane),
            window: Mutex::new(window),
            terminal: Arc::clone(&terminal),
            writer: Arc::clone(&writer),
            alive: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            sink: Mutex::new(Sink::Buffering(Vec::new())),
            sent_images: Mutex::new(HashSet::new()),
            status: Mutex::new(info.first().and_then(|line| parse_viewer_status(line))),
            session: RwLock::new(None),
        });
        spawn_drain(
            Arc::clone(&link),
            reader,
            session_id.to_string(),
            self.socket.clone(),
            self.streaming_server.read().clone(),
        );
        self.sessions
            .write()
            .insert(session_id.to_string(), Arc::clone(&link));
        self.pipes.write().insert(
            session_id.to_string(),
            Arc::new(MuxUploadPipe::new(
                Arc::clone(&link),
                self.socket.clone(),
                self.reply_timeout,
            )),
        );

        let input: Box<dyn Write + Send> = Box::new(SendKeysWriter {
            link: Arc::clone(&link),
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
        {
            let status = link.status.lock();
            *link.session.write() = Some(Arc::downgrade(session));
            if let Some((size, viewers)) = &*status {
                session.update_mux_status(size.clone(), viewers.clone());
            }
        }
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
        // (the daemon re-fits to the smallest-viewer minimum); the mirror
        // follows via %layout-change. The pane is read per request so a
        // workspace-active mirror's size contributions follow its
        // retarget.
        let resize_rx = session.get_resize_receiver();
        let writer = Arc::clone(&link.writer);
        let link_for_resize = Arc::clone(&link);
        tokio::spawn(async move {
            let mut rx = resize_rx.lock().await;
            while let Some((cols, rows)) = rx.recv().await {
                // The writer mutex is shared with the input path and the
                // socket write can block on the daemon: both stay off the
                // runtime worker.
                let pane = link_for_resize.pane();
                let writer = Arc::clone(&writer);
                let sent = tokio::task::spawn_blocking(move || {
                    let mut guard = writer.lock();
                    let stream = guard
                        .as_mut()
                        .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?;
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
        self.pipes.write().remove(session_id);
        if let Some(link) = self.sessions.write().remove(session_id) {
            link.closed.store(true, Ordering::Relaxed);
            *link.sink.lock() = Sink::Closed;
            if let Some(mut writer) = link.writer.lock().take() {
                // Wake the drain on a quiet pane so it drops MuxLines too.
                let _ = writeln!(writer, "list-panes");
                let _ = writer.flush();
            }
        }
    }

    fn close_on_last_disconnect(&self) -> bool {
        true
    }

    fn upload_pipe(&self, session_id: &str) -> Option<std::sync::Arc<dyn UploadForwarder>> {
        self.pipes
            .read()
            .get(session_id)
            .cloned()
            .map(|pipe| pipe as std::sync::Arc<dyn UploadForwarder>)
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
    socket: PathBuf,
    server: Option<std::sync::Weak<StreamingServer>>,
) {
    std::thread::spawn(move || {
        let mut parser = TmuxControlParser::new(true);
        let mut in_reply = false;
        let mut reply_body = Vec::new();
        let mut next_status = Instant::now();
        loop {
            if link.closed.load(Ordering::Relaxed) {
                break;
            }
            // The reply scan's pane filter is a pass-level snapshot; the
            // notification arms below read the link live so a retarget that
            // happens mid-batch takes effect for the remaining lines.
            let pane_num = link.pane();
            let pane_id = format!("%{pane_num}");
            if Instant::now() >= next_status {
                if query_viewer_status(&link).is_err() {
                    break;
                }
                next_status = Instant::now() + Duration::from_secs(1);
            }
            let line = match reader
                .rx
                .recv_timeout(next_status.saturating_duration_since(Instant::now()))
            {
                Ok(Ok(line)) => line,
                Err(RecvTimeoutError::Timeout) => continue,
                _ => break,
            };
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
                        link.pane(),
                        reply_body.join(" | ")
                    );
                } else {
                    if line.starts_with(&format!("{pane_id} ")) {
                        apply_viewer_status(&link, &line);
                    }
                    reply_body.push(line);
                }
                continue;
            }
            let mut framed = line.into_bytes();
            framed.push(b'\n');
            for notification in parser.parse(&framed) {
                match notification {
                    // The pane filter reads the link live, not the pass's
                    // snapshot: a batch can carry `%workspaces-changed`
                    // followed by more `%output`, and lines for the OLD
                    // pane must not survive the retarget that happened
                    // mid-batch.
                    TmuxNotification::Output { pane_id: id, data }
                        if id == format!("%{}", link.pane()) =>
                    {
                        let (dispatch_batch, events) = {
                            let mut term = link.terminal.write();
                            let batch = term.process_deferred(&data);
                            term.drain_responses();
                            // The events the batch raised drain under the same
                            // lock: a GraphicsAdded fetches its pixels here,
                            // once, never per subscriber.
                            (batch, term.poll_events())
                        };
                        dispatch_batch.deliver();
                        forward(&link, &data);
                        for event in events {
                            let TerminalEvent::GraphicsAdded { image_id, row } = event else {
                                // Mirrors stream text via %output itself; only
                                // the frame-bearing event needs converting.
                                continue;
                            };
                            if !link.claim_image(image_id) {
                                continue;
                            }
                            let Some(frame) = inline_image_frame(
                                &link.terminal.read(),
                                image_id,
                                Some(link.pane()),
                            ) else {
                                continue;
                            };
                            if let Some(server) = server.as_ref().and_then(std::sync::Weak::upgrade)
                            {
                                server.send_to_session(
                                    &session_id,
                                    ServerMessage::graphics_added(row as u16),
                                );
                                server.send_to_session(&session_id, frame);
                            }
                        }
                    }
                    TmuxNotification::LayoutChange {
                        window_id,
                        window_layout,
                        ..
                    } if window_id == link.window() => {
                        let Some((cols, rows)) = pane_size_in_layout(&window_layout, link.pane())
                        else {
                            // The window's new layout no longer holds the
                            // pane: it was killed or reaped.
                            link.alive.store(false, Ordering::Relaxed);
                            continue;
                        };
                        // Observer delivery waits for the guard to drop
                        // (ARC-001).
                        let dispatch_batch = {
                            let mut term = link.terminal.write();
                            (term.size() != (cols, rows)).then(|| term.resize_deferred(cols, rows))
                        };
                        if let Some(batch) = dispatch_batch {
                            batch.deliver();
                            if let Some(server) = server.as_ref().and_then(std::sync::Weak::upgrade)
                            {
                                server.send_to_session(
                                    &session_id,
                                    ServerMessage::resize(cols as u16, rows as u16),
                                );
                            }
                        }
                    }
                    TmuxNotification::ClientAttached { .. }
                    | TmuxNotification::ClientLeft { .. } => {
                        next_status = Instant::now();
                    }
                    // A desktop switch-workspace (or any roster move)
                    // re-resolves the followed pane; pinned (`pane-N`)
                    // mirrors never enter this arm. Re-resolution is
                    // idempotent — a rename or add resolves to the same
                    // pane and applies nothing.
                    TmuxNotification::WorkspacesChanged if link.follows_active => {
                        follow_active_workspace(
                            &link,
                            &session_id,
                            &socket,
                            MUX_REPLY_TIMEOUT,
                            &server,
                        );
                    }
                    TmuxNotification::WindowClose { window_id } if window_id == link.window() => {
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

/// The seed command: `refresh-client -t %N -G` — the screen replay plus a
/// `%graphics-begin`/`%graphics-end` section carrying the pane's graphics
/// placements, so a mirror's `Connected.graphics` replays images placed
/// before attach. An `%error` first line means an older daemon rejected
/// the unknown flag: retry the plain form, which seeds screen-only,
/// exactly the pre-`-G` behavior.
fn seed_reply(
    writer: &mut LocalStream,
    reader: &MuxLines,
    pane: u32,
    timeout: Duration,
) -> io::Result<Vec<String>> {
    let reply = command(
        writer,
        reader,
        &format!("refresh-client -t %{pane} -G"),
        timeout,
    )?;
    if reply.first().is_some_and(|line| line.starts_with("%error")) {
        return command(
            writer,
            reader,
            &format!("refresh-client -t %{pane}"),
            timeout,
        );
    }
    Ok(reply)
}

/// A resolved active-workspace pane, everything the re-seed needs.
struct WorkspaceTarget {
    session: String,
    pane: u32,
    window: String,
    cols: usize,
    rows: usize,
    /// The `pane-info -M` reply line, re-parsed into the viewer-status
    /// snapshot the viewers of the new pane start from.
    status_line: String,
    /// The `refresh-client -t %NEW -G` replay's screen part — the reply
    /// with its graphics section split off.
    screen: String,
    /// The seed's graphics placements, ingested after the screen replay.
    graphics: Vec<SeedGraphicFrame>,
}

/// One graphics placement the seed carried: the daemon's store entry in
/// the frame shape [`apply_inline_image`] ingests.
struct SeedGraphicFrame {
    id: u64,
    col: u32,
    row: u32,
    width: u32,
    height: u32,
    data: Vec<u8>,
}

/// Daemon placement ids plus this offset keep seed graphics out of the
/// mirror process's own graphic-id counter range: a placement parsed
/// after attach draws ids from the mirror's counter (small numbers), and
/// an unchanged seed id colliding with it would put two graphics under
/// one id in the mirror store.
const SEED_GRAPHICS_ID_OFFSET: u64 = 1 << 62;

/// Marker lines framing the seed's graphics section — the daemon side
/// writes them (crates/par-mux `graphics_seed_section`); keep the two
/// spellings in sync. A pane can print any text, so the split anchors on
/// the LAST begin marker.
const GRAPHICS_SEED_BEGIN: &str = "%graphics-begin";
const GRAPHICS_SEED_END: &str = "%graphics-end";

/// Split a seed reply into its screen part and its graphics frames. The
/// section is optional: a flagless reply (older daemon, or no graphics
/// on the pane) is all screen.
fn split_graphics_seed(lines: &[String]) -> (String, Vec<SeedGraphicFrame>) {
    let Some(begin) = lines.iter().rposition(|line| line == GRAPHICS_SEED_BEGIN) else {
        return (lines.join("\n"), Vec::new());
    };
    let Some(span) = lines[begin + 1..]
        .iter()
        .position(|line| line == GRAPHICS_SEED_END)
    else {
        return (lines.join("\n"), Vec::new());
    };
    let screen = lines[..begin].join("\n");
    let frames = lines[begin + 1..begin + 1 + span]
        .iter()
        .filter_map(|line| parse_seed_graphic(line))
        .collect();
    (screen, frames)
}

/// Parse one `%gfx <id> <col> <row> <width> <height> <base64-rgba>` line;
/// `None` skips a malformed line rather than failing the whole seed.
fn parse_seed_graphic(line: &str) -> Option<SeedGraphicFrame> {
    use base64::Engine as _;
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() != 7 || fields[0] != "%gfx" {
        return None;
    }
    let id = fields[1].parse().ok()?;
    let col = fields[2].parse().ok()?;
    let row = fields[3].parse().ok()?;
    let width = fields[4].parse().ok()?;
    let height = fields[5].parse().ok()?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(fields[6])
        .ok()?;
    Some(SeedGraphicFrame {
        id,
        col,
        row,
        width,
        height,
        data,
    })
}

/// Ingest the seed's placements into the mirror store so
/// `Connected.graphics` replays them. Oldest first, so the store stays
/// chronological and the connect replay (newest first) matches the
/// daemon's own selection. Seed ids ride [`SEED_GRAPHICS_ID_OFFSET`]; a
/// frame over the wire cap is skipped exactly like an oversize live
/// frame.
fn ingest_graphics_seed(terminal: &mut Terminal, frames: &[SeedGraphicFrame]) {
    for frame in frames.iter().rev() {
        if frame.data.len() > MAX_INLINE_IMAGE_BYTES {
            crate::debug_error!(
                "STREAMING",
                "seed graphic {} not ingested: {} payload bytes exceed the {}-byte cap",
                frame.id,
                frame.data.len(),
                MAX_INLINE_IMAGE_BYTES
            );
            continue;
        }
        let message = ServerMessage::inline_image(
            SEED_GRAPHICS_ID_OFFSET + frame.id,
            None,
            "rgba".to_string(),
            frame.width,
            frame.height,
            frame.data.clone(),
            frame.col,
            frame.row,
        );
        let _ = apply_inline_image(terminal, &message);
    }
}

/// Re-resolve the daemon's active workspace pane and, when it moved,
/// re-seed the mirror IN PLACE — same streaming session, no reconnect
/// (card 01a16ab7): reset the mirror terminal, replay the new pane's
/// screen, and push a fresh screen-restore sequence to the attached
/// viewers as an `Output` frame.
///
/// Resolution runs over a short-lived daemon connection: the resolution
/// commands must not interleave with the mirror connection's own
/// status-query reply blocks, where a reply block read here could be
/// misattributed. The drain being briefly blocked IS the ordering
/// guarantee: everything it applies before the retarget belongs to the
/// old pane, and after the seed everything it applies is filtered to the
/// new pane id.
fn follow_active_workspace(
    link: &Arc<MirrorLink>,
    session_id: &str,
    socket: &std::path::Path,
    timeout: Duration,
    server: &Option<std::sync::Weak<StreamingServer>>,
) {
    if link.closed.load(Ordering::Relaxed) {
        return;
    }
    match resolve_workspace_target(socket, link, timeout) {
        Ok(Some(target)) => apply_workspace_target(link, session_id, server, target),
        Ok(None) => {}
        Err(err) => {
            crate::debug_error!("STREAMING", "workspace-active re-resolution failed: {err}")
        }
    }
}

/// Resolve the daemon's active workspace pane over its own short-lived
/// connection, with everything the re-seed needs, or `None` when the
/// followed pane is already the active one. Errors leave the current pane
/// mirrored: the next `%workspaces-changed` retries.
fn resolve_workspace_target(
    socket: &std::path::Path,
    link: &MirrorLink,
    timeout: Duration,
) -> io::Result<Option<WorkspaceTarget>> {
    let stream = connect_local_stream(socket)?;
    let mut writer = stream.try_clone()?;
    let reader = MuxLines::spawn(stream)?;
    let (session, pane) = query_active_workspace_pane(&mut writer, &reader, timeout)?;
    if pane == link.pane() {
        return Ok(None);
    }
    let info = command(
        &mut writer,
        &reader,
        &format!("pane-info -t %{pane} -M"),
        timeout,
    )?;
    let (window, cols, rows) = info
        .first()
        .and_then(|l| parse_pane_info(l))
        .ok_or_else(|| io::Error::other(format!("bad pane-info reply {info:?}")))?;
    let status_line = info.first().cloned().unwrap_or_default();
    let seed = seed_reply(&mut writer, &reader, pane, timeout)?;
    let (screen, graphics) = split_graphics_seed(&seed);
    Ok(Some(WorkspaceTarget {
        session,
        pane,
        window,
        cols,
        rows,
        status_line,
        screen,
        graphics,
    }))
}

/// Swap the mirror to the new pane: retarget, reset the terminal, replay
/// the seed, refresh the viewer-status snapshot, then redraw the viewers
/// in place with the same screen-restore sequence a fresh connect would
/// carry.
fn apply_workspace_target(
    link: &Arc<MirrorLink>,
    session_id: &str,
    server: &Option<std::sync::Weak<StreamingServer>>,
    target: WorkspaceTarget,
) {
    if link.closed.load(Ordering::Relaxed) {
        return;
    }
    debug_log_active_workspace(&target.session, target.pane);
    link.set_target(target.pane, target.window.clone());
    let mut batches = Vec::new();
    let mut resized = false;
    {
        let mut term = link.terminal.write();
        term.reset();
        if term.size() != (target.cols, target.rows) {
            resized = true;
            batches.push(term.resize_deferred(target.cols, target.rows));
        }
        batches.push(term.process_deferred(target.screen.as_bytes()));
        term.drain_responses();
        ingest_graphics_seed(&mut term, &target.graphics);
    }
    for batch in batches {
        batch.deliver();
    }
    if !target.status_line.is_empty() {
        apply_viewer_status(link, &target.status_line);
    }
    let restore = link.terminal.read().export_screen_restore_sequence();
    if resized {
        if let Some(server) = server.as_ref().and_then(std::sync::Weak::upgrade) {
            server.send_to_session(
                session_id,
                ServerMessage::resize(target.cols as u16, target.rows as u16),
            );
        }
    }
    forward(link, restore.as_bytes());
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

/// One session's client-upload pipe ([`UploadForwarder`]). The upload
/// verbs run on their OWN short-lived daemon control connection — the
/// mirror link's connection is read solely by its drain thread, so a
/// request/reply verb cannot share it (the same shape
/// [`follow_active_workspace`] resolves over).
///
/// Ordering is the point: a client may send `upload_chunk` and
/// `upload_commit` back-to-back, so the verbs are serialized through ONE
/// bounded op queue drained by ONE worker thread (spawned with the first
/// verb). The worker alone owns the daemon connection, every verb's reply
/// block is read there, and the pane is resolved at COMMIT time from the
/// link so a retargeted workspace-active mirror pastes into the pane it
/// currently mirrors. Callers block (bounded by the reply deadline) on a
/// one-shot reply channel — the same bounded-wait shape the input path's
/// blocking writer accepts.
pub struct MuxUploadPipe {
    link: Arc<MirrorLink>,
    socket: PathBuf,
    reply_timeout: Duration,
    /// The op queue + its worker, created lazily by the first verb.
    ops: Mutex<Option<OpsHandle>>,
    /// The daemon-side id of the current/last upload: written by the
    /// worker, read by the handlers for the `upload_finished` reports.
    upload_id: Arc<Mutex<Option<u64>>>,
}

struct OpsHandle {
    sender: std::sync::mpsc::SyncSender<UploadOp>,
    _worker: std::thread::JoinHandle<()>,
}

/// One queued verb. `reply` is the caller's one-shot channel; the worker
/// sends exactly once per op.
enum UploadOp {
    Begin {
        filename: String,
        total_bytes: u64,
        reply: std::sync::mpsc::SyncSender<io::Result<u64>>,
    },
    Chunk {
        data: Vec<u8>,
        reply: std::sync::mpsc::SyncSender<io::Result<()>>,
    },
    Commit {
        reply: std::sync::mpsc::SyncSender<io::Result<String>>,
    },
    Abort {
        reply: std::sync::mpsc::SyncSender<io::Result<()>>,
    },
}

/// Verbs queued ahead of the worker before a producer blocks — dozens of
/// full-size chunks of slack; the producer (the session's ws task) is the
/// backpressure point past that.
const UPLOAD_OP_QUEUE_DEPTH: usize = 64;

/// The worker's exclusive daemon connection pair for one upload's
/// lifetime.
struct PipeConn {
    writer: LocalStream,
    reader: MuxLines,
}

/// The worker's exclusive daemon state: one connection pair for the
/// upload's lifetime and the shared id slot.
struct UploadWorker {
    conn: Option<PipeConn>,
    upload_id: Arc<Mutex<Option<u64>>>,
    link: Arc<MirrorLink>,
    socket: PathBuf,
    reply_timeout: Duration,
}

impl UploadWorker {
    fn connect(&mut self) -> io::Result<PipeConn> {
        let stream = connect_local_stream(&self.socket)?;
        let writer = stream.try_clone()?;
        let reader = MuxLines::spawn(stream)?;
        Ok(PipeConn { writer, reader })
    }

    fn run(mut self, receiver: std::sync::mpsc::Receiver<UploadOp>) {
        while let Ok(op) = receiver.recv() {
            match op {
                UploadOp::Begin {
                    filename,
                    total_bytes,
                    reply,
                } => {
                    let _ = reply.send(self.begin(&filename, total_bytes));
                }
                UploadOp::Chunk { data, reply } => {
                    let _ = reply.send(self.chunk(&data));
                }
                UploadOp::Commit { reply } => {
                    let _ = reply.send(self.commit());
                }
                UploadOp::Abort { reply } => {
                    let _ = reply.send(self.abort());
                }
            }
        }
    }

    fn begin(&mut self, filename: &str, total_bytes: u64) -> io::Result<u64> {
        if self.conn.is_some() {
            return Err(io::Error::other(
                "an upload is already in progress on this session",
            ));
        }
        let mut fresh = self.connect()?;
        let reply = command(
            &mut fresh.writer,
            &fresh.reader,
            &format!("upload-begin {} {total_bytes}", quote_upload_word(filename)),
            self.reply_timeout,
        )?;
        let upload_id = reply
            .first()
            .and_then(|line| line.trim().parse::<u64>().ok())
            .ok_or_else(|| {
                io::Error::other(format!("upload-begin reply was not an id: {reply:?}"))
            })?;
        *self.upload_id.lock() = Some(upload_id);
        self.conn = Some(fresh);
        Ok(upload_id)
    }

    fn chunk(&mut self, data: &[u8]) -> io::Result<()> {
        let Some(conn) = self.conn.as_mut() else {
            return Err(io::Error::other("no upload in progress on this session"));
        };
        let Some(upload_id) = *self.upload_id.lock() else {
            return Err(io::Error::other("no upload id"));
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        command(
            &mut conn.writer,
            &conn.reader,
            &format!("upload-chunk {upload_id} {encoded}"),
            self.reply_timeout,
        )?;
        Ok(())
    }

    fn commit(&mut self) -> io::Result<String> {
        let Some(mut conn) = self.conn.take() else {
            return Err(io::Error::other("no upload in progress on this session"));
        };
        let Some(upload_id) = *self.upload_id.lock() else {
            return Err(io::Error::other("no upload id"));
        };
        let pane = self.link.pane();
        let reply = command(
            &mut conn.writer,
            &conn.reader,
            // The commit grammar reads LEADING flags only: -t first, then
            // the upload id (MUX.md's Command Reference row).
            &format!("upload-commit -t %{pane} {upload_id}"),
            self.reply_timeout,
        )?;
        let path = reply.first().cloned().unwrap_or_default();
        if path.is_empty() {
            return Err(io::Error::other("upload-commit reply carried no path"));
        }
        Ok(path)
    }

    fn abort(&mut self) -> io::Result<()> {
        let Some(mut conn) = self.conn.take() else {
            return Err(io::Error::other("no upload in progress on this session"));
        };
        if let Some(upload_id) = *self.upload_id.lock() {
            let _ = command(
                &mut conn.writer,
                &conn.reader,
                &format!("upload-abort {upload_id}"),
                self.reply_timeout,
            );
        }
        Ok(())
    }
}

impl MuxUploadPipe {
    fn new(link: Arc<MirrorLink>, socket: PathBuf, reply_timeout: Duration) -> Self {
        Self {
            link,
            socket,
            reply_timeout,
            ops: Mutex::new(None),
            upload_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Queue one op and wait (bounded) for the worker's reply. The op
    /// queue and its worker are created by the first call; the worker is
    /// a plain thread (the verbs block on daemon I/O) that exits when the
    /// pipe — and with it the sender — drops.
    fn call<T>(
        &self,
        make: impl FnOnce(std::sync::mpsc::SyncSender<io::Result<T>>) -> UploadOp,
    ) -> io::Result<T> {
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        {
            let mut ops = self.ops.lock();
            if ops.is_none() {
                let (sender, receiver) =
                    std::sync::mpsc::sync_channel::<UploadOp>(UPLOAD_OP_QUEUE_DEPTH);
                let worker = UploadWorker {
                    conn: None,
                    upload_id: Arc::clone(&self.upload_id),
                    link: Arc::clone(&self.link),
                    socket: self.socket.clone(),
                    reply_timeout: self.reply_timeout,
                };
                let handle = std::thread::Builder::new()
                    .name("mux-upload-worker".to_string())
                    .spawn(move || worker.run(receiver))
                    .map_err(|err| {
                        io::Error::other(format!("cannot spawn upload worker: {err}"))
                    })?;
                *ops = Some(OpsHandle {
                    sender,
                    _worker: handle,
                });
            }
            ops.as_ref()
                .expect("just initialized")
                .sender
                .send(make(reply_tx))
                .map_err(|_| io::Error::other("the upload worker is gone"))?;
        }
        reply_rx.recv_timeout(self.reply_timeout).map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "no upload verb reply within the deadline",
            )
        })?
    }
}

/// One shell-quoted word for a control line (the bounded quoting grammar
/// the daemon parses: single quotes with the close-escape-reopen idiom),
/// so a filename with spaces survives the wire.
fn quote_upload_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

impl UploadForwarder for MuxUploadPipe {
    fn begin(&self, filename: &str, total_bytes: u64) -> io::Result<u64> {
        self.call(|reply| UploadOp::Begin {
            filename: filename.to_string(),
            total_bytes,
            reply,
        })
    }

    fn chunk(&self, data: &[u8]) -> io::Result<()> {
        self.call(|reply| UploadOp::Chunk {
            data: data.to_vec(),
            reply,
        })
    }

    fn commit(&self) -> io::Result<String> {
        self.call(|reply| UploadOp::Commit { reply })
    }

    fn abort(&self) -> io::Result<()> {
        self.call(|reply| UploadOp::Abort { reply })
    }

    fn upload_id(&self) -> Option<u64> {
        *self.upload_id.lock()
    }
}

/// par-mux's load-independent waits: the hang ceiling every positive wait
/// in `tests` takes (one source of truth across the mux test suites).
#[cfg(test)]
#[path = "../../crates/par-mux/src/mux/test_wait.rs"]
mod test_wait;

#[cfg(test)]
mod tests {
    use super::test_wait::CEILING;
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

    /// A streaming server whose sessions mirror panes of the daemon. The
    /// handshake's reply deadline is the hang ceiling: the production 10 s
    /// expired on `pane-info` at host load ~200 against a healthy daemon.
    fn streaming(socket: &std::path::Path) -> Arc<StreamingServer> {
        let factory = Arc::new(
            MuxSessionFactory::new(socket, MuxPaneSelector::FromSessionId)
                .with_reply_timeout(CEILING),
        );
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
        let deadline = Instant::now() + CEILING;
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

    /// Wait until `marker` shows in the DAEMON's capture of `pane`.
    fn wait_pane(control: &mut MuxClient, pane: &str, marker: &str) {
        wait(marker, || {
            let text = control
                .send(&format!("capture-pane -t {pane}"))
                .expect("capture")
                .join("\n");
            (!has_line(&text, marker)).then_some(text)
        });
    }

    /// Open the streaming session with an explicit id (the
    /// `workspace-active` tests use the follow mode's id directly).
    fn mirror_session(server: &Arc<StreamingServer>, session_id: &str) -> Arc<StreamSessionState> {
        let params = ConnectionParams {
            session_id: session_id.to_string(),
            readonly: false,
            preset: None,
        };
        server.resolve_session(&params).expect("mux-backed session")
    }

    #[test]
    fn active_workspace_resolution_parses_the_daemon_replies() {
        // Bare `switch-client` answers the active workspace's active
        // session, or an empty body when there is none.
        assert_eq!(
            active_session_from_reply(&["$5".to_string()]),
            Some("$5".to_string())
        );
        assert_eq!(active_session_from_reply(&[]), None);
        assert_eq!(active_session_from_reply(&["".to_string()]), None);
        assert_eq!(
            active_session_from_reply(&["%0 @1 80x24".to_string()]),
            None
        );
        // `list-panes -t $N` rows are `%N leaf marker`; only `*` counts.
        assert_eq!(
            active_pane_from_listing(&["%7 0 *".to_string(), "%8 1 -".to_string()]),
            Some(7)
        );
        assert_eq!(
            active_pane_from_listing(&["%8 1 -".to_string(), "%7 0 *".to_string()]),
            Some(7)
        );
        assert_eq!(active_pane_from_listing(&["%8 1 -".to_string()]), None);
        assert_eq!(active_pane_from_listing(&[]), None);
    }

    /// Card 01a16ab7 criterion 3: with a mirror following the daemon's
    /// active workspace, a desktop `select-workspace` mid-mirror moves the
    /// viewer to the new workspace's pane — same streaming session, no
    /// reconnect, and the input path follows with it.
    #[test]
    fn a_workspace_active_mirror_follows_a_desktop_workspace_switch() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane0) = daemon();

        // A second workspace; `new-workspace` selects it, so switch back
        // to `main` before the viewer attaches.
        control
            .send("new-workspace -n second")
            .expect("new-workspace");
        let second_pane = control
            .send("list-panes -F '#{pane_id} #{session_name}'")
            .expect("list-panes")
            .into_iter()
            .find(|l| l.trim().ends_with("second"))
            .and_then(|l| {
                l.split_whitespace()
                    .next()
                    .and_then(|p| p.trim_start_matches('%').parse::<u32>().ok())
            })
            .expect("the second workspace's pane");
        let pane1 = format!("%{second_pane}");
        control
            .send("select-workspace -t main")
            .expect("back to main");

        // Seed each pane's screen with its workspace marker.
        control
            .send(&format!("send-keys -t {pane0} 'echo WS1-MARKER' Enter"))
            .expect("send-keys");
        wait_pane(&mut control, &pane0, "WS1-MARKER");
        control
            .send(&format!("send-keys -t {pane1} 'echo WS2-MARKER' Enter"))
            .expect("send-keys");
        wait_pane(&mut control, &pane1, "WS2-MARKER");

        // Criterion 1: `workspace-active` resolves the ACTIVE workspace's
        // pane (main's), seeded from it, with nothing of workspace two.
        let server = streaming(&socket);
        let session = mirror_session(&server, ACTIVE_WORKSPACE_SESSION_ID);
        let seeded = session.terminal.read().content();
        assert!(
            has_line(&seeded, "WS1-MARKER"),
            "the mirror seeds from the active workspace's pane:\n{seeded}"
        );
        assert!(
            !has_line(&seeded, "WS2-MARKER"),
            "the other workspace must not leak into the fresh mirror:\n{seeded}"
        );
        let registered = server
            .get_session(ACTIVE_WORKSPACE_SESSION_ID)
            .expect("session registered");

        // Criteria 2+3: the desktop switches workspaces; the SAME session
        // re-seeds to the new workspace's pane without a reconnect.
        control.send("select-workspace -t second").expect("switch");
        wait_mirror(&session, "WS2-MARKER");
        let switched = session.terminal.read().content();
        assert!(
            !has_line(&switched, "WS1-MARKER"),
            "the re-seed reset the mirror; old pane content must be gone:\n{switched}"
        );
        let after = server
            .get_session(ACTIVE_WORKSPACE_SESSION_ID)
            .expect("session still registered after the switch");
        assert!(
            Arc::ptr_eq(&registered, &after),
            "the switch must not close or recreate the streaming session"
        );

        // Input follows: typed lines now reach the new workspace's pane.
        type_line(&session, "echo FOLLOW-INPUT");
        wait_pane(&mut control, &pane1, "FOLLOW-INPUT");
        let old = control
            .send(&format!("capture-pane -t {pane0}"))
            .expect("capture")
            .join("\n");
        assert!(
            !has_line(&old, "FOLLOW-INPUT"),
            "post-switch input must reach the new pane only:\n{old}"
        );
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

    /// A pane graphic streams to the mirror's viewers: the raw `%output`
    /// decodes on the mirror, and the raised GraphicsAdded becomes the cheap
    /// announce plus the pane-tagged InlineImage frame — pixels included,
    /// sent once (a bounded quiet window after the first frame stays empty;
    /// the drain acts per event, never per poll).
    #[test]
    fn a_pane_graphic_streams_as_an_inline_image_frame() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane) = daemon();
        let pane_num: u32 = pane.trim_start_matches('%').parse().expect("pane number");

        let server = streaming(&socket);
        let session = mirror(&server, &pane);
        let mut updates = session.broadcast_tx.subscribe();

        // A 2x2 RGB kitty image as PTY output: the hex line is a printf the
        // pane's shell runs (input queues in the pty until the shell reads
        // it, so one send executes exactly once whenever the shell is up).
        // RGB decodes to RGBA in the store: 4 pixels x 4 bytes.
        let line = b"printf '\\033_Ga=T,f=24,i=77,s=2,v=2;AAAAAAAAAAAAAAAA\\033\\\\'\r";
        let hex: String = line
            .iter()
            .map(|b| format!("{b:02x} "))
            .collect::<String>()
            .trim_end()
            .to_string();
        control
            .send(&format!("send-keys -t {pane} -H {hex}"))
            .expect("send-keys");

        let deadline = Instant::now() + CEILING;
        let mut frames = Vec::new();
        let mut announces = 0usize;
        loop {
            while let Ok(msg) = updates.try_recv() {
                match msg {
                    ServerMessage::GraphicsAdded { .. } => announces += 1,
                    ServerMessage::InlineImage { .. } => frames.push(msg),
                    _ => {}
                }
            }
            if !frames.is_empty() || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !frames.is_empty(),
            "the graphic never streamed as a frame; pane capture:\n{}\nmirror graphics: {}",
            control
                .send(&format!("capture-pane -t {pane}"))
                .expect("capture")
                .join("\n"),
            session.terminal.read().graphics_count()
        );
        let ServerMessage::InlineImage {
            image_id,
            pane_id,
            format,
            width,
            height,
            col,
            row,
            data,
        } = &frames[0]
        else {
            panic!("not an InlineImage frame");
        };
        assert_eq!(*pane_id, Some(pane_num), "the frame names its pane");
        assert_eq!(format, "rgba", "store pixels are RGBA");
        assert_eq!((*width, *height), (2, 2));
        assert_eq!(data.len(), 16, "2x2 RGB decoded to RGBA");
        assert!(*image_id > 0);
        let terminal = session.terminal.read();
        let stored = terminal
            .all_graphics()
            .iter()
            .find(|g| g.id == *image_id)
            .expect("the mirror store holds the graphic");
        assert_eq!(
            stored.position,
            (*col as usize, *row as usize),
            "the frame carries the graphic's placement"
        );
        assert!(announces >= 1, "the cheap announce rides along");

        // Sent once: a bounded quiet window after the first frame sees no
        // second frame for the same placement.
        std::thread::sleep(Duration::from_millis(300));
        let dupes = std::iter::from_fn(|| updates.try_recv().ok())
            .filter(|m| matches!(m, ServerMessage::InlineImage { .. }))
            .count();
        assert_eq!(dupes, 0, "the frame must not re-stream on later polls");
    }

    /// A graphic placed BEFORE the mirror attaches replays to a late
    /// joiner through `Connected.graphics`: the seed's `-G` section
    /// carries the pane's placements into the mirror store (the card's
    /// regression — the seed reproduced the visible grid but not the
    /// images, so late joiners saw a pane without its graphics), and a
    /// placement after attach still streams live as an InlineImage frame.
    #[test]
    fn a_graphic_placed_before_attach_replays_in_connected_graphics() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane) = daemon();
        let pane_num: u32 = pane.trim_start_matches('%').parse().expect("pane number");

        // The image emission and a marker ride one send-keys: the marker
        // reaching the pane capture proves the pane terminal processed
        // the image bytes above it, so the mirror below attaches AFTER
        // the placement deterministically.
        let line =
            b"printf '\\033_Ga=T,f=24,i=77,s=2,v=2;AAAAAAAAAAAAAAAA\\033\\\\'; echo GFX-PLACED\r";
        let hex: String = line
            .iter()
            .map(|b| format!("{b:02x} "))
            .collect::<String>()
            .trim_end()
            .to_string();
        control
            .send(&format!("send-keys -t {pane} -H {hex}"))
            .expect("send-keys");
        wait("GFX-PLACED in the pane", || {
            let text = control
                .send(&format!("capture-pane -t {pane}"))
                .expect("capture")
                .join("\n");
            (!has_line(&text, "GFX-PLACED")).then_some(text)
        });

        let server = streaming(&socket);
        let session = mirror(&server, &pane);
        let connected = session.build_connect_message("late-joiner", false);
        let ServerMessage::Connected { graphics, .. } = connected else {
            panic!("expected a Connected message");
        };
        assert_eq!(
            graphics.len(),
            1,
            "the pre-attach graphic replays to the late joiner: {graphics:?}"
        );
        let frame = &graphics[0];
        assert_eq!(frame.format, "rgba", "store pixels replay as RGBA");
        assert_eq!((frame.width, frame.height), (2, 2));
        assert_eq!(frame.data.len(), 16, "2x2 RGB decoded to RGBA");
        assert_eq!(frame.pane_id, None, "replay frames tag no pane");
        assert!(
            frame.image_id >= SEED_GRAPHICS_ID_OFFSET,
            "seed ids ride the offset, out of the mirror counter's range"
        );
        // The read guard is scoped: holding it past this block would block
        // the drain's write for the live leg below, and the live frame
        // would never arrive.
        {
            let mirror = session.terminal.read();
            let stored = mirror
                .all_graphics()
                .iter()
                .find(|g| g.id == frame.image_id)
                .expect("the seed ingested into the mirror store");
            assert_eq!((stored.width, stored.height), (2, 2));
            assert_eq!(
                stored.position,
                (frame.col as usize, frame.row as usize),
                "the replay carries the stored placement"
            );
        }

        // The live leg is untouched: a placement after attach still
        // arrives as the pane-tagged InlineImage frame.
        let mut updates = session.broadcast_tx.subscribe();
        let line = b"printf '\\033_Ga=T,f=24,i=88,s=2,v=2;BBBBBBBBBBBBBBBB\\033\\\\'\r";
        let hex: String = line
            .iter()
            .map(|b| format!("{b:02x} "))
            .collect::<String>()
            .trim_end()
            .to_string();
        control
            .send(&format!("send-keys -t {pane} -H {hex}"))
            .expect("send-keys");
        let deadline = Instant::now() + CEILING;
        let mut frames = Vec::new();
        loop {
            while let Ok(msg) = updates.try_recv() {
                if let ServerMessage::InlineImage { .. } = msg {
                    frames.push(msg);
                }
            }
            if !frames.is_empty() || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !frames.is_empty(),
            "a placement after attach must still stream live; mirror graphics: {} pane capture:\n{}",
            session.terminal.read().graphics_count(),
            control
                .send(&format!("capture-pane -t {pane}"))
                .expect("capture")
                .join("\n"),
        );
        let ServerMessage::InlineImage {
            image_id,
            pane_id,
            data,
            ..
        } = &frames[0]
        else {
            panic!("not an InlineImage frame");
        };
        assert_eq!(*pane_id, Some(pane_num), "the live frame names its pane");
        assert!(
            *image_id < SEED_GRAPHICS_ID_OFFSET,
            "live frames draw ids from the mirror's own counter"
        );
        assert_eq!(data.len(), 16, "the live frame carries the decoded pixels");
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

        // A viewer resize is a deliberate request: it is recorded as the
        // viewer's sizing contribution and the pane re-fits to the
        // smallest-viewer minimum — here the viewer alone, 70x20. The
        // mirror follows via %layout-change.
        session.resize_tx.send((70, 20)).expect("resize request");
        wait("the pane and mirror at 70x20", || {
            let pane_now = pane_size(&mut control, &pane);
            let mirror_now = session.terminal.read().size();
            (pane_now != (70, 20) || mirror_now != (70, 20))
                .then(|| format!("pane {pane_now:?}, mirror {mirror_now:?}"))
        });

        // A larger report from elsewhere (the desktop) does NOT lift the
        // grid: the attached viewer still displays the window at 70x20,
        // and the window's extent is the componentwise minimum over its
        // viewers. The desktop's report only becomes its own contribution.
        control
            .send(&format!("refresh-client -t {pane} -C 90x25"))
            .expect("desktop resize");
        wait("the pane and mirror still at the viewer's 70x20", || {
            let pane_now = pane_size(&mut control, &pane);
            let mirror_now = session.terminal.read().size();
            (pane_now != (70, 20) || mirror_now != (70, 20))
                .then(|| format!("pane {pane_now:?}, mirror {mirror_now:?}"))
        });
    }

    #[test]
    fn viewer_status_tracks_attach_membership_and_unchanged_grid_constraints() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane) = daemon();
        control
            .send(&format!("refresh-client -t {pane} -C 80x24"))
            .unwrap();
        let server = streaming(&socket);
        let session = mirror(&server, &pane);
        let initial = session.pane_size_status().expect("initial status");
        let ServerMessage::PaneSizeStatus {
            cols,
            rows,
            column_viewer: Some(first),
            ..
        } = initial
        else {
            panic!("status")
        };
        assert_eq!((cols, rows), (80, 24));
        let mut updates = session.broadcast_tx.subscribe();
        let mut other = MuxClient::connect(&socket).unwrap();
        other
            .send(&format!("refresh-client -t {pane} -C 80x24"))
            .unwrap();
        wait("two attach viewers", || {
            let presence = session.viewer_presence();
            (!matches!(presence, ServerMessage::ViewerPresence { count: 2, .. }))
                .then(|| format!("{presence:?}"))
        });
        assert!(std::iter::from_fn(|| updates.try_recv().ok())
            .any(|m| matches!(m, ServerMessage::ViewerPresence { count: 2, .. })));
        drop(control);
        wait("constraint owner changed without resizing", || {
            let status = session.pane_size_status();
            (!matches!(&status, Some(ServerMessage::PaneSizeStatus { cols: 80, rows: 24, column_viewer: Some(owner), .. }) if owner != &first))
                .then(|| format!("{status:?}"))
        });
        assert!(
            std::iter::from_fn(|| updates.try_recv().ok()).any(|m| matches!(
                m,
                ServerMessage::PaneSizeStatus {
                    cols: 80,
                    rows: 24,
                    ..
                }
            ))
        );
        wait("one remaining attach viewer", || {
            let presence = session.viewer_presence();
            (!matches!(presence, ServerMessage::ViewerPresence { count: 1, .. }))
                .then(|| format!("{presence:?}"))
        });
        drop(other);
        wait("last attach viewer left", || {
            let presence = session.viewer_presence();
            (!matches!(presence, ServerMessage::ViewerPresence { count: 0, .. }))
                .then(|| format!("{presence:?}"))
        });
    }

    #[test]
    fn closing_a_resized_mirror_withdraws_its_size_claim() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane) = daemon();
        control
            .send(&format!("refresh-client -t {pane} -C 100x30"))
            .expect("desktop size");
        let server = streaming(&socket);
        let session = mirror(&server, &pane);
        session.resize_tx.send((40, 12)).expect("phone fit");
        wait("phone size", || {
            let size = pane_size(&mut control, &pane);
            (size != (40, 12)).then(|| format!("pane {size:?}"))
        });

        assert!(server.close_session(&session.id, "viewer left".to_string()));
        wait("desktop size after mirror teardown", || {
            let size = pane_size(&mut control, &pane);
            (size != (100, 30)).then(|| format!("pane {size:?}"))
        });
    }

    async fn next_wire_message(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
        mut matches: impl FnMut(&ServerMessage) -> bool,
    ) -> ServerMessage {
        use futures_util::StreamExt;
        tokio::time::timeout(CEILING, async {
            loop {
                let frame = socket.next().await.expect("open websocket").expect("frame");
                let message = match frame {
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        crate::streaming::decode_server_message(&bytes).expect("protobuf")
                    }
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        serde_json::from_str(&text).expect("JSON")
                    }
                    _ => continue,
                };
                if matches(&message) {
                    return message;
                }
            }
        })
        .await
        .expect("wire message before deadline")
    }

    /// Card criterion 3: a phone drives a chunked upload through the
    /// mux-control channel (the streamer's per-session daemon connection
    /// surface, reached here over the shared WebSocket dispatch), and the
    /// pane receives the bracket-pasted materialized path. The daemon-side
    /// file holds exactly the bytes the client sent.
    #[test]
    fn a_client_upload_reaches_the_pane_as_a_bracket_pasted_path() {
        use crate::streaming::protocol::ClientMessage;

        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane) = daemon();
        let server = streaming(&socket);
        let session = mirror(&server, &pane);
        let pane_num: u32 = pane.trim_start_matches('%').parse().expect("pane number");

        // A WebSocket client (the phone) attaches to the mirrored pane.
        let params = ConnectionParams::from_uri_query(Some(&format!("session={}", session.id)));
        let (mut phone, connection) = rt.block_on(async {
            let (phone, connection) = tokio::io::duplex(65536);
            (
                tokio_tungstenite::WebSocketStream::from_raw_socket(
                    phone,
                    tokio_tungstenite::tungstenite::protocol::Role::Client,
                    None,
                )
                .await,
                tokio_tungstenite::WebSocketStream::from_raw_socket(
                    connection,
                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                    None,
                )
                .await,
            )
        });
        let serving = Arc::clone(&server);
        let websocket = rt.spawn(async move {
            serving
                .handle_test_ws(connection, &params)
                .await
                .expect("websocket session");
        });
        wait("websocket registered", || {
            (session.client_count() != 1).then(|| "no websocket client".to_string())
        });

        // The wire is binary protobuf (Text frames are logged and skipped
        // by both transports), so the phone's messages encode through the
        // same encoder a real client uses.
        async fn send_msg(
            phone: &mut tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
            msg: ClientMessage,
        ) {
            use futures_util::SinkExt;
            use tokio_tungstenite::tungstenite::Message;
            let bytes = crate::streaming::proto::encode_client_message(&msg)
                .expect("encode client message");
            phone
                .send(Message::binary(bytes))
                .await
                .expect("client send");
        }
        // Begin: the daemon id rides back as the direct upload_started reply.
        rt.block_on(send_msg(
            &mut phone,
            ClientMessage::UploadBegin {
                filename: "phone-upload.txt".into(),
                total_bytes: 11,
            },
        ));
        let upload_id = match rt.block_on(async {
            next_wire_message(&mut phone, |m| {
                matches!(m, ServerMessage::UploadStarted { .. })
            })
            .await
        }) {
            ServerMessage::UploadStarted { upload_id } => upload_id,
            other => panic!("expected upload_started, got {other:?}"),
        };
        assert!(upload_id > 0, "begin must report the daemon id");

        // Chunk + commit back-to-back: the pipe's single worker serializes
        // them, so commit can never overtake the chunk it follows.
        rt.block_on(send_msg(
            &mut phone,
            ClientMessage::UploadChunk {
                data: b"hello world".to_vec(),
            },
        ));
        rt.block_on(send_msg(&mut phone, ClientMessage::UploadCommit));
        let path = match rt.block_on(async {
            next_wire_message(&mut phone, |m| {
                matches!(m, ServerMessage::UploadFinished { .. })
            })
            .await
        }) {
            ServerMessage::UploadFinished {
                upload_id: finished_id,
                path,
                error,
            } => {
                assert!(
                    error.is_none(),
                    "commit failed: {error:?} (id {finished_id})"
                );
                assert_eq!(finished_id, upload_id, "the finished id matches begin");
                path.expect("a committed upload_finished carries the path")
            }
            other => panic!("expected upload_finished, got {other:?}"),
        };
        assert!(
            path.contains("phone-upload.txt"),
            "the pasted path carries the client's name: {path}"
        );
        assert_eq!(
            std::fs::read(&path).expect("materialized bytes"),
            b"hello world".to_vec(),
            "the staged file holds exactly the chunk bytes"
        );
        std::fs::remove_file(&path).expect("test cleanup");

        // The pane received the path (plain shell: no bracketed-paste
        // markers, the raw path lands at the prompt). Substring match:
        // the shared staging directory suffixes the name -N when a
        // previous run's committed file still holds the primary name.
        wait("the pasted path in the pane", || {
            let text = control
                .send(&format!("capture-pane -t {pane}"))
                .expect("capture")
                .join("\n");
            (!text.contains("phone-upload.txt")).then_some(text)
        });

        // Settled state allows a fresh begin; the abort settles with an
        // `aborted` upload_finished (its spool file is deleted daemon-side).
        rt.block_on(send_msg(
            &mut phone,
            ClientMessage::UploadBegin {
                filename: "phone-upload-2.txt".into(),
                total_bytes: 2,
            },
        ));
        rt.block_on(send_msg(&mut phone, ClientMessage::UploadAbort));
        match rt.block_on(async {
            next_wire_message(&mut phone, |m| {
                matches!(m, ServerMessage::UploadFinished { .. })
            })
            .await
        }) {
            ServerMessage::UploadFinished {
                error: Some(reason),
                ..
            } => {
                assert_eq!(reason, "aborted");
            }
            other => panic!("expected the aborted settlement, got {other:?}"),
        };

        // The upload ran against pane %N — the mirror's pane, addressed by
        // the daemon verb the pipe forwarded (pane_num only pins the test).
        assert_eq!(link_pane(&session.id), Some(pane_num));

        rt.block_on(async {
            phone.close(None).await.expect("phone closes");
            websocket.await.expect("websocket task");
        });
    }

    /// Test-only bridge: the mirror link's pane for `session_id`, so the
    /// e2e pins the upload landed on the mirrored pane.
    fn link_pane(session_id: &str) -> Option<u32> {
        // The mirror id is `pane-N` by construction in these tests.
        session_id
            .strip_prefix("pane-")
            .and_then(|n| n.parse().ok())
    }

    #[test]
    fn last_websocket_disconnect_releases_the_mirror() {
        use tokio_tungstenite::tungstenite::protocol::Role;

        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, _serving, mut control, pane) = daemon();
        control
            .send(&format!("refresh-client -t {pane} -C 100x30"))
            .expect("desktop size");
        let server = streaming(&socket);
        let session = mirror(&server, &pane);
        let params = ConnectionParams::from_uri_query(Some(&format!("session={}", session.id)));
        let (mut phone, connection) = rt.block_on(async {
            let (phone, connection) = tokio::io::duplex(65536);
            (
                tokio_tungstenite::WebSocketStream::from_raw_socket(phone, Role::Client, None)
                    .await,
                tokio_tungstenite::WebSocketStream::from_raw_socket(connection, Role::Server, None)
                    .await,
            )
        });
        let serving = Arc::clone(&server);
        let websocket = rt.spawn(async move {
            serving
                .handle_test_ws(connection, &params)
                .await
                .expect("websocket session");
        });
        wait("websocket registered", || {
            (session.client_count() != 1).then(|| "no websocket client".to_string())
        });
        rt.block_on(async {
            next_wire_message(&mut phone, |m| {
                matches!(
                    m,
                    ServerMessage::PaneSizeStatus {
                        cols: 100,
                        rows: 30,
                        ..
                    }
                )
            })
            .await;
            next_wire_message(&mut phone, |m| {
                matches!(m, ServerMessage::ViewerPresence { count: 2, .. })
            })
            .await;
        });
        session.resize_tx.send((40, 12)).expect("phone fit");
        wait("phone size", || {
            let size = pane_size(&mut control, &pane);
            (size != (40, 12)).then(|| format!("pane {size:?}"))
        });
        rt.block_on(async {
            next_wire_message(&mut phone, |m| matches!(m, ServerMessage::PaneSizeStatus { cols: 40, rows: 12, column_viewer: Some(owner), .. } if owner.starts_with("stream:"))).await;
        });
        let (mut second_phone, second_connection) = rt.block_on(async {
            let (phone, connection) = tokio::io::duplex(65536);
            (
                tokio_tungstenite::WebSocketStream::from_raw_socket(phone, Role::Client, None)
                    .await,
                tokio_tungstenite::WebSocketStream::from_raw_socket(connection, Role::Server, None)
                    .await,
            )
        });
        let serving = Arc::clone(&server);
        let params = ConnectionParams::from_uri_query(Some(&format!("session={}", session.id)));
        let second_websocket = rt.spawn(async move {
            serving
                .handle_test_ws(second_connection, &params)
                .await
                .expect("second viewer");
        });
        wait("second websocket registered", || {
            (session.client_count() != 2).then(|| "second viewer absent".to_string())
        });
        rt.block_on(async {
            next_wire_message(&mut phone, |m| {
                matches!(m, ServerMessage::ViewerPresence { count: 3, .. })
            })
            .await;
            next_wire_message(&mut second_phone, |m| {
                matches!(m, ServerMessage::ViewerPresence { count: 3, .. })
            })
            .await;
        });
        rt.block_on(async {
            phone.close(None).await.expect("phone closes");
            websocket.await.expect("websocket task");
        });
        rt.block_on(async {
            next_wire_message(&mut second_phone, |m| {
                matches!(m, ServerMessage::ViewerPresence { count: 2, .. })
            })
            .await;
        });
        assert_eq!(session.client_count(), 1);
        assert!(server.get_session(&session.id).is_some());
        assert_eq!(pane_size(&mut control, &pane), (40, 12));
        rt.block_on(async {
            second_phone.close(None).await.expect("last viewer closes");
            second_websocket.await.expect("last websocket task");
        });
        wait("desktop size after websocket disconnect", || {
            let size = pane_size(&mut control, &pane);
            (size != (100, 30)).then(|| format!("pane {size:?}"))
        });
        assert!(server.get_session(&session.id).is_none());
        let reconnected = mirror(&server, &pane);
        assert_eq!(reconnected.terminal.read().size(), (100, 30));
        type_line(&reconnected, TYPED_AFTER);
        wait_mirror(&reconnected, "DELTA-AFTER");
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
            let deadline = Instant::now() + CEILING;
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
            let deadline = Instant::now() + CEILING;
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
                    let deadline = Instant::now() + CEILING;
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

    /// The drain thread closes the mirror off the Tokio runtime when the
    /// daemon dies mid-session: the close must complete — registry removal
    /// AND the delayed factory teardown — instead of panicking on a bare
    /// `tokio::spawn` (the production streamer panic at server.rs:551 in
    /// 0.61.x, observed when the upstream daemon died under live sessions).
    #[test]
    fn dead_upstream_closes_the_mirror_off_the_runtime() {
        let rt = runtime();
        let _entered = rt.enter();
        let (_dir, socket, serving, _control, pane) = daemon();
        let factory = Arc::new(
            MuxSessionFactory::new(&socket, MuxPaneSelector::FromSessionId)
                .with_reply_timeout(CEILING),
        );
        let server = Arc::new(StreamingServer::with_factory(
            "127.0.0.1:0".to_string(),
            StreamingConfig::default(),
            Arc::clone(&factory) as Arc<dyn SessionFactory>,
        ));
        factory.set_streaming_server(Arc::clone(&server));
        let session_id = format!("pane-{}", pane.trim_start_matches('%'));
        let session = mirror(&server, &pane);

        // The mirror is live: typed input lands in the pane and its echo
        // reaches the mirror's terminal.
        type_line(&session, "echo MIRROR-ALIVE-MARKER");
        wait_mirror(&session, "MIRROR-ALIVE-MARKER");

        // Kill the upstream: the drain thread sees EOF and closes the
        // session from a plain std thread, outside any Tokio context.
        drop(serving);
        wait("closed mirror and torn-down factory link", || {
            let closed = server.get_session(&session_id).is_none();
            let link_count = factory.mirror_link_count();
            (!(closed && link_count == 0)).then(|| format!("closed={closed} links={link_count}"))
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
