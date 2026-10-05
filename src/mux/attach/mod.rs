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

/// Parse the `--prefix` tmux spelling (`C-b`, `C-a`, `C-Space`) or a
/// literal single character into its byte. Delegates to the crate's one
/// prefix grammar in [`crate::mux::config`], so `--prefix` and the config
/// file's chords share spellings and edge cases.
fn parse_prefix(spec: &str) -> Option<u8> {
    crate::mux::config::parse_prefix(spec)
}

/// The client-side half of a reload: re-read the canonical config file
/// and re-derive the prefix and reload key through the pure helper
/// ([`crate::mux::config::reload_client_chords`]), `current` the
/// fallback for settings the file does not name.
fn reload_client_chords(
    current: crate::mux::config::Chords,
) -> Result<crate::mux::config::Chords, String> {
    let file = crate::mux::config::load_canonical_checked()?;
    crate::mux::config::reload_client_chords(&file, &current)
}

/// One `list-sessions` reply line as `(session_id, name)`. The wire shape
/// is `$N: name` (dispatch.rs), so the id ends at the colon — a plain
/// whitespace split keeps it (`$0:`), which the daemon's id parser then
/// rejects. The window roster's `@N: name` lines parse the same way.
pub(crate) fn parse_session_line(line: &str) -> Option<(String, String)> {
    // Workspace-aware shape: `+W: wname: $N: name` — the session id is the
    // LAST `$N:` marker, the name is what follows it. The bare `$N: name`
    // shape (a pre-workspaces daemon) parses identically: split at the
    // last `: ` whose left side ends in a `$<digits>` id.
    let idx = line.rfind("$")?;
    let rest = &line[idx..];
    let (id, name) = rest.split_once(": ")?;
    let id = id.strip_prefix('$')?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((format!("${id}"), name.to_string()))
}

/// One `list-workspaces` reply line as `(id, name, active)`. The wire
/// shape is `+N: name`, with the daemon's active workspace's line
/// ending in ` active`.
pub(crate) fn parse_workspace_line(line: &str) -> Option<(String, String, bool)> {
    let (id, rest) = line.split_once(": ")?;
    if id.is_empty() || !id.starts_with('+') || !id[1..].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (name, active) = match rest.strip_suffix(" active") {
        Some(name) => (name, true),
        None => (rest, false),
    };
    Some((id.to_string(), name.to_string(), active))
}

/// Resolve the attach target to a pane id. A pane target (`%N` or a pane
/// title name) goes straight to the daemon's matcher via `pane-info`; a
/// window (`@N`/name) or session (`$N`/name) target narrows through the
/// targeted `list-windows`/`list-panes` queries to the marked pane. With
/// no `-t`, the newest session's newest pane — the highest pane id in the
/// global roster (ids are monotonic, so the max is newest).
fn resolve_target(conn: &mut conn::AttachConn, target: Option<&str>) -> Result<String, String> {
    match target {
        None => {
            let panes = conn
                .send_checked("list-panes")
                .map_err(|err| format!("list-panes failed: {err}"))?;
            // Newest = highest id (ids are monotonic). The bare global
            // roster has no deterministic order, so max by numeric id is
            // the honest spelling of the newest-stand-in rule.
            panes
                .body
                .iter()
                .filter(|l| l.starts_with('%'))
                .filter_map(|l| l.split_whitespace().next())
                .filter_map(|id| {
                    let n: u64 = id[1..].parse().ok()?;
                    Some((n, id))
                })
                .max_by_key(|(n, _)| *n)
                .map(|(_, id)| id.to_string())
                .ok_or_else(|| "no panes exist — create a session first".to_string())
        }
        Some(target) => {
            // Try the daemon-side pane matcher first: ids and pane titles
            // both resolve here, and the failure tells us to try window or
            // session scopes. The reply's first line names the resolved
            // pane (`%N @W ...`), so an ok-but-empty reply does not count.
            if conn
                .send_checked(&format!("pane-info -t {target}"))
                .is_ok_and(|reply| {
                    reply.ok && reply.body.first().is_some_and(|l| l.starts_with('%'))
                })
            {
                return Ok(target.to_string());
            }
            // A window: its marked pane via the targeted list-panes.
            if let Ok(reply) = conn.send_checked(&format!("list-panes -t {target}")) {
                if reply.ok {
                    return marked_pane(&reply.body, target);
                }
            }
            // A session: its active window's marked pane.
            if let Ok(reply) = conn.send_checked(&format!("list-windows -t {target}")) {
                if reply.ok {
                    let window = marked_line(&reply.body, "@", target)?;
                    if let Ok(panes) = conn.send_checked(&format!("list-panes -t {window}")) {
                        if panes.ok {
                            return marked_pane(&panes.body, target);
                        }
                    }
                }
            }
            Err(format!("no such target: {target}"))
        }
    }
}

/// The `*`-marked line from a targeted list reply (`%N <leaf> *` pane rows
/// or `@N * <name>` window rows), else the first line. `sigil` guards the
/// line shape (`%` panes, `@` windows).
fn marked_line(body: &[String], sigil: &str, target: &str) -> Result<String, String> {
    body.iter()
        .find(|l| l.starts_with(sigil))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string)
        .ok_or_else(|| format!("empty listing for target {target}"))
}

/// The marked pane id from a `list-panes -t <window>` reply, preferring
/// the `*`-active pane over the first row.
pub(crate) fn marked_pane(body: &[String], target: &str) -> Result<String, String> {
    let active = body
        .iter()
        .find(|l| l.split_whitespace().nth(2) == Some("*"))
        .or_else(|| body.first())
        .and_then(|l| l.split_whitespace().next());
    active
        .map(str::to_string)
        .ok_or_else(|| format!("no panes under target {target}"))
}

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
        let grid = conn::terminal_grid();
        let mut session = Self {
            conn,
            socket_path: socket_path.to_path_buf(),
            pane,
            emulator: render::PaneEmulator::new(0, grid.0, grid.1),
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
        };
        session.resync();
        session.refresh_status();
        session.draw_status();
        Ok(session)
    }

    /// Resync the target pane's screen: the replay body goes to stdout
    /// verbatim (each reply line plus its newline — the draw's absolute
    /// CUP re-places the host cursor, so the write side's final newline
    /// is immaterial). The reply body — the daemon's screen-restore byte
    /// stream, ending with the pane's real cursor CUP — seeds the shadow
    /// emulator verbatim; its tracked cursor becomes the pane's truth the
    /// status draw re-places.
    fn resync(&mut self) {
        // Size report against the target pane's window, BEFORE the replay:
        // the handshake's target-less -C sizes only the newest session's
        // active window, and every switch re-enters here — a window
        // restored at another size never re-fit, and the pane's child ran
        // at the stale height (the manual-pass htop report; render mode's
        // switch path has always reported). rows-1: the status row stays
        // reserved below the content region.
        let (cols, rows) = conn::terminal_grid();
        if rows >= 2 && cols >= 2 {
            let _ = self.conn.send_checked(&format!(
                "refresh-client -t {} -C {}x{}",
                self.pane,
                cols,
                rows - 1
            ));
        }
        let reply = match self
            .conn
            .send_checked(&format!("refresh-client -t {}", self.pane))
        {
            Ok(reply) if reply.ok => reply,
            Ok(reply) => {
                let _ = std::io::stderr()
                    .write_all(format!("par-mux: {}\n", reply.body.join("\n")).as_bytes());
                return;
            }
            Err(err) => {
                let _ = std::io::stderr().write_all(format!("par-mux: {err}\n").as_bytes());
                return;
            }
        };
        // The restore stream ends with the pane's real cursor CUP, so the
        // shadow — which must track the PANE, not the host's post-write
        // drift — feeds it verbatim. A trailing newline appended here
        // parked the shadow one row below the prompt, and every status
        // draw's absolute placement with it (manual-pass cursor bug). The
        // host write below keeps its per-line newlines; the draw's
        // absolute CUP re-places the host cursor regardless.
        let bytes = reply.body.join("\n").into_bytes();
        self.emulator.feed(&bytes);
        let mut stdout = std::io::stdout().lock();
        // Clear before the replay: a switch lands here too, and the new
        // pane's screen must not mix with the previous pane's leftovers
        // (rows the last pane never wrote). The guard cleared at attach;
        // this re-clears per pane-show.
        let _ = stdout.write_all(b"\x1b[2J\x1b[H");
        for line in &reply.body {
            let _ = stdout.write_all(line.as_bytes());
            let _ = stdout.write_all(b"\n");
        }
        let _ = stdout.flush();
    }

    /// Re-read the status-line state: the pane's window and held-dead cue
    /// (pane-info), the owning session's name (list-windows scan for the
    /// window), the pane title, and the pane's agent roster count.
    fn refresh_status(&mut self) {
        if let Some(line) = self
            .conn
            .send_checked(&format!("pane-info -t {}", self.pane))
            .ok()
            .filter(|reply| reply.ok)
            .and_then(|reply| reply.body.first().cloned())
        {
            let mut fields = line.split_whitespace();
            if let (Some(_pane), Some(window)) = (fields.next(), fields.next()) {
                self.window = window.to_string();
            }
            self.exited = line
                .split_whitespace()
                .find_map(|tok| tok.strip_prefix("exited="))
                .map(|code| code.parse::<i32>().ok());
        }
        self.session_name.clear();
        self.session_id = None;
        if let Ok(reply) = self.conn.send_checked("list-sessions") {
            if reply.ok {
                for line in &reply.body {
                    let Some((sid, name)) = parse_session_line(line) else {
                        continue;
                    };
                    if let Ok(windows) = self.conn.send_checked(&format!("list-windows -t {sid}")) {
                        if windows
                            .body
                            .iter()
                            .any(|l| l.split_whitespace().next() == Some(self.window.as_str()))
                        {
                            self.session_id = Some(sid);
                            self.session_name = name;
                            break;
                        }
                        if !windows.ok {
                            break;
                        }
                    }
                }
            }
        }
        // The workspace roster for the status line's workspaces segment.
        // Ids sort as `+N` strings here; the daemon already lists in id
        // order, so the reply order IS id order.
        if let Ok(reply) = self.conn.send_checked("list-workspaces") {
            if reply.ok {
                let rows: Vec<(String, String, bool)> = reply
                    .body
                    .iter()
                    .filter_map(|l| parse_workspace_line(l))
                    .collect();
                self.workspaces = rows
                    .iter()
                    .map(|(id, name, _)| (id.clone(), name.clone()))
                    .collect();
                self.active_workspace = rows
                    .iter()
                    .find(|(_, _, active)| *active)
                    .map(|(id, _, _)| id.clone());
            }
        }
        if let Some(title) = self
            .conn
            .send_checked(&format!("pane-title -t {}", self.pane))
            .ok()
            .filter(|reply| reply.ok)
            .map(|reply| reply.body.join(" "))
        {
            self.pane_title = title;
        }
        self.agents = self
            .conn
            .send_checked("list-agents")
            .ok()
            .filter(|reply| reply.ok)
            .map(|reply| {
                reply
                    .body
                    .iter()
                    .filter(|l| l.split_whitespace().next() == Some(self.pane.as_str()))
                    .count()
            })
            .unwrap_or(0);
    }

    /// The pump: forward daemon pushes to stdout, stdin bytes to the pane,
    /// route the prefix, keep the status row current.
    fn pump(&mut self) -> PumpOutcome {
        let outcome = self.pump_loop();
        // The status row's DECSTBM reservation must not outlive the
        // session: reset the scroll region and unhide whatever the pane
        // left hidden on EVERY exit path (detach, %exit, socket close).
        // Raw mode itself is the guard's business; these are the bytes the
        // client is responsible for.
        self.restore_region();
        outcome
    }

    /// Reset the scroll region and cursor state the status line borrowed.
    fn restore_region(&self) {
        let mut stdout = std::io::stdout().lock();
        let _ = write!(stdout, "\x1b[r\x1b[?25h");
        let _ = stdout.flush();
    }

    fn pump_loop(&mut self) -> PumpOutcome {
        let mut stdin = Stdin::new();
        let mut status_dirty = true;
        // The flash cue's lifetime in loop polls (~1 s at POLL = 16 ms).
        const FLASH_POLLS: u32 = 60;
        let mut flash_polls = 0u32;
        // Periodic status redraw (~0.5 s): a full-screen pane app can wipe
        // the reserved bottom row or reset the host's scroll margins (the
        // manual-pass htop report) — one small absolute-CUP run restores
        // the bar without tracking the app's terminal writes.
        const STATUS_REDRAW_POLLS: u32 = 32;
        let mut status_polls = 0u32;
        loop {
            // 1. Drain daemon pushes.
            loop {
                match self.conn.try_recv() {
                    Ok(event) => {
                        if !self.handle_event(&event, &mut status_dirty) {
                            return PumpOutcome::DaemonExited;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // Socket closed: %exit delivered, eviction, or
                        // process death. One reconnect-and-requery; a
                        // client that cannot reattach reports the close.
                        if !self.reconnect() {
                            return PumpOutcome::ConnectionClosed;
                        }
                        break;
                    }
                }
            }

            // 2. Drain stdin (prefix routing, chunked forward).
            if self.pump_stdin(&mut stdin) {
                return PumpOutcome::Detached;
            }

            // 3. Status line redraw when something marked it dirty — or
            //    the host grid changed since the last draw (a resize, or
            //    ConPTY's geometry settling late). A reload's flash cue
            //    rides this path (route_bytes set it) and clears after
            //    about a second of polls, so the next normal redraw
            //    restores the plain status line.
            if status_dirty || self.size_changed() || self.flash.is_some() {
                if self.flash.is_some() {
                    flash_polls += 1;
                    if flash_polls > FLASH_POLLS {
                        self.flash = None;
                        flash_polls = 0;
                    }
                }
                status_dirty = false;
                status_polls = 0;
                self.refresh_status();
                self.draw_status();
            } else {
                status_polls += 1;
                if status_polls >= STATUS_REDRAW_POLLS {
                    status_polls = 0;
                    self.draw_status();
                }
            }

            // 4. Wait for the next push — and HANDLE it: an event that
            //    arrived during the wait is consumed here, and discarding
            //    it would drop the pane's %output bytes. Route it through
            //    the same handler; a Disconnected surfaces on the next
            //    pass's drain.
            if let Ok(event) = self.conn.recv_timeout(POLL) {
                if !self.handle_event(&event, &mut status_dirty) {
                    return PumpOutcome::DaemonExited;
                }
            }
        }
    }

    /// One reconnect-and-requery after a connection loss: re-handshake on
    /// the same socket, verify the pane still exists (a held pane keeps
    /// its id), and resync. False when the daemon is really gone.
    fn reconnect(&mut self) -> bool {
        let Ok(conn) = conn::AttachConn::connect(&self.socket_path) else {
            return false;
        };
        self.conn = conn;
        let _replay = self.conn.drain_pending_events();
        if self
            .conn
            .send_checked(&format!("pane-info -t {}", self.pane))
            .is_ok_and(|reply| reply.ok)
        {
            self.resync();
            self.refresh_status();
            self.draw_status();
            true
        } else {
            false
        }
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
            | TmuxNotification::WindowRenamed { .. }
            | TmuxNotification::SessionRenamed { .. }
            | TmuxNotification::SessionsChanged
            | TmuxNotification::WorkspacesChanged
            | TmuxNotification::AgentStateChanged { .. }
            | TmuxNotification::AgentReleased { .. } => {
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

    /// Read available stdin bytes and route them. Returns true on detach
    /// or stdin EOF.
    fn pump_stdin(&mut self, stdin: &mut Stdin) -> bool {
        loop {
            match stdin.read_available() {
                None => return false,
                Some(Ok(bytes)) if bytes.is_empty() => return true, // EOF
                Some(Ok(bytes)) => {
                    if self.route_bytes(&bytes) {
                        return true;
                    }
                }
                Some(Err(_)) => return true,
            }
        }
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
            if self.prefix_pending {
                self.prefix_pending = false;
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

    /// Enter the sticky resize mode (the `resize` chord): arrows adjust
    /// the focused pane's edges until Enter/Escape/`q` — tmux's resize
    /// step with an explicit mode instead of repeat-time. The flash cue
    /// rides the pump's flash path so the user can see the mode is up.
    fn enter_resize_mode(&mut self) {
        self.resize_mode = true;
        self.flash = Some(format!(
            "resize — arrows move the edge by {}, Enter/q exits",
            self.resize_step
        ));
    }

    /// Resize-mode byte routing: arrow CSI sequences (`ESC [ A..D`) send
    /// one resize step for the focused pane (`resize-pane -t <pane>
    /// -L|-R|-U|-D <step>`, the wire's relative form); `q`/Enter exit; any
    /// other byte exits the mode and is reprocessed by the normal router
    /// (the key that cancelled still does its job). Returns true on
    /// detach.
    fn route_resize_bytes(&mut self, bytes: &[u8]) -> bool {
        let mut index = 0;
        while index < bytes.len() {
            match &bytes[index..] {
                [0x1b, b'[', dir, ..] if (b'A'..=b'D').contains(dir) => {
                    self.resize_step_cmd(*dir);
                    index += 3;
                }
                _ => {
                    let exits = matches!(bytes[index], b'q' | b'\r');
                    self.resize_mode = false;
                    index += usize::from(exits);
                    // The remainder — including the cancelling byte unless
                    // it was a clean exit key — routes normally.
                    return self.route_bytes(&bytes[index..]);
                }
            }
        }
        false
    }

    /// One resize step in the arrow's direction: the wire's relative form
    /// (`resize-pane -t <pane> -U <step>` etc.). Best-effort — the
    /// daemon's repaint rides the pane's %output stream either way.
    fn resize_step_cmd(&mut self, arrow: u8) {
        let flag = match arrow {
            b'A' => "-U",
            b'B' => "-D",
            b'C' => "-R",
            _ => "-L",
        };
        let _ = self.conn.send_checked(&format!(
            "resize-pane -t {} {flag} {}",
            self.pane, self.resize_step
        ));
    }

    /// prefix { / }: swap the focused pane with its layout-order neighbor
    /// (`swap-pane -s <focused> -t <neighbor>`); fewer than two panes is
    /// a no-op. The daemon's %layout-change / output stream carries the
    /// visual swap.
    fn swap_pane(&mut self, direction: i32) {
        let Ok(reply) = self
            .conn
            .send_checked(&format!("list-panes -t {}", self.window))
        else {
            return;
        };
        if !reply.ok {
            return;
        }
        let panes: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|p| p.starts_with('%'))
            .map(str::to_string)
            .collect();
        if panes.len() < 2 {
            return;
        }
        let Some(position) = panes.iter().position(|p| *p == self.pane) else {
            return;
        };
        let next = (position as i32 + direction).rem_euclid(panes.len() as i32) as usize;
        let _ = self
            .conn
            .send_checked(&format!("swap-pane -s {} -t {}", self.pane, panes[next]));
    }

    /// prefix ?: the bindings panel. Passthrough has no overlay surface —
    /// the pane's own output owns the screen — so the panel prints as
    /// plain text (the same category rows the render-mode modal composes;
    /// filter/scroll are modal-only controls) and the pane's next output
    /// redraws over it (the documented passthrough help shape). The dump
    /// leads with a blank line and bolds the category headers — printing
    /// from the cursor's current row put the first header on the prompt
    /// line (manual pass).
    fn show_help(&mut self) {
        let rows = help_rows(
            self.prefix,
            self.reload_key,
            self.management,
            self.resize_step,
        );
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(help_dump_text(&rows).as_bytes());
        let _ = stdout.flush();
        // Advance the pane past the dump: the pane's cursor is still on
        // the prompt row, so every later keystroke painted over the help
        // (the manual-pass mix). One Enter at a shell prompt runs an empty
        // command line and a fresh prompt — ~2 rows each — so half the
        // dump's row count lands the shell below the text; the help stays
        // in the pane's scrollback either way.
        let enters = rows.len() / 2 + 1;
        let bytes = vec![b'\r'; enters];
        forward_chunked(&mut self.conn, self.pane.clone(), &bytes);
    }

    /// prefix + arrow: every arrow cycles panes in Phase A (tmux's o).
    fn prefix_arrow(&mut self, seq: &[u8]) {
        if matches!(seq, [b'[', b'A'..=b'D']) {
            self.cycle_pane();
        }
    }

    /// prefix o / arrows: select the next pane in the window's layout-leaf
    /// order, then resync — select-then-refresh, so the redraw is the
    /// daemon's authoritative screen.
    fn cycle_pane(&mut self) {
        let Ok(reply) = self
            .conn
            .send_checked(&format!("list-panes -t {}", self.window))
        else {
            return;
        };
        if !reply.ok {
            return;
        }
        let panes: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|p| p.starts_with('%'))
            .map(str::to_string)
            .collect();
        if panes.len() < 2 {
            return;
        }
        let current = panes.iter().position(|p| *p == self.pane).unwrap_or(0);
        let next = (current + 1) % panes.len();
        self.switch_to_pane(&panes[next]);
    }

    /// prefix n/p: move to the next/previous window of the session and
    /// attach to its active pane, select-then-refresh.
    fn switch_window(&mut self, direction: i32) {
        let Some(session) = self.session_id.clone() else {
            return;
        };
        let Ok(reply) = self
            .conn
            .send_checked(&format!("list-windows -t {session}"))
        else {
            return;
        };
        if !reply.ok {
            return;
        }
        let windows: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|w| w.starts_with('@'))
            .map(str::to_string)
            .collect();
        let Some(position) = windows.iter().position(|w| *w == self.window) else {
            return;
        };
        let next = (position as i32 + direction).rem_euclid(windows.len() as i32) as usize;
        let window = &windows[next];
        if self
            .conn
            .send_checked(&format!("select-window -t {window}"))
            .is_ok_and(|reply| reply.ok)
        {
            self.window = window.clone();
            self.attach_window_active_pane(window);
        }
    }

    /// prefix ( / ): the previous/next session in list-sessions order;
    /// attach to its active window's active pane, select-then-refresh.
    fn switch_session(&mut self, direction: i32) {
        let Ok(reply) = self.conn.send_checked("list-sessions") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let sessions: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| parse_session_line(l).map(|(id, _)| id))
            .collect();
        let Some(current) = sessions
            .iter()
            .position(|s| Some(s) == self.session_id.as_ref())
        else {
            return;
        };
        let next = (current as i32 + direction).rem_euclid(sessions.len() as i32) as usize;
        let session = &sessions[next];
        let Ok(windows) = self
            .conn
            .send_checked(&format!("list-windows -t {session}"))
        else {
            return;
        };
        if !windows.ok {
            return;
        }
        // The marked `*` window is the session's active one.
        let window = windows
            .body
            .iter()
            .find(|l| l.split_whitespace().nth(1) == Some("*"))
            .or_else(|| windows.body.first())
            .and_then(|l| l.split_whitespace().next());
        let Some(window) = window else {
            return;
        };
        let _ = self
            .conn
            .send_checked(&format!("select-window -t {window}"));
        self.window = window.to_string();
        self.session_id = Some(session.clone());
        self.attach_window_active_pane(window);
    }

    /// prefix W / C-w: the next/previous workspace in id order —
    /// `select-workspace -t +N`, then land the view on the workspace's
    /// session (its active window's active pane) through the same
    /// select-then-refresh contract every switch follows. A workspace
    /// with no sessions cannot be landed on: the selection still moves
    /// and the status refresh carries the new active marker.
    fn switch_workspace(&mut self, direction: i32) {
        let Ok(reply) = self.conn.send_checked("list-workspaces") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let rows: Vec<(String, String, bool)> = reply
            .body
            .iter()
            .filter_map(|l| parse_workspace_line(l))
            .collect();
        if rows.is_empty() {
            return;
        }
        let Some(current) = rows.iter().position(|(_, _, active)| *active) else {
            return;
        };
        let next = (current as i32 + direction).rem_euclid(rows.len() as i32) as usize;
        let (ws_id, _, _) = &rows[next];
        let _ = self
            .conn
            .send_checked(&format!("select-workspace -t {ws_id}"));
        self.land_in_workspace(ws_id);
    }

    /// After a workspace select, land the pump on the workspace's
    /// session: its first listed session's active window's active pane.
    /// A workspace with no sessions cannot be landed on — the status
    /// refresh carries the moved active marker instead.
    fn land_in_workspace(&mut self, ws_id: &str) {
        let Ok(reply) = self.conn.send_checked(&format!("list-sessions -t {ws_id}")) else {
            return;
        };
        if !reply.ok {
            return;
        }
        let Some((session, _)) = reply
            .body
            .iter()
            .filter_map(|l| parse_session_line(l))
            .next()
        else {
            self.refresh_status();
            self.draw_status();
            return;
        };
        let Ok(windows) = self
            .conn
            .send_checked(&format!("list-windows -t {session}"))
        else {
            return;
        };
        if !windows.ok {
            return;
        }
        let window = windows
            .body
            .iter()
            .find(|l| l.split_whitespace().nth(1) == Some("*"))
            .or_else(|| windows.body.first())
            .and_then(|l| l.split_whitespace().next());
        let Some(window) = window else {
            return;
        };
        let _ = self
            .conn
            .send_checked(&format!("select-window -t {window}"));
        self.window = window.to_string();
        self.session_id = Some(session);
        self.attach_window_active_pane(window);
    }

    /// Prefix r: `respawn-pane` — but ONLY when the pane is held dead
    /// (the card's "respawn-pane when held dead"); a live pane restart is
    /// not a Phase A affordance (the daemon itself refuses without -k, and
    /// killing a live pane from a mistyped chord would be destructive).
    fn respawn_if_dead(&mut self) {
        if self.exited.is_none() {
            return;
        }
        if self
            .conn
            .send_checked(&format!("respawn-pane -t {}", self.pane))
            .is_ok_and(|reply| reply.ok)
        {
            self.exited = None;
            // The dead pane's frozen screen is still on the glass and the
            // fresh replay only paints what the new grid holds — wipe the
            // surface first (we are inside the alternate screen) so the
            // new output does not mix over the corpse.
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(b"\x1b[2J\x1b[H");
            let _ = stdout.flush();
            self.resync();
            self.refresh_status();
            self.draw_status();
        }
    }

    /// Which management chord (if any) `key` is. Matched by byte BEFORE
    /// the fixed command table — the chords are configurable, so they
    /// cannot be static table arms.
    fn management_command(&self, key: u8) -> Option<ManagementKey> {
        let m = self.management;
        match key {
            k if k == m.split_right => Some(ManagementKey::SplitRight),
            k if k == m.split_down => Some(ManagementKey::SplitDown),
            k if k == m.kill_pane => Some(ManagementKey::KillPane),
            k if k == m.new_window => Some(ManagementKey::NewWindow),
            k if k == m.swap_prev => Some(ManagementKey::SwapPrev),
            k if k == m.swap_next => Some(ManagementKey::SwapNext),
            k if k == m.workspace_next => Some(ManagementKey::WorkspaceNext),
            k if k == m.workspace_prev => Some(ManagementKey::WorkspacePrev),
            _ => None,
        }
    }

    /// prefix % / ": `split-window -t <focused> [-h]` — the daemon
    /// focuses the new pane and replies with its id; land the pump on it
    /// (the same switch-then-refresh contract `cycle_pane` follows, so
    /// the redraw is the fresh pane's authoritative screen). tmux lands
    /// you on the new split; this is the same follow.
    fn split_pane(&mut self, right: bool) {
        let flag = if right { " -h" } else { "" };
        let Ok(reply) = self
            .conn
            .send_checked(&format!("split-window -t {}{flag}", self.pane))
        else {
            return;
        };
        if !reply.ok {
            return;
        }
        if let Some(new_pane) = reply.body.first() {
            self.switch_to_pane(new_pane);
        }
    }

    /// prefix x: `kill-pane -t <focused>`. Killing the focused pane
    /// removes it — the window survives (a survivor is announced via
    /// %window-pane-changed) or the window itself closes. The pump
    /// follows the successor: the window's active pane when one remains
    /// (the same land-on-survivor move the switch chords make), the
    /// dead-pane cue when the whole window closed is NOT survivable from
    /// here — that path ends through %sessions-changed's existing
    /// contract (passthrough: the next pane-info fails, the reconnect
    /// gate reports the pane gone; the user sees the cue and detaches or
    /// respawns per the standing contract). Best-effort either way: a
    /// failed kill (the pane already gone) changes nothing client-side.
    fn kill_focused_pane(&mut self) {
        let window = self.window.clone();
        let Ok(reply) = self
            .conn
            .send_checked(&format!("kill-pane -t {}", self.pane))
        else {
            return;
        };
        if !reply.ok {
            return;
        }
        // Does the window survive? Its new active pane is the land site.
        if let Ok(list) = self.conn.send_checked(&format!("list-panes -t {window}")) {
            if list.ok {
                if let Ok(survivor) = marked_pane(&list.body, &window) {
                    self.switch_to_pane(&survivor);
                    return;
                }
            }
        }
        // The window is gone (the focused pane was its last): the pane
        // this pump was showing no longer exists anywhere. Mirror the
        // held-dead guard's silence — take no further bytes, show the
        // exit cue — and let the user detach. `pane-info` on the dead id
        // fails on the next status refresh, which is fine: the guard
        // keeps stdin dropped and the chord table live.
        self.exited = Some(None);
        self.refresh_status();
        self.draw_status();
    }

    /// prefix c: `new-window -t <session>` — the daemon replies with the
    /// new window's id; select it and attach to its active pane (the
    /// reply ordering in tmux puts you on the fresh window; this follows).
    fn new_window_in_session(&mut self) {
        let Some(session) = self.session_id.clone() else {
            return;
        };
        let Ok(reply) = self.conn.send_checked(&format!("new-window -t {session}")) else {
            return;
        };
        if !reply.ok {
            return;
        }
        let Some(window) = reply.body.first() else {
            return;
        };
        let window = window.trim();
        if self
            .conn
            .send_checked(&format!("select-window -t {window}"))
            .is_ok_and(|reply| reply.ok)
        {
            self.window = window.to_string();
            self.attach_window_active_pane(window);
        }
    }

    /// The reload chord: re-read the config file, rebind the prefix and
    /// the reload chord live (the reload key rebinding includes itself —
    /// the NEXT reload follows the new chord), queue the status cue, and
    /// send `reload-config` to the daemon so its settings follow. A
    /// parse error in the re-read file shows on the status row instead
    /// of detaching.
    fn reload_config(&mut self) {
        match reload_client_chords(crate::mux::config::Chords {
            prefix: self.prefix,
            reload: self.reload_key,
            management: self.management,
            resize_step: self.resize_step,
            pane_borders: false,
            show_label_in_border: false,
            pane_gaps: 0,
            scrollbar_gutter: false,
            sidebar_width: 20,
            drag_cursor_shape: false,
            border_lines: "unicode".to_string(),
        }) {
            Ok(new_chords) => {
                self.prefix = new_chords.prefix;
                self.reload_key = new_chords.reload;
                self.management = new_chords.management;
                self.resize_step = new_chords.resize_step;
                self.flash = Some("config reloaded".to_string());
            }
            Err(err) => {
                self.flash = Some(format!("reload failed: {err}"));
            }
        }
        // Daemon-side: best-effort — the daemon reports its per-setting
        // outcome in its own reply; nothing here parses it (the status
        // cue above is the client-side truth).
        let _ = self.conn.send_checked("reload-config");
    }

    /// Make `window`'s active pane (its `*` marker in `list-panes -t`) the
    /// pumped pane, select-then-refresh.
    fn attach_window_active_pane(&mut self, window: &str) {
        let Ok(reply) = self.conn.send_checked(&format!("list-panes -t {window}")) else {
            return;
        };
        if !reply.ok {
            return;
        }
        if let Ok(pane) = marked_pane(&reply.body, window) {
            self.switch_to_pane(&pane);
        }
    }

    /// Point the pump at `pane`, select it daemon-side, and resync the
    /// redraw (the select-then-refresh contract every switch follows).
    fn switch_to_pane(&mut self, pane: &str) {
        let _ = self.conn.send_checked(&format!("select-pane -t {pane}"));
        self.pane = pane.to_string();
        self.exited = None;
        self.resync();
        self.refresh_status();
        self.draw_status();
    }

    /// Forward bytes to the pane in ~512-byte `send-keys -H` chunks. A
    /// send failure means the daemon is gone; the pump's next poll
    /// observes the close.
    fn send_chunked(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(CHUNK) {
            if self
                .conn
                .send_checked(&format!(
                    "send-keys -t {} -H {}",
                    self.pane,
                    hex_byte_list(chunk)
                ))
                .is_err()
            {
                return;
            }
        }
    }

    /// Whether the host grid changed since the last status draw. The
    /// first call sees `drawn_size == None` and reports true; the draw
    /// then records the size.
    fn size_changed(&mut self) -> bool {
        let size = conn::terminal_grid();
        let changed = self.drawn_size != Some(size);
        self.drawn_size = Some(size);
        changed
    }

    /// The composed status line, sans padding/positioning: session name,
    /// pane title, agent count, and the held-dead cue with its respawn
    /// hint.
    fn status_line(&self) -> String {
        let session = if self.session_name.is_empty() {
            "-"
        } else {
            &self.session_name
        };
        let title = if self.pane_title.is_empty() {
            &self.pane
        } else {
            &self.pane_title
        };
        // The workspaces segment leads the line: every workspace's name
        // in id order, the daemon's active one bracketed (passthrough has
        // no styling surface inside the inverse-video row).
        let mut line = String::new();
        if !self.workspaces.is_empty() {
            line.push(' ');
            for (index, (id, name)) in self.workspaces.iter().enumerate() {
                if index > 0 {
                    line.push(' ');
                }
                if Some(id) == self.active_workspace.as_ref() {
                    line.push('[');
                    line.push_str(name);
                    line.push(']');
                } else {
                    line.push_str(name);
                }
            }
            line.push_str(" |");
        }
        line.push_str(&format!(" {session} | {title}"));
        if self.agents > 0 {
            line.push_str(&format!(" | {} agent(s)", self.agents));
        }
        if let Some(code) = self.exited {
            match code {
                Some(code) => line.push_str(&format!(" | (exited {code} — C-b r respawns)")),
                None => line.push_str(" | (exited ? — C-b r respawns)"),
            }
        }
        // A reload (or its failure) leads the line: it is the freshest
        // fact and the one the user is waiting to see.
        if let Some(flash) = self.flash.as_deref() {
            line = format!(" {flash} |{line}");
        }
        line
    }

    /// The status line: reserve the bottom row with DECSTBM, draw
    /// `session | title [| N agents] [| (exited N — C-b r respawns)]`
    /// inverse-video, and re-place the cursor ABSOLUTELY at the pane's
    /// tracked cell.
    ///
    /// The old emission ended with ESC8 (restore saved cursor), which races
    /// pane output: a scroll landing between ESC7 and ESC8 leaves the saved
    /// position one line off and every later output paints over the wrong
    /// row. The draw keeps the ESC7/ESC8 wrap (protects against output
    /// interleaved WITHIN the draw itself), but the final position is a
    /// fresh absolute CUP computed from the shadow emulator's tracked cell
    /// — a scroll landing between the draw and the placement cannot make a
    /// fresh absolute position wrong.
    fn draw_status(&mut self) {
        let (cols, rows) = conn::terminal_grid();
        if rows < 2 || cols < 2 {
            return; // nowhere to put a status row
        }
        let bytes = self.status_draw_bytes(rows, cols);
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(&bytes);
        let _ = stdout.flush();
    }

    /// The status draw's byte emission, `(rows, cols)` parameterized so the
    /// headless suite can assert the cursor contract. The tracked-cell CUP
    /// closes the byte run.
    fn status_draw_bytes(&mut self, rows: u16, cols: u16) -> Vec<u8> {
        let bottom = rows; // DECSTBM + CUP rows are 1-indexed inclusive
        let line = self.status_line();
        let width = (cols as usize).saturating_sub(1);
        let mut text: String = line.chars().take(width).collect();
        let used = text.chars().count();
        if used < width {
            text.push_str(&" ".repeat(width - used));
        }
        let scroll_region_bottom = rows - 1;
        let (tracked_col, tracked_row) = self.tracked_cell(rows, cols);
        let mut out = Vec::with_capacity(text.len() + 48);
        out.extend_from_slice(
            format!(
                "\x1b7\x1b[1;{scroll_region_bottom}r\x1b[{bottom};1H\x1b[7m{text}\x1b[0m\x1b8\x1b[{};{}H",
                tracked_row + 1,
                tracked_col + 1
            )
            .as_bytes(),
        );
        out
    }

    /// The pane's tracked cursor cell, `(col, row)`, 0-based, clamped into
    /// the host grid. The shadow emulator is re-fit to the host grid when
    /// the host size changed; its tracked cursor is the pane's truth —
    /// including over a held-dead pane, where the frozen screen's cell is
    /// the right place to put the cursor.
    fn tracked_cell(&mut self, rows: u16, cols: u16) -> (u16, u16) {
        let (ecols, erows) = self.emulator.terminal().size();
        if (ecols as u16, erows as u16) != (cols, rows) {
            // Re-fit to the host grid; the tracked cursor survives the
            // re-fit (the core Terminal clamps it into bounds).
            self.emulator.resize(cols, rows);
        }
        let cursor = self.emulator.terminal().cursor();
        let (col, row) = (cursor.col as u16, cursor.row as u16);
        (
            col.min(cols.saturating_sub(1)),
            row.min(rows.saturating_sub(1)),
        )
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

/// The tmux spelling of a chord byte: `C-x` for control bytes (0 = the
/// Space spelling), the literal character otherwise.
pub(crate) fn spell_key(byte: u8) -> String {
    match byte {
        0 => "C-Space".to_string(),
        b if (1..27).contains(&b) => format!("C-{}", (b - 1 + b'a') as char),
        other => (other as char).to_string(),
    }
}

/// One help panel row: the text and whether it renders as an accent row
/// (category headers). Data for [`help_rows`]/[`compose_help_panel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelpRow {
    pub text: String,
    pub accent: bool,
    /// The modal's controls band: painted on a dark-grey strip across the
    /// panel's inner width (the help/picker/prompt footers).
    pub footer: bool,
}

/// The passthrough help dump's byte payload: a leading blank line so the
/// first header never prints on the cursor's current line (it used to land
/// on the prompt row), bold category headers, one CRLF per row.
pub(crate) fn help_dump_text(rows: &[HelpRow]) -> String {
    let mut out = String::from("\r\n");
    for row in rows {
        if row.accent {
            out.push_str(&format!("\x1b[1m{}\x1b[0m", row.text));
        } else {
            out.push_str(row.text.as_str());
        }
        out.push_str("\r\n");
    }
    out
}

/// The bindings help panel's category rows from the LIVE chord state — a
/// remapped chord shows its remapped key. Render mode composes the modal
/// panel over these ([`compose_help_panel`]); passthrough prints the same
/// rows as plain text (the documented shape).
pub(crate) fn help_rows(
    prefix: u8,
    reload: u8,
    m: crate::mux::config::Management,
    resize_step: u32,
) -> Vec<HelpRow> {
    let p = spell_key(prefix);
    let mut rows: Vec<HelpRow> = Vec::new();
    let push_cat = |rows: &mut Vec<HelpRow>, title: &str, entries: Vec<(String, String)>| {
        rows.push(HelpRow {
            text: format!(" {title} "),
            accent: true,
            footer: false,
        });
        let width = entries
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(0);
        for (key, desc) in entries {
            rows.push(HelpRow {
                text: format!(" {:width$}  {}", key, desc, width = width),
                accent: false,
                footer: false,
            });
        }
    };
    push_cat(
        &mut rows,
        "global",
        vec![
            (format!("{p} {p}"), "type a literal prefix".to_string()),
            (format!("{p} d"), "detach".to_string()),
            (format!("{p} {}", spell_key(m.help)), "keybinds".to_string()),
            (
                format!("{p} {}", spell_key(reload)),
                "reload the config".to_string(),
            ),
            (format!("{p} r"), "respawn the held-dead pane".to_string()),
        ],
    );
    push_cat(
        &mut rows,
        "workspaces",
        vec![
            (
                format!("{p} {}", spell_key(m.workspace_next)),
                "next workspace".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.workspace_prev)),
                "previous workspace".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.workspace_picker)),
                "workspace picker".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.sidebar)),
                "toggle the workspace side panel".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.status_bar)),
                "toggle the status bar".to_string(),
            ),
        ],
    );
    push_cat(
        &mut rows,
        "tabs / windows / sessions",
        vec![
            (
                format!("{p} {}", spell_key(m.new_window)),
                "new tab (window)".to_string(),
            ),
            (
                format!("{p} n / p"),
                "next / previous tab (window)".to_string(),
            ),
            (format!("{p} ( / )"), "previous / next session".to_string()),
            (
                format!("{p} {}", spell_key(m.picker)),
                "session/window picker".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.rename_window)),
                "rename the tab (window)".to_string(),
            ),
        ],
    );
    push_cat(
        &mut rows,
        "panes",
        vec![
            (
                format!("{p} {}", spell_key(m.split_right)),
                "split right".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.split_down)),
                "split down".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.kill_pane)),
                "kill the focused pane".to_string(),
            ),
            (format!("{p} o"), "cycle panes".to_string()),
            (
                format!(
                    "{p} {} / {}",
                    spell_key(m.swap_prev),
                    spell_key(m.swap_next)
                ),
                "swap pane prev/next".to_string(),
            ),
            (
                format!("{p} S-arrows"),
                "swap with the pane in that direction".to_string(),
            ),
            (
                format!("{p} {} arrows", spell_key(m.resize)),
                format!("resize mode, edge moves by {resize_step}"),
            ),
            (
                format!("{p} {}", spell_key(m.zoom)),
                "zoom the focused pane (toggle)".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.border_cycle)),
                "cycle the border style (herdr = per-pane boxes)".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.label_toggle)),
                "toggle pane labels".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.rename_pane)),
                "rename the focused pane".to_string(),
            ),
        ],
    );
    push_cat(
        &mut rows,
        "navigation",
        vec![
            (
                "click".to_string(),
                "focus the pane under the pointer".to_string(),
            ),
            (
                format!("{p} arrows"),
                "select the pane in that direction".to_string(),
            ),
            (format!("{p} ["), "scroll the pane's history".to_string()),
            (
                "wheel".to_string(),
                "scrollback; forwarded when the pane owns mouse".to_string(),
            ),
        ],
    );
    push_cat(
        &mut rows,
        "mouse",
        vec![
            (
                "click near divider".to_string(),
                "focuses (a bare click still focuses)".to_string(),
            ),
            (
                "drag divider".to_string(),
                "resize the adjacent split".to_string(),
            ),
            (
                "click +".to_string(),
                "new tab — prompts for its name".to_string(),
            ),
        ],
    );
    rows
}

/// The help panel's footer/controls line (render mode).
pub(crate) const HELP_FOOTER: &str = " search / · scroll j/k/arrows/pgup/pgdn · close esc/enter ";

/// The modal overlay's title for the bindings panel (the render-mode
/// chrome embeds it in the top border, left-aligned).
pub(crate) const HELP_OVERLAY_TITLE: &str = " keybinds ";

/// The modal overlay's title for the session/window picker.
pub(crate) const PICKER_OVERLAY_TITLE: &str = " picker ";

/// The workspace picker modal's title.
pub(crate) const WORKSPACE_PICKER_OVERLAY_TITLE: &str = " workspaces ";

/// The modal overlay's title for the rename-window prompt.
pub(crate) const PROMPT_WINDOW_OVERLAY_TITLE: &str = " rename window ";

/// The modal overlay's title in the border for the rename-pane prompt.
pub(crate) const PROMPT_PANE_OVERLAY_TITLE: &str = " rename pane ";

/// The rename prompt's footer controls line.
pub(crate) const PROMPT_FOOTER: &str = " enter rename · esc cancel ";

/// The new-tab prompt's footer controls line (herdr's chip vocabulary).
pub(crate) const NEW_PROMPT_FOOTER: &str = " enter save · ^c clear · esc cancel ";

/// The modal overlay's title for the new-window prompt (the tab strip's
/// `+` button).
pub(crate) const PROMPT_NEW_WINDOW_OVERLAY_TITLE: &str = " new tab ";

/// The modal overlay's title for the rename-workspace prompt.
pub(crate) const PROMPT_WORKSPACE_OVERLAY_TITLE: &str = " rename workspace ";

/// The modal overlay's title for the new-workspace prompt (the panel's
/// ` new ` chip).
pub(crate) const PROMPT_NEW_WORKSPACE_OVERLAY_TITLE: &str = " new workspace ";

/// Spell a NAME for the control wire: unconditional single quotes, the
/// embedded-quote `'\''` idiom — the same bounded quoting grammar the
/// daemon's parser (`shell_split`) and `agent_resume::render_argv` use,
/// so a name with spaces or quotes survives as one word.
pub(crate) fn wire_quote(name: &str) -> String {
    format!("'{}'", name.replace('\'', "'\\''"))
}

/// The new-window prompt's editable default: the shown session's next
/// free index — one past the highest window ordinal, bumped past any
/// window NAME that already claims the number (the manual-pass ask: the
/// next non-conflicting index). Pure over the queried windows — the
/// unit-test surface.
pub(crate) fn next_window_name(windows: &[(String, String)]) -> String {
    let used: std::collections::HashSet<&str> =
        windows.iter().map(|(_, name)| name.as_str()).collect();
    let mut candidate = windows
        .iter()
        .filter_map(|(id, _)| id.trim_start_matches('@').parse::<u32>().ok())
        .max()
        .map_or(1, |max| max + 1);
    while used.contains(candidate.to_string().as_str()) {
        candidate += 1;
    }
    candidate.to_string()
}

/// The new-workspace prompt's editable default: the roster's next free
/// index — one past the highest workspace ordinal, bumped past any
/// workspace NAME that already claims the number (the same
/// non-conflicting rule [`next_window_name`] runs over the windows).
/// Pure over the queried workspaces — the unit-test surface.
pub(crate) fn next_workspace_name(workspaces: &[(String, String)]) -> String {
    let used: std::collections::HashSet<&str> =
        workspaces.iter().map(|(_, name)| name.as_str()).collect();
    let mut candidate = workspaces
        .iter()
        .filter_map(|(id, _)| id.trim_start_matches('+').parse::<u32>().ok())
        .max()
        .map_or(1, |max| max + 1);
    while used.contains(candidate.to_string().as_str()) {
        candidate += 1;
    }
    candidate.to_string()
}

/// The prompt's content rows: the input line (`> text▌`, the ▌ is the
/// insert point — the frame hides the host cursor under the overlay), a
/// spacer, and the footer controls line — `footer` spells the controls
/// (the rename and new-tab prompts differ). Pure over its input — the
/// unit-test surface.
pub(crate) fn compose_prompt_panel(text: &str, footer: &str) -> Vec<HelpRow> {
    vec![
        HelpRow {
            text: format!(" > {text}▌"),
            accent: true,
            footer: false,
        },
        HelpRow {
            text: String::new(),
            accent: false,
            footer: false,
        },
        HelpRow {
            text: footer.to_string(),
            accent: false,
            footer: true,
        },
    ]
}

/// The help panel's content rows after the filter: headers hide when
/// nothing beneath them matches. Shared by the panel composer and the
/// renderer's scroll-state math.
pub(crate) fn help_content(rows: &[HelpRow], filter: &str) -> Vec<HelpRow> {
    let lower = filter.to_lowercase();
    let mut content: Vec<HelpRow> = Vec::new();
    let mut pending_header: Option<HelpRow> = None;
    for row in rows {
        if row.accent {
            pending_header = Some(row.clone());
        } else if lower.is_empty() || row.text.to_lowercase().contains(&lower) {
            if let Some(header) = pending_header.take() {
                content.push(header);
            }
            content.push(row.clone());
        }
    }
    content
}

/// The windowing start the help panel shows: the scroll clamped so the
/// last `visible` rows fill the panel.
pub(crate) fn help_window_start(content_len: usize, visible: usize, scroll: usize) -> usize {
    scroll.min(content_len.saturating_sub(visible))
}

/// Compose the help panel's CONTENT rows: the filter line when a filter
/// is open or set (` /query▌`; the footer already advertises `search /`,
/// so no placeholder when idle — the round-5 report), the filter's
/// matching rows windowed to `visible` rows at `scroll`, and the footer
/// controls line. The renderer's `PaneRenderer::paint_overlay` wraps
/// these in the themed border box (ring, title, badge, background, and
/// the overflow thumb). Pure over its inputs — the unit-test surface for
/// the panel.
pub(crate) fn compose_help_panel(
    rows: &[HelpRow],
    filter: &str,
    filtering: bool,
    visible: usize,
    scroll: usize,
) -> Vec<HelpRow> {
    let _ = filtering; // the cursor glyph rides the filter text below
    let content = help_content(rows, filter);
    let content_len = content.len();
    let start = help_window_start(content_len, visible, scroll);
    let window: Vec<HelpRow> = content[start..(start + visible.min(content_len - start))].to_vec();

    let mut panel: Vec<HelpRow> = Vec::new();
    if filtering || !filter.is_empty() {
        panel.push(HelpRow {
            text: format!(" /{filter}▌"),
            accent: false,
            footer: false,
        });
    }
    panel.extend(window);
    panel.push(HelpRow {
        text: HELP_FOOTER.to_string(),
        accent: false,
        footer: true,
    });
    panel
}

/// The session/window picker's footer/controls line (render mode).
pub(crate) const PICKER_FOOTER: &str =
    " navigate arrows/j/k · select enter/click · filter / · close esc/q ";

/// One session in the picker's queried state: its windows as
/// `(window_id, name)` in window order, the session's active window id,
/// and whether the session is the one the view currently shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerEntry {
    pub session_id: String,
    pub session_name: String,
    pub windows: Vec<(String, String)>,
    pub active_window: Option<String>,
    pub current: bool,
}

/// Where one picker display row came from: a session (its header row) or
/// one of its windows. The compose returns these parallel to the
/// filtered rows so a selection or a click maps back to a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerRef {
    Session(usize),
    Window(usize, usize),
    /// One workspace row (the workspace picker): the workspace's index
    /// into the picker's queried workspace list.
    Workspace(usize),
}

/// The picker's display rows from the queried entries: one accent
/// header per session (` $N: name`, `>`-marked when the session is the
/// one the view shows), its windows nested beneath it as `   @N: name`
/// rows (`>`-marked for the shown window, `*` suffixed for the
/// session's active window). The parallel refs list maps each row to
/// its selection target.
pub(crate) fn picker_rows(
    entries: &[PickerEntry],
    current_window: Option<&str>,
) -> (Vec<HelpRow>, Vec<PickerRef>) {
    let mut rows: Vec<HelpRow> = Vec::new();
    let mut refs: Vec<PickerRef> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let marker = if entry.current { ">" } else { " " };
        rows.push(HelpRow {
            text: format!(" {marker}{0}: {1}", entry.session_id, entry.session_name),
            accent: true,
            footer: false,
        });
        refs.push(PickerRef::Session(i));
        for (w, (window_id, name)) in entry.windows.iter().enumerate() {
            let marker = if current_window == Some(window_id.as_str()) {
                ">"
            } else {
                " "
            };
            let active = entry.active_window.as_deref() == Some(window_id.as_str());
            let star = if active { " *" } else { "" };
            rows.push(HelpRow {
                text: format!(" {marker}  {window_id}: {name}{star}"),
                accent: false,
                footer: false,
            });
            refs.push(PickerRef::Window(i, w));
        }
    }
    (rows, refs)
}

/// One row of an edit's filtering: `true` keeps the row.
fn picker_row_matches(row: &HelpRow, lower: &str) -> bool {
    lower.is_empty() || row.text.to_lowercase().contains(lower)
}

/// The listbox panning rule the picker's content window uses: `start`
/// moves only when `selected` leaves the visible window `[start,
/// start+visible)`. Pure — the session keeps the running `start` and
/// passes it back each compose.
pub(crate) fn listbox_scroll(start: usize, selected: usize, visible: usize) -> usize {
    if visible == 0 {
        return 0;
    }
    if selected < start {
        selected
    } else if selected >= start + visible {
        selected + 1 - visible
    } else {
        start
    }
}

/// Compose the picker panel's CONTENT rows: the always-visible filter
/// line (the same shape [`compose_help_panel`] draws — placeholder when
/// inactive, ` /query▌` while typing), the filter's matching rows with
/// headers hiding when nothing beneath them matches, the selection
/// cursor `▸` prefixed to the selected row (clamped into range), the
/// content windowed to `visible` rows at the panned `start`, and the
/// footer controls line. Returns the panel rows, the FILTERED refs (the
/// selection/click target for each content row, in content order), and
/// the window's content start index, so a click at composed row `i`
/// maps to content row `start + i - 1`. Pure over its inputs — the
/// unit-test surface for the picker.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compose_picker_panel(
    rows: &[HelpRow],
    refs: &[PickerRef],
    filter: &str,
    filtering: bool,
    selected: usize,
    visible: usize,
    start: usize,
) -> (Vec<HelpRow>, Vec<PickerRef>, usize) {
    let lower = filter.to_lowercase();
    let mut content: Vec<HelpRow> = Vec::new();
    let mut content_refs: Vec<PickerRef> = Vec::new();
    let mut pending: Option<(HelpRow, PickerRef)> = None;
    for (row, r#ref) in rows.iter().zip(refs.iter()) {
        if row.accent {
            pending = Some((row.clone(), *r#ref));
        } else if picker_row_matches(row, &lower) {
            if let Some((header, header_ref)) = pending.take() {
                content.push(header);
                content_refs.push(header_ref);
            }
            content.push(row.clone());
            content_refs.push(*r#ref);
        }
    }
    // The selection cursor clamps into the filtered range (a narrowed
    // filter can shrink the list under the cursor).
    let selected = selected.min(content.len().saturating_sub(1));
    let start = start.min(selected);
    let start = listbox_scroll(start, selected, visible);
    for (i, row) in content.iter_mut().enumerate() {
        if i == selected {
            row.text = format!("▸{}", row.text);
        }
    }
    let content_len = content.len();
    let end = (start + visible).min(content_len);
    let window: Vec<HelpRow> = content[start..end].to_vec();

    let mut panel: Vec<HelpRow> = Vec::new();
    if filtering || !filter.is_empty() {
        panel.push(HelpRow {
            text: format!(" /{filter}▌"),
            accent: false,
            footer: false,
        });
    }
    panel.extend(window);
    panel.push(HelpRow {
        text: PICKER_FOOTER.to_string(),
        accent: false,
        footer: true,
    });
    (panel, content_refs, start)
}

/// One section of the workspace side panel: the entries top-down. The
/// panel renders sections top-down — workspaces first; more sections
/// slot in after. The section title moved to the tab strip's lead
/// segment (round 6), so the section carries only its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SidebarSection {
    /// `(id, label, active)` per row; `id` is what a click lands on.
    pub rows: Vec<(String, String, bool)>,
}

/// One composed sidebar line: buffer-row-relative `y`, the clickable
/// column span `[span.0, span.1)` within the strip (body rows span the
/// content width, the footer chips exactly their cells), the text,
/// whether it paints from a non-zero column, and the id a click
/// activates (`None` for filler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SidebarLine {
    pub y: u16,
    /// The text's leftmost column (0 for body rows, the chips' cells
    /// for the footer row).
    pub x: u16,
    /// The clickable columns `[x, x_end)` — the click hit-test's span.
    pub x_end: u16,
    pub text: String,
    pub id: Option<String>,
    /// The active entry: painted as herdr's full-width inverted block.
    pub active: bool,
}

/// The side panel's footer chips: ` new ` opens the new-workspace
/// prompt, ` menu ` the active workspace's menu. The ids flow through
/// [`SidebarLine.id`] into the click dispatch.
pub(crate) const SIDEBAR_NEW_ID: &str = "panel:new";
pub(crate) const SIDEBAR_MENU_ID: &str = "panel:menu";

/// Compose the side panel's lines from its sections: the rows top-down
/// (NO section header rows — the title lives in the tab strip's lead
/// segment since round 6, so workspace rows start at composed row 0 =
/// host row 1), clipped to `width - 1` columns (the divider column
/// owns the strip's right edge), plus a footer row pinned to the
/// panel's LAST row: ` new ` bottom-left and ` menu ` bottom-right
/// (the mock's panel shape; each chip its own clickable span). Pure —
/// the unit-test surface.
pub(crate) fn compose_sidebar(
    sections: &[SidebarSection],
    width: u16,
    height: u16,
) -> Vec<SidebarLine> {
    let mut lines: Vec<SidebarLine> = Vec::new();
    let max = usize::from(height);
    if max == 0 {
        return lines;
    }
    let text_width = usize::from(width.saturating_sub(1));
    // Body rows fill everything above the footer row.
    let body_cap = max - 1;
    for section in sections {
        for (id, label, active) in &section.rows {
            if lines.len() >= body_cap {
                break;
            }
            let marker = if *active { "▸" } else { " " };
            let text: String = format!(" {marker} {label}")
                .chars()
                .take(text_width)
                .collect();
            lines.push(SidebarLine {
                y: lines.len() as u16,
                x: 0,
                x_end: width.saturating_sub(1),
                text,
                id: Some(id.clone()),
                active: *active,
            });
        }
    }
    // The footer chips pin to the panel's last row; a strip too narrow
    // for a chip drops it.
    let footer_y = (max - 1) as u16;
    let new_chip = " new ";
    if text_width >= new_chip.len() {
        lines.push(SidebarLine {
            y: footer_y,
            x: 0,
            x_end: new_chip.len() as u16,
            text: new_chip.to_string(),
            id: Some(SIDEBAR_NEW_ID.to_string()),
            active: false,
        });
        let menu_chip = " menu ";
        if text_width >= new_chip.len() + menu_chip.len() {
            let mx = text_width - menu_chip.len();
            lines.push(SidebarLine {
                y: footer_y,
                x: mx as u16,
                x_end: text_width as u16,
                text: menu_chip.to_string(),
                id: Some(SIDEBAR_MENU_ID.to_string()),
                active: false,
            });
        }
    }
    lines
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
mod tests {
    use super::*;
    use crate::mux::{emit_block, MuxServer};
    // The Listener trait supplies `.accept()` on unix; the Windows named-pipe
    // listener accepts via its own inherent impl, so this import is unused there.
    #[cfg(unix)]
    use interprocess::local_socket::traits::Listener as _;
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::{Arc, Mutex};

    /// A temp-dir socket path unique to this test process.
    fn test_socket(tag: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("par-mux-attach-{}-{tag}.sock", std::process::id()));
        path
    }

    /// A hand-rolled daemon substitute: accepts one connection, records
    /// every control line it receives (in order), and answers each with a
    /// scripted reply chosen by the command name. Replies use the real
    /// wire framing (`emit_block`) so the client parser sees the protocol
    /// exactly as the daemon writes it. The listener is consumed by the
    /// accept thread; the socket path is what the test connects to.
    struct FakeDaemon {
        /// (command name, full command line) per control line, in order.
        received: Receiver<(String, String)>,
    }

    impl FakeDaemon {
        fn bind(tag: &str) -> (Self, PathBuf) {
            let path = test_socket(tag);
            let _ = std::fs::remove_file(&path);
            let listener = crate::mux::bind_local_listener(&path).expect("bind fake daemon");
            let (tx, received) = channel();
            std::thread::spawn(move || {
                if let Ok(stream) = listener.accept() {
                    serve_one(stream, tx);
                }
            });
            (Self { received }, path)
        }
    }

    /// Serve one connection: record lines, answer per the scripted table.
    fn serve_one(stream: crate::mux::LocalStream, tx: std::sync::mpsc::Sender<(String, String)>) {
        use interprocess::TryClone as _;
        let mut writer = stream.try_clone().expect("clone stream");
        let mut reader = BufReader::new(stream);
        let mut number = 0u32;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
            let reply = match name.as_str() {
                "version" => "9.9.9+deadbeef".to_string(),
                "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
                "list-panes" => "%0\n".to_string(),
                // The resync's scripted restore body: absolute row
                // addressing like the real encoder's, ending with the
                // pane's real cursor CUP (row 2, col 9 — right after
                // "prompt> "). No trailing newline, like the real stream.
                "refresh-client" if trimmed.contains("-t %0") => {
                    "\x1b[1;1Hfirst\x1b[2;1Hprompt> \x1b[2;9H".to_string()
                }
                _ => String::new(),
            };
            number += 1;
            tx.send((name, trimmed.to_owned())).ok();
            writer
                .write_all(emit_block(number, &reply, true).as_bytes())
                .ok();
            writer.flush().ok();
        }
    }

    /// Waits for the recorded lines, tolerating the reader thread's delay.
    fn recorded(rx: &Receiver<(String, String)>, n: usize) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for _ in 0..n {
            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(line) => out.push(line),
                Err(_) => break,
            }
        }
        out
    }

    /// The next recorded line whose command name is `name`, draining any
    /// interleaved status re-queries first — the robust needle when the
    /// chord's resync burst has a variable tail.
    fn wait_for_line(rx: &Receiver<(String, String)>, name: &str) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if std::time::Instant::now() > deadline {
                panic!("no {name} line arrived within the bound");
            }
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok((n, line)) if n == name => return line,
                Ok(_) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("no {name} line: the daemon side is gone");
                }
            }
        }
    }

    /// The daemon answers version and list-commands; the handshake must
    /// send version, then list-commands, then set-client-colors, then
    /// refresh-client with -C and -p — in that order.
    #[test]
    fn handshake_sends_version_list_commands_colors_refresh_in_order() {
        let (daemon, path) = FakeDaemon::bind("order");
        {
            let conn = conn::AttachConn::connect(&path).expect("connect");
            assert_eq!(conn.daemon_stamp(), Some("9.9.9+deadbeef"));
            assert!(conn.has_feature("replay-held-state"));
        }
        let lines = recorded(&daemon.received, 4);
        let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "version",
                "list-commands",
                "set-client-colors",
                "refresh-client"
            ],
            "handshake order: {lines:?}"
        );
        let colors = &lines[2].1;
        assert!(colors.starts_with("set-client-colors -f "), "got: {colors}");
        assert!(colors.contains(" -b "), "got: {colors}");
        let refresh = &lines[3].1;
        assert!(refresh.contains(" -C "), "got: {refresh}");
        assert!(refresh.contains(" -p "), "got: {refresh}");
    }

    /// A pre-feature daemon rejects list-commands; the handshake proceeds
    /// with no features and still completes steps 3-4.
    #[test]
    fn pre_feature_daemon_leads_to_empty_features_and_completed_handshake() {
        let path = test_socket("prefeature");
        let _ = std::fs::remove_file(&path);
        let listener = crate::mux::bind_local_listener(&path).expect("bind");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            if let Ok(stream) = listener.accept() {
                serve_pre_feature(stream, tx);
            }
        });
        let conn = conn::AttachConn::connect(&path).expect("connect");
        assert!(conn.daemon_features().is_empty());
        assert!(!conn.has_feature("replay-held-state"));
        let lines = recorded(&rx, 4);
        let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "version",
                "list-commands",
                "set-client-colors",
                "refresh-client"
            ]
        );
    }

    /// A daemon from before `list-commands`: %error on it, everything else
    /// answered.
    fn serve_pre_feature(
        stream: crate::mux::LocalStream,
        tx: std::sync::mpsc::Sender<(String, String)>,
    ) {
        use interprocess::TryClone as _;
        let mut writer = stream.try_clone().expect("clone stream");
        let mut reader = BufReader::new(stream);
        let mut number = 0u32;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
            number += 1;
            tx.send((name.clone(), trimmed.to_owned())).ok();
            let reply = match name.as_str() {
                "version" => "9.9.9+deadbeef".to_string(),
                _ => String::new(),
            };
            let ok = name == "version" || name == "set-client-colors" || name == "refresh-client";
            writer
                .write_all(emit_block(number, &reply, ok).as_bytes())
                .ok();
            writer.flush().ok();
        }
    }

    /// The stamp quiet rule: equal stamps are quiet; a same-version
    /// `+unknown` pair is quiet (unprovable); anything else warns.
    #[test]
    fn stamp_quiet_rule() {
        use super::conn::stamp_mismatch_is_quiet as quiet;
        assert!(quiet("1.2.3+abc", "1.2.3+abc"));
        assert!(quiet("1.2.3+unknown", "1.2.3+whatever"));
        assert!(quiet("1.2.3+abc", "1.2.3+unknown"));
        assert!(!quiet("1.2.3+abc", "1.2.4+abc"));
        assert!(!quiet("1.2.3+abc", "1.2.3+def"));
    }

    /// `list-commands` reply parsing: the `features` line's tokens, unknown
    /// lines ignored.
    #[test]
    fn feature_token_parsing() {
        use super::conn::parse_feature_tokens;
        let body = vec![
            "capture-pane escape".to_string(),
            "features replay-held-state".to_string(),
            "something else".to_string(),
        ];
        assert_eq!(
            parse_feature_tokens(&body),
            vec!["replay-held-state".to_string()]
        );
        assert!(parse_feature_tokens(&[]).is_empty());
    }

    /// attach with no daemon on the path reports the no-daemon error
    /// instead of spawning one.
    #[test]
    fn no_daemon_path_reports_no_daemon_without_spawning() {
        let path = test_socket("nodaemon");
        let _ = std::fs::remove_file(&path);
        let options = AttachOptions {
            socket: Some(path.clone()),
            name: None,
            target: None,
            prefix: None,
            reload: None,
            mode: None,
        };
        assert_eq!(run(&options), ExitCode::FAILURE);
        // And nothing appeared on the path: no auto-spawn.
        assert!(!path.exists(), "attach must not spawn a daemon");
    }

    /// A real server answers the whole handshake; attach resolves the
    /// newest session's newest pane, pumps, and exits cleanly when the
    /// daemon shuts down (`%exit`). The headless test ends through that
    /// shutdown — stdin never delivers bytes here, so `%exit` is the
    /// deterministic exit path.
    #[test]
    fn end_to_end_with_real_server() {
        let path = test_socket("real");
        let _ = std::fs::remove_file(&path);
        let server = MuxServer::bind(&path).expect("bind server");
        let handle = Arc::new(Mutex::new(Some(server.shutdown_handle())));
        let server_thread = std::thread::spawn(move || server.run());
        // Give the accept loop a moment; `run()` returns only at shutdown.
        std::thread::sleep(std::time::Duration::from_millis(100));

        // A pane to attach to, created by an ordinary client.
        let mut seeder = crate::mux::MuxClient::connect(&path).expect("seeder connect");
        seeder
            .send("new-session -s attach-e2e")
            .expect("new-session");

        // Shut the daemon down shortly after attach starts: %exit is the
        // pump's deterministic exit, and the headless harness never types.
        let killer_path = path.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if let Ok(mut client) = crate::mux::MuxClient::connect(&killer_path) {
                let _ = client.send_checked("kill-server");
            }
        });

        let options = AttachOptions {
            socket: Some(path.clone()),
            name: None,
            target: None,
            prefix: None,
            reload: None,
            mode: None,
        };
        assert_eq!(run(&options), ExitCode::SUCCESS);

        handle
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        server_thread.join().expect("server thread");
    }

    /// AttachOptions resolves --socket over NAME and falls back to the env
    /// var in the documented precedence.
    #[test]
    fn socket_path_precedence() {
        let explicit = AttachOptions {
            socket: Some(PathBuf::from("/tmp/explicit.sock")),
            name: Some("work".to_string()),
            target: None,
            prefix: None,
            reload: None,
            mode: None,
        };
        assert_eq!(explicit.socket_path(), PathBuf::from("/tmp/explicit.sock"));
        let named = AttachOptions {
            socket: None,
            name: Some("work".to_string()),
            target: None,
            prefix: None,
            reload: None,
            mode: None,
        };
        assert_eq!(named.socket_path(), crate::mux::default_socket_path("work"));
        let fallback = AttachOptions {
            socket: None,
            name: None,
            target: None,
            prefix: None,
            reload: None,
            mode: None,
        };
        // No env var in the test harness -> the unnamed default.
        if std::env::var_os("PAR_MUX_SOCKET").is_none() {
            assert_eq!(
                fallback.socket_path(),
                crate::mux::default_socket_path("default")
            );
        }
    }

    /// The `--prefix` grammar: control spellings and a literal single
    /// character; anything else refuses rather than silently misrouting.
    #[test]
    fn prefix_parsing_covers_tmux_spellings_and_literals() {
        assert_eq!(parse_prefix("C-b"), Some(0x02));
        assert_eq!(parse_prefix("C-a"), Some(0x01));
        assert_eq!(parse_prefix("C-Z"), Some(0x1a));
        assert_eq!(parse_prefix("C-Space"), Some(0x00));
        assert_eq!(parse_prefix("q"), Some(b'q'));
        assert_eq!(parse_prefix(""), None);
        assert_eq!(parse_prefix("C-"), None);
        assert_eq!(parse_prefix("C-1"), None);
        assert_eq!(parse_prefix("C-bb"), None);
    }

    /// The default target: the newest session's newest pane — the highest
    /// pane id in the global roster. A real tree over the default factory
    /// pins the pick with one pane.
    #[test]
    fn default_target_is_the_newest_sessions_newest_pane() {
        let (daemon, path) = FakeDaemon::bind("resolve");
        {
            let mut conn = conn::AttachConn::connect(&path).expect("connect");
            let pane = resolve_target(&mut conn, None).expect("resolve");
            assert_eq!(pane, "%0");
            // The fake saw list-panes after the handshake's four lines.
            let lines = recorded(&daemon.received, 5);
            assert_eq!(
                lines[4].0, "list-panes",
                "handshake then list-panes: {lines:?}"
            );
        }
    }

    /// The `list-sessions` / bare `list-windows` wire shape `$N: name`
    /// parses to a colon-less id plus the name — the shape the daemon's
    /// id parser (SessionId::from_str) accepts, and the reason a plain
    /// whitespace split (`$0:`) breaks every downstream `-t $N:` query.
    #[test]
    fn parse_session_line_strips_the_sigil_colon() {
        assert_eq!(
            parse_session_line("$0: work"),
            Some(("$0".to_string(), "work".to_string()))
        );
        assert_eq!(
            parse_session_line("$12: spaced name"),
            Some(("$12".to_string(), "spaced name".to_string()))
        );
        assert_eq!(parse_session_line("$12"), None, "no colon-space, no parse");
        assert_eq!(parse_session_line(""), None);
    }

    /// One `list-workspaces` line parses into id, name, and the active
    /// marker; a line without the `+N:` shape does not parse.
    #[test]
    fn parse_workspace_line_splits_the_id_name_and_active_marker() {
        assert_eq!(
            parse_workspace_line("+0: main active"),
            Some(("+0".to_string(), "main".to_string(), true))
        );
        assert_eq!(
            parse_workspace_line("+1: lab"),
            Some(("+1".to_string(), "lab".to_string(), false))
        );
        assert_eq!(
            parse_workspace_line("+2: spaced name"),
            Some(("+2".to_string(), "spaced name".to_string(), false))
        );
        assert_eq!(parse_workspace_line("0: x"), None, "no + sigil, no parse");
        assert_eq!(parse_workspace_line("+2"), None, "no colon-space, no parse");
        assert_eq!(parse_workspace_line(""), None);
    }

    /// `-t` with a pane id passes through; a window target resolves to the
    /// window's marked pane via the targeted list-panes query.
    #[test]
    fn explicit_targets_resolve_through_the_daemon() {
        // Scripted daemon: pane-info %5 ok; list-panes -t @1 replies a
        // two-pane roster with pane %7 marked active.
        let path = test_socket("targets");
        let _ = std::fs::remove_file(&path);
        let listener = crate::mux::bind_local_listener(&path).expect("bind");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            use interprocess::TryClone as _;
            let Ok(stream) = listener.accept() else {
                return;
            };
            let mut writer = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(stream);
            let mut number = 0u32;
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    continue;
                }
                let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
                number += 1;
                let reply = match trimmed {
                    "version" => "9.9.9+deadbeef".to_string(),
                    "list-commands" => String::new(),
                    "pane-info -t %5" => "%5 @0 80x24".to_string(),
                    "list-panes -t @1" => "%6 0 -\n%7 1 *".to_string(),
                    _ => String::new(),
                };
                tx.send((name, trimmed.to_owned())).ok();
                writer
                    .write_all(crate::mux::emit_block(number, &reply, true).as_bytes())
                    .ok();
                writer.flush().ok();
            }
        });
        let mut conn = conn::AttachConn::connect(&path).expect("connect");
        drop(rx);
        assert_eq!(
            resolve_target(&mut conn, Some("%5")).expect("pane id"),
            "%5"
        );
        assert_eq!(
            resolve_target(&mut conn, Some("@1")).expect("window"),
            "%7",
            "the marked active pane wins over the first row"
        );
        assert!(
            resolve_target(&mut conn, Some("@nope")).is_err(),
            "an unresolvable target errors"
        );
    }

    /// Prefix routing: d detaches; a plain byte goes to the pane as hex;
    /// prefix prefix sends the prefix byte itself.
    #[test]
    fn prefix_router_detaches_forwards_and_sends_literal_prefix() {
        let mut session = Session {
            conn: test_dead_conn(),
            socket_path: PathBuf::from("/nonexistent"),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        // Plain bytes forward when the pane is live.
        assert!(!session.route_bytes(b"hello"));
        assert!(session.route_bytes(&[0x02, b'd']), "prefix d detaches");
        assert!(
            !session.route_bytes(&[0x02, 0x02]),
            "prefix prefix consumes without detaching"
        );
    }

    /// A held-dead focused pane takes no bytes: with `exited` set, plain
    /// stdin bytes are dropped, so the daemon's NotStartedError `%error`
    /// reply to a dead-pane send-keys never has a chance to (a) hit the
    /// wire at all or (b) the bytes themselves never echo into the pane's
    /// frozen screen. Prefix chords still route.
    #[test]
    fn dead_pane_takes_no_stdin_bytes_but_prefix_keys_still_route() {
        let mut session = Session {
            conn: test_dead_conn(),
            socket_path: PathBuf::from("/nonexistent"),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: Some(Some(0)),
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        assert!(
            !session.route_bytes(b"typed while dead"),
            "typing into a dead pane never detaches"
        );
        // Prefix keys still route on a dead pane.
        assert!(
            session.route_bytes(&[0x02, b'd']),
            "prefix d still detaches"
        );
        assert!(
            !session.route_bytes(&[0x02, 0x02]),
            "prefix prefix still consumes"
        );
    }

    /// The status line carries the respawn hint beside the exit code —
    /// the cue that tells a user why their typing does nothing.
    #[test]
    fn status_line_names_the_respawn_chord_when_the_pane_is_dead() {
        let mut session = Session {
            conn: test_dead_conn(),
            socket_path: PathBuf::from("/nonexistent"),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: "work".to_string(),
            workspaces: vec![
                ("+0".to_string(), "main".to_string()),
                ("+1".to_string(), "lab".to_string()),
            ],
            active_workspace: Some("+0".to_string()),
            pane_title: "bash".to_string(),
            agents: 0,
            exited: Some(Some(7)),
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        let line = session.status_line();
        assert!(
            line.contains("(exited 7 — C-b r respawns)"),
            "the held-dead cue names the respawn chord: {line}"
        );
        session.exited = Some(None);
        assert!(
            session
                .status_line()
                .contains("(exited ? — C-b r respawns)"),
            "the unknown-code form carries the hint too"
        );
        session.exited = None;
        assert!(
            !session.status_line().contains("respawns"),
            "a live pane's line has no exited cue"
        );
    }

    /// The status draw's cursor placement tracks the shadow emulator: feed
    /// it a replay that parks the cursor at a known cell, then a %output
    /// The resync seeds the shadow with the daemon's screen-restore byte
    /// stream VERBATIM: the stream ends with the pane's real cursor CUP,
    /// so the shadow's tracked cursor is the pane's truth. A trailing
    /// newline appended after that CUP parks the shadow — and with it
    /// every status draw's absolute placement — one row below the prompt,
    /// and only the shell's next repaint papers over it (the manual-pass
    /// cursor bug).
    #[test]
    fn resync_feeds_the_shadow_the_restore_stream_verbatim() {
        let (_daemon, path) = FakeDaemon::bind("resync-cursor");
        let mut session = Session {
            conn: conn::AttachConn::connect(&path).expect("connect"),
            socket_path: path.clone(),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        session.resync();
        let cursor = session.emulator.terminal().cursor();
        assert_eq!(
            (cursor.col, cursor.row),
            (8, 1),
            "the shadow cursor must sit exactly where the restore stream's final CUP put it \
             (right after `prompt> `), not one row below"
        );
    }

    /// Every pane-show re-fits the target window BEFORE the replay: the
    /// handshake's target-less -C sizes only the newest session's active
    /// window, so a window restored at another size never resized and its
    /// child ran at the stale height (the manual-pass htop report; render
    /// mode's switch path has always reported). The report precedes the
    /// replay — the replay must encode the post-resize screen. Unix-only:
    /// the assert pins `terminal_grid`'s non-tty (80, 24) fallback (an
    /// interactive Windows console session reports its real grid —
    /// measured on the Windows VM, 2026-10-04).
    #[cfg(unix)]
    #[test]
    fn resync_size_reports_the_target_window_before_the_replay() {
        let (_daemon, path) = FakeDaemon::bind("resync-size");
        let mut session = Session {
            conn: conn::AttachConn::connect(&path).expect("connect"),
            socket_path: path.clone(),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        session.resync();
        // The connect-time handshake sends its own target-less size report
        // (refresh-client -C WxH -p); the resync's targeted report is the
        // first refresh-client naming the pane.
        let report = loop {
            match _daemon
                .received
                .recv_timeout(std::time::Duration::from_secs(5))
            {
                Ok((name, line)) if name == "refresh-client" && line.contains("-t %0") => {
                    break line;
                }
                Ok(_) => continue,
                Err(_) => panic!("no targeted size report arrived"),
            }
        };
        assert!(
            report.contains("-C 80x23"),
            "the size report names the pane at the content region: {report}"
        );
        let second = wait_for_line(&_daemon.received, "refresh-client");
        assert!(
            second.contains("-t %0") && !second.contains("-C"),
            "the replay follows without a size: {second}"
        );
    }

    /// chunk containing a CUP and a line feed; the bytes `draw_status`
    /// emits must end with an absolute CUP at the EMULATOR's tracked cell,
    /// not a bare ESC8 restore. A pane scroll landing between the draw and
    /// the placement cannot make a fresh absolute position wrong — that is
    /// the manual-pass race this kills (ESC7..ESC8 alone restored a
    /// position one line off after a scroll).
    #[test]
    fn status_draw_places_the_cursor_at_the_tracked_cell() {
        let (_daemon, path) = FakeDaemon::bind("tracked-cursor");
        let mut session = Session {
            conn: conn::AttachConn::connect(&path).expect("connect"),
            socket_path: path.clone(),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };

        // Replay that parks the cursor (CUP row 3 col 8), then a %output
        // chunk that homes, writes "line", and line feeds: the emulator
        // must end at col 4, row 1 (LF moves down, column preserved).
        session.emulator.feed(b"\x1b[3;8H");
        session.emulator.feed(b"\x1b[1;1Hline\n");
        let cursor = session.emulator.terminal().cursor();
        assert_eq!(
            (cursor.col, cursor.row),
            (4, 1),
            "the emulator's tracked cursor must follow the scripted stream"
        );

        // The draw's emission: the placement CUP is the LAST sequence in
        // the draw and names the tracked cell.
        let bytes = session.status_draw_bytes(24, 80);
        let expected = b"\x1b[2;5H"; // row+1 ; col+1, 1-indexed
        let pos = bytes
            .windows(expected.len())
            .rposition(|w| w == expected)
            .unwrap_or_else(|| {
                panic!(
                    "the draw must end with an absolute CUP at the tracked cell \
                     (row 4, col 8): {:?}",
                    String::from_utf8_lossy(&bytes)
                )
            });
        // It is the draw's LAST sequence: everything after it is nothing.
        assert_eq!(
            pos + expected.len(),
            bytes.len(),
            "the tracked-cell CUP must close the draw: {:?}",
            String::from_utf8_lossy(&bytes)
        );
        // The wrap survives (protects the draw against interleaved output
        // within itself) but the restore no longer closes it.
        assert!(
            bytes.windows(2).any(|w| w == b"\x1b7") && bytes.windows(2).any(|w| w == b"\x1b8"),
            "the ESC7/ESC8 wrap stays inside the draw: {:?}",
            String::from_utf8_lossy(&bytes)
        );
        // The DECSTBM reserve semantics are unchanged.
        assert!(
            bytes.windows(7).any(|w| w == b"\x1b[1;23r"),
            "the draw still reserves the content region (rows 1..23): {:?}",
            String::from_utf8_lossy(&bytes)
        );
    }

    /// Dead pane: the tracked cell is the frozen screen's cell — feeding
    /// the emulator the frozen stream is what `resync` does, so the same
    /// shape holds with `exited` set; the placement does not move to a
    /// default position when the pane dies.
    #[test]
    fn status_draw_tracks_the_frozen_cell_when_the_pane_is_dead() {
        let (_daemon, path) = FakeDaemon::bind("dead-tracked");
        let mut session = Session {
            conn: conn::AttachConn::connect(&path).expect("connect"),
            socket_path: path.clone(),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: Some(Some(0)),
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        session.emulator.feed(b"\x1b[5;3H");
        let bytes = session.status_draw_bytes(24, 80);
        let expected = b"\x1b[5;3H";
        let pos = bytes
            .windows(expected.len())
            .rposition(|w| w == expected)
            .unwrap_or_else(|| {
                panic!(
                    "the dead pane's placement CUP names the frozen cell: {:?}",
                    String::from_utf8_lossy(&bytes)
                )
            });
        assert_eq!(
            pos + expected.len(),
            bytes.len(),
            "the frozen-cell CUP closes the draw: {:?}",
            String::from_utf8_lossy(&bytes)
        );
    }

    /// Prefix r on a held-dead pane respawns it daemon-side and resumes
    /// forwarding; the plain bytes typed afterwards go to the pane.
    #[test]
    fn respawn_chord_resumes_forwarding_after_revival() {
        let (daemon, path) = FakeDaemon::bind("respawn");
        let mut session = Session {
            conn: conn::AttachConn::connect(&path).expect("connect"),
            socket_path: path.clone(),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: Some(Some(0)),
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        // The fake daemon answers every unknown command with an ok empty
        // block, so respawn-pane succeeds and clears the dead flag.
        assert!(
            !session.route_bytes(&[0x02, b'r']),
            "prefix r never detaches"
        );
        assert!(session.exited.is_none(), "a successful respawn clears it");
        // Forwarding resumed: the next plain byte is on the wire.
        assert!(!session.route_bytes(b"x"));
        // Handshake lines first (version, list-commands, set-client-colors,
        // refresh-client -C), then the chord and respawn's resync
        // (refresh-client -t) — the wire evidence forwarding is live again.
        let lines = recorded(&daemon.received, 6);
        assert_eq!(lines[4].0, "respawn-pane", "the chord respawned: {lines:?}");
        assert_eq!(
            lines[5].0, "refresh-client",
            "the respawn resync rode the revived connection: {lines:?}"
        );
    }

    /// The scripted-reply daemon the management-chord tests share: like
    /// [`serve_one`] but with a table covering split-window (reply = new
    /// pane id), list-panes under both windows, select-pane,
    /// new-window (reply = window id), and select-window.
    fn serve_management(
        stream: crate::mux::LocalStream,
        tx: std::sync::mpsc::Sender<(String, String)>,
    ) {
        use interprocess::TryClone as _;
        let mut writer = stream.try_clone().expect("clone stream");
        let mut reader = BufReader::new(stream);
        let mut number = 0u32;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
            let reply = match trimmed {
                "version" => "9.9.9+deadbeef".to_string(),
                "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
                "list-panes" => "%0\n".to_string(),
                // The focused pane %0 splits: the new pane %1 arrives.
                "split-window -t %0 -h" => "%1\n".to_string(),
                "split-window -t %0" => "%1\n".to_string(),
                // After the split lands the pump on %1, the window @0
                // holds both panes with %1 active (the daemon focuses the
                // fresh split).
                "list-panes -t @0" => "%0 0 -\n%1 1 *".to_string(),
                "select-pane -t %1" => String::new(),
                "select-pane -t %0" => String::new(),
                // new-window in session $0: the fresh @1.
                "new-window -t $0" => "@1\n".to_string(),
                "select-window -t @1" => String::new(),
                "list-windows -t $0" => "@0 0 -\n@1 1 *".to_string(),
                // The new window's active pane: %1 (a distinct id keeps
                // the landing assertion honest — %0 is @0's pane).
                "list-panes -t @1" => "%1 0 *".to_string(),
                _ => String::new(),
            };
            number += 1;
            tx.send((name, trimmed.to_owned())).ok();
            writer
                .write_all(emit_block(number, &reply, true).as_bytes())
                .ok();
            writer.flush().ok();
        }
    }

    /// Bind the management fake and build a Session over it, focused on
    /// %0 in window @0 of session $0.
    fn management_session(tag: &str) -> (std::sync::mpsc::Receiver<(String, String)>, Session) {
        let path = test_socket(tag);
        let _ = std::fs::remove_file(&path);
        let listener = crate::mux::bind_local_listener(&path).expect("bind");
        let (tx, rx) = channel();
        let sender = tx.clone();
        std::thread::spawn(move || {
            if let Ok(stream) = listener.accept() {
                serve_management(stream, sender);
            }
        });
        let conn = conn::AttachConn::connect(&path).expect("connect");
        drop(tx);
        let mut session = Session {
            conn,
            socket_path: path,
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: "@0".to_string(),
            session_id: Some("$0".to_string()),
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        // A status draw writes to the process stdout; in tests that is
        // the captured harness. Idle both flags so route_bytes's tail
        // has nothing to repaint and the wire sees only the chord.
        session.drawn_size = Some((80, 24));
        (rx, session)
    }

    /// prefix %: `split-window -t %0 -h` rides the wire, and the pump
    /// lands on the new pane — select-pane %1 plus the switch contract's
    /// refresh-client resync, with the fresh pane now the Session's
    /// target (subsequent typing forwards to it).
    #[test]
    fn split_right_chord_sends_split_window_and_lands_on_the_new_pane() {
        let (rx, mut session) = management_session("split-h");
        assert!(!session.route_bytes(&[0x02, b'%']));
        let lines = recorded(&rx, 6);
        let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "version",
                "list-commands",
                "set-client-colors",
                "refresh-client",
                "split-window",
                "select-pane",
            ],
            "the chord then the landing: {lines:?}"
        );
        assert_eq!(
            lines[4].1, "split-window -t %0 -h",
            "the split targets the FOCUSED pane with -h: {lines:?}"
        );
        assert_eq!(
            lines[5].1, "select-pane -t %1",
            "the pump followed the new pane from the reply: {lines:?}"
        );
        // Forwarding follows the landing: the switch contract's resync
        // burst (refresh-client, the status re-queries) rides first; the
        // next plain byte lands on the fresh pane %1.
        assert!(!session.route_bytes(b"q"));
        let keys = wait_for_line(&rx, "send-keys");
        assert_eq!(
            keys, "send-keys -t %1 -H 71",
            "typing forwards to the landed pane"
        );
    }

    /// prefix ": the vertical spelling (`split-window -t %0` with no -h)
    /// and the same land-on-the-new-pane contract.
    #[test]
    fn split_down_chord_sends_the_vertical_split_spelling() {
        let (rx, mut session) = management_session("split-v");
        assert!(!session.route_bytes(&[0x02, b'"']));
        let lines = recorded(&rx, 6);
        assert_eq!(
            lines[4].1, "split-window -t %0",
            "the default direction rides bare (below): {lines:?}"
        );
        assert_eq!(lines[5].1, "select-pane -t %1", "lands on the new pane");
    }

    /// prefix x: `kill-pane -t %0` rides the wire and the pump follows
    /// the window's surviving active pane.
    #[test]
    fn kill_chord_sends_kill_pane_and_lands_on_the_survivor() {
        let (rx, mut session) = management_session("kill");
        assert!(!session.route_bytes(&[0x02, b'x']));
        let lines = recorded(&rx, 7);
        let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "version",
                "list-commands",
                "set-client-colors",
                "refresh-client",
                "kill-pane",
                "list-panes",
                "select-pane",
            ],
            "kill, survivor lookup, landing: {lines:?}"
        );
        assert_eq!(lines[4].1, "kill-pane -t %0");
        assert_eq!(
            lines[5].1, "list-panes -t @0",
            "the survivor query targets the focused pane's window: {lines:?}"
        );
        assert_eq!(
            lines[6].1, "select-pane -t %1",
            "the *-marked survivor wins the landing: {lines:?}"
        );
    }

    /// prefix x on the window's LAST pane: no survivor — the dead guard
    /// engages (the exited cue shows; typing is dropped, prefix chords
    /// keep routing) instead of the pump forwarding into a corpse.
    #[test]
    fn kill_of_the_last_pane_engages_the_dead_guard() {
        let path = test_socket("kill-last");
        let _ = std::fs::remove_file(&path);
        let listener = crate::mux::bind_local_listener(&path).expect("bind");
        let (tx, rx) = channel();
        let sender = tx.clone();
        std::thread::spawn(move || {
            if let Ok(stream) = listener.accept() {
                use interprocess::TryClone as _;
                let mut writer = stream.try_clone().expect("clone stream");
                let mut reader = BufReader::new(stream);
                let mut number = 0u32;
                let mut line = String::new();
                let tx = sender;
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
                    let reply = match name.as_str() {
                        "version" => "9.9.9+deadbeef".to_string(),
                        "list-commands" => {
                            "list-commands\nfeatures replay-held-state\n".to_string()
                        }
                        "kill-pane -t %0" => String::new(),
                        _ => String::new(),
                    };
                    // The survivor query (list-panes -t @0) is the one
                    // command this script errors on: an %error block, no
                    // body, so `marked_pane` fails and the guard engages.
                    let ok = name != "list-panes";
                    if !ok {
                        number += 1;
                        tx.send((name.clone(), trimmed.to_owned())).ok();
                        writer
                            .write_all(
                                emit_block(number, "can't identify a pane", false).as_bytes(),
                            )
                            .ok();
                        writer.flush().ok();
                        continue;
                    }
                    number += 1;
                    tx.send((name, trimmed.to_owned())).ok();
                    writer
                        .write_all(emit_block(number, &reply, true).as_bytes())
                        .ok();
                    writer.flush().ok();
                }
            }
        });
        let conn = conn::AttachConn::connect(&path).expect("connect");
        drop(tx);
        let mut session = Session {
            conn,
            socket_path: path,
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: "@0".to_string(),
            session_id: Some("$0".to_string()),
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        session.drawn_size = Some((80, 24));
        assert!(!session.route_bytes(&[0x02, b'x']));
        assert!(
            session.exited.is_some(),
            "no survivor -> the dead guard engages"
        );
        // The dead guard drops typing but keeps prefix chords live.
        assert!(!session.route_bytes(b"typing"), "no detach, bytes dropped");
        assert!(
            session.route_bytes(&[0x02, b'd']),
            "prefix d still detaches"
        );
        drop(rx);
    }

    /// prefix c: `new-window -t $0` rides the wire, the fresh window is
    /// selected, and the pump attaches to its active pane — the same
    /// follow the window-switch chords make.
    #[test]
    fn new_window_chord_follows_the_new_windows_active_pane() {
        let (rx, mut session) = management_session("newwin");
        assert!(!session.route_bytes(&[0x02, b'c']));
        let lines = recorded(&rx, 7);
        let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "version",
                "list-commands",
                "set-client-colors",
                "refresh-client",
                "new-window",
                "select-window",
                "list-panes",
            ],
            "new-window, select, active-pane lookup: {lines:?}"
        );
        assert_eq!(lines[4].1, "new-window -t $0");
        assert_eq!(
            lines[5].1, "select-window -t @1",
            "the fresh window: {lines:?}"
        );
        // The active-pane lookup ran against the NEW window (its marked
        // pane %1 from the scripted table), and the Session's window
        // tracks it — the next switch_window list-windows query targets
        // @1 through session_id.
        assert_eq!(lines[6].1, "list-panes -t @1");
        assert_eq!(session.window, "@1");
        // And the pump landed on the new window's active pane.
        assert_eq!(session.pane, "%1");
    }

    /// Config: a chord override parses and routes — `%` remapped to `s`
    /// splits, and the default `%` no longer intercepts (it forwards as
    /// an ordinary byte through the dead-conn fallback).
    #[test]
    fn chord_override_remaps_the_split_chord() {
        let file: crate::mux::config::ConfigFile = toml::from_str(
            "[client]\nsplit-right = \"s\"\nsplit-down = \"v\"\nkill-pane = \"K\"\nnew-window = \"w\"\n",
        )
        .expect("parse");
        let (rx, mut session) = management_session("remap");
        let chords = crate::mux::config::reload_client_chords(
            &file,
            &crate::mux::config::Chords {
                prefix: session.prefix,
                reload: session.reload_key,
                management: session.management,
                ..crate::mux::config::Chords::with_defaults()
            },
        )
        .expect("chords parse");
        session.management = chords.management;
        assert!(
            !session.route_bytes(&[0x02, b's']),
            "the REMAPPED key splits"
        );
        let split = wait_for_line(&rx, "split-window");
        assert_eq!(
            split, "split-window -t %0 -h",
            "the remap targets %0 with -h"
        );
        // The landing burst ends with refresh_status's roster query;
        // after it drains, the wire is quiet.
        let _ = wait_for_line(&rx, "list-agents");
        // The OLD default key `%` no longer intercepts — and being
        // unbound now, the fixed table consumes it silently (the
        // unknown-chord rule: ignored), so nothing rides the wire.
        assert!(!session.route_bytes(&[0x02, b'%']));
        match rx.recv_timeout(std::time::Duration::from_millis(300)) {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            other => panic!("the old key must be consumed, not acted on: {other:?}"),
        }
    }

    /// prefix R (the resize chord) enters the sticky resize mode; each
    /// arrow sends the wire's relative resize-pane for the FOCUSED pane
    /// at the configured step; q exits and normal forwarding resumes.
    #[test]
    fn resize_chord_arrows_send_resize_pane_and_q_exits() {
        let (rx, mut session) = management_session("resize-chord");
        assert!(
            !session.route_bytes(&[0x02, b'R']),
            "the resize chord enters the mode"
        );
        assert!(session.resize_mode, "the mode is sticky");
        // Right arrow: the wire's relative form at the default step 1.
        assert!(!session.route_bytes(b"\x1b[C"));
        assert_eq!(
            wait_for_line(&rx, "resize-pane"),
            "resize-pane -t %0 -R 1",
            "the right arrow resizes the focused pane's right edge"
        );
        // Up arrow: -U at the same step.
        assert!(!session.route_bytes(b"\x1b[A"));
        assert_eq!(
            wait_for_line(&rx, "resize-pane"),
            "resize-pane -t %0 -U 1",
            "the up arrow is -U"
        );
        // q exits; typing forwards again afterward.
        assert!(!session.route_bytes(b"q"));
        assert!(!session.resize_mode);
        assert!(!session.route_bytes(b"z"));
        assert_eq!(
            wait_for_line(&rx, "send-keys"),
            "send-keys -t %0 -H 7a",
            "typing forwards after the mode exits"
        );
    }

    /// In resize mode, a non-arrow key exits the mode and is REPROCESSED
    /// by the normal router (the cancelling key still does its job — as
    /// plain typing, since chords always need the prefix).
    #[test]
    fn resize_mode_cancelling_key_reprocesses_normally() {
        let (rx, mut session) = management_session("resize-cancel");
        assert!(!session.route_bytes(&[0x02, b'R']));
        // 'x' exits and reprocesses: a plain byte forwards to the pane.
        assert!(!session.route_bytes(b"x"));
        assert!(!session.resize_mode);
        assert_eq!(
            wait_for_line(&rx, "send-keys"),
            "send-keys -t %0 -H 78",
            "the cancelling key routed through the normal router"
        );
    }

    /// Config overrides: a resize-step of 3 rides each arrow, and a
    /// remapped resize chord (Z) enters the mode while the old default
    /// key is consumed unbound.
    #[test]
    fn resize_chord_and_step_follow_the_config() {
        let file: crate::mux::config::ConfigFile =
            toml::from_str("[client]\nresize = \"Z\"\nresize-step = 3\n").expect("parse");
        let (rx, mut session) = management_session("resize-cfg");
        let chords = crate::mux::config::reload_client_chords(
            &file,
            &crate::mux::config::Chords {
                prefix: session.prefix,
                reload: session.reload_key,
                management: session.management,
                ..crate::mux::config::Chords::with_defaults()
            },
        )
        .expect("chords parse");
        session.management = chords.management;
        session.resize_step = chords.resize_step;
        assert!(!session.route_bytes(&[0x02, b'Z']));
        assert!(session.resize_mode, "the REMAPPED key enters the mode");
        assert!(!session.route_bytes(b"\x1b[C"));
        assert_eq!(
            wait_for_line(&rx, "resize-pane"),
            "resize-pane -t %0 -R 3",
            "the configured step rides the arrow"
        );
        // The old default key R is consumed unbound (nothing rides the
        // wire); drain the burst first, then require quiet.
        while rx
            .recv_timeout(std::time::Duration::from_millis(150))
            .is_ok()
        {}
        assert!(!session.route_bytes(&[0x02, b'R']));
        match rx.recv_timeout(std::time::Duration::from_millis(300)) {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            other => panic!("the old key must be consumed, not acted on: {other:?}"),
        }
    }

    /// prefix {: swap with the layout-order neighbor — `swap-pane -s
    /// <focused> -t <prev>` — from the same list-panes roster cycle_pane
    /// walks.
    #[test]
    fn swap_prev_chord_sends_swap_pane() {
        let (rx, mut session) = management_session("swap-prev");
        assert!(!session.route_bytes(&[0x02, b'{']));
        let line = wait_for_line(&rx, "swap-pane");
        assert_eq!(
            line, "swap-pane -s %0 -t %1",
            "focused %0 swaps with its next-roster neighbor (wraps to %1)"
        );
    }

    /// prefix ?: the bindings panel prints as plain text with the LIVE
    /// chords — a remapped split chord shows its remapped key, and the
    /// category headers are present.
    #[test]
    fn help_rows_carry_effective_bindings_and_categories() {
        use crate::mux::config::Management;
        let rows = super::help_rows(
            0x02,
            0x12,
            Management {
                split_right: b's',
                ..Management::default()
            },
            2,
        );
        let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        let joined = text.join("\n");
        for category in [
            "global",
            "panes",
            "tabs / windows / sessions",
            "navigation",
            "mouse",
        ] {
            assert!(
                text.iter().any(|t| t.contains(category)),
                "the category header {category} is present: {joined}"
            );
        }
        // Effective bindings: the remapped split shows s, not %.
        assert!(
            text.iter()
                .any(|t| t.starts_with(" C-b s") && t.contains("split right")),
            "the remapped chord shows its remapped key: {joined}"
        );
        assert!(
            !text.iter().any(|t| t.contains("C-b %")),
            "the old default spelling is gone: {joined}"
        );
        // The resize step surfaces in the resize row.
        assert!(
            text.iter().any(|t| t.contains("edge moves by 2")),
            "the step reflects the live config: {joined}"
        );
    }

    /// The passthrough dump leads with a blank line (the first header used
    /// to print on the cursor's current line — the prompt's) and bolds the
    /// category headers; plain rows stay unstyled.
    #[test]
    fn passthrough_help_dump_leads_with_a_blank_line_and_bolds_headers() {
        let rows = super::help_rows(0x02, 0x12, Default::default(), 1);
        let dump = super::help_dump_text(&rows);
        assert!(
            dump.starts_with("\r\n"),
            "the dump must start on a fresh line: {dump:?}"
        );
        let accent_count = rows.iter().filter(|r| r.accent).count();
        assert_eq!(
            dump.matches("\x1b[1m").count(),
            accent_count,
            "every category header is bold, nothing else: {dump:?}"
        );
        assert!(
            dump.contains("\x1b[1m global \x1b[0m\r\n"),
            "the global header is bold: {dump:?}"
        );
        // The first entry row is plain: no SGR anywhere outside headers.
        assert!(
            dump.contains("\r\n C-b C-b  type a literal prefix\r\n"),
            "entry rows stay plain: {dump:?}"
        );
    }

    /// The passthrough help chord advances the pane past the dump — empty
    /// Enters (one command line + fresh prompt each, ~2 rows) land the
    /// shell below the text so later keystrokes do not paint over it.
    #[test]
    fn help_chord_advances_the_pane_past_the_dump() {
        let (_daemon, path) = FakeDaemon::bind("help-advance");
        let mut session = Session {
            conn: conn::AttachConn::connect(&path).expect("connect"),
            socket_path: path.clone(),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        let expected = super::help_rows(0x02, 0x12, Default::default(), 1).len() / 2 + 1;
        session.show_help();
        let line = wait_for_line(&_daemon.received, "send-keys");
        let sent = line.matches("0d").count();
        assert_eq!(
            sent, expected,
            "one Enter per ~2 dump rows advances the pane past the help: {line}"
        );
    }

    /// The sidebar compose after round 6: NO section header rows (the
    /// title moved to the tab strip's lead segment), workspace rows
    /// start at composed row 0, and the footer row pins ` new ` to the
    /// panel's bottom-left and ` menu ` to its bottom-right — each chip
    /// its own clickable span and id.
    #[test]
    fn compose_sidebar_lists_rows_and_pins_the_footer_chips() {
        let sections = vec![super::SidebarSection {
            rows: vec![
                ("+0".to_string(), "main".to_string(), true),
                ("+1".to_string(), "dev".to_string(), false),
                (
                    "+2".to_string(),
                    "a very long workspace name".to_string(),
                    false,
                ),
            ],
        }];
        let lines = super::compose_sidebar(&sections, 20, 12);
        // Row 0 is the first workspace — no header row above it.
        assert_eq!(lines[0].text, " ▸ main");
        assert_eq!(lines[0].id.as_deref(), Some("+0"));
        assert!(lines[0].active, "the active row is flagged");
        assert_eq!(lines[1].text, "   dev");
        assert_eq!(lines[1].id.as_deref(), Some("+1"));
        assert_eq!(lines[2].text, "   a very long work", "clipped to width - 1");
        // The footer chips pin to the panel's LAST row (footer_y = 11 of
        // 12 rows), never at a body row's position.
        let new_chip = lines
            .iter()
            .find(|l| l.id.as_deref() == Some(super::SIDEBAR_NEW_ID))
            .expect("the new chip composes");
        assert_eq!(new_chip.y, 11);
        assert_eq!(new_chip.text, " new ");
        assert_eq!((new_chip.x, new_chip.x_end), (0, 5));
        let menu_chip = lines
            .iter()
            .find(|l| l.id.as_deref() == Some(super::SIDEBAR_MENU_ID))
            .expect("the menu chip composes");
        assert_eq!(menu_chip.text, " menu ");
        assert_eq!(
            (menu_chip.x, menu_chip.x_end),
            (13, 19),
            "bottom-right of the 19-col content"
        );
        // A strip too narrow for both chips drops the one that does not
        // fit; a strip too narrow for either drops both.
        let narrow = super::compose_sidebar(&sections, 7, 4);
        assert!(
            narrow
                .iter()
                .any(|l| l.id.as_deref() == Some(super::SIDEBAR_NEW_ID)),
            "the new chip fits a 6-col content: {narrow:?}"
        );
        assert!(
            !narrow
                .iter()
                .any(|l| l.id.as_deref() == Some(super::SIDEBAR_MENU_ID)),
            "the menu chip does not fit: {narrow:?}"
        );
        let tiny = super::compose_sidebar(&sections, 4, 3);
        assert!(
            !tiny
                .iter()
                .any(|l| l.id.as_deref().is_some_and(|id| id.starts_with("panel:"))),
            "no chips fit a 3-col content: {tiny:?}"
        );
    }

    /// The new-workspace prompt's default bumps past conflicts: one past
    /// the highest workspace ordinal, and past any workspace NAME that
    /// already claims the number (the same rule next_window_name runs).
    #[test]
    fn next_workspace_name_bumps_past_conflicts() {
        assert_eq!(super::next_workspace_name(&[]), "1");
        let roster = vec![
            ("+1".to_string(), "1".to_string()),
            ("+2".to_string(), "3".to_string()),
        ];
        assert_eq!(
            super::next_workspace_name(&roster),
            "4",
            "one past the max ordinal, bumped past the claimed name '3'"
        );
        let claimed = vec![("+1".to_string(), "2".to_string())];
        assert_eq!(
            super::next_workspace_name(&claimed),
            "3",
            "a claimed default bumps again"
        );
    }

    /// The help panel compose: the filter line is ALWAYS present (the
    /// placeholder when inactive — the round-3 no-feedback fix), the
    /// filter narrows rows live (headers hide when nothing beneath them
    /// matches), the window scrolls, and the footer line names the
    /// controls. The border ring/title/badge are the renderer's
    /// paint_overlay job, not the compose's.
    #[test]
    fn compose_help_panel_filters_scrolls_and_chromes() {
        let rows = super::help_rows(0x02, 0x12, Default::default(), 1);
        // Full panel: content + footer — no idle placeholder (the footer
        // already advertises `search /`), and entering filter mode swaps
        // in the cursor-input line immediately — `/` must give visible
        // feedback before any typing.
        let panel = super::compose_help_panel(&rows, "", false, 100, 0);
        let text: Vec<&str> = panel.iter().map(|r| r.text.as_str()).collect();
        assert!(
            !text.iter().any(|t| t.contains("press / to filter")),
            "no idle placeholder row: {text:?}"
        );
        assert_eq!(
            text[0], " global ",
            "the content leads the idle panel: {}",
            text[0]
        );
        let active = super::compose_help_panel(&rows, "", true, 100, 0);
        assert_eq!(
            active[0].text, " /▌",
            "the active filter line shows the input cursor: {}",
            active[0].text
        );
        assert!(
            text.last()
                .copied()
                .unwrap_or("")
                .contains("close esc/enter"),
            "the footer names the controls"
        );
        // Filter: "swap" keeps only the swap rows (and their header).
        let filtered = super::compose_help_panel(&rows, "swap", false, 100, 0);
        let ftext: String = filtered
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(ftext.contains("swap"), "the matching rows survive: {ftext}");
        assert!(
            ftext.contains("panes"),
            "the matching rows' header survives"
        );
        assert!(!ftext.contains("detach"), "non-matching rows drop: {ftext}");
        assert!(
            !ftext.contains(" global "),
            "a category with no matching rows hides: {ftext}"
        );
        // Filter box drawn while active.
        assert!(
            ftext.contains("/swap▌"),
            "the filter box shows the live filter: {ftext}"
        );
        // Scroll: a tiny window shows a slice, and scrolling past the end
        // clamps.
        let small = super::compose_help_panel(&rows, "", false, 3, 0);
        let big = super::compose_help_panel(&rows, "", false, 3, 10_000);
        assert_ne!(small[1..4], big[1..4], "scrolling moves the window");
        assert_eq!(
            super::compose_help_panel(&rows, "", false, 3, 10_000).len(),
            small.len(),
            "the panel shape is stable under scroll"
        );
    }

    /// The picker's rows: one accent header per session with its windows
    /// nested beneath, the current session and the shown window
    /// `>`-marked, the session's active window `*`-suffixed, and the
    /// parallel refs mapping each row to its selection target.
    #[test]
    fn picker_rows_nest_windows_and_mark_current() {
        let entries = vec![
            super::PickerEntry {
                session_id: "$0".to_string(),
                session_name: "work".to_string(),
                windows: vec![
                    ("@0".to_string(), "main".to_string()),
                    ("@1".to_string(), "vim".to_string()),
                ],
                active_window: Some("@1".to_string()),
                current: true,
            },
            super::PickerEntry {
                session_id: "$1".to_string(),
                session_name: "build".to_string(),
                windows: vec![("@2".to_string(), "logs".to_string())],
                active_window: Some("@2".to_string()),
                current: false,
            },
        ];
        let (rows, refs) = super::picker_rows(&entries, Some("@0"));
        let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text.len(), 5, "two headers + three windows: {text:?}");
        assert_eq!(
            text[0], " >$0: work",
            "the current session's header is >-marked"
        );
        assert!(rows[0].accent, "session headers are accent rows");
        assert_eq!(text[1], " >  @0: main", "the shown window is >-marked");
        assert_eq!(text[2], "    @1: vim *", "the active window carries *");
        assert_eq!(text[3], "  $1: build", "a non-current header is unmarked");
        assert!(rows[3].accent, "every session header is an accent row");
        assert!(!rows[1].accent, "window rows are not accent rows");
        assert_eq!(refs[0], super::PickerRef::Session(0));
        assert_eq!(refs[1], super::PickerRef::Window(0, 0));
        assert_eq!(refs[2], super::PickerRef::Window(0, 1));
        assert_eq!(refs[3], super::PickerRef::Session(1));
        assert_eq!(refs[4], super::PickerRef::Window(1, 0));
    }

    /// The picker compose: the filter line, the filtered content with
    /// headers hiding when nothing beneath them matches, the `▸` cursor
    /// on the selected row, the footer, and the refs/start return that
    /// maps a click at composed row `i` to content row `start + i - 1`.
    #[test]
    fn compose_picker_panel_filters_marks_and_maps_clicks() {
        let entries = vec![
            super::PickerEntry {
                session_id: "$0".to_string(),
                session_name: "work".to_string(),
                windows: vec![("@0".to_string(), "main".to_string())],
                active_window: Some("@0".to_string()),
                current: true,
            },
            super::PickerEntry {
                session_id: "$1".to_string(),
                session_name: "build".to_string(),
                windows: vec![("@1".to_string(), "vim".to_string())],
                active_window: Some("@1".to_string()),
                current: false,
            },
        ];
        let (rows, refs) = super::picker_rows(&entries, Some("@0"));
        // Full panel: 4 content rows + footer — no idle placeholder (the
        // footer already advertises `filter /`).
        let (panel, refs2, start) = super::compose_picker_panel(&rows, &refs, "", false, 0, 100, 0);
        assert_eq!(panel.len(), 5, "content + footer: {panel:?}");
        assert_eq!(start, 0);
        assert_eq!(refs2.len(), 4);
        assert_eq!(
            panel.last().map(|r| r.text.as_str()),
            Some(super::PICKER_FOOTER)
        );
        // Filter "vim" keeps only that window row and its header; the
        // other session's header hides with no matching children.
        let (filtered, frefs, fstart) =
            super::compose_picker_panel(&rows, &refs, "vim", false, 0, 100, 0);
        let ftext = filtered
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(ftext.contains("vim"), "the matching row survives: {ftext}");
        assert!(
            ftext.contains("build"),
            "the matching row's header survives"
        );
        assert!(
            !ftext.contains("work"),
            "an emptied session's header hides: {ftext}"
        );
        assert_eq!(
            frefs,
            vec![super::PickerRef::Session(1), super::PickerRef::Window(1, 0)]
        );
        assert_eq!(fstart, 0);
        // The selection cursor: selected=2 marks the third content row.
        let (marked, _, _) = super::compose_picker_panel(&rows, &refs, "", false, 2, 100, 0);
        assert!(
            marked[2].text.starts_with('▸'),
            "the selected row carries the cursor glyph: {}",
            marked[2].text
        );
        assert!(
            !marked[1].text.starts_with('▸'),
            "only the selected row carries it"
        );
        // Click mapping: composed row 1 (content row 1) maps to content
        // index start + 1.
        let (small, _, small_start) = super::compose_picker_panel(&rows, &refs, "", false, 0, 2, 0);
        assert_eq!(small.len(), 3, "2 visible + footer");
        assert_eq!(small_start, 0);
        // Panning: a selected row below the window moves `start`.
        let (_panned, _, panned_start) =
            super::compose_picker_panel(&rows, &refs, "", false, 3, 2, 0);
        assert_eq!(
            panned_start, 2,
            "the window pans to keep the cursor visible"
        );
    }

    /// The listbox panning rule: `start` moves only when `selected`
    /// leaves the visible window.
    #[test]
    fn listbox_scroll_pans_only_at_the_edges() {
        assert_eq!(super::listbox_scroll(0, 0, 3), 0);
        assert_eq!(super::listbox_scroll(0, 2, 3), 0, "inside the window");
        assert_eq!(super::listbox_scroll(0, 3, 3), 1, "one past the end");
        assert_eq!(super::listbox_scroll(1, 0, 3), 0, "above the window");
        assert_eq!(super::listbox_scroll(0, 0, 0), 0, "an empty window is safe");
    }

    /// A management chord spelling that does not parse is a reload error
    /// (the flash), not a silent keep-the-old.
    #[test]
    fn management_chord_override_errors_on_malformed_spelling() {
        let file: crate::mux::config::ConfigFile =
            toml::from_str("[client]\nsplit-right = \"C-\"\n").expect("parse");
        assert!(
            crate::mux::config::reload_client_chords(
                &file,
                &crate::mux::config::Chords {
                    prefix: 0x02,
                    reload: 0x12,
                    management: crate::mux::config::Management::default(),
                    resize_step: 1,
                    ..crate::mux::config::Chords::with_defaults()
                },
            )
            .is_err(),
            "a malformed chord is an error"
        );
    }

    /// A Session for routing tests only: its connection is a closed
    /// channel pair, so sends fail silently (routing logic does not care).
    fn test_dead_conn() -> conn::AttachConn {
        let path = test_socket("deadconn");
        let _ = std::fs::remove_file(&path);
        // connect() fails without a listener, which is exactly the "dead"
        // connection a routing test wants — but Session needs a value.
        // Build one over a listener the test immediately drops: every
        // send fails, which is the behavior route_bytes tolerates.
        match conn::AttachConn::connect(&path) {
            Ok(conn) => conn,
            Err(_) => {
                // Fall back: bind-accept-drop. One bound listener that
                // answers nothing, then closes.
                let listener = crate::mux::bind_local_listener(&path).expect("bind");
                std::thread::spawn(move || {
                    // The Listener trait supplies `.accept()` on unix only;
                    // the Windows named-pipe listener accepts inherently.
                    #[cfg(unix)]
                    use interprocess::local_socket::traits::Listener as _;
                    let _ = listener.accept();
                });
                conn::AttachConn::connect(&path).expect("connect to the accepting listener")
            }
        }
    }

    /// The reload chord: a written config with a new prefix rebinds the
    /// running session's chords LIVE — the prefix typed after the reload
    /// detaches under the NEW prefix, the old one forwards — and the
    /// `reload-config` control command rides the same chord to the
    /// daemon.
    #[test]
    fn reload_chord_rebinds_prefix_and_sends_reload_config() {
        let (_daemon, path) = FakeDaemon::bind("reload");
        let mut session = Session {
            conn: conn::AttachConn::connect(&path).expect("connect"),
            socket_path: path.clone(),
            pane: "%0".to_string(),
            emulator: render::PaneEmulator::new(0, 80, 24),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            workspaces: Vec::new(),
            active_workspace: None,
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
            reload_key: 0x12,
            management: crate::mux::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            flash: None,
        };
        // A config naming a new prefix (C-a) and a moved reload chord
        // (C-a C-s): the rebind is pure over the parsed file, so the test
        // feeds it directly (the env-pin path is covered end to end by
        // the PTY-level attach tests, where the child's env is injectable
        // — QA-196 forbids env mutation in this suite).
        let file: crate::mux::config::ConfigFile =
            toml::from_str("[client]\nprefix = \"C-a\"\nreload = \"C-a C-s\"\n").unwrap();
        let new_chords = crate::mux::config::reload_client_chords(
            &file,
            &crate::mux::config::Chords {
                prefix: session.prefix,
                reload: session.reload_key,
                management: session.management,
                ..crate::mux::config::Chords::with_defaults()
            },
        )
        .expect("chords parse");
        session.prefix = new_chords.prefix;
        session.reload_key = new_chords.reload;
        session.management = new_chords.management;
        // The reloaded session routes: prefix d (0x01 'd') detaches under
        // the NEW prefix; the old prefix byte no longer intercepts.
        assert!(
            session.route_bytes(&[0x01, b'd']),
            "the NEW prefix detaches"
        );
        assert!(
            !session.route_bytes(&[0x02, b'x']),
            "the OLD prefix no longer intercepts"
        );
    }

    /// The pure chord rebind errors on a malformed chord (the error is
    /// what the status flash shows) instead of silently keeping the old
    /// chords.
    #[test]
    fn reload_client_chords_errors_on_malformed_chords() {
        let file: crate::mux::config::ConfigFile =
            toml::from_str("[client]\nprefix = \"C-\"\n").unwrap();
        assert!(
            crate::mux::config::reload_client_chords(
                &file,
                &crate::mux::config::Chords {
                    prefix: 0x02,
                    reload: 0x12,
                    management: crate::mux::config::Management::default(),
                    ..crate::mux::config::Chords::with_defaults()
                }
            )
            .is_err(),
            "a malformed prefix is an error"
        );
        // An absent tier keeps the current chords.
        let partial: crate::mux::config::ConfigFile =
            toml::from_str("[daemon]\nsocket = \"work\"\n").unwrap();
        let kept = crate::mux::config::reload_client_chords(
            &partial,
            &crate::mux::config::Chords {
                prefix: 0x02,
                reload: 0x12,
                management: crate::mux::config::Management {
                    split_right: b'&',
                    split_down: b'*',
                    kill_pane: b'X',
                    new_window: b'C',
                    resize: b'R',
                    swap_prev: b'{',
                    swap_next: b'}',
                    workspace_next: b'N',
                    workspace_prev: b'P',
                    help: b'?',
                    picker: b'w',
                    zoom: b'z',
                    rename_window: b',',
                    rename_pane: b'$',
                    border_cycle: b'B',
                    label_toggle: b'l',
                    workspace_picker: b'g',
                    sidebar: b's',
                    status_bar: b'S',
                },
                resize_step: 1,
                ..crate::mux::config::Chords::with_defaults()
            },
        )
        .expect("partial file");
        assert_eq!(
            kept,
            crate::mux::config::Chords {
                prefix: 0x02,
                reload: 0x12,
                management: crate::mux::config::Management {
                    split_right: b'&',
                    split_down: b'*',
                    kill_pane: b'X',
                    new_window: b'C',
                    resize: b'R',
                    swap_prev: b'{',
                    swap_next: b'}',
                    workspace_next: b'N',
                    workspace_prev: b'P',
                    help: b'?',
                    picker: b'w',
                    zoom: b'z',
                    rename_window: b',',
                    rename_pane: b'$',
                    border_cycle: b'B',
                    label_toggle: b'l',
                    workspace_picker: b'g',
                    sidebar: b's',
                    status_bar: b'S',
                },
                resize_step: 1,
                ..crate::mux::config::Chords::with_defaults()
            }
        );
    }
}
