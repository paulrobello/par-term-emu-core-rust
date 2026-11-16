//! Row streaming between the pane and its viewers: the drain thread that
//! applies `%output` and layout changes to the mirror and forwards them,
//! and the `send-keys -H` input path back to the pane.

use super::control::MuxLines;
use super::mirror::{
    apply_pane_cwd, apply_viewer_status, cwd_changed_message, follow_active_workspace,
    publish_process_state, query_viewer_status, record_cwd, reseed_after_respawn, MirrorLink, Sink,
};
use super::MUX_REPLY_TIMEOUT;
use crate::mux::LocalStream;
use crate::streaming::inline_image_frame_struct;
use crate::streaming::protocol::ServerMessage;
use crate::streaming::server::StreamingServer;
use crate::streaming::session::OutputChunk;
use crate::terminal::TerminalEvent;
use crate::tmux_control::{TmuxControlParser, TmuxNotification};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Bytes of input per `send-keys -H` command. Hex triples the size on the
/// wire, so a 256 KiB paste becomes many bounded lines instead of one.
const INPUT_CHUNK: usize = 1024;

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
pub(super) struct SendKeysWriter {
    pub(super) link: Arc<MirrorLink>,
    pub(super) stream: Arc<Mutex<Option<LocalStream>>>,
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

// Bound lookahead when the socket reader has not delivered the adjacent
// damage line. Generous on purpose: recv_timeout returns early on arrival,
// so the full window elapses only when the link is genuinely silent — the
// reader thread merely being descheduled must not cost the pairing hint.
const DAMAGE_PAIR_WINDOW: Duration = Duration::from_millis(200);

/// Read the daemon connection in order: apply the pane's `%output` to the
/// mirror and forward it to viewers, re-fit on `%layout-change`, and mark the
/// link dead when the pane or the connection goes away.
///
/// The server is held weakly: this thread's own lifetime is tied to the
/// mirror link the factory owns, and the factory is owned by the server —
/// a strong capture here would close the loop that kept the server (and
/// every session's state) alive after the last real owner dropped.
pub(super) fn spawn_drain(
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
        // Reused scratch for the Output guard's live pane-id comparison so
        // the per-notification check allocates nothing (QA-004).
        let mut pane_guard_buf = String::new();
        loop {
            if link.closed.load(Ordering::Relaxed) {
                break;
            }
            // The reply scan's pane filter is a pass-level snapshot; the
            // notification arms below read the link live so a retarget that
            // happens mid-batch takes effect for the remaining lines.
            let pane_num = link.pane();
            let pane_id = format!("%{pane_num}");
            // Reply-scan filter prefix, hoisted: the per-line comparison
            // below must not allocate (QA-004).
            let pane_prefix = format!("{pane_id} ");
            if Instant::now() >= next_status {
                if query_viewer_status(&link).is_err() {
                    break;
                }
                next_status = Instant::now() + Duration::from_secs(1);
            }
            let first = match reader
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
            // Batch every line the socket already delivered (card
            // 01a19ca4): the daemon writes a pane's `%output` and its
            // `%pane-damage` as ONE fan-out write, so batching what has
            // arrived keeps the pair adjacent for the Output arm below. A
            // missed pairing only costs the hint - output whose damage
            // fields are absent decodes as "unknown, full scan" - never
            // correctness.
            let mut queue: VecDeque<String> = VecDeque::new();
            queue.push_back(first);
            while let Ok(more) = reader.rx.try_recv() {
                match more {
                    Ok(line) => queue.push_back(line),
                    Err(_) => break,
                }
            }
            while let Some(line) = queue.pop_front() {
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
                        if line.starts_with(&pane_prefix) {
                            apply_viewer_status(&link, &line);
                            // A cwd token on the row the mirror polls every
                            // second: a plain shell's `cd` (no OSC 7)
                            // still reaches the viewers, within one poll
                            // (card 01a1ce6a criterion 2).
                            if let Some(update) = apply_pane_cwd(&link, &line) {
                                if let Some(server) =
                                    server.as_ref().and_then(std::sync::Weak::upgrade)
                                {
                                    server
                                        .send_to_session(&session_id, cwd_changed_message(&update));
                                }
                            }
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
                            if id == live_pane_id(&link, &mut pane_guard_buf) =>
                        {
                            // The damage line, when the daemon sent one, is the
                            // VERY next line (one fan-out write). Peek at the
                            // queue's front: this pane's `%pane-damage` is
                            // parsed through the same parser and consumed
                            // here; anything else stays for the next
                            // iteration. An unpaired damage notification (the
                            // `_` arm) is dropped - its `%output` already
                            // applied, and absent fields just mean scan.
                            let damage =
                                take_adjacent_damage(&mut parser, &mut queue, &id, |timeout| {
                                    reader.rx.recv_timeout(timeout)
                                });
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
                            forward(&link, &data, damage);
                            for event in events {
                                // A bare BEL in the pane rings THIS pane's
                                // mirror only: the drain thread owns one
                                // session_id per pane, so the frame reaches
                                // exactly the clients subscribed to it
                                // (per-client EventType filters apply in
                                // ws_session's should_send).
                                if matches!(event, TerminalEvent::BellRang(_)) {
                                    if let Some(server) =
                                        server.as_ref().and_then(std::sync::Weak::upgrade)
                                    {
                                        server.send_to_session(&session_id, ServerMessage::bell());
                                    }
                                }
                                // The pane's shell reported a new directory
                                // via OSC 7 (the instant path; the pane-info
                                // poll covers shells without integration):
                                // forward it with the sender's SEC-128
                                // locality verdict (card 01a1ce6a criteria
                                // 1+3), and keep the link's copy current so
                                // the next Connected names it.
                                if let TerminalEvent::CwdChanged(cwd) = event {
                                    record_cwd(&link, cwd.new_cwd.clone(), cwd.hostname.clone());
                                    let is_local =
                                        par_term_emu_core::shell_integration::osc7_host_is_local(
                                            cwd.hostname.as_deref(),
                                        );
                                    if let Some(server) =
                                        server.as_ref().and_then(std::sync::Weak::upgrade)
                                    {
                                        server.send_to_session(
                                            &session_id,
                                            ServerMessage::cwd_changed_full(
                                                cwd.old_cwd,
                                                cwd.new_cwd,
                                                cwd.hostname,
                                                cwd.username,
                                                cwd.timestamp,
                                                is_local,
                                            ),
                                        );
                                    }
                                    continue;
                                }
                                let TerminalEvent::GraphicsAdded { image_id, row } = event else {
                                    // Mirrors stream text via %output itself; only
                                    // the frame-bearing event needs converting.
                                    continue;
                                };
                                if !link.claim_image(image_id) {
                                    continue;
                                }
                                let pane_label = link.pane_label();
                                link.graphics_panes
                                    .lock()
                                    .insert(image_id, pane_label.clone());
                                let Some(mut frame) = inline_image_frame_struct(
                                    &link.terminal.read(),
                                    image_id,
                                    Some(link.pane()),
                                ) else {
                                    continue;
                                };
                                frame.pane = Some(pane_label.clone());
                                if let Some(server) =
                                    server.as_ref().and_then(std::sync::Weak::upgrade)
                                {
                                    server.send_to_session(
                                        &session_id,
                                        ServerMessage::graphics_added_with_pane(
                                            row as u16,
                                            Some(pane_label),
                                        ),
                                    );
                                    server.send_to_session(&session_id, ServerMessage::from(frame));
                                }
                            }
                        }
                        TmuxNotification::LayoutChange {
                            window_id,
                            window_layout,
                            ..
                        } if window_id == link.window() => {
                            let Some((cols, rows)) =
                                pane_size_in_layout(&window_layout, link.pane())
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
                                (term.size() != (cols, rows))
                                    .then(|| term.resize_deferred(cols, rows))
                            };
                            if let Some(batch) = dispatch_batch {
                                batch.deliver();
                                if let Some(server) =
                                    server.as_ref().and_then(std::sync::Weak::upgrade)
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
                        TmuxNotification::WindowClose { window_id }
                            if window_id == link.window() =>
                        {
                            link.alive.store(false, Ordering::Relaxed);
                        }
                        // Held-pane process lifecycle: the daemon names the
                        // pane (and the status when it read one) as the
                        // process exits under remain-on-exit; the pane, its
                        // window and this mirror stream all stay open. The
                        // pane filter reads the link live, so a retarget that
                        // happened mid-batch drops the old pane's exit.
                        TmuxNotification::PaneExited { pane_id, exit_code }
                            if pane_id == format!("%{}", link.pane()) =>
                        {
                            publish_process_state(&link, &pane_id, true, exit_code);
                        }
                        // respawn-pane: same pane id, fresh terminal. The
                        // re-seed clears the frozen screen AND the exited
                        // state on this same stream, no reconnect.
                        TmuxNotification::PaneRespawned { pane_id }
                            if pane_id == format!("%{}", link.pane()) =>
                        {
                            reseed_after_respawn(
                                &link,
                                &session_id,
                                &socket,
                                MUX_REPLY_TIMEOUT,
                                &server,
                            );
                        }
                        TmuxNotification::Exit => {
                            link.alive.store(false, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
            }
            if !link.alive.load(Ordering::Relaxed) {
                break;
            }
        }
        // End of the pane's byte stream: flush a held partial sequence so a
        // glyph severed at the final chunk edge surfaces as the replacement
        // char instead of vanishing with the link (card 01a1b3c3). The tail
        // goes out lossy DIRECTLY — a re-feed through `forward` would
        // re-stash an incomplete lead sequence and the tail would never
        // reach the viewer.
        let tail = std::mem::take(&mut *link.pending_utf8.lock());
        if !tail.is_empty() {
            send_chunk(
                &link,
                OutputChunk::plain(String::from_utf8_lossy(&tail).into_owned()),
            );
        }
        link.alive.store(false, Ordering::Relaxed);
        if !link.closed.load(Ordering::Relaxed) {
            if let Some(server) = server.and_then(|weak| weak.upgrade()) {
                server.close_session(&session_id, "mux pane closed".to_string());
            }
        }
    });
}

/// Incrementally decode pane bytes into text, holding back a trailing
/// incomplete UTF-8 sequence so a glyph severed at a relay chunk seam is
/// completed by the next chunk instead of becoming U+FFFD on both sides of
/// the seam (card 01a1b3c3). Genuinely invalid bytes keep
/// `from_utf8_lossy` semantics: one replacement char per bad sequence.
fn take_streaming_utf8(pending: &mut Vec<u8>, data: &[u8]) -> String {
    pending.extend_from_slice(data);
    let mut out = String::new();
    while !pending.is_empty() {
        match std::str::from_utf8(pending) {
            Ok(_) => {
                out.push_str(&String::from_utf8_lossy(pending));
                pending.clear();
            }
            Err(e) => {
                let up_to = e.valid_up_to();
                if up_to > 0 {
                    out.push_str(&String::from_utf8_lossy(&pending[..up_to]));
                }
                match e.error_len() {
                    // Invalid bytes, not an incomplete tail: emit the
                    // replacement char and rescan the remainder.
                    Some(n) => {
                        out.push('\u{FFFD}');
                        pending.drain(..up_to + n);
                    }
                    // A lead byte announcing more continuation bytes than
                    // the buffer holds: keep them for the next chunk.
                    None => {
                        pending.drain(..up_to);
                        break;
                    }
                }
            }
        }
    }
    out
}

/// Hand pane output to the session broadcaster, or buffer it until
/// `setup_session` connects one. `damage` is the chunk the daemon reported
/// for these bytes (`None` = not supplied - the chunk goes out without
/// damage fields, which every client reads as "full scan").
pub(super) fn forward(link: &MirrorLink, data: &[u8], damage: Option<(u64, Vec<(u32, u32)>)>) {
    let text = {
        let mut pending = link.pending_utf8.lock();
        if pending.is_empty() && std::str::from_utf8(data).is_ok() {
            String::from_utf8_lossy(data).into_owned()
        } else {
            take_streaming_utf8(&mut pending, data)
        }
    };
    let chunk = match damage {
        Some((generation, ranges)) => OutputChunk::with_damage(text, generation, ranges),
        None => OutputChunk::plain(text),
    };
    send_chunk(link, chunk);
}

/// Deliver one finished chunk: buffer it while no broadcaster is
/// connected, block-send it while one is, mark the sink closed when the
/// receiver is gone. Shared by `forward` and the stream-end flush.
fn send_chunk(link: &MirrorLink, chunk: OutputChunk) {
    let sender = {
        let mut sink = link.sink.lock();
        match &mut *sink {
            Sink::Buffering(buffer) => {
                buffer.push(chunk);
                return;
            }
            Sink::Live(sender) => sender.clone(),
            Sink::Closed => return,
        }
    };
    // The send runs OUTSIDE the sink lock: the drain thread parks here
    // while the broadcaster drains a full channel, and teardown_session
    // must not be pinned behind that park. `try_send` here was the
    // dropped-ESC corruption: a full channel silently discarded the
    // chunk, and an escape byte riding it never reached the viewer,
    // so the sequence tail painted as literal text. Blocking is the
    // lossless contract: the broadcaster loop never stops consuming
    // (its select polls recv alongside the flush timer), so the park
    // ends. Only a dropped receiver errs, and that session is gone.
    if sender.blocking_send(chunk).is_err() {
        *link.sink.lock() = Sink::Closed;
    }
}

/// The live pane-id prefix for the notification guards, written into a
/// reused buffer so the per-notification comparison allocates nothing
/// (QA-004). Reads the link live rather than the pass snapshot: a
/// mid-batch retarget must drop the OLD pane's remaining output lines.
fn live_pane_id<'a>(link: &MirrorLink, buf: &'a mut String) -> &'a str {
    use std::fmt::Write as _;
    buf.clear();
    let _ = write!(buf, "%{}", link.pane());
    buf
}

/// Look ahead one line, waiting only when the queue is empty. A nonmatching
/// front stays in place so later notifications cannot overtake it.
fn take_adjacent_damage(
    parser: &mut TmuxControlParser,
    queue: &mut VecDeque<String>,
    pane_id: &str,
    receive: impl FnOnce(Duration) -> Result<io::Result<String>, RecvTimeoutError>,
) -> Option<(u64, Vec<(u32, u32)>)> {
    if queue.is_empty() {
        if let Ok(Ok(line)) = receive(DAMAGE_PAIR_WINDOW) {
            queue.push_back(line);
        }
    }
    let damage = adjacent_damage(parser, queue.front()?, pane_id);
    if damage.is_some() {
        queue.pop_front();
    }
    damage
}

/// Parse `line` as the adjacent `%pane-damage` notification for `pane_id`,
/// through `parser` (same parser state, typed result). `None` when the line
/// is anything else - it is left for the queue's next iteration.
fn adjacent_damage(
    parser: &mut TmuxControlParser,
    line: &str,
    pane_id: &str,
) -> Option<(u64, Vec<(u32, u32)>)> {
    // Hand-rolled prefix check instead of `format!`: this runs on the
    // pane-output hot path once per line (QA-004).
    let rest = line.strip_prefix("%pane-damage ")?.strip_prefix(pane_id)?;
    if !rest.starts_with(' ') {
        return None;
    }
    let mut framed = line.as_bytes().to_vec();
    framed.push(b'\n');
    parser.parse(&framed).into_iter().find_map(|n| match n {
        TmuxNotification::OutputDamage {
            generation, ranges, ..
        } => Some((generation, ranges)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;
    use parking_lot::RwLock;
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::AtomicBool;

    /// A mirror link whose sink is `Buffering`, so a test can read back
    /// exactly the chunks `forward` produced.
    fn buffering_link() -> Arc<MirrorLink> {
        Arc::new(MirrorLink {
            pane: Arc::new(Mutex::new(1)),
            window: Mutex::new("@0".to_string()),
            follows_active: false,
            terminal: Arc::new(RwLock::new(Terminal::with_scrollback(80, 24, 100))),
            writer: Arc::new(Mutex::new(None)),
            pending_utf8: Mutex::new(Vec::new()),
            alive: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            sink: Mutex::new(Sink::Buffering(Vec::new())),
            sent_images: Mutex::new(HashSet::new()),
            graphics_panes: Arc::new(Mutex::new(HashMap::new())),
            status: Mutex::new(None),
            process: Mutex::new(None),
            cwd: Mutex::new(None),
            session: RwLock::new(None),
        })
    }

    fn buffered_text(link: &MirrorLink) -> String {
        match &*link.sink.lock() {
            Sink::Buffering(v) => v.iter().map(|c| c.data.as_str()).collect(),
            Sink::Live(_) | Sink::Closed => String::new(),
        }
    }

    /// A glyph severed at a relay chunk seam must survive: the first chunk
    /// ends mid-sequence, the second opens with its continuation bytes.
    /// Pre-fix, `from_utf8_lossy` ran per chunk and stamped U+FFFD on both
    /// sides of the seam (card 01a1b3c3).
    #[test]
    fn a_glyph_split_across_relay_chunks_stays_intact() {
        let link = buffering_link();
        let glyph = "\u{1F33F}"; // 🌿, F0 9F 8C BF
        let b = glyph.as_bytes();
        forward(&link, &b[..2], None);
        forward(&link, &b[2..], None);
        forward(&link, b" ok", None);
        let joined = buffered_text(&link);
        assert!(joined.contains(glyph), "split glyph lost: {joined:?}");
        assert!(
            !joined.contains('\u{FFFD}'),
            "seam replacement injected: {joined:?}"
        );
    }

    /// Seam buffering must not change lossy semantics: genuinely invalid
    /// bytes still map to one replacement char each, and nothing stays
    /// buffered behind them.
    #[test]
    fn an_invalid_byte_still_maps_to_one_replacement_char() {
        let link = buffering_link();
        forward(&link, b"a\xffb", None);
        forward(&link, b"\xc0\xff", None);
        let joined = buffered_text(&link);
        assert_eq!(joined, "a\u{FFFD}b\u{FFFD}\u{FFFD}");
    }

    /// A partial sequence still held when the pane's byte stream ends
    /// flushes as the replacement char at the seam instead of vanishing
    /// with the link.
    #[test]
    fn a_final_partial_sequence_flushes_at_stream_end() {
        let link = buffering_link();
        let b = "\u{1F33F}".as_bytes();
        forward(&link, &b[..2], None);
        // The reader-loop exit takes the held bytes and sends their lossy
        // form directly — a re-feed through `forward` would re-stash an
        // incomplete lead sequence and the tail would never surface.
        let tail = std::mem::take(&mut *link.pending_utf8.lock());
        send_chunk(
            &link,
            OutputChunk::plain(String::from_utf8_lossy(&tail).into_owned()),
        );
        let joined = buffered_text(&link);
        assert_eq!(joined, "\u{FFFD}");
    }

    /// Regression for the dropped-ESC corruption: the old
    /// `let _ = sender.try_send(chunk)` in [`forward`] silently discarded
    /// a chunk whenever the session's output channel was full, and an
    /// escape byte riding the dropped chunk never reached the viewer —
    /// the next chunk's `[22Gth` painted as literal text. A saturated
    /// channel with a consumer that starts draining only after production
    /// has finished must still deliver every chunk, in order.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forward_drops_nothing_when_the_output_channel_is_full() {
        let link = Arc::new(MirrorLink {
            pane: Arc::new(Mutex::new(3)),
            window: Mutex::new("@0".to_string()),
            follows_active: false,
            terminal: Arc::new(RwLock::new(Terminal::with_scrollback(80, 24, 100))),
            writer: Arc::new(Mutex::new(None)),
            alive: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            sink: Mutex::new(Sink::Closed),
            pending_utf8: Mutex::new(Vec::new()),
            sent_images: Mutex::new(HashSet::new()),
            graphics_panes: Arc::new(Mutex::new(HashMap::new())),
            status: Mutex::new(None),
            process: Mutex::new(None),
            cwd: Mutex::new(None),
            session: RwLock::new(None),
        });
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        *link.sink.lock() = Sink::Live(tx);

        // Capacity 2, six chunks, the consumer starts draining only after a
        // delay: the producer must park, not drop. Every receive is bounded,
        // so the pre-fix behavior (chunks silently dropped) fails the test
        // on a timeout instead of hanging it.
        const N: usize = 6;
        let payloads: [&[u8]; N] = [b"m0", b"m1", b"pre \x1b", b"[22Gth", b"m4", b"m5"];
        let producer = {
            let link = Arc::clone(&link);
            tokio::task::spawn_blocking(move || {
                for payload in payloads {
                    forward(&link, payload, None);
                }
            })
        };
        let mut got = Vec::new();
        for _ in 0..N {
            let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("every chunk arrives within 5s (none dropped)")
                .expect("channel stays open");
            got.push(chunk);
        }
        producer.await.expect("producer thread");
        assert_eq!(got.len(), N, "no chunk may be dropped");
        let joined: String = got.iter().map(|c| c.data.as_str()).collect();
        assert_eq!(joined, "m0m1pre \x1b[22Gthm4m5", "chunk order preserved");
        // The corruption signature: the escape sequence survived a full
        // channel intact.
        assert!(joined.contains("pre \x1b[22G"), "escape sequence intact");
    }

    #[test]
    fn damage_pair_delayed_matching_damage() {
        let mut parser = TmuxControlParser::new(true);
        let mut queue = VecDeque::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let damage = take_adjacent_damage(&mut parser, &mut queue, "%3", |timeout| {
            assert_eq!(timeout, DAMAGE_PAIR_WINDOW);
            // Deliver only after the helper has observed an empty queue.
            tx.send(Ok("%pane-damage %3 42 0-5".to_string())).unwrap();
            rx.recv_timeout(timeout)
        });
        assert_eq!(damage, Some((42, vec![(0, 5)])));
        assert!(queue.is_empty());
    }

    #[test]
    fn damage_pair_immediate_nonmatching_preserves_fifo() {
        let mut parser = TmuxControlParser::new(true);
        let mut queue = VecDeque::from([
            "%pane-damage %4 41 1-2".to_string(),
            "%pane-damage %3 42 0-5".to_string(),
        ]);
        let before = queue.clone();
        assert_eq!(
            take_adjacent_damage(&mut parser, &mut queue, "%3", |_| {
                panic!("a nonempty queue must not receive or wait")
            }),
            None
        );
        assert_eq!(queue, before);
    }

    #[test]
    fn damage_pair_unpaired_then_split_matching_recovers() {
        let mut parser = TmuxControlParser::new(true);
        let mut queue = VecDeque::new();
        assert_eq!(
            take_adjacent_damage(&mut parser, &mut queue, "%3", |_| {
                Ok(Ok("%output %3 next".to_string()))
            }),
            None
        );
        assert_eq!(queue.pop_front().as_deref(), Some("%output %3 next"));
        assert!(queue.is_empty());
        assert_eq!(
            take_adjacent_damage(&mut parser, &mut queue, "%3", |_| {
                Ok(Ok("%pane-damage %3 43 2-3".to_string()))
            }),
            Some((43, vec![(2, 3)]))
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn damage_pair_timeout_falls_back_without_losing_future_hints() {
        let mut parser = TmuxControlParser::new(true);
        let mut queue = VecDeque::new();
        assert_eq!(
            take_adjacent_damage(&mut parser, &mut queue, "%3", |timeout| {
                assert_eq!(timeout, DAMAGE_PAIR_WINDOW);
                Err(RecvTimeoutError::Timeout)
            }),
            None
        );
        assert!(queue.is_empty());
        assert_eq!(
            take_adjacent_damage(&mut parser, &mut queue, "%3", |_| {
                Ok(Ok("%pane-damage %3 44".to_string()))
            }),
            Some((44, vec![]))
        );
        assert!(queue.is_empty());
    }

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
