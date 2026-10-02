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

use crate::mux::resolve_socket_path;
use crate::tmux_control::TmuxNotification;
use std::io::Write as _;
use std::process::ExitCode;
use std::time::Duration;

/// Which rendering pipeline the attach client drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttachMode {
    /// The Phase A contract: pane bytes flow to the host terminal
    /// verbatim, which is the VT emulator. The default — the renderer is
    /// opt-in until later Phase B cards land the full TUI chrome.
    #[default]
    Passthrough,
    /// The Phase B renderer: per-pane core emulators fed by replay +
    /// `%output`, painted into layout rects through ratatui, dividers
    /// drawn between them. The host terminal only ever sees this client's
    /// own draws.
    Render,
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
    /// Render pipeline selection (`--mode`): [`AttachMode::Passthrough`]
    /// (the Phase A contract) is the default; [`AttachMode::Render`]
    /// selects the pane renderer + input router.
    pub mode: AttachMode,
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

    let session = match Session::new(conn, &path, options.target.as_deref(), prefix) {
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
/// literal single character into its byte.
fn parse_prefix(spec: &str) -> Option<u8> {
    let lower = spec.to_ascii_lowercase();
    if let Some(key) = lower.strip_prefix("c-") {
        return match key {
            "space" => Some(0x00),
            letter if letter.len() == 1 && letter.as_bytes()[0].is_ascii_lowercase() => {
                Some(letter.as_bytes()[0] - b'a' + 1)
            }
            _ => None,
        };
    }
    let bytes = spec.as_bytes();
    if bytes.len() == 1 {
        Some(bytes[0])
    } else {
        None
    }
}

/// One `list-sessions` reply line as `(session_id, name)`. The wire shape
/// is `$N: name` (dispatch.rs), so the id ends at the colon — a plain
/// whitespace split keeps it (`$0:`), which the daemon's id parser then
/// rejects. The window roster's `@N: name` lines parse the same way.
pub(crate) fn parse_session_line(line: &str) -> Option<(String, String)> {
    let (id_part, name) = line.split_once(": ")?;
    let id = id_part.split_whitespace().next()?;
    Some((id.to_string(), name.to_string()))
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
fn marked_pane(body: &[String], target: &str) -> Result<String, String> {
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
    /// The owning window, `@N` (pane-info's second field).
    window: String,
    /// The owning session's id, `$N` (resolved from the window scan).
    session_id: Option<String>,
    /// The owning session's name (status line).
    session_name: String,
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
}

impl Session {
    /// Resolve the target, resync its screen to stdout, and seed the
    /// status state.
    fn new(
        mut conn: conn::AttachConn,
        socket_path: &std::path::Path,
        target: Option<&str>,
        prefix: u8,
    ) -> Result<Self, String> {
        // Registration replay (held panes' %pane-exited, zoomed windows'
        // %layout-change) is state about OTHER panes mostly; the resync
        // below is the target's whole picture in Phase A. Drained here.
        let _replay = conn.drain_pending_events();

        let pane = resolve_target(&mut conn, target)?;
        let mut session = Self {
            conn,
            socket_path: socket_path.to_path_buf(),
            pane,
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: true,
            prefix,
            prefix_pending: false,
        };
        session.resync();
        session.refresh_status();
        session.draw_status();
        Ok(session)
    }

    /// Resync the target pane's screen: the replay body goes to stdout
    /// verbatim. The reply framing is line-based, and the body's own
    /// escape sequences ride those lines intact, so writing each line plus
    /// its newline reproduces the daemon's screen-restore byte stream.
    fn resync(&mut self) {
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
        let mut stdout = std::io::stdout().lock();
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
            //    ConPTY's geometry settling late).
            if status_dirty || self.size_changed() {
                status_dirty = false;
                self.refresh_status();
                self.draw_status();
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
        let mut line = format!(" {session} | {title}");
        if self.agents > 0 {
            line.push_str(&format!(" | {} agent(s)", self.agents));
        }
        if let Some(code) = self.exited {
            match code {
                Some(code) => line.push_str(&format!(" | (exited {code} — C-b r respawns)")),
                None => line.push_str(" | (exited ? — C-b r respawns)"),
            }
        }
        line
    }

    /// The status line: reserve the bottom row with DECSTBM, draw
    /// `session | title [| N agents] [| (exited N — C-b r respawns)]`
    /// inverse-video, and restore the scroll region and cursor.
    fn draw_status(&mut self) {
        let (cols, rows) = conn::terminal_grid();
        if rows < 2 || cols < 2 {
            return; // nowhere to put a status row
        }
        let bottom = rows; // DECSTBM rows are 1-indexed inclusive
        let line = self.status_line();
        let width = (cols as usize).saturating_sub(1);
        let mut text: String = line.chars().take(width).collect();
        let used = text.chars().count();
        if used < width {
            text.push_str(&" ".repeat(width - used));
        }
        // DECSTBM reserves the bottom row: set the region FIRST (rows
        // 1..=bottom-1 scroll; the status row stays fixed), draw on it,
        // then restore the full-screen region and the cursor position.
        // ESC 7 / ESC 8 wrap everything so pane output lands with the
        // cursor exactly where the pane left it.
        let scroll_region_bottom = rows - 1;
        let mut stdout = std::io::stdout().lock();
        let _ = write!(
            stdout,
            "\x1b7\x1b[1;{scroll_region_bottom}r\x1b[{bottom};1H\x1b[7m{text}\x1b[0m\x1b[1;{bottom}r\x1b8"
        );
        let _ = stdout.flush();
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
        let (tx, rx) = std::sync::mpsc::sync_channel::<std::io::Result<Vec<u8>>>(64);
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
        // The alternate screen: the replay must not overwrite whatever the
        // host terminal was showing before attach. Entered right after raw
        // mode (only when it entered — the alt-screen write is worthless on
        // a non-tty) and left before raw mode is dropped, so every exit
        // path (detach, %exit, socket close, error, panic unwind) restores
        // the host's screen. Render mode enters its own alt screen for its
        // mouse-capture pairing; LeaveAlternateScreen is idempotent, so the
        // guard's leave after the session's own is harmless.
        if entered {
            let _ =
                crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen);
        }
        Self { entered }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.entered {
            let _ =
                crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
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
            mode: AttachMode::default(),
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
            mode: AttachMode::default(),
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
            mode: AttachMode::default(),
        };
        assert_eq!(explicit.socket_path(), PathBuf::from("/tmp/explicit.sock"));
        let named = AttachOptions {
            socket: None,
            name: Some("work".to_string()),
            target: None,
            prefix: None,
            mode: AttachMode::default(),
        };
        assert_eq!(named.socket_path(), crate::mux::default_socket_path("work"));
        let fallback = AttachOptions {
            socket: None,
            name: None,
            target: None,
            prefix: None,
            mode: AttachMode::default(),
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
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            pane_title: String::new(),
            agents: 0,
            exited: None,
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
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
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            pane_title: String::new(),
            agents: 0,
            exited: Some(Some(0)),
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
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
            window: String::new(),
            session_id: None,
            session_name: "work".to_string(),
            pane_title: "bash".to_string(),
            agents: 0,
            exited: Some(Some(7)),
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
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

    /// Prefix r on a held-dead pane respawns it daemon-side and resumes
    /// forwarding; the plain bytes typed afterwards go to the pane.
    #[test]
    fn respawn_chord_resumes_forwarding_after_revival() {
        let (daemon, path) = FakeDaemon::bind("respawn");
        let mut session = Session {
            conn: conn::AttachConn::connect(&path).expect("connect"),
            socket_path: path.clone(),
            pane: "%0".to_string(),
            window: String::new(),
            session_id: None,
            session_name: String::new(),
            pane_title: String::new(),
            agents: 0,
            exited: Some(Some(0)),
            drawn_size: None,
            settling: false,
            prefix: 0x02,
            prefix_pending: false,
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
}
