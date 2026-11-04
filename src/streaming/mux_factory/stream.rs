//! Row streaming between the pane and its viewers: the drain thread that
//! applies `%output` and layout changes to the mirror and forwards them,
//! and the `send-keys -H` input path back to the pane.

use super::control::MuxLines;
use super::mirror::{
    apply_viewer_status, follow_active_workspace, publish_process_state, query_viewer_status,
    reseed_after_respawn, MirrorLink, Sink,
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

// Bound lookahead when the socket reader has not delivered the adjacent damage line.
const DAMAGE_PAIR_WINDOW: Duration = Duration::from_millis(20);

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
        link.alive.store(false, Ordering::Relaxed);
        if !link.closed.load(Ordering::Relaxed) {
            if let Some(server) = server.and_then(|weak| weak.upgrade()) {
                server.close_session(&session_id, "mux pane closed".to_string());
            }
        }
    });
}

/// Hand pane output to the session broadcaster, or buffer it until
/// `setup_session` connects one. `damage` is the chunk the daemon reported
/// for these bytes (`None` = not supplied - the chunk goes out without
/// damage fields, which every client reads as "full scan").
pub(super) fn forward(link: &MirrorLink, data: &[u8], damage: Option<(u64, Vec<(u32, u32)>)>) {
    let chunk = match damage {
        Some((generation, ranges)) => OutputChunk::with_damage(
            String::from_utf8_lossy(data).into_owned(),
            generation,
            ranges,
        ),
        None => OutputChunk::plain(String::from_utf8_lossy(data).into_owned()),
    };
    match &mut *link.sink.lock() {
        Sink::Buffering(buffer) => buffer.push(chunk),
        Sink::Live(sender) => {
            let _ = sender.try_send(chunk);
        }
        Sink::Closed => {}
    }
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
    if !line.starts_with(&format!("%pane-damage {pane_id} ")) {
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

    #[test]
    fn damage_pair_delayed_matching_damage() {
        let mut parser = TmuxControlParser::new(true);
        let mut queue = VecDeque::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let damage = take_adjacent_damage(&mut parser, &mut queue, "%3", |timeout| {
            assert_eq!(timeout, Duration::from_millis(20));
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
                assert_eq!(timeout, Duration::from_millis(20));
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
