//! Session lifecycle: [`MuxSessionFactory`] construction, pane selection
//! (including the active-workspace query), and the [`SessionFactory`]
//! create/setup/teardown of a mirror session.

use super::control::{command, parse_cwd_token, parse_cwdhost_token, parse_pane_info, MuxLines};
use super::mirror::{
    ingest_graphics_seed, parse_pane_exited_token, parse_viewer_status, seed_reply,
    split_graphics_seed, MirrorLink, Sink,
};
use super::stream::{spawn_drain, SendKeysWriter};
use super::upload::{quote_upload_word, MuxUploadPipe};
use super::{MuxPaneSelector, MuxSessionFactory, ACTIVE_WORKSPACE_SESSION_ID, MUX_REPLY_TIMEOUT};
use crate::mux::{connect_local_stream, LocalStream};
use crate::streaming::error::StreamingError;
use crate::streaming::protocol::ServerMessage;
use crate::streaming::server::{
    SessionFactory, SessionFactoryResult, StreamingServer, UploadForwarder,
};
use crate::streaming::session::{OutputChunk, StreamSessionState};
use crate::terminal::Terminal;
use interprocess::TryClone as _;
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
    pub(super) fn with_reply_timeout(mut self, timeout: Duration) -> Self {
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
    pub(super) fn mirror_link_count(&self) -> usize {
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
pub(super) fn query_active_workspace_pane(
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
pub(super) fn debug_log_active_workspace(session: &str, pane: u32) {
    crate::debug_error!(
        "STREAMING",
        "workspace-active mirror resolves to session {session} pane %{pane}"
    )
}

/// Hand every attach-buffered chunk to the session's output channel.
///
/// `try_send` alone dropped the overflow past the channel's capacity
/// (silently — a dropped attach-buffered chunk is the bare-ESC corruption
/// par-term recorded), so a Full retry parks briefly and retries: the
/// broadcaster loop is already running (sessions.rs spawns it before this
/// call) and frees capacity as it drains. `Closed` means the session died
/// mid-flush; the remaining chunks have no audience, so the flush stops.
fn flush_buffered(sender: &tokio::sync::mpsc::Sender<OutputChunk>, chunks: Vec<OutputChunk>) {
    for chunk in chunks {
        let mut pending = Some(chunk);
        while let Some(chunk) = pending.take() {
            match sender.try_send(chunk) {
                Ok(()) => break,
                Err(tokio::sync::mpsc::error::TrySendError::Full(back)) => {
                    pending = Some(back);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
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
            crate::streaming::metrics::Metrics::global().mux_socket_error(what);
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
        let seed_ids = ingest_graphics_seed(&mut terminal, &seed_graphics);
        let terminal = Arc::new(RwLock::new(terminal));

        let writer = Arc::new(Mutex::new(Some(writer)));
        let graphics_panes = Arc::new(Mutex::new(HashMap::new()));
        let seed_pane = format!("pane-{pane}");
        {
            let mut map = graphics_panes.lock();
            for id in seed_ids {
                map.insert(id, seed_pane.clone());
            }
        }
        let link = Arc::new(MirrorLink {
            follows_active: session_id == ACTIVE_WORKSPACE_SESSION_ID,
            pane: Arc::new(Mutex::new(pane)),
            window: Mutex::new(window),
            terminal: Arc::clone(&terminal),
            writer: Arc::clone(&writer),
            alive: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            sink: Mutex::new(Sink::Buffering(Vec::new())),
            pending_utf8: Mutex::new(Vec::new()),
            sent_images: Mutex::new(HashSet::new()),
            graphics_panes: Arc::clone(&graphics_panes),
            status: Mutex::new(info.first().and_then(|line| parse_viewer_status(line))),
            // The pane-info row just fetched is the lifecycle evidence:
            // the attached viewer reads the pane's current process state,
            // including a state the pane reached before this attach.
            process: Mutex::new({
                let (exited, code) =
                    parse_pane_exited_token(info.first().map(String::as_str).unwrap_or(""))
                        .unwrap_or((false, None));
                let state = ServerMessage::PaneProcessState {
                    pane_id: format!("%{pane}"),
                    state: if exited { "exited" } else { "running" }.to_string(),
                    exit_code: exited.then_some(code).flatten(),
                };
                if exited {
                    Some(state)
                } else {
                    None
                }
            }),
            session: RwLock::new(None),
            cwd: Mutex::new(info.first().and_then(|line| {
                let cwd = parse_cwd_token(line)?;
                let host = parse_cwdhost_token(line);
                Some((cwd, host))
            })),
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
            *session.graphics_panes.write() = Some(Arc::clone(&link.graphics_panes));
            if link.follows_active {
                *session.connected_pane.write() = Some(Arc::clone(&link.pane));
            }
            if let Some((size, viewers)) = &*status {
                session.update_mux_status(size.clone(), viewers.clone());
            }
        }
        if let Some(process) = link.process.lock().clone() {
            session.update_pane_process(process);
        }
        // The pane's cwd as the daemon knows it: the `Connected.cwd`
        // fallback for a pane whose mirror has not seen an OSC 7 (card
        // 01a1ce6a criterion 1). Later cwd changes reach the session
        // through `record_cwd`.
        if let Some((cwd, host)) = link.cwd.lock().clone() {
            session.update_mux_cwd(cwd, host);
        }
        let sender = session.get_output_sender();
        {
            let mut sink = link.sink.lock();
            if let Sink::Buffering(chunks) = &mut *sink {
                // Drain in order, each chunk with its own damage — the
                // broadcaster merges per batch window. `try_send` alone
                // dropped the overflow past the channel's capacity — a
                // dropped attach-buffered chunk is the bare-ESC corruption
                // par-term recorded — so retry on Full; the broadcaster
                // loop is already running (spawned before this call) and
                // frees capacity as it drains.
                flush_buffered(&sender, std::mem::take(chunks));
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

    fn pane_create(&self, mux_session: &str, name: Option<&str>) -> io::Result<(String, String)> {
        // One verb on its own short-lived daemon control connection (the
        // upload verbs' shape): a mirror link's connection is read solely
        // by its drain thread, so a request/reply verb cannot share one.
        // The window's single pane is resolved with `list-panes -t` right
        // after the create, on the same connection.
        let stream = connect_local_stream(&self.socket).inspect_err(|_| {
            crate::streaming::metrics::Metrics::global().mux_socket_error("pane-create-connect");
        })?;
        let mut writer = stream.try_clone()?;
        let reader = MuxLines::spawn(stream)?;
        let name_part = name
            .map(|n| format!(" -n {}", quote_upload_word(n)))
            .unwrap_or_default();
        let window = command(
            &mut writer,
            &reader,
            &format!(
                "new-window -t {}{}",
                quote_upload_word(mux_session),
                name_part
            ),
            self.reply_timeout,
        )?;
        let window_id = window
            .first()
            .map(|line| line.trim().to_string())
            .filter(|id| id.starts_with('@'))
            .ok_or_else(|| {
                io::Error::other(format!("new-window reply carried no window id: {window:?}"))
            })?;
        let panes = command(
            &mut writer,
            &reader,
            &format!("list-panes -t {window_id}"),
            self.reply_timeout,
        )?;
        let pane_id = panes
            .first()
            .and_then(|line| line.split_whitespace().next())
            .filter(|id| id.starts_with('%'))
            .map(str::to_string)
            .ok_or_else(|| io::Error::other(format!("the new window {window_id} has no panes")))?;
        Ok((window_id, pane_id))
    }

    fn is_session_alive(&self, session_id: &str) -> bool {
        self.sessions
            .read()
            .get(session_id)
            .is_some_and(|link| link.alive.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

/// Regression for the dropped-ESC corruption at attach: the old
/// `let _ = sender.try_send(chunk.clone())` in `setup_session` silently
/// dropped buffered chunks past the channel's capacity, so a pane that
/// produced more than the capacity while unattached delivered a hole to
/// the first viewer. `flush_buffered` parks and retries on Full; with a
/// capacity-2 channel, five chunks and a consumer that starts draining
/// only after the flush has finished, every chunk must land, in order.
#[cfg(test)]
mod flush_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flush_buffered_drops_nothing_when_the_output_channel_is_full() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        const N: usize = 5;
        let chunks: Vec<OutputChunk> = (0..N)
            .map(|i| OutputChunk::plain(format!("chunk{i} ")))
            .collect();

        // Capacity 2, five chunks, the consumer starts draining only after
        // a delay: the flusher must park on Full, not drop. Every receive
        // is bounded, so the pre-fix behavior (overflow dropped) fails the
        // test on a timeout instead of hanging it.
        let flusher = tokio::task::spawn_blocking(move || flush_buffered(&tx, chunks));
        let mut got = Vec::new();
        for _ in 0..N {
            let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("every buffered chunk arrives within 5s (none dropped)")
                .expect("channel stays open");
            got.push(chunk);
        }
        flusher.await.expect("flusher thread");
        let joined: String = got.iter().map(|c| c.data.as_str()).collect();
        assert_eq!(joined, "chunk0 chunk1 chunk2 chunk3 chunk4 ");
    }
}
