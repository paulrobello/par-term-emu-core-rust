//! The mirror side of a session: the [`MirrorLink`] to its pane, the
//! viewer-status protocol, the seed (screen replay plus graphics
//! placements), and the `workspace-active` follow that re-seeds in place.

use super::control::{command, parse_pane_info, MuxLines};
use super::lifecycle::{debug_log_active_workspace, query_active_workspace_pane};
use super::stream::forward;
use crate::mux::{connect_local_stream, LocalStream};
use crate::streaming::apply_inline_image;
use crate::streaming::protocol::{ServerMessage, MAX_INLINE_IMAGE_BYTES};
use crate::streaming::server::StreamingServer;
use crate::streaming::session::StreamSessionState;
use crate::terminal::Terminal;
use interprocess::TryClone as _;
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// One streaming session's link to its pane.
pub(super) struct MirrorLink {
    /// The pane this mirror serves. `workspace-active` mirrors retarget it
    /// when the daemon's active workspace moves; input and size
    /// contributions read it at call time so they follow. Shared with the
    /// session (`setup_session`, follows-active links only) so the
    /// Connected handshake names the pane being mirrored right now.
    pub(super) pane: Arc<Mutex<u32>>,
    /// The pane's window (`@W`), which its layout changes and close name.
    /// Retargeted together with `pane` so the pair stays consistent.
    pub(super) window: Mutex<String>,
    /// Set at creation: only [`ACTIVE_WORKSPACE_SESSION_ID`](super::ACTIVE_WORKSPACE_SESSION_ID) mirrors
    /// re-resolve on `%workspaces-changed`; `pane-N` mirrors stay pinned.
    pub(super) follows_active: bool,
    pub(super) terminal: Arc<RwLock<Terminal>>,
    pub(super) writer: Arc<Mutex<Option<LocalStream>>>,
    /// False once the daemon connection closed or the pane went away.
    pub(super) alive: AtomicBool,
    /// Set by teardown so the drain thread stops forwarding.
    pub(super) closed: AtomicBool,
    /// Output buffered until `setup_session` connects the broadcaster
    /// (`Buffering`), then the broadcaster's sender (`Live`).
    pub(super) sink: Mutex<Sink>,
    /// Graphics ids already streamed as InlineImage frames. A placement
    /// raises `GraphicsAdded` exactly once per mirror, but re-fits re-seed
    /// and a future seed carrying graphics could re-raise an id — this is
    /// the once-only guard, so a graphic is sent once, never on every poll.
    pub(super) sent_images: Mutex<HashSet<u64>>,
    /// Pane attribution per graphics id in this mirror's store: seed
    /// frames carry the seed request's pane, live frames the pane served
    /// at placement time. Shared with the streaming session so
    /// `build_connect_message` stamps `Connected.graphics`; a re-seed
    /// replaces the entries of the frames it replaces.
    pub(super) graphics_panes: Arc<Mutex<HashMap<u64, String>>>,
    pub(super) status: Mutex<Option<(ServerMessage, Vec<String>)>>,
    pub(super) session: RwLock<Option<std::sync::Weak<StreamSessionState>>>,
}

impl MirrorLink {
    pub(super) fn pane(&self) -> u32 {
        *self.pane.lock()
    }

    pub(super) fn window(&self) -> String {
        self.window.lock().clone()
    }

    /// Claim a graphics id for streaming; `false` when this mirror already
    /// sent it, so a re-raised announce never re-streams the frame.
    pub(super) fn claim_image(&self, image_id: u64) -> bool {
        self.sent_images.lock().insert(image_id)
    }

    /// Move the mirror to a new pane, window pair. The drain thread is the
    /// only writer, so the pair is never observed half-applied.
    pub(super) fn set_target(&self, pane: u32, window: String) {
        *self.pane.lock() = pane;
        *self.window.lock() = window;
    }

    /// The session-style id (`pane-N`) of the pane served right now.
    pub(super) fn pane_label(&self) -> String {
        format!("pane-{}", self.pane())
    }
}

pub(super) enum Sink {
    Buffering(Vec<u8>),
    Live(tokio::sync::mpsc::Sender<String>),
    Closed,
}

pub(super) fn parse_viewer_status(line: &str) -> Option<(ServerMessage, Vec<String>)> {
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

pub(super) fn apply_viewer_status(link: &MirrorLink, line: &str) {
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

pub(super) fn query_viewer_status(link: &MirrorLink) -> io::Result<()> {
    let mut writer = link.writer.lock();
    let writer = writer
        .as_mut()
        .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?;
    writeln!(writer, "pane-info -t %{} -M", link.pane())?;
    writer.flush()
}

/// The seed command: `refresh-client -t %N -G` — the screen replay plus a
/// `%graphics-begin`/`%graphics-end` section carrying the pane's graphics
/// placements, so a mirror's `Connected.graphics` replays images placed
/// before attach. An `%error` first line means an older daemon rejected
/// the unknown flag: retry the plain form, which seeds screen-only,
/// exactly the pre-`-G` behavior.
pub(super) fn seed_reply(
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
pub(super) struct SeedGraphicFrame {
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
pub(super) const SEED_GRAPHICS_ID_OFFSET: u64 = 1 << 62;

/// Marker lines framing the seed's graphics section — the daemon side
/// writes them (crates/par-mux `graphics_seed_section`); keep the two
/// spellings in sync. A pane can print any text, so the split anchors on
/// the LAST begin marker.
const GRAPHICS_SEED_BEGIN: &str = "%graphics-begin";
const GRAPHICS_SEED_END: &str = "%graphics-end";

/// Split a seed reply into its screen part and its graphics frames. The
/// section is optional: a flagless reply (older daemon, or no graphics
/// on the pane) is all screen.
pub(super) fn split_graphics_seed(lines: &[String]) -> (String, Vec<SeedGraphicFrame>) {
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
/// frame. Returns the store ids actually ingested, so the caller can tag
/// each with the seed's pane in its side map.
pub(super) fn ingest_graphics_seed(
    terminal: &mut Terminal,
    frames: &[SeedGraphicFrame],
) -> Vec<u64> {
    let mut ingested: Vec<u64> = Vec::new();
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
            None,
        );
        let _ = apply_inline_image(terminal, &message);
        ingested.push(SEED_GRAPHICS_ID_OFFSET + frame.id);
    }
    ingested
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
pub(super) fn follow_active_workspace(
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
    let ingested: Vec<u64>;
    {
        let mut term = link.terminal.write();
        term.reset();
        if term.size() != (target.cols, target.rows) {
            resized = true;
            batches.push(term.resize_deferred(target.cols, target.rows));
        }
        batches.push(term.process_deferred(target.screen.as_bytes()));
        term.drain_responses();
        ingested = ingest_graphics_seed(&mut term, &target.graphics);
    }
    {
        // The re-seed replaced the store: pane attribution follows it, so
        // stale entries (old-pane seeds, pre-switch live frames) are
        // dropped and the new seed's ids tag the NEW pane.
        let pane_label = link.pane_label();
        let mut map = link.graphics_panes.lock();
        map.retain(|id, _| ingested.contains(id));
        for id in &ingested {
            map.insert(*id, pane_label.clone());
        }
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
