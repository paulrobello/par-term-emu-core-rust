//! The mirror side of a session: the [`MirrorLink`] to its pane, the
//! viewer-status protocol, the seed (screen replay plus graphics
//! placements), and the `workspace-active` follow that re-seeds in place.

use super::control::{command, parse_cwd_token, parse_cwdhost_token, parse_pane_info, MuxLines};
use super::lifecycle::{debug_log_active_workspace, query_active_workspace_pane};
use super::stream::forward;
use crate::mux::{connect_local_stream, LocalStream};
use crate::streaming::apply_inline_image;
use crate::streaming::protocol::{ServerMessage, MAX_INLINE_IMAGE_BYTES};
use crate::streaming::server::StreamingServer;
use crate::streaming::session::{OutputChunk, StreamSessionState};
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
    /// Output chunks buffered until `setup_session` connects the
    /// broadcaster (`Buffering`), then the broadcaster's sender (`Live`).
    pub(super) sink: Mutex<Sink>,
    /// Trailing bytes of a multi-byte UTF-8 sequence severed at a relay
    /// chunk seam: `forward` holds them back until the sequence's
    /// continuation bytes arrive with the next chunk, so `from_utf8_lossy`
    /// never sees a half-glyph. Pane output is a byte stream — a glyph has
    /// no obligation to align with the daemon's `%output` line boundaries.
    pub(super) pending_utf8: Mutex<Vec<u8>>,
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
    /// The pane's process lifecycle (`pane_process_state`), seeded from
    /// the pane-info row at attach and kept current by the drain loop, so
    /// a session created after a transition still connects with the
    /// pane's current state.
    pub(super) process: Mutex<Option<ServerMessage>>,
    /// The pane's cwd and its OSC 7 hostname (card 01a1ce6a), seeded from
    /// the attach's pane-info row and kept current by the drain loop (OSC
    /// 7 events and the pane-info poll). Serves the `Connected.cwd`
    /// fallback for a session created before the mirror saw any OSC 7,
    /// and the dedupe state for cwd-change broadcasts.
    pub(super) cwd: Mutex<Option<(String, Option<String>)>>,
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
    /// Chunks buffered until `setup_session` connects the broadcaster;
    /// each keeps its damage.
    Buffering(Vec<OutputChunk>),
    Live(tokio::sync::mpsc::Sender<OutputChunk>),
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

/// The pane-info row's `exited=<code|?>` token: `Some((true, code))` for
/// a held-dead pane (code `None` when the daemon had no status to read),
/// `None` when the pane is running — the token rides only on a dead pane,
/// so absence is the daemon's own running verdict, not an inference.
pub(super) fn parse_pane_exited_token(line: &str) -> Option<(bool, Option<i32>)> {
    line.split_whitespace()
        .find_map(|field| field.strip_prefix("exited="))
        .map(|value| (true, value.parse::<i32>().ok()))
}

/// Record the pane's process lifecycle on the link and push it to the
/// session's viewers (`update_pane_process` broadcasts only on change).
/// The link's copy serves the next session created for this pane, so a
/// viewer connecting after a transition still reads the current state.
pub(super) fn publish_process_state(
    link: &MirrorLink,
    pane_id: &str,
    exited: bool,
    exit_code: Option<i32>,
) {
    let message = ServerMessage::PaneProcessState {
        pane_id: pane_id.to_string(),
        state: if exited { "exited" } else { "running" }.to_string(),
        exit_code: exited.then_some(exit_code).flatten(),
    };
    *link.process.lock() = Some(message.clone());
    if let Some(session) = link
        .session
        .read()
        .as_ref()
        .and_then(std::sync::Weak::upgrade)
    {
        session.update_pane_process(message);
    }
}

/// One cwd change to broadcast: the prior path (when known), the new path,
/// and its OSC 7 hostname (`None` when the cwd did not come from an OSC 7
/// report).
pub(super) struct CwdUpdate {
    pub(super) old: Option<String>,
    pub(super) new: String,
    pub(super) host: Option<String>,
}

/// Record `cwd`/`host` as the pane's state (card 01a1ce6a). Returns the
/// update to broadcast — `None` when nothing changed, so the 1s pane-info
/// poll stays silent while the OSC 7 arm and the poll agree. Also pushes
/// the new state to a live session, so `Connected` for the NEXT client
/// names the pane's cwd even though the mirror never saw an OSC 7.
pub(super) fn record_cwd(
    link: &MirrorLink,
    cwd: String,
    host: Option<String>,
) -> Option<CwdUpdate> {
    let mut state = link.cwd.lock();
    if state.as_ref() == Some(&(cwd.clone(), host.clone())) {
        return None;
    }
    let old = state.as_ref().map(|(c, _)| c.clone());
    *state = Some((cwd.clone(), host.clone()));
    drop(state);
    let update = CwdUpdate {
        old,
        new: cwd,
        host,
    };
    if let Some(session) = link
        .session
        .read()
        .as_ref()
        .and_then(std::sync::Weak::upgrade)
    {
        session.update_mux_cwd(update.new.clone(), update.host.clone());
    }
    Some(update)
}

/// The `CwdChanged` message for `update`, with the sender's SEC-128
/// locality verdict computed from the OSC 7 hostname.
pub(super) fn cwd_changed_message(update: &CwdUpdate) -> ServerMessage {
    let is_local = par_term_emu_core::shell_integration::osc7_host_is_local(update.host.as_deref());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    ServerMessage::cwd_changed_full(
        update.old.clone(),
        update.new.clone(),
        update.host.clone(),
        None,
        now,
        is_local,
    )
}

/// Parse the cwd tokens off a `pane-info` reply row and, when they name a
/// different (path, host) than the link's last state, record the change
/// for the caller to broadcast.
pub(super) fn apply_pane_cwd(link: &MirrorLink, line: &str) -> Option<CwdUpdate> {
    let cwd = super::control::parse_cwd_token(line)?;
    let host = super::control::parse_cwdhost_token(line);
    record_cwd(link, cwd, host)
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
    // The respawn path reuses this apply with an unnamed target; only a
    // workspace retarget logs the active-workspace resolution.
    if !target.session.is_empty() {
        debug_log_active_workspace(&target.session, target.pane);
    }
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
    // The re-seed lands on whatever state the pane is in now — a retarget
    // follows the daemon's active pane, a respawn restarts it — and the
    // pane-info row is the evidence, so viewers of the OLD pane state
    // converge without a reconnect.
    let (exited, code) = parse_pane_exited_token(&target.status_line).unwrap_or((false, None));
    publish_process_state(link, &format!("%{}", link.pane()), exited, code);
    // The pane (possibly a different one) the mirror now serves: its cwd
    // replaces the old pane's, and connected viewers learn the switch as a
    // CwdChanged (card 01a1ce6a). A row without cwd tokens (older daemon)
    // clears the state so the next Connected does not name the previous
    // pane's directory.
    let new_cwd = parse_cwd_token(&target.status_line)
        .map(|cwd| (cwd, parse_cwdhost_token(&target.status_line)));
    match new_cwd {
        Some((cwd, host)) => {
            if let Some(update) = record_cwd(link, cwd, host) {
                if let Some(server) = server.as_ref().and_then(std::sync::Weak::upgrade) {
                    server.send_to_session(session_id, cwd_changed_message(&update));
                }
            }
        }
        None => {
            *link.cwd.lock() = None;
        }
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
    forward(link, restore.as_bytes(), None);
}

/// Re-seed the mirror in place after `%pane-respawned`: the daemon gave
/// the pane a fresh terminal, so the held pane's frozen screen is stale
/// and incremental output would paint over it. The seed runs over a
/// short-lived connection (the mirror's own reader cannot absorb a reply
/// block mid-drain), then the same in-place apply a retarget uses resets
/// the mirror and pushes the fresh screen to the viewers. The pane-info
/// row fetched here also re-derives the process state — `running` — so
/// the exited state clears on the same connection, no reconnect.
pub(super) fn reseed_after_respawn(
    link: &Arc<MirrorLink>,
    session_id: &str,
    socket: &std::path::Path,
    timeout: Duration,
    server: &Option<std::sync::Weak<StreamingServer>>,
) {
    if link.closed.load(Ordering::Relaxed) {
        return;
    }
    let pane = link.pane();
    let result = (|| -> io::Result<WorkspaceTarget> {
        let stream = connect_local_stream(socket)?;
        let mut writer = stream.try_clone()?;
        let reader = MuxLines::spawn(stream)?;
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
        Ok(WorkspaceTarget {
            session: String::new(),
            pane,
            window,
            cols,
            rows,
            status_line,
            screen,
            graphics,
        })
    })();
    match result {
        Ok(target) => apply_workspace_target(link, session_id, server, target),
        Err(err) => {
            // The state still clears: the notification is the evidence.
            publish_process_state(link, &format!("%{pane}"), false, None);
            crate::debug_error!("STREAMING", "respawn re-seed failed: {err}")
        }
    }
}
