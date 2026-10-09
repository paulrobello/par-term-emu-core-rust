//! `par-mux attach` — the attach client (feature `attach`).
//!
//! Phase A: one pane fullscreen; the HOST terminal is the VT emulator. The
//! client is a byte pump + key pump + minimal status line: pane replay and
//! `%output` bytes flow to stdout verbatim, stdin bytes flow to the pane as
//! chunked `send-keys -H`, and a bottom status row is reserved with DECSTBM.
//!
//! Phase B adds the layout parser and pane renderer ([`layout`],
//! [`render`]) and the [`AttachMode`] seam: [`run`] stays passthrough (the
//! Phase A contract), and [`run_with_mode`] selects the renderer, which
//! mirrors every visible pane in its own core emulator and paints the
//! window through ratatui.

pub mod conn;
pub mod input;
pub mod layout;
pub mod render;
pub mod status;
pub mod tabs;

// Non-render scaffolding (ARC-003): panel composition (help, prompts,
// picker, sidebar) and target/list-line parsing. The passthrough
// `Session` type, its constructor and its two dispatchers (`route_bytes`,
// `handle_event`), and the raw terminal I/O (`Stdin`, `TerminalGuard`,
// whose private methods `render` calls) stay here; the rest of the
// `Session` impl is split by concern into `pump`, `actions`, `navigate`,
// and `status_row`.
mod actions;
mod navigate;
mod panels;
mod pump;
mod status_row;
mod targets;
use panels::*;
use targets::*;

use crate::mux::resolve_socket_path;
use crate::tmux_control::TmuxNotification;
use std::io::Write as _;
use std::process::ExitCode;
use std::time::Duration;

/// Which rendering pipeline the attach client drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttachMode {
    /// The Phase B renderer: per-pane core emulators fed by replay +
    /// `%output`, painted into layout rects through ratatui, dividers
    /// drawn between them. The host terminal only ever sees this client's
    /// own draws. THE DEFAULT — the owner's ruling (2026-10-04);
    /// passthrough is the opt-out byte pump.
    #[default]
    Render,
    /// The Phase A contract: pane bytes flow to the host terminal
    /// verbatim, which is the VT emulator.
    Passthrough,
}

/// Attach entry point. Returns the process exit code.
///
/// Standing client-mode contract: a failed connect is reported as
/// "no daemon running on <path>" and exits 1 — attach never starts a
/// daemon, matching `--cmd`/`--stop` (docs/MUX.md Client Mode).
pub fn run(options: &AttachOptions) -> ExitCode {
    run_with_mode(options, AttachMode::default())
}

/// Attach entry point with an explicit mode — the seam later Phase B
/// cards (and any embedder) select through. [`run`] is this with
/// [`AttachMode::Passthrough`].
pub fn run_with_mode(options: &AttachOptions, mode: AttachMode) -> ExitCode {
    match mode {
        AttachMode::Passthrough => run_passthrough(options),
        AttachMode::Render => render::run_render_session(options),
    }
}

/// The Phase A passthrough entry, unchanged.
fn run_passthrough(options: &AttachOptions) -> ExitCode {
    match run_inner(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(AttachError::NoDaemon(path)) => {
            eprintln!(
                "par-mux: no daemon running on {} — start one with `par-mux --socket {}`",
                path.display(),
                path.display()
            );
            ExitCode::FAILURE
        }
        Err(AttachError::Handshake(err)) => {
            eprintln!("par-mux: attach failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Everything `par-mux attach [-t TARGET] [--prefix KEY] [NAME | --socket PATH]`
/// parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachOptions {
    /// Target daemon: an explicit `--socket` path wins over a positional
    /// `NAME`, which falls back to `$PAR_MUX_SOCKET`/the default socket.
    pub socket: Option<std::path::PathBuf>,
    /// Positional daemon name (the default-path shorthand).
    pub name: Option<String>,
    /// `-t TARGET`: the initial pane/window/session target, tmux-style —
    /// resolved against the daemon's tree (id or name). `None` attaches to
    /// the newest session's newest pane (the global `list-panes` order).
    pub target: Option<String>,
    /// `--prefix KEY`: the detach key chord, e.g. `C-b` (tmux spelling),
    /// parsed by [`parse_prefix`].
    pub prefix: Option<String>,
    /// The config-file reload chord (`[client] reload`, e.g. `C-b C-r`):
    /// the key pressed after the prefix that re-reads the config file and
    /// rebinds the chords live. Set by the CLI layer from the config
    /// resolution; `None` keeps the built-in default (`C-b C-r`).
    pub reload: Option<String>,
    /// Render pipeline selection (`--mode`): `None` (no flag) resolves
    /// from the config file's `[client] mode`, defaulting to
    /// [`AttachMode::Render`]; an explicit flag wins over the file.
    /// [`AttachMode::Passthrough`] is the Phase A byte-pump contract;
    /// [`AttachMode::Render`] is the pane renderer + input router.
    pub mode: Option<AttachMode>,
}

impl AttachOptions {
    /// Resolve the daemon socket path (same precedence as the daemon and
    /// `--cmd`: explicit `--socket`, then `NAME`, then `$PAR_MUX_SOCKET`,
    /// then the unnamed default).
    pub fn socket_path(&self) -> std::path::PathBuf {
        resolve_socket_path(
            self.socket.as_deref(),
            self.name.as_deref(),
            std::env::var_os("PAR_MUX_SOCKET").as_deref(),
        )
    }
}

/// Error paths the attach entry point maps to exit codes.
#[derive(Debug)]
pub enum AttachError {
    /// No live daemon owns the socket. Attach never auto-spawns one.
    NoDaemon(std::path::PathBuf),
    /// The socket is served but the handshake failed mid-way (or a
    /// client-side argument, e.g. `--prefix`, did not parse).
    Handshake(std::io::Error),
}

/// How many bytes one chunked `send-keys -H` carries. The wire is
/// line-based, so a stdin burst is split into about-this-many-byte
/// commands; hex doubles the bytes on the wire, and 512 keeps each command
/// line comfortably inside a reader's line buffer.
const CHUNK: usize = 512;

/// Poll interval of the pump loop: how long the daemon-push wait runs
/// before the loop checks stdin again. Short enough that typing feels
/// instant; long enough that an idle session costs ~0 wakeups per second
/// of note.
const POLL: Duration = Duration::from_millis(16);

/// The attach session: connect, handshake, resolve the target pane, run
/// the byte/key pump, and restore the terminal on every exit path.
fn run_inner(options: &AttachOptions) -> Result<(), AttachError> {
    let path = options.socket_path();
    let conn = conn::AttachConn::connect(&path).map_err(|_| AttachError::NoDaemon(path.clone()))?;

    // Client-contract warnings: stamp mismatch (warn-only). Written BEFORE
    // raw mode so they render as ordinary terminal lines.
    if let Some(warning) = &conn.warnings().stamp_mismatch {
        eprintln!("{warning}");
    }

    let prefix = match options.prefix.as_deref() {
        Some(spec) => match parse_prefix(spec) {
            Some(byte) => byte,
            None => {
                return Err(AttachError::Handshake(std::io::Error::other(format!(
                    "invalid --prefix {spec:?} (expected the tmux spelling, e.g. C-b)"
                ))))
            }
        },
        None => C_B,
    };

    // Terminal setup/teardown guard: raw mode while attached, restored on
    // every exit path — including error/panic unwind.
    let guard = TerminalGuard::enter();

    // The reload chord key: the CLI layer passes the `[client] reload`
    // spelling from the resolved config; `None` keeps the built-in
    // default `C-b C-r`. An unparseable chord fails the attach — the
    // same contract `--prefix` follows.
    let reload_key = match options.reload.as_deref() {
        Some(chord) => chord.to_string(),
        None => "C-b C-r".to_string(),
    };
    let reload_key = match crate::mux::config::reload_chord_key(&reload_key) {
        Ok(key) => key,
        Err(err) => {
            drop(guard);
            return Err(AttachError::Handshake(std::io::Error::other(err)));
        }
    };
    // The management chords (split/kill/new-window) and the resize
    // affordances (the resize-mode chord + its step) come from the config
    // file — main.rs hands prefix/reload over explicitly, the management
    // keys resolve here against the same canonical file, the SAME pure
    // parser the live reload runs (one grammar, one error shape). A
    // malformed chord fails the attach like a malformed --prefix; a
    // broken FILE stays the lenient startup rule (warn-and-defaults —
    // the strict error is the reload's, per docs/MUX.md).
    let chords = match crate::mux::config::reload_client_chords(
        &crate::mux::config::load_canonical(),
        &crate::mux::config::Chords {
            prefix,
            reload: reload_key,
            management: crate::mux::config::Management::default(),
            ..crate::mux::config::Chords::with_defaults()
        },
    ) {
        Ok(chords) => chords,
        Err(err) => {
            drop(guard);
            return Err(AttachError::Handshake(std::io::Error::other(err)));
        }
    };
    let session = match Session::new(
        conn,
        &path,
        options.target.as_deref(),
        prefix,
        reload_key,
        chords.management,
        chords.resize_step,
    ) {
        Ok(session) => session,
        Err(err) => {
            // The guard drop restores the terminal before the message
            // renders.
            drop(guard);
            return Err(AttachError::Handshake(std::io::Error::other(err)));
        }
    };
    let mut session = session;
    let outcome = session.pump();
    drop(guard);
    match outcome {
        PumpOutcome::Detached | PumpOutcome::ConnectionClosed => Ok(()),
        PumpOutcome::DaemonExited => {
            eprintln!("par-mux: daemon exited");
            Ok(())
        }
    }
}

/// How the pump ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PumpOutcome {
    /// The user pressed prefix d.
    Detached,
    /// The daemon (or the socket) went away; the one reconnect failed.
    ConnectionClosed,
    /// The daemon said `%exit` (graceful shutdown).
    DaemonExited,
}

/// tmux's default prefix, C-b.
pub(crate) const C_B: u8 = 0x02;

/// The live attach session: owns the connection, the target pane, and the
/// status-line state, and runs the byte/key pump.
struct Session {
    conn: conn::AttachConn,
    /// The socket path, kept for the reconnect-on-eviction path.
    socket_path: std::path::PathBuf,
    /// The pane whose bytes we pump, `%N`.
    pane: String,
    /// The pane's local shadow emulator, fed the same byte stream
    /// passthrough forwards to the host terminal (the `refresh-client`
    /// replay in [`Session::resync`], and every `%output` chunk). Its
    /// tracked cursor is the pane's truth: after every status draw the
    /// cursor is placed ABSOLUTELY at the tracked cell, which kills the
    /// ESC7/ESC8 race where a pane scroll landing between save and
    /// restore makes the restored position one line off and every later
    /// output paints over the wrong row.
    emulator: render::PaneEmulator,
    /// The owning window, `@N` (pane-info's second field).
    window: String,
    /// The owning session's id, `$N` (resolved from the window scan).
    session_id: Option<String>,
    /// The owning session's name (status line).
    session_name: String,
    /// Every workspace as `(id, name)`, id order, plus the daemon's
    /// active one (status line's workspaces segment).
    workspaces: Vec<(String, String)>,
    active_workspace: Option<String>,
    /// The pane's effective title (status line).
    pane_title: String,
    /// Agents rostered on the target pane (status line).
    agents: usize,
    /// Held-dead cue: `Some(code)`/`Some(None)` once the pane is held
    /// (`(exited N)` on the status line), `None` while running.
    exited: Option<Option<i32>>,
    /// The host grid the status row was last drawn at. A resize (or a
    /// ConPTY geometry settling late) changes it, and the status row must
    /// follow.
    drawn_size: Option<(u16, u16)>,
    /// A startup repaint flood (the daemon's re-fit repaint after the
    /// handshake's size report lands, amplified by ConPTY) can wipe the
    /// one-shot startup status draw. Redraw once after the first `%output`
    /// burst since session start — output flowing again is what proved
    /// the flood passed.
    settling: bool,
    /// The detach prefix byte.
    prefix: u8,
    /// The prefix byte arrived; the next stdin byte is the command key.
    prefix_pending: bool,
    /// The reload chord: the key byte matched after the prefix, and the
    /// chord spelling for the status cue. Both are live-rebindable.
    reload_key: u8,
    /// The window/pane management chords (split % / split " / kill x /
    /// new-window c): the key bytes matched after the prefix, before the
    /// fixed command table. Live-rebindable by the reload.
    management: crate::mux::config::Management,
    /// Cells per resize step (config `resize-step`).
    resize_step: u32,
    /// Sticky resize mode: arrows adjust the focused pane's edges; any
    /// other key leaves the mode.
    resize_mode: bool,
    /// A transient confirmation cue drawn once on the status row and
    /// cleared on the next redraw cycle (`config reloaded`, or the
    /// reload's failure text).
    flash: Option<String>,
    /// Bracketed-paste tracking over the raw stdin stream: pasted bytes
    /// skip the prefix scan.
    paste: PasteTracker,
}

/// The bracketed-paste opener a host in DECSET 2004 mode sends.
const PASTE_START: &[u8] = b"\x1b[200~";
/// The bracketed-paste terminator.
const PASTE_END: &[u8] = b"\x1b[201~";

/// Passthrough's paste state. Passthrough forwards raw stdin (markers
/// included, so the pane sees its own framing); it only needs to know
/// whether a byte is inside a paste body. `carry` holds the last five
/// bytes so a marker split across stdin bursts is still recognized.
#[derive(Debug, Default)]
struct PasteTracker {
    in_paste: bool,
    carry: Vec<u8>,
}

impl PasteTracker {
    /// Account one forwarded byte, updating `in_paste` when it completes a
    /// marker.
    fn observe(&mut self, byte: u8) {
        self.carry.push(byte);
        if self.carry.ends_with(PASTE_START) {
            self.in_paste = true;
        } else if self.carry.ends_with(PASTE_END) {
            self.in_paste = false;
        }
        let keep = PASTE_START.len() - 1;
        if self.carry.len() > keep {
            self.carry.drain(..self.carry.len() - keep);
        }
    }
}

impl Session {
    /// Resolve the target, resync its screen to stdout, and seed the
    /// status state.
    fn new(
        mut conn: conn::AttachConn,
        socket_path: &std::path::Path,
        target: Option<&str>,
        prefix: u8,
        reload_key: u8,
        management: crate::mux::config::Management,
        resize_step: u32,
    ) -> Result<Self, String> {
        // Registration replay (held panes' %pane-exited, zoomed windows'
        // %layout-change) is state about OTHER panes mostly; the resync
        // below is the target's whole picture in Phase A. Drained here.
        let _replay = conn.drain_pending_events();

        let pane = resolve_target(&mut conn, target)?;
        let (cols, rows) = conn::terminal_grid();
        let (pane_cols, pane_rows) = status_row::pane_grid(cols, rows);
        let mut session = Self {
            conn,
            socket_path: socket_path.to_path_buf(),
            pane,
            emulator: render::PaneEmulator::new(0, pane_cols, pane_rows),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: true,
            prefix,
            prefix_pending: false,
            reload_key,
            management,
            resize_step,
            resize_mode: false,
            flash: None,
            paste: PasteTracker::default(),
        };
        session.resync();
        session.refresh_status();
        session.draw_status();
        Ok(session)
    }

    /// One daemon push. Returns false only on `%exit`.
    fn handle_event(&mut self, event: &TmuxNotification, status_dirty: &mut bool) -> bool {
        match event {
            TmuxNotification::Output { pane_id, data } if *pane_id == self.pane => {
                self.emulator.feed(data);
                let mut stdout = std::io::stdout().lock();
                let _ = stdout.write_all(data);
                let _ = stdout.flush();
                // The startup repaint flood (the daemon's re-fit repaint
                // after the handshake's size report lands, amplified by
                // ConPTY) can wipe the one-shot startup status draw. Output
                // flowing again after the session started means the flood
                // has passed — mark the settle redraw.
                if self.settling {
                    self.settling = false;
                    *status_dirty = true;
                }
            }
            TmuxNotification::Exit => return false,
            TmuxNotification::PaneExited { pane_id, exit_code } if *pane_id == self.pane => {
                self.exited = Some(*exit_code);
                *status_dirty = true;
            }
            TmuxNotification::PaneRespawned { pane_id } if *pane_id == self.pane => {
                self.exited = None;
                *status_dirty = true;
            }
            TmuxNotification::PaneTitleChanged { pane_id, title } if *pane_id == self.pane => {
                self.pane_title = title.clone();
                *status_dirty = true;
            }
            TmuxNotification::LayoutChange { .. }
            | TmuxNotification::WindowAdd { .. }
            | TmuxNotification::WindowClose { .. }
            | TmuxNotification::WindowRenamed { .. }
            | TmuxNotification::SessionRenamed { .. }
            | TmuxNotification::SessionsChanged
            | TmuxNotification::WorkspacesChanged
            | TmuxNotification::SessionWindowChanged { .. }
            | TmuxNotification::ClientSessionChanged { .. }
            | TmuxNotification::AgentStateChanged { .. }
            | TmuxNotification::AgentReleased { .. }
            | TmuxNotification::ClientAttached { .. }
            | TmuxNotification::ClientLeft { .. } => {
                // Geometry/roster/name changes can move the status facts;
                // re-query before the next redraw.
                *status_dirty = true;
            }
            // Everything else (output for other panes, lifecycle elsewhere)
            // is not this single-pane view's concern in Phase A.
            _ => {}
        }
        true
    }

    /// Route one stdin burst through the prefix scanner and the chunked
    /// forwarder. Returns true if the burst detached.
    fn route_bytes(&mut self, bytes: &[u8]) -> bool {
        if self.resize_mode {
            // Resize mode owns the burst: arrows resize, exits hand the
            // remainder back to the normal router (the key that cancelled
            // still does its job).
            return self.route_resize_bytes(bytes);
        }
        let mut to_send: Vec<u8> = Vec::with_capacity(bytes.len());
        let mut detached = false;
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            index += 1;
            let in_paste = self.paste.in_paste;
            self.paste.observe(byte);
            if in_paste {
                // A paste body is text, never a chord.
                to_send.push(byte);
                continue;
            }
            if self.prefix_pending {
                self.prefix_pending = false;
                if byte == 0x1b && bytes[index..].starts_with(&PASTE_START[1..]) {
                    // A paste opener cancels the pending prefix (tmux's
                    // rule); the marker forwards as the pane's framing.
                    to_send.push(byte);
                    continue;
                }
                if byte == self.prefix {
                    // prefix prefix: forward the prefix byte itself
                    // (tmux's rule for typing a literal C-b).
                    to_send.push(self.prefix);
                    continue;
                }
                if byte == 0x1b {
                    // An arrow chord: collect the rest of the CSI sequence.
                    let taken = bytes[index..].iter().take(2).copied().collect::<Vec<u8>>();
                    index += taken.len();
                    self.prefix_arrow(&taken);
                    continue;
                }
                // The reload chord matches by byte BEFORE the fixed
                // command table: it is configurable, so it cannot be a
                // static table arm, and the default (C-r, 0x12) does not
                // collide with respawn (`r`, 0x72).
                if byte == self.reload_key {
                    self.reload_config();
                    continue;
                }
                // The management chords match by byte BEFORE the fixed
                // command table: they are configurable, so they cannot be
                // static arms, and the defaults (`%`, `"`, `x`, `c`) are
                // consumed unbound by the table today.
                match self.management_command(byte) {
                    Some(ManagementKey::SplitRight) => self.split_pane(true),
                    Some(ManagementKey::SplitDown) => self.split_pane(false),
                    Some(ManagementKey::KillPane) => self.kill_focused_pane(),
                    Some(ManagementKey::NewWindow) => self.new_window_in_session(),
                    Some(ManagementKey::SwapPrev) => self.swap_pane(-1),
                    Some(ManagementKey::SwapNext) => self.swap_pane(1),
                    Some(ManagementKey::WorkspaceNext) => self.switch_workspace(1),
                    Some(ManagementKey::WorkspacePrev) => self.switch_workspace(-1),
                    // Render-mode-only chords: passthrough has no zoom,
                    // no overlay surface for the rename prompt, no
                    // divider glyphs to cycle, and no status bar or
                    // side panel. Consumed, not forwarded.
                    Some(
                        ManagementKey::Zoom
                        | ManagementKey::RenameWindow
                        | ManagementKey::RenamePane
                        | ManagementKey::BorderCycle
                        | ManagementKey::Labels
                        | ManagementKey::WorkspacePicker
                        | ManagementKey::Sidebar
                        | ManagementKey::StatusBar,
                    ) => {}
                    None => {
                        // The resize chord: a sticky mode — arrows adjust
                        // the focused pane's edges until Enter/Escape/q.
                        if byte == self.management.resize {
                            self.enter_resize_mode();
                            continue;
                        }
                        // The help chord: the bindings panel, printed as
                        // plain text (passthrough has no overlay surface;
                        // the pane's next output redraws over it).
                        if byte == self.management.help {
                            self.show_help();
                            continue;
                        }
                        match prefix_command(byte) {
                            PrefixKey::Detach => detached = true,
                            PrefixKey::CyclePane => self.cycle_pane(),
                            PrefixKey::NextWindow => self.switch_window(1),
                            PrefixKey::PrevWindow => self.switch_window(-1),
                            PrefixKey::NextSession => self.switch_session(1),
                            PrefixKey::PrevSession => self.switch_session(-1),
                            PrefixKey::Respawn => self.respawn_if_dead(),
                            PrefixKey::None => {}
                        }
                    }
                }
            } else if byte == self.prefix {
                self.prefix_pending = true;
            } else {
                to_send.push(byte);
            }
        }
        // A held-dead focused pane takes no bytes: the daemon answers every
        // send-keys to it with the NotStartedError %error, and the pane's
        // own PTY write would echo the bytes into its frozen screen either
        // way. Drop the run (the user sees the (exited N) status cue and a
        // respawn hint instead of their typing polluting the frozen view);
        // prefix chords already routed above, so detach/respawn still work.
        if self.exited.is_some() && !to_send.is_empty() {
            return detached;
        }
        self.send_chunked(&to_send);
        detached
    }
}

/// Forward bytes to `pane` in ~512-byte `send-keys -H` chunks — the
/// shared spelling of `Session::send_chunked`, for the render session's
/// stdin path.
pub(crate) fn forward_chunked(conn: &mut conn::AttachConn, pane: String, bytes: &[u8]) {
    for chunk in bytes.chunks(CHUNK) {
        if conn
            .send_checked(&format!(
                "send-keys -t {} -H {}",
                pane,
                hex_byte_list(chunk)
            ))
            .is_err()
        {
            return;
        }
    }
}

/// The `send-keys -H` wire spelling: one space-separated hex byte pair
/// per byte (`1b 5b 41`), as the daemon's bounded grammar parses — a
/// single concatenated run (`1b5b41`) is rejected as an invalid byte.
fn hex_byte_list(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 3);
    for byte in bytes {
        let _ = write!(out, "{byte:02x} ");
    }
    out.pop(); // the trailing space
    out
}

/// What a non-prefix command byte maps to.
enum PrefixKey {
    /// `d` — detach.
    Detach,
    /// `o` and the arrow chords — cycle panes.
    CyclePane,
    /// `n` / `p` — next / previous window.
    NextWindow,
    PrevWindow,
    /// `)` / `(` — next / previous session.
    NextSession,
    PrevSession,
    /// `r` — respawn the pane when it is held dead.
    Respawn,
    /// Consume the byte, do nothing (unbound keys).
    None,
}

/// Which management chord `key` is (the enum `Session::management_command`
/// and the render router match on; the four actions live on `Session` as
/// `split_pane`, `kill_focused_pane`, and `new_window_in_session`, and on
/// `WindowSession` as `management_chord`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagementKey {
    /// Split the focused pane right (`split-window -h`).
    SplitRight,
    /// Split the focused pane down (`split-window`, default direction).
    SplitDown,
    /// Kill the focused pane (`kill-pane`).
    KillPane,
    /// New window in the focused pane's session (`new-window`).
    NewWindow,
    /// Swap the focused pane with the previous pane in layout order
    /// (`swap-pane`).
    SwapPrev,
    /// Swap the focused pane with the next pane in layout order
    /// (`swap-pane`).
    SwapNext,
    /// Select the next workspace in id order (`select-workspace -t +N`)
    /// and land the view on the workspace's session.
    WorkspaceNext,
    /// Select the previous workspace in id order (`select-workspace
    /// -t +N`) and land the view on the workspace's session.
    WorkspacePrev,
    /// Toggle the focused pane between its layout rect and the full
    /// window (tmux's zoom).
    Zoom,
    /// Open the rename prompt for the shown window.
    RenameWindow,
    /// Open the rename prompt for the focused pane's user title.
    RenamePane,
    /// Cycle the divider/border glyph set.
    BorderCycle,
    /// Toggle pane titles embedded in the pane borders (render mode).
    Labels,
    /// Open the workspace picker modal (the session/window picker's
    /// workspace sibling).
    WorkspacePicker,
    /// Toggle the workspace side panel (render mode).
    Sidebar,
    /// Toggle the status bar (render mode).
    StatusBar,
}

/// The prefix command table: d detach; o / arrows cycle panes; n/p
/// next/prev window; ( ) prev/next session; r respawn-pane when held
/// dead. An unbound key is consumed silently (tmux drops it too).
fn prefix_command(key: u8) -> PrefixKey {
    match key {
        b'd' => PrefixKey::Detach,
        b'o' => PrefixKey::CyclePane,
        b'n' => PrefixKey::NextWindow,
        b'p' => PrefixKey::PrevWindow,
        b'(' => PrefixKey::PrevSession,
        b')' => PrefixKey::NextSession,
        b'r' => PrefixKey::Respawn,
        other if other == b' ' || other.is_ascii_alphabetic() || other.is_ascii_digit() => {
            // A printable key that means nothing in Phase A: consumed.
            PrefixKey::None
        }
        _ => PrefixKey::None,
    }
}

/// Raw stdin byte source. Raw mode (the guard) makes reads byte-wise and
/// non-echoing. A dedicated thread owns the blocking read and feeds a
/// channel, so `read_available` never blocks the pump: on Windows a
/// ConPTY-hosted stdin read parks until a key arrives (there is no poll
/// for console input), and a pump parked in that read would never reach
/// the daemon-push wait where a daemon death surfaces — the client would
/// hang forever after the daemon exited. With the channel, the pump's
/// non-blocking drain and its bounded notification wait are what observe
/// the disconnect. Byte order is preserved (one reader, FIFO channel);
/// Unix behavior is unchanged beyond who performs the same read.
pub(crate) struct Stdin {
    rx: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
}

impl Stdin {
    fn new() -> Self {
        Self::new_with_primer(Vec::new())
    }

    /// Like [`Self::new`], but `primer` bytes are delivered to the pump
    /// BEFORE anything the tty delivers afterward. The OSC 11 background
    /// probe consumes stdin bytes during its window (keystrokes that land
    /// between the query and the deadline); this is how they get back
    /// into the stream instead of being eaten.
    fn new_with_primer(primer: Vec<u8>) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<std::io::Result<Vec<u8>>>(64);
        if !primer.is_empty() {
            let _ = tx.send(Ok(primer));
        }
        std::thread::spawn(move || {
            use std::io::Read as _;
            let mut handle = std::io::stdin().lock();
            let mut buf = [0u8; 4096];
            loop {
                match handle.read(&mut buf) {
                    Ok(0) => {
                        // EOF: the host terminal closed. Tell the pump and
                        // stop reading.
                        let _ = tx.send(Ok(Vec::new()));
                        break;
                    }
                    Ok(n) => {
                        if tx.send(Ok(buf[..n].to_vec())).is_err() {
                            break; // pump is gone
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                        continue;
                    }
                    Err(err) => {
                        let _ = tx.send(Err(err));
                        break;
                    }
                }
            }
        });
        Self { rx }
    }

    /// Read whatever is available, or `Some(Ok(vec![]))` on EOF. `None`
    /// when nothing is pending (would block).
    fn read_available(&mut self) -> Option<std::io::Result<Vec<u8>>> {
        match self.rx.try_recv() {
            Ok(read) => Some(read),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // The reader thread ended (EOF or error already
                // delivered). Report idle; the pump treats a delivered
                // empty read as EOF and detaches.
                None
            }
        }
    }
}

/// RAII raw-mode guard: enters raw mode on construction, restores on drop —
/// including on error/panic unwind, so a failed attach never leaves the
/// host terminal in raw mode.
pub(crate) struct TerminalGuard {
    entered: bool,
}

impl TerminalGuard {
    fn enter() -> Self {
        // Raw mode needs a tty; under a test harness (no tty) this is a
        // no-op so the scaffolding stays exercisable headless.
        let entered = crossterm::terminal::enable_raw_mode().is_ok();
        // NO alternate screen here: a pane app's own 1049h/1049l pair (htop,
        // vim, …) would be the SECOND entry, and its leave pops the host
        // back to its main screen — the app-exit screen replaces the pane,
        // every later client draw (status bar) paints the host main, and
        // detach leaves it all behind (the manual-pass htop report). tmux
        // control mode does not enter the host alt either. The replay
        // instead clears the screen first (the prior content stays in the
        // host's scrollback), and detach clears again so nothing we or the
        // pane apps painted outlives the session.
        if entered {
            let _ = std::io::stdout()
                .write_all(b"\x1b[2J\x1b[H")
                .and_then(|()| std::io::stdout().flush());
        }
        Self { entered }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.entered {
            let _ = std::io::stdout()
                .write_all(b"\x1b[2J\x1b[H")
                .and_then(|()| std::io::stdout().flush());
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}

#[cfg(test)]
mod tests;
