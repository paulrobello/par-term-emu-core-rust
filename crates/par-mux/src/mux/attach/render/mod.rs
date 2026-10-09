//! The Phase B attach pane renderer: per-pane core emulators mapped onto a
//! ratatui [`Buffer`], damage-diffed at frame cadence.
//!
//! One [`crate::terminal::Terminal`] instance per visible pane is fed the
//! pane's `refresh-client` replay and `%output` bytes — the same bytes a
//! passthrough client forwards to the host terminal — so each pane's grid
//! is exactly what the daemon's pane emulator holds (capture-pane ground
//! truth). [`PaneRenderer::render_frame`] paints every pane's grid into
//! its layout rect of a full-window ratatui `Buffer`, draws the dividers
//! on top, and returns the cell diff against the previous frame — the
//! caller flushes only those cells, and a whole output flood between two
//! frames collapses into one diff.

use crate::cell::CellFlags;
use crate::color::{Color as CoreColor, NamedColor};
use crate::cursor::CursorStyle;
use crate::keyboard::TermKeyEvent;
use crate::mouse::MouseMode;
use crate::mux::attach::input::{InputParser, SgrMouse, Token};
use crate::mux::attach::layout::PaneRect;
use crate::mux::attach::status::{self, Segment, StatusRow};
use crate::mux::attach::tabs::TabStrip;
use crate::mux::attach::targets::{
    active_window_row, list_window_ids, next_in_cycle, next_workspace, session_active_window,
};
use crate::mux::attach::{layout, HelpRow, ManagementKey};
use crate::mux::layout::PaneChrome;
use crate::terminal::Terminal;
use crate::tmux_control::TmuxNotification;
use ratatui::buffer::{Buffer, Cell as RtCell, CellDiffOption};
use ratatui::layout::Rect as RtRect;
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::mpsc::RecvTimeoutError;

mod input_route;
mod modal;
mod navigate;
mod renderer;
mod session;
mod sidebar;

use renderer::*;

/// How long the renderer waits between frames at most — the frame cadence
/// output floods coalesce into. Matches Phase A's pump poll interval.
pub const FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

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
    /// The pane id this emulator mirrors, the layout string's leaf number.
    pub pane_id: u32,
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
    pub fn new(pane_id: u32, cols: u16, rows: u16) -> Self {
        Self {
            pane_id,
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
    pub fn mouse_encoding(&self) -> crate::mouse::MouseEncoding {
        self.term.mouse_encoding()
    }

    /// The pane's DECCKM application-cursor mode, for the key re-encoder.
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
    /// The workspace side panel's width in columns (0 = hidden). The
    /// daemon's layout is the REDUCED grid (window_size reports the
    /// width minus this); every layout x paints offset right by it, and
    /// the strip columns hold the panel.
    sidebar_w: u16,
    /// The panel's sections (queried workspace roster, more to come) —
    /// `None` while hidden or before the first refresh.
    sidebar_sections: Option<Vec<super::SidebarSection>>,
    /// Per-pane effective titles (`pane-title`: the user `-T` label when
    /// set, else the pane's OSC title) — what border labels paint. The
    /// daemon is authoritative; the client re-queries on the throttled
    /// status refresh.
    user_titles: HashMap<u32, String>,
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
    // any-event tracking get drags through the same path. Restored on
    // every exit path below.
    let (cols, rows) = super::conn::terminal_grid();
    let _guard = super::TerminalGuard::enter();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    );

    let mut session = WindowSession::new(cols, rows);
    // The client configuration was resolved once for both modes
    // (`resolve_client`: CLI over file over defaults) before the
    // terminal was touched; the session seeds from it.
    let file = &client.file;
    let chords = &client.chords;
    session.prefix = chords.prefix;
    session.literal = chords.prefix;
    session.reload_key = chords.reload;
    session.management = chords.management;
    session.resize_step = chords.resize_step;
    session.render_opts.sidebar_width = chords.sidebar_width;
    // `sidebar-on-launch`: the panel width lands on the renderer BEFORE
    // the seed, so the seed's first `refresh-client -C` already reports
    // the reduced grid and the daemon's first division reserves it.
    session.sidebar_on = chords.sidebar_on_launch;
    session
        .renderer
        .set_sidebar_width(session.effective_render_opts().sidebar_width);
    // The border style first, the explicit `pane-borders` flag after it —
    // the style implies a paint mode (herdr = per-pane boxes), and the
    // explicit config key still overrides for any glyph set.
    if !session.set_border_lines(&chords.border_lines) {
        eprintln!(
            "par-mux: [client] border-lines {:?} unknown — valid: unicode, double, heavy, ascii, herdr",
            chords.border_lines
        );
    }
    // An explicit `pane-borders` key overrides the border style's
    // implied paint mode; an absent key keeps it (herdr, the default
    // style, implies per-pane boxes).
    if let Some(on) = file.client.pane_borders {
        session.set_pane_borders(on);
    } else {
        session.set_pane_borders(matches!(session.render_opts.glyphs, Glyphs::Herdr));
    }
    session.set_show_label_in_border(chords.show_label_in_border);
    session.set_pane_gaps(chords.pane_gaps);
    session.set_scrollbar_gutter(chords.scrollbar_gutter);
    session.drag_cursor_shape = chords.drag_cursor_shape;
    let eff = crate::mux::config::resolve(file, &crate::mux::config::Overrides::default());
    session.set_border_colors(
        crate::mux::config::parse_hex_color(&eff.border_active_color)
            .map(|(r, g, b)| RtColor::Rgb(r, g, b)),
        crate::mux::config::parse_hex_color(&eff.border_color)
            .map(|(r, g, b)| RtColor::Rgb(r, g, b)),
    );

    // The OSC 11 background probe: raw mode is up and the pump's stdin
    // reader has not started, so the probe is briefly the tty's only
    // reader. The probed bg fills every frame cell; a probe FAILURE
    // leaves the fill terminal-default (no assumed color). A corrected
    // set-client-colors rides the probe so the daemon's theme record
    // follows the host.
    let probe = super::conn::probe_background();
    if let Some((r, g, b)) = probe.0 {
        session.set_background(Some(RtColor::Rgb(r, g, b)));
        let _ = conn.send_checked(&format!(
            "set-client-colors -f ffffff -b {r:02x}{g:02x}{b:02x}"
        ));
    }
    // Keystrokes the probe consumed get back into the stream instead of
    // being eaten.
    session.stdin_primer = probe.1;

    let outcome = session.run(&mut conn, options.target.as_deref(), &mut StdoutSink);

    // Restore: leave the alt screen, release the mouse, show the cursor.
    // The TerminalGuard (raw mode) drops after.
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    outcome?;
    Ok(())
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
    /// A `%layout-change` whose re-fit + replay the pump still owes.
    pending_layout: Option<Vec<PaneRect>>,
    /// A shared-selection move the pump still owes: the window another
    /// client's select-window landed the shown session on (parked by the
    /// nonblocking event handler, performed by the pump with the queries
    /// and reseed it needs the connection for).
    pending_follow_window: Option<String>,
    /// A shared-selection move the pump still owes: the displayed session
    /// a select-workspace landed on (`%client-session-changed`). The pump
    /// resolves the session's active window by query before reseeding.
    pending_follow_session: Option<String>,
    /// The bottom row: queried state + paint/diff pair.
    status: status::StatusState,
    status_row: status::StatusRow,
    /// The top row: the shown session's windows, clickable.
    tab_strip: TabStrip,
    /// Whether the status state is stale and needs a re-query before the
    /// next paint (the throttled re-query on agent/sessions churn).
    status_dirty: bool,
    /// Whether scroll mode is up on the focused pane.
    scroll_mode: bool,
    /// The reload chord: the key byte matched after the prefix, and the
    /// detach prefix itself — both live-rebindable by the reload.
    prefix: u8,
    reload_key: u8,
    /// The window/pane management chords (split % / split " / kill x /
    /// new-window c): the key bytes matched after the prefix. Live
    /// rebindable by the reload.
    management: super::super::config::Management,
    /// Cells per resize step (config `resize-step`): each arrow press in
    /// resize mode and each cell of a divider drag.
    resize_step: u32,
    /// Sticky resize mode: arrows adjust the focused pane's edges; Enter,
    /// Escape, and `q` exit; any other key leaves the mode (consumed —
    /// keys must not leak into the pane during a modal chord).
    resize_mode: bool,
    /// The bindings help panel is up (any key dismisses it).
    help_mode: bool,
    /// The help panel's filter box state (the `/` control).
    help_filter: String,
    /// Whether the filter box is actively typing.
    help_filtering: bool,
    /// The help panel's content-window scroll offset.
    help_scroll: usize,
    /// The session/window picker is up (the `picker` chord). The same
    /// modal chrome as the help panel; selection-driven instead of
    /// scroll-driven.
    picker_mode: bool,
    /// The picker's filter box state (the `/` control).
    picker_filter: String,
    /// Whether the picker's filter box is actively typing.
    picker_filtering: bool,
    /// The picker's selection cursor, over the FILTERED content rows.
    picker_selected: usize,
    /// The picker's content-window start (the listbox panning state).
    picker_start: usize,
    /// The queried picker state (sessions + their windows, as opened).
    picker_entries: Vec<super::PickerEntry>,
    /// The picker's current composed content refs (one per filtered
    /// content row, in content order) — the selection/click target map.
    picker_refs: Vec<super::PickerRef>,
    /// The picker's composed panel length (filter + content + footer) —
    /// the click hit-test's boundary between content rows and the
    /// footer.
    picker_panel_len: usize,
    /// When set, the open picker is the WORKSPACE picker: the queried
    /// `(id, name, active)` roster its rows compose from. `None` = the
    /// session/window picker (or no picker open).
    picker_workspaces: Option<Vec<(String, String, bool)>>,
    /// The divider drag in flight, if any.
    drag: Option<DragState>,
    /// The session's configured display options (background probe,
    /// glyphs, pane chrome, side-panel width, border colors), kept
    /// OUTSIDE the renderer: every renderer rebuild (resize re-fit,
    /// re-seed) constructs from [`Self::effective_render_opts`], so no
    /// option can be dropped by a rebuild. Setters update this first,
    /// then the live renderer.
    render_opts: RenderOptions,
    /// Whether the side panel is shown (toggled by the `sidebar`
    /// chord); its width is `render_opts.sidebar_width`.
    sidebar_on: bool,
    /// The pump refits the grid (resize_to + full repaint) on the next
    /// loop — set by the sidebar toggle, whose chord has no sink.
    pending_grid_refit: bool,
    /// While a divider drag is live, the host cursor carries the resize
    /// shape (best-effort DECSCUSR steady block; restored on drag end).
    drag_cursor_shape: bool,
    /// The zoom cue (the daemon's `resize-pane -Z` state, as the chord
    /// last saw it) — bolds a ` Z |` head on the status row. The daemon
    /// holds the truth; a window switch resets the cue.
    zoomed: bool,
    /// The daemon's last known (unzoomed) tree layout, so the
    /// prefix+arrow navigation can aim through a zoom.
    daemon_layout: Vec<PaneRect>,
    /// The rename prompt is up (the modal input overlay).
    prompt_mode: bool,
    /// The prompt's edit buffer.
    prompt_text: String,
    /// What the prompt edits (rename pane/window, or a new window's
    /// name — the tab strip's `+`).
    prompt_target: PromptTarget,
    /// The context menu's modal state: which window/workspace it
    /// targets and the action per overlay row. `None` when no menu is
    /// up.
    menu: Option<MenuState>,
    /// The status bar's visibility (the `status_bar` chord toggles it;
    /// shown by default).
    status_bar_on: bool,
    /// Stdin bytes the OSC 11 background probe consumed before the pump
    /// started — primed back into the stdin stream on the first pump.
    stdin_primer: Vec<u8>,
    /// The literal prefix byte to forward when the user types prefix
    /// prefix (rebindable, so it is state, not the C_B constant).
    literal: u8,
    /// A transient confirmation/error cue drawn in place of the status
    /// line's head and cleared after about a second.
    flash: Option<String>,
    /// The flash's remaining lifetime in frame ticks.
    flash_ticks: u32,
    /// The cursor state the last `place_cursor` reported, `Some(None)`
    /// initially (repaint_all hides the cursor): the guard that keeps a
    /// quiet pump from re-emitting identical cursor escapes every frame
    /// tick, while a flushed frame (whose per-cell CUP left the terminal
    /// cursor wherever the last diff cell sits) always repositions.
    cursor_placed: Option<Option<(u16, u16, CursorStyle)>>,
    /// A mouse path (the command menu's `detach` row) asked to end the
    /// view; the pump takes it after the mouse token and exits the same
    /// way prefix `d` does.
    detach_requested: bool,
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

impl WindowSession {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            window: String::new(),
            renderer: PaneRenderer::new(
                cols,
                rows.saturating_sub(2),
                RenderOptions::default().glyphs,
            ),
            pending_layout: None,
            pending_follow_window: None,
            pending_follow_session: None,
            status: status::StatusState::default(),
            status_row: status::StatusRow::new(cols),
            tab_strip: TabStrip::new(cols),
            status_dirty: true,
            scroll_mode: false,
            prefix: crate::mux::attach::C_B,
            reload_key: 0x12, // C-r
            management: super::super::config::Management::default(),
            resize_step: 1,
            resize_mode: false,
            help_mode: false,
            help_filter: String::new(),
            help_filtering: false,
            help_scroll: 0,
            picker_mode: false,
            picker_filter: String::new(),
            picker_filtering: false,
            picker_selected: 0,
            picker_start: 0,
            picker_entries: Vec::new(),
            picker_refs: Vec::new(),
            picker_panel_len: 0,
            picker_workspaces: None,
            drag: None,
            render_opts: RenderOptions::default(),
            sidebar_on: false,
            pending_grid_refit: false,
            drag_cursor_shape: false,
            zoomed: false,
            daemon_layout: Vec::new(),
            prompt_mode: false,
            prompt_text: String::new(),
            prompt_target: PromptTarget::Window(String::new()),
            menu: None,
            status_bar_on: true,
            stdin_primer: Vec::new(),
            literal: crate::mux::attach::C_B,
            flash: None,
            flash_ticks: 0,
            cursor_placed: Some(None),
            detach_requested: false,
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
    /// size (SIGWINCH lands as a size change), frame at cadence.
    fn pump(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        let mut stdin = super::Stdin::new_with_primer(std::mem::take(&mut self.stdin_primer));
        let mut prefix_pending = false;
        let mut current_size = self.renderer.window_size();
        loop {
            // 1. Drain daemon pushes.
            let mut disconnected = false;
            loop {
                match conn.try_recv() {
                    Ok(event) => {
                        if self.handle_event(event) == EventOutcome::End {
                            return Ok(());
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            if disconnected {
                return Ok(()); // the daemon is gone; a clean exit-0 like passthrough's %exit path
            }

            // 1b. Apply a parked layout change: re-fit the emulators and
            //     re-seed them from fresh daemon replays.
            if let Some(layout) = self.pending_layout.take() {
                self.daemon_layout = layout.clone();
                self.renderer.apply_layout(layout);
                self.replay_all_panes(conn);
            }

            // 1b. Parked shared-selection follows: the reseed (and the
            //     session follow's list-windows query) need the
            //     connection, so the nonblocking handler only parks them
            //     and the pump performs them here. No select is ever sent
            //     — the daemon already moved the shared pointer, and a
            //     re-select would echo the notification back as a loop.
            if let Some(window) = self.pending_follow_window.take() {
                if window != self.window {
                    self.reseed_window(conn, &window);
                }
            }
            if let Some(session) = self.pending_follow_session.take() {
                if let Some(window) = session_active_window(conn, &session) {
                    if window != self.window {
                        self.reseed_window(conn, &window);
                    }
                }
            }

            // 2. Stdin: prefix routing (d detaches, [ enters scroll mode,
            //    n/p/(/) switch windows/sessions), then keys/mouse to the
            //    focused pane.
            if self.pump_stdin(conn, &mut stdin, &mut prefix_pending) {
                return Ok(());
            }

            // 2b. A sidebar toggle parked a grid refit: the chord has no
            //     flush sink, so the pump performs the resize_to here —
            //     its repaint_all is what erases the vacated region.
            if self.pending_grid_refit {
                self.pending_grid_refit = false;
                let (host_cols, host_rows) = super::conn::terminal_grid();
                let content_rows = host_rows.saturating_sub(1 + u16::from(self.status_bar_on));
                self.resize_to(conn, host_cols, content_rows, sink)?;
                // The refit reconstructed the renderer, dropping the
                // strip's sections; re-query so the next frame paints
                // them (a changed mark rides the same dirty flag).
                if self.sidebar_on {
                    self.refresh_sidebar(conn);
                }
            }

            // 3. Host resize (SIGWINCH): report the new grid, re-fit. The
            //    daemon's window renders into the rows between the tab
            //    strip and the status bar, so the size report carries the
            //    content height.
            let (host_cols, host_rows) = super::conn::terminal_grid();
            let content = (
                host_cols,
                host_rows.saturating_sub(1 + u16::from(self.status_bar_on)),
            );
            if content != current_size {
                current_size = content;
                self.resize_to(conn, content.0, content.1, sink)?;
                // A resize wiped the screen; the rows repaint whole.
                self.status_row.invalidate();
                self.draw_status_row();
                self.tab_strip.invalidate();
                self.draw_tab_strip();
            }

            // 3b. Status: the throttled re-query. Any %agent-state-changed
            //     / %agent-telemetry-changed / %sessions-changed (and
            //     renames) marked the state stale; one re-query per burst
            //     serves them all. The shown session being gone ends the
            //     view (docs/MUX.md's %sessions-changed client contract);
            //     only the shown WINDOW being gone (its last pane exited
            //     or was killed) lands on the session's active window
            //     instead — tmux semantics: the session outlives a tab.
            if self.status_dirty {
                self.status_dirty = false;
                let focused = self.renderer.focused().unwrap_or(0);
                match self.status.refresh(conn, &self.window, focused) {
                    Ok(()) => {
                        self.refresh_pane_titles(conn);
                        if self.sidebar_on {
                            self.refresh_sidebar(conn);
                        }
                    }
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
                            _ => return Ok(()),
                        }
                    }
                    Err(status::StatusError::Query) => {} // stale state survives; the next mark retries
                }
                self.draw_status_row();
                // The strip rides the same queried state: window
                // add/close/rename refreshes it here.
                self.draw_tab_strip();
            }

            // 4. Frame whatever accumulated (panes + the status row's own
            //    diff), then wait for the next push — the frame cadence
            //    floods coalesce into. The reload flash rides the frame
            //    cadence and clears after about a second of ticks.
            if self.flash.is_some() {
                self.flash_ticks += 1;
                if self.flash_ticks > 60 {
                    self.flash = None;
                    self.flash_ticks = 0;
                    self.draw_status_row();
                }
            }
            // 3c. Divider drag: apply the accumulated delta at frame
            //     cadence — one resize-pane per unapplied cell.
            self.apply_drag(conn);
            self.frame(sink);
            match conn.recv_timeout(super::POLL) {
                Ok(event) => {
                    if self.handle_event(event) == EventOutcome::End {
                        return Ok(());
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
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
                // Zoom truth is per-window and the daemon's: another
                // client's zoom (or a select-away unzoom) reaches this one
                // only through the flags, and a one-pane zoom leaves the
                // geometry unchanged — so the cue updates outside the
                // geometry guard below.
                let zoomed = window_raw_flags.contains('Z');
                if zoomed != self.zoomed {
                    self.zoomed = zoomed;
                    self.status_dirty = true;
                }
                match layout::parse_layout_triple(
                    &window_layout,
                    &window_visible_layout,
                    &window_raw_flags,
                ) {
                    Ok(next) if next != self.renderer.layout() => {
                        // A geometry change re-fit the daemon's panes; the
                        // local emulators re-fit (cleared) with the layout
                        // and re-seed from fresh replays — the pump's
                        // step 1b, since replay needs the connection.
                        self.pending_layout = Some(next);
                    }
                    Ok(_) => {}
                    Err(err) => eprintln!("par-mux: {err}"),
                }
                EventOutcome::Continue
            }
            TmuxNotification::Exit => EventOutcome::End,
            // The status facts: agent churn, session churn, and renames
            // all re-query (the contract's throttled re-query; one burst
            // of events collapses into one refresh in the pump's step 3b).
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
                    self.pending_follow_window = Some(window_id.clone());
                }
                self.status_dirty = true;
                EventOutcome::Continue
            }
            TmuxNotification::ClientSessionChanged { session_id, .. } => {
                // The displayed session moved (a workspace switch): park
                // the follow; the pump resolves the session's active
                // window by query and reseeds. Re-selecting here would
                // echo the notification back into a loop.
                self.pending_follow_session = Some(session_id.clone());
                self.status_dirty = true;
                EventOutcome::Continue
            }
            TmuxNotification::AgentStateChanged { .. }
            | TmuxNotification::AgentReleased { .. }
            | TmuxNotification::AgentTelemetryChanged { .. }
            | TmuxNotification::SessionsChanged
            | TmuxNotification::WorkspacesChanged
            | TmuxNotification::WindowRenamed { .. }
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
