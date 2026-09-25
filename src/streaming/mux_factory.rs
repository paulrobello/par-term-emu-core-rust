//! Streaming sessions backed by par-mux panes.
//!
//! [`MuxSessionFactory`] is a [`SessionFactory`] whose sessions mirror a pane
//! the par-mux daemon owns instead of spawning a PTY of their own. The pane
//! keeps ONE terminal, in the daemon; each streaming session holds a mirror
//! `Terminal` fed by the daemon's screen replay plus its `%output` deltas, and
//! is never a second owner (par-mux.md D2/R7). Several streaming sessions may
//! mirror the same pane; each has its own daemon connection.
//!
//! One connection per session, read in order by a drain thread. The seed
//! (`refresh-client -t %N`) is read synchronously in `create_session`, and
//! `%output` pushed before its reply block is discarded (the replay already
//! reflects it); everything after is applied. Ordering is the reason this does
//! not use [`crate::mux::MuxClient`], which splits replies and notifications
//! into separate channels.
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
use std::sync::Arc;

/// Bytes of input per `send-keys -H` command. Hex triples the size on the
/// wire, so a 256 KiB paste becomes many bounded lines instead of one.
const INPUT_CHUNK: usize = 1024;

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
    sessions: RwLock<HashMap<String, Arc<MirrorLink>>>,
    streaming_server: RwLock<Option<Arc<StreamingServer>>>,
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
            sessions: RwLock::new(HashMap::new()),
            streaming_server: RwLock::new(None),
        }
    }

    /// Scrollback lines kept by each mirror terminal (default 10 000).
    pub fn with_scrollback(mut self, lines: usize) -> Self {
        self.scrollback = lines;
        self
    }

    /// The server whose sessions this factory closes when a pane goes away
    /// and to which it announces mirror re-fits.
    pub fn set_streaming_server(&self, server: Arc<StreamingServer>) {
        *self.streaming_server.write() = Some(server);
    }

    fn pane_for(
        &self,
        session_id: &str,
        writer: &mut LocalStream,
        reader: &mut Lines,
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
                let body = command(writer, reader, "list-panes")?;
                body.iter()
                    .find_map(|l| l.trim().strip_prefix('%').and_then(|n| n.parse().ok()))
                    .ok_or_else(|| io::Error::other("the daemon has no panes"))
            }
        }
    }
}

/// The daemon connection's reader, line by line.
type Lines = std::io::Lines<BufReader<LocalStream>>;

/// Send one command and read its reply block, skipping pushed lines read
/// along the way. `%error` becomes an `Err`.
fn command(writer: &mut LocalStream, reader: &mut Lines, line: &str) -> io::Result<Vec<String>> {
    writeln!(writer, "{line}")?;
    writer.flush()?;
    let mut body: Option<Vec<String>> = None;
    loop {
        let Some(next) = reader.next() else {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "the daemon closed the connection",
            ));
        };
        let next = next?;
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
fn send_keys_line(pane: u32, chunk: &[u8]) -> String {
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
        let mut reader = BufReader::new(stream).lines();

        // Any %output these queries race is superseded by the seed below.
        let pane = self
            .pane_for(session_id, &mut writer, &mut reader)
            .map_err(|e| fail("cannot select a pane", e))?;
        let info = command(&mut writer, &mut reader, &format!("pane-info -t %{pane}"))
            .map_err(|e| fail("pane-info", e))?;
        let (window, cols, rows) = info
            .first()
            .and_then(|l| parse_pane_info(l))
            .ok_or_else(|| fail("pane-info", io::Error::other(format!("bad reply {info:?}"))))?;

        let mut terminal = Terminal::with_scrollback(cols, rows, self.scrollback);
        let seed = command(
            &mut writer,
            &mut reader,
            &format!("refresh-client -t %{pane}"),
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
                let mut stream = writer.lock();
                let sent = writeln!(stream, "refresh-client -t %{pane} -C {cols}x{rows}")
                    .and_then(|()| stream.flush());
                if sent.is_err() {
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
fn spawn_drain(
    link: Arc<MirrorLink>,
    reader: Lines,
    session_id: String,
    server: Option<Arc<StreamingServer>>,
) {
    std::thread::spawn(move || {
        let pane_id = format!("%{}", link.pane);
        let mut parser = TmuxControlParser::new(true);
        let mut in_reply = false;
        for line in reader {
            let Ok(line) = line else { break };
            if link.closed.load(Ordering::Relaxed) {
                break;
            }
            // Reply blocks (input and resize acks) carry nothing to mirror.
            if line.starts_with("%begin") {
                in_reply = true;
                continue;
            }
            if in_reply {
                if line.starts_with("%end") || line.starts_with("%error") {
                    in_reply = false;
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
                        if let (true, Some(server)) = (changed, server.as_ref()) {
                            server.send_to_session(
                                &session_id,
                                ServerMessage::resize(cols as u16, rows as u16),
                            );
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
            if let Some(server) = server {
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

    /// A served daemon with one session; returns its socket dir (keep it
    /// alive), socket path, a control client, and the first pane id `%N`.
    fn daemon() -> (tempfile::TempDir, PathBuf, MuxClient, String) {
        let dir = tempfile::Builder::new()
            .prefix("par-mux-str-")
            .tempdir()
            .expect("temp dir");
        let socket = dir.path().join("s");
        let server = MuxServer::bind(&socket).expect("bind");
        std::thread::spawn(move || server.run());
        let mut control = MuxClient::connect(&socket).expect("control client");
        control.send("new-session -s stream").expect("new-session");
        let pane = control
            .send("list-panes")
            .expect("list-panes")
            .into_iter()
            .map(|l| l.trim().to_string())
            .find(|l| l.starts_with('%'))
            .expect("a pane");
        (dir, socket, control, pane)
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
            .expect("pty_writer lock")
            .clone()
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

    #[tokio::test(flavor = "multi_thread")]
    async fn a_mirror_is_seeded_from_the_pane_then_follows_its_output() {
        let (_dir, socket, mut control, pane) = daemon();

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

    #[tokio::test(flavor = "multi_thread")]
    async fn the_mirror_starts_at_the_pane_size_and_a_viewer_resize_refits_the_pane() {
        let (_dir, socket, mut control, pane) = daemon();
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

    #[tokio::test(flavor = "multi_thread")]
    async fn two_viewers_of_one_pane_both_stay_live() {
        let (_dir, socket, _control, pane) = daemon();

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

    #[tokio::test(flavor = "multi_thread")]
    async fn a_killed_pane_marks_its_mirror_dead() {
        let (_dir, socket, mut control, pane) = daemon();
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

    #[test]
    fn input_becomes_hex_send_keys_the_daemon_parses() {
        let line = send_keys_line(7, b"a\x1b\"'\n");
        assert_eq!(
            crate::mux::parse_command(&line).expect("the daemon accepts the line"),
            crate::mux::MuxCommand::SendKeys {
                pane: crate::mux::PaneId(7),
                keys: b"a\x1b\"'\n".to_vec(),
            }
        );
    }
}
