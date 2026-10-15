//! The Phase B attach pane renderer: per-pane core emulators mapped onto a
//! ratatui [`Buffer`], damage-diffed at frame cadence.
//!
//! One [`par_term_emu_core::terminal::Terminal`] instance per visible pane is fed the
//! pane's `refresh-client` replay and `%output` bytes — the same bytes a
//! passthrough client forwards to the host terminal — so each pane's grid
//! is exactly what the daemon's pane emulator holds (capture-pane ground
//! truth). [`PaneRenderer::render_frame`] paints every pane's grid into
//! its layout rect of a full-window ratatui `Buffer`, draws the dividers
//! on top, and returns the cell diff against the previous frame — the
//! caller flushes only those cells, and a whole output flood between two
//! frames collapses into one diff.

use crate::mux::attach::input::{InputParser, Token};
use crate::mux::attach::layout::PaneRect;
use crate::mux::attach::status;
use crate::mux::attach::targets::session_active_window;
use crate::mux::attach::{layout, HelpRow};
use crate::mux::layout::PaneChrome;
use par_term_emu_core::cursor::CursorStyle;
use par_term_emu_core::mouse::MouseMode;
use par_term_emu_core::terminal::Terminal;
use par_term_emu_core::tmux_control::TmuxNotification;
use ratatui::buffer::{Buffer, Cell as RtCell};
use ratatui::style::Color as RtColor;
use std::collections::HashMap;
use std::io::Write as _;
use std::sync::mpsc::RecvTimeoutError;

mod chrome;
mod cursor;
mod drag;
mod geometry;
mod input_route;
mod modal;
mod modal_help;
mod modal_menu;
mod modal_picker;
mod modal_prompt;
mod modal_settings;
mod navigate;
mod notify;
mod paint;
mod pane_chrome;
mod pending_cmd;
mod renderer;
mod session;
mod sidebar;
mod sidebar_state;
mod sink;
mod status_patch;
mod toast;

use chrome::ChromeRows;
use cursor::CursorState;
use sidebar_state::SidebarState;

use renderer::{RenderOptions, StdoutSink};

/// How long a status-row flash stays up.
const FLASH_LIFETIME: std::time::Duration = std::time::Duration::from_secs(1);

/// The glyph set dividers draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    /// UTF-8 box drawing (`│ ─ ┼`): the default for any terminal that
    /// accepts UTF-8.
    Unicode,
    /// UTF-8 double-line box drawing (`║ ═ ╬`, corners ╔ ╗ ╚ ╝).
    Double,
    /// UTF-8 heavy-line box drawing (`┃ ━ ╋`, corners ┏ ┓ ┗ ┛).
    Heavy,
    /// VT100 ACS spelling (`| - +`): the fallback for charset-hostile
    /// terminals.
    Ascii,
    /// herdr's look: UTF-8 rounded box drawing (`╭ ╮ ╰ ╯`) with every
    /// pane drawing its own complete box (not the shared dividers).
    Herdr,
}

impl Glyphs {
    /// The config spelling of the set (the `border-lines` value it
    /// round-trips as).
    fn name(self) -> &'static str {
        match self {
            Glyphs::Unicode => "unicode",
            Glyphs::Double => "double",
            Glyphs::Heavy => "heavy",
            Glyphs::Ascii => "ascii",
            Glyphs::Herdr => "herdr",
        }
    }

    /// The next set in the border-cycle chord's rotation.
    fn next(self) -> Glyphs {
        match self {
            Glyphs::Unicode => Glyphs::Double,
            Glyphs::Double => Glyphs::Heavy,
            Glyphs::Heavy => Glyphs::Ascii,
            Glyphs::Ascii => Glyphs::Herdr,
            Glyphs::Herdr => Glyphs::Unicode,
        }
    }

    fn vertical(self) -> &'static str {
        match self {
            Glyphs::Unicode => "│",
            Glyphs::Double => "║",
            Glyphs::Heavy => "┃",
            Glyphs::Ascii => "|",
            Glyphs::Herdr => "│",
        }
    }

    fn horizontal(self) -> &'static str {
        match self {
            Glyphs::Unicode => "─",
            Glyphs::Double => "═",
            Glyphs::Heavy => "━",
            Glyphs::Ascii => "-",
            Glyphs::Herdr => "─",
        }
    }

    fn cross(self) -> &'static str {
        match self {
            Glyphs::Unicode => "┼",
            Glyphs::Double => "╬",
            Glyphs::Heavy => "╋",
            Glyphs::Ascii => "+",
            Glyphs::Herdr => "┼",
        }
    }

    /// The modal border ring's rounded corners (ASCII fallback: `+`).
    fn corner_top_left(self) -> &'static str {
        match self {
            Glyphs::Unicode => "╭",
            Glyphs::Double => "╔",
            Glyphs::Heavy => "┏",
            Glyphs::Ascii => "+",
            Glyphs::Herdr => "╭",
        }
    }

    /// The modal border ring's rounded corners (ASCII fallback: `+`).
    fn corner_top_right(self) -> &'static str {
        match self {
            Glyphs::Unicode => "╮",
            Glyphs::Double => "╗",
            Glyphs::Heavy => "┓",
            Glyphs::Ascii => "+",
            Glyphs::Herdr => "╮",
        }
    }

    /// The modal border ring's rounded corners (ASCII fallback: `+`).
    fn corner_bottom_left(self) -> &'static str {
        match self {
            Glyphs::Unicode => "╰",
            Glyphs::Double => "╚",
            Glyphs::Heavy => "┗",
            Glyphs::Ascii => "+",
            Glyphs::Herdr => "╰",
        }
    }

    /// The modal border ring's rounded corners (ASCII fallback: `+`).
    fn corner_bottom_right(self) -> &'static str {
        match self {
            Glyphs::Unicode => "╯",
            Glyphs::Double => "╝",
            Glyphs::Heavy => "┛",
            Glyphs::Ascii => "+",
            Glyphs::Herdr => "╯",
        }
    }
}

/// One pane's local emulator: the core [`Terminal`] fed the pane's wire
/// bytes, at the pane's layout geometry.
pub struct PaneEmulator {
    term: Terminal,
    /// Client-side scroll offset into the pane's scrollback (0 = live).
    /// Driven by the wheel when the pane does not own mouse mode; every
    /// keystroke forwarded to the pane snaps it back to live.
    scroll: usize,
    /// Scroll-mode hold: pane output does not snap the view back to live
    /// while held (the prefix-[ keyboard scroll viewport). The offset is
    /// clamped to the new scrollback extent instead, so the view stays
    /// put relative to the BOTTOM of history as lines push in.
    hold_scroll: bool,
}

impl PaneEmulator {
    /// A fresh emulator at `cols` x `rows`.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            term: Terminal::new(cols as usize, rows as usize),
            scroll: 0,
            hold_scroll: false,
        }
    }

    /// The emulator's grid state.
    pub fn terminal(&self) -> &Terminal {
        &self.term
    }

    /// The client-side scroll offset (0 = live bottom).
    pub fn scroll_offset(&self) -> usize {
        self.scroll
    }

    /// Whether the pane owns mouse tracking (its emulator has a mouse mode
    /// other than off — tracked from the pane's own DECSET 1000/1002/1003
    /// and replay bytes).
    pub fn owns_mouse(&self) -> bool {
        self.term.mouse_mode() != MouseMode::Off
    }

    /// The pane's mouse encoding (its negotiated 1005/1006/1015), for the
    /// router's forward decision.
    #[cfg(test)]
    pub fn mouse_encoding(&self) -> par_term_emu_core::mouse::MouseEncoding {
        self.term.mouse_encoding()
    }

    /// The pane's DECCKM application-cursor mode, for the key re-encoder.
    #[cfg(test)]
    pub fn application_cursor(&self) -> bool {
        self.term.application_cursor()
    }

    /// Feed the pane's byte stream — a `refresh-client` replay body or a
    /// `%output` chunk — through the emulator. Any fed byte resets the
    /// pane's client scroll to live: the pane wrote, so the view snaps
    /// to the bottom. While scroll hold is set ([`Self::set_scroll_hold`],
    /// the prefix-[ viewport), the offset survives and clamps to the new
    /// scrollback extent instead.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.term.process(bytes);
        // The daemon's PTY session already answers the pane's queries and
        // owns its bells; a mirror must never answer on its behalf, and
        // undrained queues would grow with the pane's output (SEC-207).
        let _ = self.term.drain_responses();
        let _ = self.term.drain_bell_events();
        if self.hold_scroll {
            let max = self.term.active_grid().scrollback_len();
            self.scroll = self.scroll.min(max);
        } else {
            self.scroll = 0;
        }
    }

    /// Scroll this pane's client-side view by `delta` lines (positive =
    /// up into history). Clamped to the scrollback extent; returns the
    /// resulting offset. Pane output ([`Self::feed`]) resets to live.
    pub fn scroll_by(&mut self, delta: isize) -> usize {
        let max = self.term.active_grid().scrollback_len();
        let current = self.scroll as isize;
        self.scroll = (current + delta).clamp(0, max as isize) as usize;
        self.scroll
    }

    /// Set the scroll-mode hold. Entering the prefix-[ viewport holds;
    /// leaving clears it (clearing does NOT snap — the caller snaps
    /// explicitly with [`Self::scroll_to_live`]).
    pub fn set_scroll_hold(&mut self, hold: bool) {
        self.hold_scroll = hold;
    }

    /// Snap the client view back to the live bottom.
    pub fn scroll_to_live(&mut self) {
        self.scroll = 0;
    }

    /// Re-fit to a new geometry (`refresh-client -C` re-division).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(cols as usize, rows as usize);
    }
}

/// One divider boundary between two adjacent layout rects: the
/// orientation and the pane ids on the left/above (`a`) and
/// right/below (`b`). The drag hit-test returns it; the drag state and
/// the renderer's highlight carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DividerHit {
    /// A vertical boundary (a left of b)?
    pub vertical: bool,
    /// The pane left of (or above) the boundary.
    pub a: u32,
    /// The pane right of (or below) the boundary.
    pub b: u32,
}

/// A modal overlay's scroll state: `(scrolled-past, visible, total)`
/// rows — the painter draws a border thumb from it when the content
/// overflows.
pub(crate) type OverlayScroll = (usize, usize, usize);

/// The modal overlay the renderer paints over the frame: its title, its
/// composed rows, and its optional scroll state.
pub(crate) type Overlay = (&'static str, Vec<HelpRow>, Option<OverlayScroll>);

/// The frame renderer: pane emulators + layout rects + the double buffers
/// the damage diff reads.
pub struct PaneRenderer {
    emulators: HashMap<u32, PaneEmulator>,
    /// Current layout rects, window-relative.
    layout: Vec<PaneRect>,
    /// The focused pane id — its adjacent dividers highlight.
    focused: Option<u32>,
    width: u16,
    height: u16,
    glyphs: Glyphs,
    /// The background every frame cell is filled with before painting —
    /// the host terminal's resolved background (the OSC 10/11 probe). When
    /// the probe FAILED, `None` leaves bg-fill cells at the terminal
    /// default (no color painted), so a failed probe can never mismatch
    /// the theme; black only ever appears when the probe SUCCEEDED with
    /// black.
    bg: Option<RtColor>,
    /// The divider currently dragged (orientation + pane ids), drawn
    /// reversed so the edge being moved stands out.
    drag_divider: Option<(bool, u32, u32)>,
    /// The help/picker overlay's title, rows, and optional scroll state
    /// `(scrolled-past, visible, total)` — painted as the themed modal
    /// over the frame while `Some` (the help or picker chord). Rows carry
    /// their accent flag so the painter styles headers/border ring in the
    /// accent color; the scroll state draws a border thumb when the
    /// content overflows.
    overlay: Option<Overlay>,
    /// Reversed spans `(row, from, to)` over the overlay's inner columns.
    overlay_highlights: Vec<(usize, usize, usize)>,
    /// Where the overlay anchors instead of centering (the command menu
    /// beside its chip); cleared with the overlay.
    overlay_anchor: Option<(u16, u16)>,
    /// The in-app notification card (notify-delivery `inside`).
    toast: Option<toast::Toast>,
    /// Per-pane border boxes instead of shared dividers (config
    /// `pane-borders`); `show_label_in_border` embeds the pane's title in
    /// the top edge. Both default off.
    pane_borders: bool,
    show_label_in_border: bool,
    /// Visible gap bands between panes (config `pane-gaps`): every
    /// pane's chrome and content paint into the rect inset by this many
    /// cells per side; the band cells stay at the frame's theme-bg fill.
    /// Default 0 — today's edge-to-edge tiling.
    pane_gaps: u16,
    /// The session's frame geometry (ARC-128): the content rect every
    /// layout rect maps through. Its `sidebar_w` is the side panel's
    /// width (0 = hidden) — the daemon's layout is the REDUCED grid, so
    /// every layout x paints offset right by it and the strip columns
    /// hold the panel.
    geometry: geometry::FrameGeometry,
    /// The panel's sections (queried workspace roster, more to come) —
    /// `None` while hidden or before the first refresh.
    sidebar_sections: Option<Vec<super::SidebarSection>>,
    /// The area split: the workspace area's share of the panel's rows
    /// (default [`super::SIDEBAR_SPLIT_DEFAULT`]). Set by the split drag
    /// and the persisted client state.
    sidebar_split: f32,
    /// The panel is collapsed to the status rail (its strip is
    /// [`super::SIDEBAR_RAIL_WIDTH`] columns).
    sidebar_collapsed: bool,
    /// Per-pane effective titles (`pane-title`: the user `-T` label when
    /// set, else the pane's OSC title) — what border labels paint. The
    /// daemon is authoritative; the client re-queries on the throttled
    /// status refresh only for panes whose title went stale. Each entry
    /// pairs the title with the emulator's OSC title as of the query, so
    /// a later OSC change marks the entry stale.
    user_titles: HashMap<u32, (String, String)>,
    /// Reserve a right-edge gutter column in each pane rect (config
    /// `scrollbar-gutter`): content narrows by one; the gutter renders a
    /// minimal position indicator while the pane's client scroll offset
    /// is > 0. Default off.
    scrollbar_gutter: bool,
    /// The daemon reserves the declared per-pane chrome (`refresh-client`
    /// feature `chrome`): emulators mirror the PTY grid, the rect less the
    /// chrome, rather than the full rect.
    reserved_chrome: bool,
    /// The frame being painted.
    buffer: Buffer,
    /// The last frame handed out; `render_frame` diffs against it.
    prev_buffer: Buffer,
    /// Border colors (config `border-active-color` / `border-color`,
    /// `#rrggbb` hex; `None` = the built-ins — bright-cyan accent for the
    /// focused boundary half, DIM for unfocused dividers).
    border_active: Option<RtColor>,
    border_plain: Option<RtColor>,
    dirty: bool,
}

/// Sink the flush loop writes through, so the frame path is testable
/// headless (the real sink writes to stdout; a test records cells).
pub(crate) trait FlushSink {
    /// Write one frame's diff.
    fn flush(&mut self, diff: &[(u16, u16, RtCell)]);
    /// Bracket a full repaint (first frame, resize re-fit): the real sink
    /// clears the screen and hides the cursor here.
    fn repaint_all(&mut self);
    /// Position (and shape) the host cursor for the frame just flushed:
    /// `Some((x, y, style))` places it at the window-relative 0-based
    /// cell with the DECSCUSR shape, `None` hides it. Default: nothing
    /// (the cursor state the repaint_all/flush left stands).
    fn place_cursor(&mut self, _cursor: Option<(u16, u16, CursorStyle)>) {
        let _ = _cursor;
    }
    /// Write raw bytes to the host between frames — a notification
    /// escape, which moves no cursor and paints no cell. Default: drop
    /// them.
    fn write_raw(&mut self, _bytes: &[u8]) {}
}

/// Opt-in diagnostics (`PAR_MUX_CURSOR_TRACE=1`) for the intermittent
/// invisible-cursor report (joe in a render attach): each host cursor
/// write and each [`PaneRenderer::focused_cursor`] `None` reason lands
/// on stderr, so the next occurrence names the hider. Cheap when unset
/// (one `OnceLock` read per call). In render mode stderr is normally the
/// same tty as the frame, so redirect it (`2>/tmp/cursor.log`) or the
/// trace lines paint into the screen being diagnosed.
pub(crate) fn cursor_trace() -> bool {
    use std::sync::OnceLock;
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| {
        std::env::var("PAR_MUX_CURSOR_TRACE").is_ok_and(|v| v != "0" && !v.is_empty())
    })
}

/// Write one cursor-trace line to stderr when `PAR_MUX_CURSOR_TRACE` is set.
pub(crate) fn trace_cursor(line: std::fmt::Arguments<'_>) {
    if cursor_trace() {
        let _ = writeln!(std::io::stderr(), "par-mux cursor-trace: {line}");
    }
}

/// Trace one `focused_cursor` `None` reason (when tracing is on) and
/// yield `None`, so the lookup chain keeps its early-return shape.
pub(crate) fn trace_none<T>(reason: std::fmt::Arguments<'_>) -> Option<T> {
    trace_cursor(format_args!("focused_cursor None ({reason})"));
    None
}

/// Opt-in input diagnostics (`PAR_MUX_INPUT_TRACE=1`): every host mouse
/// report, the router's hit, and every byte forwarded to a pane land on
/// stderr with timestamps, so a live misbehavior repro names the events
/// that reached the pane (the htop menu defect hunt). Redirect stderr
/// (`2>/tmp/input.log`) or the lines paint into the screen.
pub(crate) fn input_trace() -> bool {
    use std::sync::OnceLock;
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| {
        std::env::var("PAR_MUX_INPUT_TRACE").is_ok_and(|v| v != "0" && !v.is_empty())
    })
}

/// Write one input-trace line to stderr when `PAR_MUX_INPUT_TRACE` is set.
pub(crate) fn trace_input(line: std::fmt::Arguments<'_>) {
    if input_trace() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(std::io::stderr(), "par-mux input-trace[{t}]: {line}");
    }
}

/// The Phase B render-mode attach session: connect, seed the window's
/// panes from the daemon (layout via a size-report `%layout-change`,
/// per-pane state via the `refresh-client -t` replays), then pump
/// `%output` into the per-pane emulators at frame cadence until detach or
/// the connection ends.
pub(crate) fn run_render_session(
    options: &super::AttachOptions,
    client: &super::ResolvedClient,
) -> std::process::ExitCode {
    match render_session_inner(options, client) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("par-mux: attach failed: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn render_session_inner(
    options: &super::AttachOptions,
    client: &super::ResolvedClient,
) -> Result<(), String> {
    let path = options.socket_path();
    let mut conn = crate::mux::attach::conn::AttachConn::connect(&path)
        .map_err(|_| format!("no daemon running on {}", path.display()))?;
    if let Some(warning) = &conn.warnings().stamp_mismatch {
        eprintln!("{warning}");
    }

    // The host grid — the renderer's window extent (the window-size
    // policy resizes the daemon's window to it). Raw mode + alternate
    // screen + mouse capture: unlike passthrough, the renderer owns the
    // whole screen and routes mouse events itself. SGR+motion capture
    // (1002 + 1006) is what the host can report; panes that negotiate
    // any-event tracking get drags through the same path. Both guards
    // restore on every exit path, `?` and unwind included. Locals drop in
    // reverse declaration order, so the screen guard (declared second)
    // leaves the alternate screen BEFORE the terminal guard leaves raw
    // mode.
    let (cols, rows) = super::conn::terminal_grid();
    let terminal_guard = super::TerminalGuard::enter();
    let _screen_guard = RenderScreenGuard::enter(terminal_guard.entered());

    let mut session = WindowSession::new(cols, rows);
    // The client configuration was resolved once for both modes
    // (`resolve_client`: CLI over file over defaults) before the
    // terminal was touched; the session seeds from it.
    let chords = &client.chords;
    session.render_opts.sidebar_width = chords.sidebar_width;
    // The panel's persisted state (the dragged width and split, the
    // chevron's collapse) overrides the config's defaults; the session
    // writes it back on every drag release and chevron press.
    if let Some(path) = super::prefs::prefs_path() {
        let prefs = super::prefs::load(&path);
        if let Some(width) = prefs.sidebar_width {
            session.render_opts.sidebar_width =
                width.clamp(sidebar::SIDEBAR_WIDTH_MIN, sidebar::SIDEBAR_WIDTH_MAX);
        }
        if let Some(split) = prefs.sidebar_split.filter(|s| s.is_finite()) {
            session.renderer.set_sidebar_split(split.clamp(0.1, 0.9));
        }
        let collapsed = prefs.sidebar_collapsed.unwrap_or(false);
        session.sidebar.set_collapsed(collapsed);
        session.renderer.set_sidebar_collapsed(collapsed);
        session.sidebar.set_prefs_path(path);
    }
    session.settings_path = crate::mux::config::config_file_path();
    // `sidebar-on-launch`: the panel width lands on the renderer BEFORE
    // the seed, so the seed's first `refresh-client -C` already reports
    // the reduced grid and the daemon's first division reserves it.
    session.sidebar.set(chords.sidebar_on_launch);
    session.chrome_geometry_changed();
    session.renderer.set_geometry(session.geometry);
    if !session.apply_chords(chords, &client.file) {
        eprintln!(
            "par-mux: [client] border-lines {:?} unknown — valid: unicode, double, heavy, ascii, herdr",
            chords.border_lines
        );
    }
    for warning in crate::mux::config::display_option_warnings(chords) {
        eprintln!("par-mux: {warning}");
    }

    // The host probe: OSC 11 background + XTWINOPS CSI 16 t cell pixels,
    // run with raw mode up and before the pump's stdin reader starts, so
    // the probe is briefly the tty's only reader. The probed bg fills
    // every frame cell; a probe FAILURE leaves the fill terminal-default
    // (no assumed color). A corrected set-client-colors rides the probe
    // so the daemon's theme record follows the host, and probed cell
    // pixels follow as their own `refresh-client -C … -p …` — the
    // handshake already reported the 10x20 construction default and the
    // daemon's client cell-size record is latest-report-wins.
    let probe = super::conn::probe_host_terminal();
    if let Some((r, g, b)) = probe.background {
        session.set_background(Some(RtColor::Rgb(r, g, b)));
        let _ = conn.send_checked(&format!(
            "set-client-colors -f ffffff -b {r:02x}{g:02x}{b:02x}"
        ));
    }
    if let Some((cw, ch)) = probe.cell_pixels {
        let (cols, rows) = super::conn::terminal_grid();
        let _ = conn.send_checked(&format!("refresh-client -C {cols}x{rows} -p {cw}x{ch}"));
    }
    // Keystrokes the probe consumed get back into the stream instead of
    // being eaten.
    session.stdin_primer = probe.primer;

    session.run(&mut conn, options.target.as_deref(), &mut StdoutSink)
}

/// RAII owner of the render client's alternate screen and mouse
/// capture: entering switches to the alternate screen and enables SGR
/// mouse capture; dropping releases the mouse, leaves the alternate
/// screen, and shows the cursor, ignoring errors. Without a tty (raw
/// mode never came up) it emits nothing.
pub(crate) struct RenderScreenGuard {
    active: bool,
}

impl RenderScreenGuard {
    fn enter(tty: bool) -> Self {
        if tty {
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::terminal::EnterAlternateScreen,
                crossterm::event::EnableMouseCapture
            );
        }
        Self { active: tty }
    }
}

impl Drop for RenderScreenGuard {
    fn drop(&mut self) {
        #[cfg(test)]
        screen_guard_drops::record();
        if self.active {
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::event::DisableMouseCapture,
                crossterm::terminal::LeaveAlternateScreen,
                crossterm::cursor::Show
            );
        }
    }
}

/// Per-thread count of [`RenderScreenGuard`] drops, so a test can prove
/// the guard restores on an early error return.
#[cfg(test)]
pub(super) mod screen_guard_drops {
    use std::cell::Cell;

    thread_local! {
        static DROPS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn record() {
        DROPS.with(|d| d.set(d.get() + 1));
    }

    pub(crate) fn count() -> usize {
        DROPS.with(Cell::get)
    }
}

/// A divider drag in flight. `Pending` is a press near a divider that has
/// not moved yet — it must not focus or forward (a divider press is not a
/// click-through); the first motion promotes it to `Active`, which
/// accumulates the pointer's signed cell delta from the press point for
/// the pump's frame-cadence application.
enum DragState {
    Pending {
        divider: DividerHit,
        x: u16,
        y: u16,
    },
    Active {
        divider: DividerHit,
        x: u16,
        y: u16,
        applied: i32,
        pending: i32,
        /// When the last resize step was sent — the drag's send
        /// throttle. Every step is a daemon layout change that repaints
        /// the whole window, and pointer motion arrives far faster than
        /// that should happen.
        last_sent: Option<std::time::Instant>,
    },
}

/// One render-mode session's live state: the window being mirrored, its
/// renderer, the status row, and any layout change waiting to be applied
/// (applying one needs the connection for the re-seeding replays, so the
/// event handler parks it for the pump).
struct WindowSession {
    /// The window id this session mirrors, `@N`.
    window: String,
    renderer: PaneRenderer,
    /// Work the nonblocking handlers parked for the pump (it needs the
    /// connection or the flush sink): see [`PendingWork`].
    pending: PendingWork,
    /// The bottom row: queried state + paint/diff pair.
    status: status::StatusState,
    /// The status row, tab strip, status-bar toggle and flash cue.
    chrome: ChromeRows,
    /// Whether the status state is stale and needs a re-query before the
    /// next paint (the throttled re-query on agent/sessions churn).
    status_dirty: bool,
    /// The modal that owns the keyboard (and, for most, the pointer), if
    /// any: see [`Modal`]. One field, so two open modals are
    /// unrepresentable.
    modal: Modal,
    /// The reload chord: the key byte matched after the prefix, and the
    /// detach prefix itself — both live-rebindable by the reload.
    prefix: u8,
    reload_key: u8,
    /// The window/pane management chords (split % / split " / kill x /
    /// new-window c): the key bytes matched after the prefix. Live
    /// rebindable by the reload.
    management: crate::mux::config::Management,
    /// Cells per resize step (config `resize-step`): each arrow press in
    /// resize mode and each cell of a divider drag.
    resize_step: u32,
    /// The side panel's agent entry lines (config `agent-rows`).
    agent_rows: Vec<Vec<String>>,
    /// Agent state-change notifications: the delivery, the states that
    /// notify, and the terminal escapes the next frame writes.
    notify: notify::NotifyState,
    /// The config's `sidebar-on-launch` (the settings dialog shows and
    /// edits it; the live panel's visibility is `sidebar`).
    sidebar_on_launch: bool,
    /// The config file the settings dialog writes; set only by a real
    /// attach, so a unit-test session never writes.
    settings_path: Option<std::path::PathBuf>,
    /// When the roster was last re-queried for a missing project.
    project_poll: std::time::Instant,
    /// The divider drag in flight, if any.
    drag: Option<DragState>,
    /// The session's configured display options (background probe,
    /// glyphs, pane chrome, side-panel width, border colors), kept
    /// OUTSIDE the renderer as the configured record; the in-place refit
    /// reads the panel's effective width from
    /// [`Self::effective_render_opts`]. Setters update this first, then
    /// the live renderer.
    render_opts: RenderOptions,
    /// Whether the side panel is shown (toggled by the `sidebar`
    /// chord); its width is `render_opts.sidebar_width`.
    sidebar: SidebarState,
    /// A sidebar drag in flight (width or split); `None` idle.
    pub(super) sidebar_drag: Option<sidebar::SidebarDrag>,
    /// The zoom cue (the daemon's `resize-pane -Z` state, as the chord
    /// last saw it) — bolds a ` Z |` head on the status row. The daemon
    /// holds the truth; a window switch resets the cue.
    zoomed: bool,
    /// The daemon's last known (unzoomed) tree layout, so the
    /// prefix+arrow navigation can aim through a zoom.
    daemon_layout: Vec<PaneRect>,
    /// Stdin bytes the OSC 11 background probe consumed before the pump
    /// started — primed back into the stdin stream on the first pump.
    stdin_primer: Vec<u8>,
    /// Input bytes `forward_keys` could not send under the outstanding-reply
    /// bound, as (pane, bytes) in typed order; flushed at the head of pump
    /// step 3.
    stdin_backlog: Vec<(String, Vec<u8>)>,
    /// Continuations awaiting their command replies: see [`pending_cmd`].
    pending_cmds: pending_cmd::PendingCommands,
    /// Generation of the latest user navigation: see [`WindowSession::begin_nav`].
    nav_gen: u64,
    /// The literal prefix byte to forward when the user types prefix
    /// prefix (rebindable, so it is state, not the C_B constant).
    literal: u8,
    /// Host-cursor placement bookkeeping: the emit guard, the heartbeat
    /// clock, and the drag-shape option (see [`CursorState`]).
    cursor: CursorState,
    /// A mouse path (the command menu's `detach` row) asked to end the
    /// view; the pump takes it after the mouse token and exits the same
    /// way prefix `d` does.
    detach_requested: bool,
    /// When the frame hold started (step 9's coalescing window): a push
    /// burst still streaming must not frame a partial state and re-frame
    /// a millisecond later — the whole-screen flash-and-repaint of the
    /// manual-pass htop report. `None` while no hold is running.
    frame_hold_since: Option<std::time::Instant>,
    /// An `%exit` (or other `End` outcome) the synchronous drain around
    /// a refit's layout change consumed ([`Self::drain_around_layout`]);
    /// the pump ends the view on it right after the refit returns.
    pending_end: bool,
    /// The frame's chrome layout over the host grid (strip, status bar,
    /// side panel, content) — the one owner of the host ↔ frame ↔
    /// content mapping. Recomputed by [`Self::refresh_geometry`] at the
    /// top of every pump tick and before every refit, so paint, hit-test,
    /// and size report read one consistent snapshot per frame.
    geometry: geometry::FrameGeometry,
    /// The stdin tokenizer. Session-level (not per pump call) because a
    /// paste or escape sequence can span two stdin bursts, and the pump
    /// returns between them every frame tick.
    parser: InputParser,
}

/// What the modal prompt edits: rename flows seeded from the live name,
/// and the `+`/` new ` chips' create flows seeded with the next free
/// index. The rename flows carry their target's id (the context menu
/// renames a window the view may not be showing; the shown-window
/// paths pass the shown id).
#[derive(Debug, Clone, PartialEq, Eq)]
enum PromptTarget {
    /// The focused pane's sticky user title.
    Pane,
    /// A window's name, by id (`@N`).
    Window(String),
    /// A new window's name (the daemon creates the window on commit).
    NewWindow,
    /// A workspace's name, by id (`+N`).
    Workspace(String),
    /// A new workspace's name (the daemon creates it on commit).
    NewWorkspace,
    /// A `[client]` setting edited as text (the settings dialog's
    /// number/hex/chord rows): what to validate and write on commit,
    /// plus the dialog state to restore when the prompt closes.
    Setting(Box<modal_settings::SettingEdit>),
}

/// Which entity an open context menu targets.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MenuTarget {
    /// The tab context menu: the window id (`@N`) the actions run
    /// against.
    Tab(String),
    /// The workspace menu: the workspace id (`+N`) the actions run
    /// against.
    Workspace(String),
    /// The side panel's ` menu ` chip: client commands with no target
    /// entity (keybinds, reload config, detach).
    Commands,
}

/// The context menus' action vocabulary: rename/close per target, the
/// tab menu's add-tab, the workspace menu's new-workspace, and the
/// command menu's keybinds/reload-config/detach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Rename,
    Close,
    AddTab,
    NewWorkspace,
    Keybinds,
    Settings,
    ReloadConfig,
    Detach,
}

/// The open menu's state: its target plus the action dispatched per
/// overlay row (the parallel mapping `compose_menu_panel` returns; row
/// `None` entries — the header and the footer — consume the click).
#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuState {
    target: MenuTarget,
    actions: Vec<Option<MenuAction>>,
}

/// The modal that owns input. At most one is up; opening one assigns
/// [`WindowSession::modal`], replacing whatever was there.
///
/// Dispatch precedence, kept identical across the three dispatchers:
/// a functional key ([`WindowSession::route_key`]) goes to whichever
/// modal is up, then to the pane. A plain run
/// ([`WindowSession::route_plain`]) goes to every modal except
/// `Scroll` first; the scroll viewport sees the run through the prefix
/// scanner, so a prefix chord still works while scrolling. The pointer
/// ([`WindowSession::mouse_hit`]) checks the menu before the side
/// panel and a drag in flight, and the prompt, picker, and help panel
/// after them; `Scroll` and `Resize` leave the pointer to the panes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum Modal {
    /// No modal: input goes to the panes.
    #[default]
    None,
    /// The prefix-[ keyboard scroll viewport on the focused pane.
    Scroll,
    /// The rename/create prompt.
    Prompt(PromptState),
    /// A context menu.
    Menu(MenuState),
    /// The bindings help panel.
    Help(HelpState),
    /// The session/window picker or the workspace picker.
    Picker(PickerState),
    /// The settings dialog (the command menu's `settings`).
    Settings(modal_settings::SettingsState),
    /// Sticky resize mode: arrows adjust the focused pane's edges; Enter,
    /// Escape, and `q` exit; any other key leaves the mode (consumed, so
    /// keys never leak into the pane during a modal chord).
    Resize,
}

impl Modal {
    /// Whether the host cursor hides while this modal is up. Every modal
    /// that paints an overlay or owns the keys hides it; the scroll
    /// viewport keeps it (the pane's cursor maps through the scroll).
    fn hides_cursor(&self) -> bool {
        matches!(
            self,
            Modal::Prompt(_)
                | Modal::Menu(_)
                | Modal::Help(_)
                | Modal::Picker(_)
                | Modal::Settings(_)
                | Modal::Resize
        )
    }
}

/// The rename/create prompt's state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptState {
    /// The edit buffer.
    text: String,
    /// What the prompt edits (rename pane/window, or a new window's
    /// name — the tab strip's `+`).
    target: PromptTarget,
}

/// The bindings help panel's state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct HelpState {
    /// The filter box's contents (the `/` control).
    filter: String,
    /// Whether the filter box is actively typing.
    filtering: bool,
    /// The content window's scroll offset.
    scroll: usize,
}

/// The picker's state: the session/window picker, or the workspace
/// picker when `workspaces` is set.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PickerState {
    /// The filter box's contents (the `/` control).
    filter: String,
    /// Whether the filter box is actively typing.
    filtering: bool,
    /// The selection cursor, over the FILTERED content rows.
    selected: usize,
    /// The content-window start (the listbox panning state).
    start: usize,
    /// The queried session/window roster, as opened.
    entries: Vec<super::PickerEntry>,
    /// The current composed content refs (one per filtered content row,
    /// in content order): the selection/click target map.
    refs: Vec<super::PickerRef>,
    /// The composed panel length (filter + content + footer): the click
    /// hit-test's boundary between content rows and the footer.
    panel_len: usize,
    /// When set, this is the WORKSPACE picker: the queried
    /// `(id, name, active)` roster its rows compose from.
    workspaces: Option<Vec<(String, String, bool)>>,
}

/// Work the nonblocking event handlers and the sink-less chords park for
/// the pump, which owns the connection and the flush sink.
#[derive(Debug, Default)]
struct PendingWork {
    /// A `%layout-change` whose re-fit + replay the pump still owes.
    layout: Option<Vec<PaneRect>>,
    /// A shared-selection move: the window another client's
    /// select-window landed the shown session on.
    follow_window: Option<String>,
    /// A shared-selection move: the displayed session a
    /// select-workspace landed on (`%client-session-changed`); the pump
    /// resolves its active window by query before reseeding.
    follow_session: Option<String>,
    /// Refit the grid (resize_to + full repaint) on the next loop: set by
    /// the sidebar and status-bar toggles, whose chords have no sink.
    grid_refit: bool,
}

impl PendingWork {
    /// Forget work parked against a view a reseed just replaced: a
    /// follow aimed at the old window is done (the reseed IS the follow
    /// landing), and a parked layout was parsed against the old pane set.
    fn clear_view_work(&mut self) {
        self.layout = None;
        self.follow_window = None;
        self.follow_session = None;
    }
}

impl WindowSession {
    fn new(cols: u16, rows: u16) -> Self {
        // The chords start at the canonical defaults (ARC-131: one source,
        // `Chords::with_defaults`); the launch config overrides them. The
        // display options start at the chrome-free RenderOptions baseline
        // and take their configured values from the same chords in
        // `render_session_inner`. The status bar starts shown and the side
        // panel hidden (the launch config applies its width afterwards).
        let chords = crate::mux::config::Chords::with_defaults();
        let geometry = geometry::FrameGeometry::new(cols, rows, true, 0);
        let (frame_w, frame_h) = geometry.frame_size();
        Self {
            window: String::new(),
            renderer: PaneRenderer::new(frame_w, frame_h, RenderOptions::default().glyphs),
            pending: PendingWork::default(),
            status: status::StatusState::default(),
            chrome: ChromeRows::new(cols),
            status_dirty: true,
            modal: Modal::None,
            prefix: chords.prefix,
            reload_key: chords.reload,
            management: chords.management,
            resize_step: chords.resize_step,
            agent_rows: chords.agent_rows.clone(),
            notify: notify::NotifyState::default(),
            sidebar_on_launch: chords.sidebar_on_launch,
            settings_path: None,
            drag: None,
            render_opts: RenderOptions::default(),
            sidebar: SidebarState::default(),
            sidebar_drag: None,
            zoomed: false,
            daemon_layout: Vec::new(),
            stdin_primer: Vec::new(),
            stdin_backlog: Vec::new(),
            pending_cmds: pending_cmd::PendingCommands::default(),
            nav_gen: 0,
            literal: chords.prefix,
            cursor: CursorState::new(std::time::Instant::now()),
            project_poll: std::time::Instant::now(),
            detach_requested: false,
            frame_hold_since: None,
            pending_end: false,
            geometry,
            parser: InputParser::default(),
        }
    }

    /// Resolve the initial window, seed it, run the pump. `sink` receives
    /// the frames.
    fn run(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        target: Option<&str>,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        self.seed(conn, target, sink)?;
        self.pump(conn, sink)
    }

    /// The pump: route daemon pushes into the renderer, poll the host
    /// size (SIGWINCH lands as a size change), frame at cadence. The
    /// step order (drain, pending, stdin, pending, resize, status, flash,
    /// drag, frame, wait) is the contract; `Break` ends the view with a
    /// clean exit.
    fn pump(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        let mut stdin = super::Stdin::new_with_primer(std::mem::take(&mut self.stdin_primer));
        let mut prefix_pending = false;
        let mut current_size = self.renderer.window_size();
        loop {
            // 0. This tick's frame geometry: every hit-test and paint
            //    below reads this snapshot rather than the live tty size.
            self.refresh_geometry();

            // 1. Drain daemon pushes, then resolve arrived replies.
            if self.drain_pushes(conn).is_break() {
                return Ok(());
            }
            self.dispatch_replies(conn);

            // 2. Parked work the event handlers queued (a layout re-fit,
            //    shared-selection follows), so this tick's input routes
            //    against the newest layout.
            if self.pump_pending(conn, sink)?.is_break() {
                return Ok(());
            }

            // 3. Stdin: first retry bytes the backpressure bound parked,
            //    then prefix routing (d detaches, [ enters scroll mode,
            //    n/p/(/) switch windows/sessions), then keys/mouse to the
            //    focused pane.
            self.flush_stdin_backlog(conn);
            if self.pump_stdin(conn, &mut stdin, &mut prefix_pending) {
                return Ok(());
            }

            // 4. Parked work the chords just queued (a sidebar or status
            //    bar toggle's grid refit): the chords have no flush sink.
            if self.pump_pending(conn, sink)?.is_break() {
                return Ok(());
            }

            // 5. Host resize (SIGWINCH): report the new grid, re-fit.
            if self.pump_resize(conn, sink, &mut current_size)?.is_break() {
                return Ok(());
            }

            // 6a. A panel agent with no project yet: the host probe fills it
            //     on its 30 s sweep and broadcasts nothing, so re-query
            //     the roster on a slow cadence until every agent has one.
            if self.project_poll_due(std::time::Instant::now()) {
                self.status_dirty = true;
            }

            // 6. Status: the throttled re-query. Any %sessions-changed /
            //     %window-add / %window-close (and the other events whose
            //     payload lacks the fact) marked the state stale; one
            //     re-query per burst serves them all. The shown session being gone ends the
            //     view (docs/MUX.md's %sessions-changed client contract);
            //     only the shown WINDOW being gone (its last pane exited
            //     or was killed) lands on the session's active window
            //     instead — tmux semantics: the session outlives a tab.
            if self.status_dirty && self.refresh_status(conn) == EventOutcome::End {
                return Ok(());
            }

            // 7. The flash clears about a second after it first rides a
            //    frame.
            self.tick_flash(std::time::Instant::now());

            // 8. Divider drag: apply the accumulated delta at frame
            //    cadence — one resize-pane per unapplied cell.
            self.apply_drag(conn);

            // 9. Frame whatever accumulated (panes + the status row's own
            //    diff), then wait for the next push — the frame cadence
            //    floods coalesce into. But hold the frame while pushes
            //    keep arriving: a burst still streaming (htop's dialog
            //    redraw lands as dozens of chunks) framed a partial state
            //    and re-framed a millisecond later — the whole-screen
            //    flash-and-repaint of the manual-pass report. The extra
            //    drain costs one try_recv when idle; the 8 ms bound keeps
            //    a firehose pane from starving frames entirely.
            let mut handled = false;
            let flow = self.drain_pushes_counting(conn, &mut handled);
            if flow.is_break() {
                return Ok(());
            }
            let hold = *self
                .frame_hold_since
                .get_or_insert_with(std::time::Instant::now);
            if handled && hold.elapsed() < std::time::Duration::from_millis(8) {
                continue; // the loop re-runs every step against the new events
            }
            self.frame_hold_since = None;
            self.frame(sink);
            if self.wait_next_push(conn).is_break() {
                return Ok(());
            }
        }
    }

    /// Pump step 1: route every queued daemon push. Breaks on `End` or a
    /// disconnected daemon (a clean exit-0 like passthrough's `%exit` path).
    fn drain_pushes(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) -> std::ops::ControlFlow<()> {
        let mut handled = false;
        match self.drain_pushes_counting(conn, &mut handled) {
            std::ops::ControlFlow::Break(()) => std::ops::ControlFlow::Break(()),
            std::ops::ControlFlow::Continue(()) => std::ops::ControlFlow::Continue(()),
        }
    }

    /// [`Self::drain_pushes`] reporting whether any event was handled —
    /// the frame-hold's signal that the burst is still streaming.
    fn drain_pushes_counting(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        handled: &mut bool,
    ) -> std::ops::ControlFlow<()> {
        loop {
            match conn.try_recv() {
                Ok(event) => {
                    *handled = true;
                    if self.handle_event(event) == EventOutcome::End {
                        return std::ops::ControlFlow::Break(());
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    return std::ops::ControlFlow::Continue(());
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return std::ops::ControlFlow::Break(());
                }
            }
        }
    }

    /// Pump steps 2 and 4: apply the parked work, then break when it
    /// ended the view.
    fn pump_pending(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        sink: &mut dyn FlushSink,
    ) -> Result<std::ops::ControlFlow<()>, String> {
        self.apply_pending(conn, sink)?;
        if std::mem::take(&mut self.pending_end) {
            return Ok(std::ops::ControlFlow::Break(()));
        }
        Ok(std::ops::ControlFlow::Continue(()))
    }

    /// Pump step 5: the daemon's window renders into the rows between the
    /// tab strip and the status bar, so the size report carries the
    /// content height.
    fn pump_resize(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        sink: &mut dyn FlushSink,
        current_size: &mut (u16, u16),
    ) -> Result<std::ops::ControlFlow<()>, String> {
        self.refresh_geometry();
        let content = self.geometry.frame_size();
        if content != *current_size {
            *current_size = content;
            self.resize_to(conn, content.0, content.1, sink)?;
            if std::mem::take(&mut self.pending_end) {
                return Ok(std::ops::ControlFlow::Break(()));
            }
            // A resize wiped the screen; the rows repaint whole.
            self.chrome.status_row.invalidate();
            self.draw_status_row();
            self.chrome.tab_strip.invalidate();
            self.draw_tab_strip();
        }
        Ok(std::ops::ControlFlow::Continue(()))
    }

    /// Pump step 9's wait: block up to one poll interval for the next
    /// push.
    fn wait_next_push(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) -> std::ops::ControlFlow<()> {
        match conn.recv_timeout(super::POLL) {
            Ok(event) => {
                if self.handle_event(event) == EventOutcome::End {
                    return std::ops::ControlFlow::Break(());
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return std::ops::ControlFlow::Break(()),
        }
        self.dispatch_replies(conn);
        std::ops::ControlFlow::Continue(())
    }

    /// Perform the work parked in [`PendingWork`], in its fixed order: a
    /// layout re-fit (only panes new to the renderer replay), the shared-selection
    /// follows, then a grid refit. No select is ever sent for a follow:
    /// the daemon already moved the shared pointer, and a re-select would
    /// echo the notification back as a loop.
    fn apply_pending(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        if let Some(layout) = self.pending.layout.take() {
            // The `%layout-change` was consumed in stream order, so the
            // `%output` after it feeds normally at the new size: no race
            // list, only the panes new to the renderer replay (ENH-044).
            self.daemon_layout = layout.clone();
            let replay = self.install_layout(layout, &[]);
            self.replay_panes(conn, &replay);
        }
        if let Some(window) = self.pending.follow_window.take() {
            if window != self.window {
                self.reseed_window(conn, &window);
            }
        }
        if let Some(session) = self.pending.follow_session.take() {
            if let Some(window) = session_active_window(conn, &session) {
                if window != self.window {
                    self.reseed_window(conn, &window);
                }
            }
        }
        if std::mem::take(&mut self.pending.grid_refit) {
            // The toggle that parked this moved the status bar or the
            // side panel: re-derive the frame before refitting to it. The
            // resize_to's repaint_all is what erases the vacated region.
            self.refresh_geometry();
            let (frame_w, frame_h) = self.geometry.frame_size();
            self.resize_to(conn, frame_w, frame_h, sink)?;
            // The refit keeps the strip's sections in place (ENH-044); the
            // re-query refreshes the active workspace mark.
            if self.sidebar.is_on() {
                self.refresh_sidebar(conn);
            }
        }
        Ok(())
    }

    /// The pump's throttled status step: re-query the bar's facts, reuse
    /// the reply rows for the side panel and pane titles, and repaint the
    /// bar and the strip. `End` when the shown session is gone.
    fn refresh_status(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) -> EventOutcome {
        self.status_dirty = false;
        match self.refresh_status_facts(conn) {
            Ok(()) => {}
            // A wedged daemon: the stale bar stands and the next
            // tick retries, instead of the pump blocking on it.
            Err(status::StatusError::TimedOut) => self.status_dirty = true,
            Err(status::StatusError::SessionGone) => {
                let landing = self
                    .status
                    .session_id
                    .clone()
                    .and_then(|session| session_active_window(conn, &session));
                match landing {
                    // The reseed refreshes the status itself.
                    Some(window) if window != self.window => {
                        self.reseed_window(conn, &window);
                    }
                    _ => return EventOutcome::End,
                }
            }
            Err(status::StatusError::Query) => {} // stale state survives; the next mark retries
        }
        self.draw_status_row();
        // The strip rides the same queried state: window
        // add/close/rename refreshes it here.
        self.draw_tab_strip();
        EventOutcome::Continue
    }

    /// The flash's frame tick at `now` (injected so tests drive the
    /// clock): arm the deadline the first time a flash is seen, clear the
    /// flash and repaint the status row once it passes.
    fn tick_flash(&mut self, now: std::time::Instant) {
        if self.chrome.tick(now) {
            self.draw_status_row();
        }
        if self.notify.toast_until.is_some_and(|until| now >= until) {
            self.notify.toast_until = None;
            self.renderer.set_toast(None);
        }
    }

    /// The shown window's `%layout-change`: the zoom cue, then the
    /// geometry diff parked for the pump.
    fn on_layout_change(&mut self, layout: &str, visible: &str, flags: &str) {
        // Zoom truth is per-window and the daemon's: another
        // client's zoom (or a select-away unzoom) reaches this one
        // only through the flags, and a one-pane zoom leaves the
        // geometry unchanged — so the cue updates outside the
        // geometry guard below.
        let zoomed = flags.contains('Z');
        if zoomed != self.zoomed {
            self.zoomed = zoomed;
            self.status_dirty = true;
        }
        match layout::parse_layout_triple(layout, visible, flags) {
            Ok(next) if next != self.renderer.layout() => {
                // A geometry change re-fit the daemon's panes; the
                // local emulators reflow with the layout and panes
                // new to the renderer replay — the pump's step 2
                // (PendingWork), since replay needs the connection.
                self.pending.layout = Some(next);
            }
            Ok(_) => {}
            Err(err) => eprintln!("par-mux: {err}"),
        }
    }

    /// One daemon push. `End` on `%exit` (or the window's pane roster
    /// emptying, which leaves nothing to render).
    fn handle_event(&mut self, event: TmuxNotification) -> EventOutcome {
        match event {
            TmuxNotification::Output { pane_id, data } => {
                self.feed_pane(&pane_id, &data);
                EventOutcome::Continue
            }
            TmuxNotification::ExtendedOutput { pane_id, data, .. } => {
                self.feed_pane(&pane_id, &data);
                EventOutcome::Continue
            }
            TmuxNotification::LayoutChange {
                window_id,
                window_layout,
                window_visible_layout,
                window_raw_flags,
            } if window_id == self.window => {
                self.on_layout_change(&window_layout, &window_visible_layout, &window_raw_flags);
                EventOutcome::Continue
            }
            TmuxNotification::Exit => EventOutcome::End,
            // The status facts that the payload does not carry (session
            // churn, window add/close, selection moves) re-query (the
            // contract's throttled re-query; one burst of events collapses
            // into one refresh — one `client-snapshot` round trip against
            // a daemon that advertises it).
            TmuxNotification::WindowPaneChanged { window_id, pane_id } => {
                // An external focus move on the shown window re-points the
                // local highlight; on any other window it is chrome-only.
                if *window_id == self.window {
                    if let Some(pane) = pane_id.strip_prefix('%').and_then(|p| p.parse().ok()) {
                        self.renderer.focus(pane);
                    }
                }
                self.status_dirty = true;
                EventOutcome::Continue
            }
            TmuxNotification::SessionWindowChanged {
                session_id,
                window_id,
            } => {
                // The shared selection moved this session to another
                // window: park the follow — the pump performs the reseed
                // (the handler runs in the nonblocking drain, where the
                // queries and replays a reseed needs are not available).
                if self.status.session_id.as_deref() == Some(session_id.as_str())
                    && window_id != self.window
                {
                    self.pending.follow_window = Some(window_id.clone());
                }
                self.status_dirty = true;
                EventOutcome::Continue
            }
            TmuxNotification::ClientSessionChanged { session_id, .. } => {
                // The displayed session moved (a workspace switch): park
                // the follow; the pump resolves the session's active
                // window by query and reseeds. Re-selecting here would
                // echo the notification back into a loop.
                self.pending.follow_session = Some(session_id.clone());
                self.status_dirty = true;
                EventOutcome::Continue
            }
            TmuxNotification::PaneTitleChanged { pane_id, title } => {
                self.on_pane_title_changed(&pane_id, &title);
                EventOutcome::Continue
            }
            // Agent churn carries its own fact: patch the cached roster
            // and rebuild the side panel's agents section from it — no
            // re-query (ENH-043). The status row shows no agents.
            TmuxNotification::AgentStateChanged {
                pane_id,
                agent,
                state,
                ws,
                ..
            } => {
                self.on_agent_changed(&pane_id, &agent, Some(&state), &ws);
                EventOutcome::Continue
            }
            TmuxNotification::AgentReleased { pane_id, agent } => {
                self.on_agent_changed(&pane_id, &agent, None, "");
                EventOutcome::Continue
            }
            // Neither the status row nor the side panel renders
            // telemetry, so a telemetry sample changes nothing this
            // client shows.
            TmuxNotification::AgentTelemetryChanged { .. } => EventOutcome::Continue,
            TmuxNotification::WindowRenamed { window_id, name } => {
                self.on_window_renamed(&window_id, &name);
                EventOutcome::Continue
            }
            TmuxNotification::SessionsChanged
            | TmuxNotification::WorkspacesChanged
            | TmuxNotification::SessionRenamed { .. }
            // A window another client added or closed changes the shown
            // session's tab roster; `new-window` sends no
            // `%sessions-changed`, so without these the tab strip missed
            // it (card 01a11bd1).
            | TmuxNotification::WindowAdd { .. }
            | TmuxNotification::WindowClose { .. }
            | TmuxNotification::ClientAttached { .. }
            | TmuxNotification::ClientLeft { .. } => {
                self.status_dirty = true;
                EventOutcome::Continue
            }
            // Everything else — lifecycle elsewhere, paste buffers — is
            // not this window renderer's concern yet.
            _ => EventOutcome::Continue,
        }
    }

    /// Stdin pump: parse tokens, route the prefix (d detaches, C-b C-b
    /// sends the literal), re-encode keys against the focused pane's
    /// input modes, and route mouse reports (click focus, wheel
    /// scrollback, pane-relative forwarding). Returns true to end the
    /// session.
    fn pump_stdin(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        stdin: &mut super::Stdin,
        prefix_pending: &mut bool,
    ) -> bool {
        loop {
            match stdin.read_available() {
                None => return false,
                Some(Ok(bytes)) if bytes.is_empty() => return true, // EOF
                Some(Ok(bytes)) => {
                    if self.route_stdin_bytes(conn, &bytes, prefix_pending) {
                        return true;
                    }
                }
                Some(Err(_)) => return true,
            }
        }
    }

    /// Tokenize one stdin burst through the session's persistent parser
    /// and route every token. Returns true to end the session.
    fn route_stdin_bytes(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        bytes: &[u8],
        prefix_pending: &mut bool,
    ) -> bool {
        let tokens = self.parser.feed(bytes);
        for token in tokens {
            match token {
                Token::Bytes(run) => {
                    if self.route_plain(&run, conn, prefix_pending) {
                        return true;
                    }
                }
                Token::Key(ev) => self.route_key(conn, &ev, prefix_pending),
                Token::Mouse(mouse) => {
                    self.route_mouse(conn, mouse);
                    if std::mem::take(&mut self.detach_requested) {
                        return true;
                    }
                }
                Token::Paste(body) => self.route_paste(conn, &body, prefix_pending),
            }
        }
        false
    }
}

/// What handling one event told the pump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventOutcome {
    Continue,
    End,
}

#[cfg(test)]
mod tests;
