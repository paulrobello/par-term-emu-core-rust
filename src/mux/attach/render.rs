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
use crate::mux::attach::layout;
use crate::mux::attach::layout::PaneRect;
use crate::terminal::Terminal;
use crate::tmux_control::TmuxNotification;
use ratatui::buffer::{Buffer, Cell as RtCell, CellDiffOption};
use ratatui::layout::Rect as RtRect;
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::mpsc::RecvTimeoutError;

/// How long the renderer waits between frames at most — the frame cadence
/// output floods coalesce into. Matches Phase A's pump poll interval.
pub const FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

/// The glyph set dividers draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    /// UTF-8 box drawing (`│ ─ ┼`): the default for any terminal that
    /// accepts UTF-8.
    Unicode,
    /// VT100 ACS spelling (`| - +`): the fallback for charset-hostile
    /// terminals.
    Ascii,
}

impl Glyphs {
    fn vertical(self) -> &'static str {
        match self {
            Glyphs::Unicode => "│",
            Glyphs::Ascii => "|",
        }
    }

    fn horizontal(self) -> &'static str {
        match self {
            Glyphs::Unicode => "─",
            Glyphs::Ascii => "-",
        }
    }

    fn cross(self) -> &'static str {
        match self {
            Glyphs::Unicode => "┼",
            Glyphs::Ascii => "+",
        }
    }
}

/// One pane's local emulator: the core [`Terminal`] fed the pane's wire
/// bytes, at the pane's layout geometry.
pub struct PaneEmulator {
    /// The pane id this emulator mirrors, the layout string's leaf number.
    pub pane_id: u32,
    term: Terminal,
}

impl PaneEmulator {
    /// A fresh emulator at `cols` x `rows`.
    pub fn new(pane_id: u32, cols: u16, rows: u16) -> Self {
        Self {
            pane_id,
            term: Terminal::new(cols as usize, rows as usize),
        }
    }

    /// The emulator's grid state.
    pub fn terminal(&self) -> &Terminal {
        &self.term
    }

    /// Feed the pane's byte stream — a `refresh-client` replay body or a
    /// `%output` chunk — through the emulator.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.term.process(bytes);
    }

    /// Re-fit to a new geometry (`refresh-client -C` re-division).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(cols as usize, rows as usize);
    }
}

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
    /// The frame being painted.
    buffer: Buffer,
    /// The last frame handed out; `render_frame` diffs against it.
    prev_buffer: Buffer,
    dirty: bool,
}

impl PaneRenderer {
    /// A renderer over a `width` x `height` window with no layout yet.
    pub fn new(width: u16, height: u16, glyphs: Glyphs) -> Self {
        let area = RtRect::new(0, 0, width, height);
        Self {
            emulators: HashMap::new(),
            layout: Vec::new(),
            focused: None,
            width,
            height,
            glyphs,
            buffer: Buffer::empty(area),
            prev_buffer: Buffer::empty(area),
            dirty: true,
        }
    }

    /// The current layout rects (window-relative), in leaf order.
    pub fn layout(&self) -> &[PaneRect] {
        &self.layout
    }

    /// The window extent the renderer paints.
    pub fn window_size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// One painted cell of the last frame — the test/introspection read
    /// of what the renderer would flush.
    pub fn cell(&self, x: u16, y: u16) -> Option<&RtCell> {
        if x < self.width && y < self.height {
            Some(&self.buffer[(x, y)])
        } else {
            None
        }
    }

    /// Install a new layout: pane terminals are created for new leaves,
    /// dropped for gone ones, and re-fit to their rect's geometry. Resize
    /// clears the emulator's grid — after a geometry change the daemon
    /// re-replays the pane (the client's resync), which re-seeds it.
    ///
    /// # Panics
    ///
    /// On an empty `layout`: a live window always has at least one pane
    /// (`%layout-change` never fires for an empty one), so an empty list
    /// is a caller contract violation, not a runtime state to degrade
    /// from.
    pub fn apply_layout(&mut self, layout: Vec<PaneRect>) {
        assert!(
            !layout.is_empty(),
            "apply_layout requires at least one pane"
        );
        let old: HashMap<u32, (u16, u16)> = self
            .layout
            .iter()
            .map(|r| (r.pane, (r.width, r.height)))
            .collect();
        let new_ids: Vec<u32> = layout.iter().map(|r| r.pane).collect();
        // Drop emulators for panes that left the layout.
        self.emulators.retain(|id, _| new_ids.contains(id));
        // Create / re-fit the rest.
        for rect in &layout {
            let changed = old.get(&rect.pane) != Some(&(rect.width, rect.height));
            let emulator = self
                .emulators
                .entry(rect.pane)
                .or_insert_with(|| PaneEmulator::new(rect.pane, rect.width, rect.height));
            if changed {
                emulator.resize(rect.width, rect.height);
            }
        }
        self.focused.get_or_insert(layout[0].pane);
        self.layout = layout;
        self.dirty = true;
    }

    /// Mark the focused pane (its dividers highlight). A pane the layout
    /// does not contain is ignored.
    pub fn focus(&mut self, pane: u32) {
        if self.layout.iter().any(|r| r.pane == pane) && self.focused != Some(pane) {
            self.focused = Some(pane);
            self.dirty = true;
        }
    }

    /// The focused pane id.
    pub fn focused(&self) -> Option<u32> {
        self.focused
    }

    /// Feed one pane's `%output` (or replay) bytes. A pane the layout does
    /// not contain is dropped — its output is not on screen.
    pub fn feed_output(&mut self, pane: u32, bytes: &[u8]) {
        if let Some(emulator) = self.emulators.get_mut(&pane) {
            emulator.feed(bytes);
            self.dirty = true;
        }
    }

    /// Whether any fed bytes or layout/focus changes have not been framed
    /// yet. A flood of ten thousand feeds between two frames still yields
    /// exactly one true here.
    pub fn needs_frame(&self) -> bool {
        self.dirty
    }

    /// Force the next `render_frame` to repaint everything (terminal
    /// restore after suspend, host resize without a layout change).
    pub fn mark_all_dirty(&mut self) {
        self.dirty = true;
    }

    /// Paint one frame and return the changed cells against the previous
    /// frame: `(x, y, cell)` triples, the exact list a flush loop writes.
    /// A no-op frame (nothing dirty, or the frame changed nothing) returns
    /// an empty diff.
    pub fn render_frame(&mut self) -> Vec<(u16, u16, RtCell)> {
        if !self.dirty {
            return Vec::new();
        }
        self.dirty = false;

        // Paint every pane's grid into its rect, then the dividers on top.
        let layout = self.layout.clone();
        for rect in &layout {
            self.paint_pane(rect);
        }
        self.paint_dividers();

        let diff = self
            .prev_buffer
            .diff(&self.buffer)
            .into_iter()
            .map(|(x, y, cell)| (x, y, cell.clone()))
            .collect::<Vec<_>>();
        // The painted frame becomes the next diff baseline. `self.buffer`
        // keeps the frame — tests and callers can read it — and the next
        // paint's per-cell `reset()` clears each rewritten cell, so no
        // whole-buffer reset is needed (the layout tiles the window).
        self.prev_buffer = self.buffer.clone();
        diff
    }

    /// Copy one pane's visible grid into the buffer at its rect.
    fn paint_pane(&mut self, rect: &PaneRect) {
        let Some(emulator) = self.emulators.get(&rect.pane) else {
            return;
        };
        let grid = emulator.terminal().active_grid();
        for row in 0..rect.height.min(grid.rows() as u16) {
            for col in 0..rect.width.min(grid.cols() as u16) {
                let Some(core_cell) = grid.get(col as usize, row as usize) else {
                    continue;
                };
                let (x, y) = (rect.x + col, rect.y + row);
                // The wide base already marked this spacer skip; painting
                // it would clear the mark (reset() clears diff_option).
                if core_cell.flags().wide_char_spacer() {
                    continue;
                }
                let cell = &mut self.buffer[(x, y)];
                cell.reset();
                // Grapheme cluster: base char plus combining marks.
                let mut symbol = String::from(core_cell.c());
                for comb in core_cell.combining() {
                    symbol.push(*comb);
                }
                cell.set_symbol(&symbol);
                cell.set_fg(map_color(core_cell.fg()));
                cell.set_bg(map_color(core_cell.bg()));
                cell.set_style(map_flags(core_cell.flags()));
                // A double-width base marks its right-hand spacer skip so
                // the diff's flush never draws into it.
                if core_cell.width() == 2 && col + 1 < rect.width {
                    self.buffer[(x + 1, y)].set_diff_option(CellDiffOption::Skip);
                }
            }
        }
    }

    /// Draw the dividers between adjacent layout rects. The daemon's
    /// geometry tiles exactly (no reserved gap), so a divider overlays the
    /// boundary column/row of the pane content — tmux's look on a grid the
    /// daemon divided gap-free. Vertical boundaries first, horizontals
    /// second, then crossing cells become the junction glyph.
    fn paint_dividers(&mut self) {
        if self.layout.len() < 2 {
            return;
        }
        let mut vertical: Vec<(u16, u16)> = Vec::new();
        let mut horizontal: Vec<(u16, u16)> = Vec::new();
        for (i, a) in self.layout.iter().enumerate() {
            for b in self.layout.iter().skip(i + 1) {
                // `a` ends where `b` starts along x, with row overlap: a
                // vertical boundary.
                if a.x + a.width == b.x && rows_overlap(a, b) {
                    for y in row_overlap(a, b) {
                        vertical.push((b.x.saturating_sub(1), y));
                    }
                }
                if b.x + b.width == a.x && rows_overlap(a, b) {
                    for y in row_overlap(a, b) {
                        vertical.push((a.x.saturating_sub(1), y));
                    }
                }
                // Same along y for a horizontal boundary.
                if a.y + a.height == b.y && cols_overlap(a, b) {
                    for x in col_overlap(a, b) {
                        horizontal.push((x, b.y.saturating_sub(1)));
                    }
                }
                if b.y + b.height == a.y && cols_overlap(a, b) {
                    for x in col_overlap(a, b) {
                        horizontal.push((x, a.y.saturating_sub(1)));
                    }
                }
            }
        }
        for (x, y) in &vertical {
            let cell = &mut self.buffer[(*x, *y)];
            cell.reset();
            cell.set_symbol(self.glyphs.vertical());
            cell.set_style(divider_style(self.focused, &self.layout, *x, *y));
        }
        for (x, y) in &horizontal {
            // A cell that is also a vertical boundary becomes the junction.
            if vertical.contains(&(*x, *y)) {
                let cell = &mut self.buffer[(*x, *y)];
                cell.set_symbol(self.glyphs.cross());
            } else {
                let cell = &mut self.buffer[(*x, *y)];
                cell.reset();
                cell.set_symbol(self.glyphs.horizontal());
                cell.set_style(divider_style(self.focused, &self.layout, *x, *y));
            }
        }
    }
}

/// The focused pane's adjacent dividers render bold; the rest dim — the
/// focused-pane highlight the card asks for.
fn divider_style(focused: Option<u32>, layout: &[PaneRect], x: u16, y: u16) -> RtStyle {
    let near_focus = focused.is_some_and(|focus| {
        layout.iter().filter(|r| r.pane == focus).any(|r| {
            // The divider cell borders the focused rect: within one column
            // of its x-extent (the boundary sits on either side of the
            // edge) on a row the rect spans, or the same along y.
            let right_edge = r.x + r.width;
            let bottom_edge = r.y + r.height;
            let x_adjacent = x + 1 >= r.x && x <= right_edge && (r.y..bottom_edge).contains(&y);
            let y_adjacent = y + 1 >= r.y && y <= bottom_edge && (r.x..right_edge).contains(&x);
            x_adjacent || y_adjacent
        })
    });
    if near_focus {
        RtStyle::default().add_modifier(RtModifier::BOLD)
    } else {
        RtStyle::default().add_modifier(RtModifier::DIM)
    }
}

fn rows_overlap(a: &PaneRect, b: &PaneRect) -> bool {
    a.y < b.y + b.height && b.y < a.y + a.height
}

fn cols_overlap(a: &PaneRect, b: &PaneRect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width
}

fn row_overlap(a: &PaneRect, b: &PaneRect) -> Vec<u16> {
    let start = a.y.max(b.y);
    let end = (a.y + a.height).min(b.y + b.height);
    (start..end).collect()
}

fn col_overlap(a: &PaneRect, b: &PaneRect) -> Vec<u16> {
    let start = a.x.max(b.x);
    let end = (a.x + a.width).min(b.x + b.width);
    (start..end).collect()
}

fn map_color(color: CoreColor) -> RtColor {
    match color {
        CoreColor::Named(named) => RtColor::Indexed(match named {
            NamedColor::Black => 0,
            NamedColor::Red => 1,
            NamedColor::Green => 2,
            NamedColor::Yellow => 3,
            NamedColor::Blue => 4,
            NamedColor::Magenta => 5,
            NamedColor::Cyan => 6,
            NamedColor::White => 7,
            NamedColor::BrightBlack => 8,
            NamedColor::BrightRed => 9,
            NamedColor::BrightGreen => 10,
            NamedColor::BrightYellow => 11,
            NamedColor::BrightBlue => 12,
            NamedColor::BrightMagenta => 13,
            NamedColor::BrightCyan => 14,
            NamedColor::BrightWhite => 15,
        }),
        CoreColor::Indexed(i) => RtColor::Indexed(i),
        CoreColor::Rgb(r, g, b) => RtColor::Rgb(r, g, b),
    }
}

fn map_flags(flags: &CellFlags) -> RtStyle {
    let mut modifiers = RtModifier::empty();
    if flags.bold() {
        modifiers |= RtModifier::BOLD;
    }
    if flags.dim() {
        modifiers |= RtModifier::DIM;
    }
    if flags.italic() {
        modifiers |= RtModifier::ITALIC;
    }
    if flags.underline() {
        modifiers |= RtModifier::UNDERLINED;
    }
    if flags.blink() {
        modifiers |= RtModifier::SLOW_BLINK;
    }
    if flags.reverse() {
        modifiers |= RtModifier::REVERSED;
    }
    if flags.hidden() {
        modifiers |= RtModifier::HIDDEN;
    }
    if flags.strikethrough() {
        modifiers |= RtModifier::CROSSED_OUT;
    }
    RtStyle::default().add_modifier(modifiers)
}

/// Sink the flush loop writes through, so the frame path is testable
/// headless (the real sink writes to stdout; a test records cells).
pub(crate) trait FlushSink {
    /// Write one frame's diff.
    fn flush(&mut self, diff: &[(u16, u16, RtCell)]);
    /// Bracket a full repaint (first frame, resize re-fit): the real sink
    /// clears the screen and hides the cursor here.
    fn repaint_all(&mut self);
}

/// The stdout sink: absolute CUP + SGR per diff cell — ratatui's diff is
/// exactly the minimal cell set, and each cell carries its full style, so
/// per-cell reset+SGR+CUP+glyph is correct if not maximal-minimal. Frame
/// cadence (16 ms) keeps the volume at TUI-ordinary levels.
struct StdoutSink;

impl FlushSink for StdoutSink {
    fn flush(&mut self, diff: &[(u16, u16, RtCell)]) {
        let mut out = String::with_capacity(diff.len() * 16);
        for (x, y, cell) in diff {
            // ratatui coordinates are 0-based; terminals are 1-based.
            let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
            write_styled(&mut out, cell);
            out.push_str(cell.symbol());
        }
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }

    fn repaint_all(&mut self) {
        let mut stdout = std::io::stdout().lock();
        // Clear, home, hide the cursor — the renderer owns the screen.
        let _ = stdout.write_all(b"\x1b[2J\x1b[H\x1b[?25l");
        let _ = stdout.flush();
    }
}

/// One cell's SGR run: reset, then emit only what differs from default.
fn write_styled(out: &mut String, cell: &RtCell) {
    out.push_str("\x1b[0m");
    if cell.fg != RtColor::Reset {
        write_color(out, cell.fg, true);
    }
    if cell.bg != RtColor::Reset {
        write_color(out, cell.bg, false);
    }
    if cell.modifier.contains(RtModifier::BOLD) {
        out.push_str("\x1b[1m");
    }
    if cell.modifier.contains(RtModifier::DIM) {
        out.push_str("\x1b[2m");
    }
    if cell.modifier.contains(RtModifier::ITALIC) {
        out.push_str("\x1b[3m");
    }
    if cell.modifier.contains(RtModifier::UNDERLINED) {
        out.push_str("\x1b[4m");
    }
    if cell.modifier.contains(RtModifier::REVERSED) {
        out.push_str("\x1b[7m");
    }
}

fn write_color(out: &mut String, color: RtColor, fg: bool) {
    let base = if fg { 38 } else { 48 };
    match color {
        RtColor::Reset => {}
        // The named variants only arise if a future mapping introduces
        // them; emit their closest indexed spelling so the match stays
        // exhaustive and the output stays 256-color-correct.
        RtColor::Black => {
            let _ = write!(out, "\x1b[{base};5;0m");
        }
        RtColor::Red => {
            let _ = write!(out, "\x1b[{base};5;1m");
        }
        RtColor::Green => {
            let _ = write!(out, "\x1b[{base};5;2m");
        }
        RtColor::Yellow => {
            let _ = write!(out, "\x1b[{base};5;3m");
        }
        RtColor::Blue => {
            let _ = write!(out, "\x1b[{base};5;4m");
        }
        RtColor::Magenta => {
            let _ = write!(out, "\x1b[{base};5;5m");
        }
        RtColor::Cyan => {
            let _ = write!(out, "\x1b[{base};5;6m");
        }
        RtColor::Gray => {
            let _ = write!(out, "\x1b[{base};5;7m");
        }
        RtColor::DarkGray => {
            let _ = write!(out, "\x1b[{base};5;8m");
        }
        RtColor::LightRed => {
            let _ = write!(out, "\x1b[{base};5;9m");
        }
        RtColor::LightGreen => {
            let _ = write!(out, "\x1b[{base};5;10m");
        }
        RtColor::LightYellow => {
            let _ = write!(out, "\x1b[{base};5;11m");
        }
        RtColor::LightBlue => {
            let _ = write!(out, "\x1b[{base};5;12m");
        }
        RtColor::LightMagenta => {
            let _ = write!(out, "\x1b[{base};5;13m");
        }
        RtColor::LightCyan => {
            let _ = write!(out, "\x1b[{base};5;14m");
        }
        RtColor::White => {
            let _ = write!(out, "\x1b[{base};5;15m");
        }
        RtColor::Indexed(i) => {
            let _ = write!(out, "\x1b[{base};5;{i}m");
        }
        RtColor::Rgb(r, g, b) => {
            let _ = write!(out, "\x1b[{base};2;{r};{g};{b}m");
        }
    }
}

/// The Phase B render-mode attach session: connect, seed the window's
/// panes from the daemon (layout via a size-report `%layout-change`,
/// per-pane state via the `refresh-client -t` replays), then pump
/// `%output` into the per-pane emulators at frame cadence until detach or
/// the connection ends.
pub(crate) fn run_render_session(options: &super::AttachOptions) -> std::process::ExitCode {
    match render_session_inner(options) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("par-mux: attach failed: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn render_session_inner(options: &super::AttachOptions) -> Result<(), String> {
    let path = options.socket_path();
    let mut conn = super::conn::AttachConn::connect(&path)
        .map_err(|_| format!("no daemon running on {}", path.display()))?;
    if let Some(warning) = &conn.warnings().stamp_mismatch {
        eprintln!("{warning}");
    }

    // The host grid — the renderer's window extent (the window-size
    // policy resizes the daemon's window to it). Raw mode + alternate
    // screen: unlike passthrough, the renderer owns the whole screen.
    let (cols, rows) = super::conn::terminal_grid();
    let _guard = super::TerminalGuard::enter();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen);

    let mut session = WindowSession::new(cols, rows);
    let outcome = session.run(&mut conn, options.target.as_deref(), &mut StdoutSink);

    // Restore: leave the alt screen, show the cursor. The TerminalGuard
    // (raw mode) drops after.
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    outcome?;
    Ok(())
}

/// One render-mode session's live state: the window being mirrored, its
/// renderer, and any layout change waiting to be applied (applying one
/// needs the connection for the re-seeding replays, so the event handler
/// parks it for the pump).
struct WindowSession {
    /// The window id this session mirrors, `@N`.
    window: String,
    renderer: PaneRenderer,
    /// A `%layout-change` whose re-fit + replay the pump still owes.
    pending_layout: Option<Vec<PaneRect>>,
}

impl WindowSession {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            window: String::new(),
            renderer: PaneRenderer::new(cols, rows, Glyphs::Unicode),
            pending_layout: None,
        }
    }

    /// Resolve the initial window, seed it, run the pump. `sink` receives
    /// the frames.
    fn run(
        &mut self,
        conn: &mut super::conn::AttachConn,
        target: Option<&str>,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        self.seed(conn, target, sink)?;
        self.pump(conn, sink)
    }

    /// Seed the window: resolve it from the target (a pane/window/session
    /// target narrows as in passthrough; none = the newest session's
    /// active window), report the renderer size against one of its panes
    /// (the `%layout-change` reply carries the layout triple), parse the
    /// layout, then replay every pane's screen into its emulator.
    fn seed(
        &mut self,
        conn: &mut super::conn::AttachConn,
        target: Option<&str>,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        // Registration replay (held panes, zoomed windows) is drained
        // ahead of the queries like passthrough does.
        let _replay = conn.drain_pending_events();

        let (window, pane) = resolve_window_and_pane(conn, target)?;
        self.window = window;

        // Size report against the window: the daemon resizes the window
        // to the renderer's grid (the T4.C window-size policy) and the
        // queued `%layout-change` carries the current layout triple —
        // including the Z flag and visible layout when zoomed.
        let (cols, rows) = self.renderer.window_size();
        conn.send_checked(&format!("refresh-client -t {pane} -C {cols}x{rows}"))
            .map_err(|err| format!("refresh-client failed: {err}"))?;
        let layout_event = conn
            .drain_pending_events()
            .into_iter()
            .find_map(|event| match event {
                TmuxNotification::LayoutChange {
                    window_id,
                    window_layout,
                    window_visible_layout,
                    window_raw_flags,
                } if window_id == self.window => {
                    Some((window_layout, window_visible_layout, window_raw_flags))
                }
                _ => None,
            })
            .ok_or_else(|| {
                format!(
                    "no %layout-change for {window} — the daemon predates layout broadcast",
                    window = self.window
                )
            })?;
        let layout = layout::parse_layout_triple(&layout_event.0, &layout_event.1, &layout_event.2)
            .map_err(|err| err.to_string())?;
        self.renderer.apply_layout(layout);

        // Replay every visible pane's state into its emulator: grid,
        // scrollback, cursor, and input modes ride the screen-restore
        // byte stream, the same stream passthrough writes to the host.
        self.replay_all_panes(conn);
        // First frame: clear + full paint.
        sink.repaint_all();
        self.frame(sink);
        Ok(())
    }

    /// The pump: route daemon pushes into the renderer, poll the host
    /// size (SIGWINCH lands as a size change), frame at cadence.
    fn pump(
        &mut self,
        conn: &mut super::conn::AttachConn,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        let mut stdin = super::Stdin::new();
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
                self.renderer.apply_layout(layout);
                self.replay_all_panes(conn);
            }

            // 2. Stdin: prefix d detaches; everything else forwards to
            //    the focused pane in chunked send-keys -H.
            if self.pump_stdin(conn, &mut stdin, &mut prefix_pending) {
                return Ok(());
            }

            // 3. Host resize (SIGWINCH): report the new grid, re-fit.
            let size = super::conn::terminal_grid();
            if size != current_size {
                current_size = size;
                self.resize_to(conn, size.0, size.1, sink)?;
            }

            // 4. Frame whatever accumulated, then wait for the next push
            //    — the frame cadence floods coalesce into.
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
            // Everything else — lifecycle elsewhere, agent churn, paste
            // buffers — is not this window renderer's concern yet.
            _ => EventOutcome::Continue,
        }
    }

    fn feed_pane(&mut self, pane_id: &str, data: &[u8]) {
        if let Some(n) = pane_id.strip_prefix('%').and_then(|n| n.parse().ok()) {
            self.renderer.feed_output(n, data);
        }
    }

    /// Replay every visible pane's state into its emulator (the seed and
    /// the post-resize re-seed share this).
    fn replay_all_panes(&mut self, conn: &mut super::conn::AttachConn) {
        for rect in self.renderer.layout().to_vec() {
            let pane = format!("%{}", rect.pane);
            if let Ok(reply) = conn.send_checked(&format!("refresh-client -t {pane}")) {
                if reply.ok {
                    let mut bytes = reply.body.join("\n").into_bytes();
                    bytes.push(b'\n');
                    self.renderer.feed_output(rect.pane, &bytes);
                }
            }
        }
    }

    /// Stdin pump: prefix routing (d detaches), chunked forward to the
    /// focused pane. Returns true to end the session.
    fn pump_stdin(
        &mut self,
        conn: &mut super::conn::AttachConn,
        stdin: &mut super::Stdin,
        prefix_pending: &mut bool,
    ) -> bool {
        loop {
            match stdin.read_available() {
                None => return false,
                Some(Ok(bytes)) if bytes.is_empty() => return true, // EOF
                Some(Ok(bytes)) => {
                    let mut to_send: Vec<u8> = Vec::with_capacity(bytes.len());
                    let mut detach = false;
                    for byte in bytes {
                        if *prefix_pending {
                            *prefix_pending = false;
                            if byte == b'd' {
                                detach = true;
                            } else if byte == super::C_B {
                                to_send.push(byte); // literal prefix
                            }
                            // Other Phase B prefix commands land with
                            // their cards; unbound keys are consumed.
                        } else if byte == super::C_B {
                            *prefix_pending = true;
                        } else {
                            to_send.push(byte);
                        }
                    }
                    if detach {
                        return true;
                    }
                    if !to_send.is_empty() {
                        super::forward_chunked(conn, self.focused_pane(), &to_send);
                    }
                }
                Some(Err(_)) => return true,
            }
        }
    }

    fn focused_pane(&self) -> String {
        self.renderer
            .focused()
            .map(|n| format!("%{n}"))
            .unwrap_or_default()
    }

    /// Host resize: report the new grid against the window (the daemon
    /// re-divides and re-broadcasts the layout), re-fit, repaint all.
    fn resize_to(
        &mut self,
        conn: &mut super::conn::AttachConn,
        cols: u16,
        rows: u16,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        let pane = self.focused_pane();
        if pane.is_empty() {
            return Ok(());
        }
        conn.send_checked(&format!("refresh-client -t {pane} -C {cols}x{rows}"))
            .map_err(|err| format!("resize report failed: {err}"))?;
        self.renderer = PaneRenderer::new(cols, rows, Glyphs::Unicode);
        // The layout broadcast the report queued re-seeds the panes; but
        // drain it here directly so the repaint is synchronous.
        let layout_event = conn
            .drain_pending_events()
            .into_iter()
            .find_map(|event| match event {
                TmuxNotification::LayoutChange {
                    window_id,
                    window_layout,
                    window_visible_layout,
                    window_raw_flags,
                } if window_id == self.window => {
                    Some((window_layout, window_visible_layout, window_raw_flags))
                }
                _ => None,
            });
        if let Some((l, v, f)) = layout_event {
            if let Ok(layout) = layout::parse_layout_triple(&l, &v, &f) {
                self.renderer.apply_layout(layout);
                for rect in self.renderer.layout().to_vec() {
                    let pane = format!("%{}", rect.pane);
                    if let Ok(reply) = conn.send_checked(&format!("refresh-client -t {pane}")) {
                        if reply.ok {
                            let mut bytes = reply.body.join("\n").into_bytes();
                            bytes.push(b'\n');
                            self.renderer.feed_output(rect.pane, &bytes);
                        }
                    }
                }
            }
        }
        sink.repaint_all();
        self.frame(sink);
        Ok(())
    }

    /// Frame the pending output at cadence.
    fn frame(&mut self, sink: &mut dyn FlushSink) {
        if self.renderer.needs_frame() {
            let diff = self.renderer.render_frame();
            if !diff.is_empty() {
                sink.flush(&diff);
            }
        }
    }
}

/// What handling one event told the pump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventOutcome {
    Continue,
    End,
}

/// Resolve a render target to `(window, one-of-its-panes)`. Pane targets
/// go through pane-info (its second field is the window); window/session
/// targets through the targeted list queries; none = the newest session's
/// active window's active pane — the same newest stand-in passthrough
/// uses.
fn resolve_window_and_pane(
    conn: &mut super::conn::AttachConn,
    target: Option<&str>,
) -> Result<(String, String), String> {
    match target {
        None => {
            let sessions = conn
                .send_checked("list-sessions")
                .map_err(|err| format!("list-sessions failed: {err}"))?;
            let session = sessions
                .body
                .iter()
                .find(|l| l.starts_with('$'))
                .and_then(|l| l.split_whitespace().next())
                .ok_or("no sessions exist — create one first")?;
            let windows = conn
                .send_checked(&format!("list-windows -t {session}"))
                .map_err(|err| format!("list-windows failed: {err}"))?;
            let window = windows
                .body
                .iter()
                .find(|l| l.split_whitespace().nth(1) == Some("*"))
                .or_else(|| windows.body.iter().find(|l| l.starts_with('@')))
                .and_then(|l| l.split_whitespace().next())
                .ok_or("the session has no windows")?;
            let pane = marked_pane_of(conn, window)?;
            Ok((window.to_string(), pane))
        }
        Some(target) => {
            // A pane: pane-info's second field names the window.
            if let Ok(reply) = conn.send_checked(&format!("pane-info -t {target}")) {
                if reply.ok {
                    if let Some(line) = reply.body.first() {
                        let mut fields = line.split_whitespace();
                        if let (Some(pane), Some(window)) = (fields.next(), fields.next()) {
                            if pane.starts_with('%') && window.starts_with('@') {
                                return Ok((window.to_string(), pane.to_string()));
                            }
                        }
                    }
                }
            }
            // A window: its marked pane.
            if let Ok(reply) = conn.send_checked(&format!("list-panes -t {target}")) {
                if reply.ok {
                    let pane = marked_pane_of(conn, target)?;
                    return Ok((target.to_string(), pane));
                }
            }
            // A session: its active window.
            if let Ok(reply) = conn.send_checked(&format!("list-windows -t {target}")) {
                if reply.ok {
                    let window = reply
                        .body
                        .iter()
                        .find(|l| l.split_whitespace().nth(1) == Some("*"))
                        .or_else(|| reply.body.iter().find(|l| l.starts_with('@')))
                        .and_then(|l| l.split_whitespace().next())
                        .ok_or("the session has no windows")?;
                    let pane = marked_pane_of(conn, window)?;
                    return Ok((window.to_string(), pane));
                }
            }
            Err(format!("no such target: {target}"))
        }
    }
}

/// The `*`-marked pane of a window (its active pane), else the first row.
fn marked_pane_of(conn: &mut super::conn::AttachConn, window: &str) -> Result<String, String> {
    let reply = conn
        .send_checked(&format!("list-panes -t {window}"))
        .map_err(|err| format!("list-panes failed: {err}"))?;
    reply
        .body
        .iter()
        .find(|l| l.split_whitespace().nth(2) == Some("*"))
        .or_else(|| reply.body.iter().find(|l| l.starts_with('%')))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string)
        .ok_or_else(|| format!("no panes under {window}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::attach::layout::{parse_layout, parse_layout_triple};

    /// The daemon's actual render output for a 50/50 vertical split of an
    /// 80x24 window (LayoutTree::render's collapsed N-ary form), matching
    /// src/mux/layout.rs's own test expectations.
    const TWO_PANE_LAYOUT: &str = "0000,80x24,0,0{40x24,0,0,1,40x24,40,0,2}";

    /// Criterion 2: two panes fed distinct content render at the rects
    /// the daemon's layout names — pane 1's bytes land in cols 0..40,
    /// pane 2's in cols 40..80, same rows.
    #[test]
    fn two_pane_split_renders_at_daemon_rects() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);

        // Pane 1: "LEFT", pane 2: "RIGHT" — through their real emulators,
        // the same bytes a replay/%output carries.
        renderer.feed_output(1, b"LEFT\r\n");
        renderer.feed_output(2, b"RIGHT\r\n");
        assert!(renderer.needs_frame());
        let diff = renderer.render_frame();
        assert!(!diff.is_empty());
        assert!(!renderer.needs_frame(), "frame cleared the dirty flag");

        let buffer_text = |x: u16, y: u16, n: u16| -> String {
            (x..x + n)
                .map(|col| renderer.buffer[(col, y)].symbol())
                .collect()
        };
        assert_eq!(buffer_text(0, 0, 4), "LEFT");
        assert_eq!(buffer_text(40, 0, 5), "RIGHT");

        // The dividers: one vertical boundary at col 39 across the row
        // overlap, drawn as UTF-8 box drawing.
        assert_eq!(renderer.buffer[(39, 0)].symbol(), "│");
        assert_eq!(renderer.buffer[(39, 23)].symbol(), "│");
        // No horizontal dividers in this layout.
        assert_eq!(renderer.buffer[(10, 11)].symbol(), " ");
    }

    /// Cell mapping: styled bytes (`capture-pane -e` equivalent — the
    /// emulator saw the same SGR the -e capture encodes) map to the
    /// ratatui cell with fg/bg/attrs intact, truecolor passing through.
    #[test]
    fn styled_cells_map_fg_bg_and_attrs() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        // Bold truecolor text on SGR-44 blue — exactly what capture-pane
        // -e would carry back as escape bytes.
        renderer.feed_output(1, b"\x1b[1;38;2;10;20;30;44mAB\x1b[0m normal \r\n");
        renderer.render_frame();

        let a = &renderer.buffer[(0, 0)];
        assert_eq!(a.symbol(), "A");
        assert_eq!(a.fg, RtColor::Rgb(10, 20, 30), "truecolor fg passthrough");
        assert_eq!(a.bg, RtColor::Indexed(4), "SGR 44 -> indexed blue bg");
        assert!(a.modifier.contains(RtModifier::BOLD));

        let n = &renderer.buffer[(3, 0)];
        assert_eq!(n.symbol(), "n");
        assert_eq!(n.fg, RtColor::Indexed(7), "post-reset default fg");
    }

    /// Criterion 3: an output flood between two frames coalesces into one
    /// frame — the dirty flag is set-once, the first render consumes it,
    /// and the second render diffs empty.
    #[test]
    fn output_flood_coalesces_at_frame_cadence() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.render_frame(); // settle the layout frame

        // 1000 floods of one line each — a realistic `cat bigfile` burst.
        for i in 0..1000 {
            renderer.feed_output(1, format!("line {i}\r\n").as_bytes());
        }
        renderer.feed_output(2, b"tail\r\n");
        assert!(renderer.needs_frame());

        let diff = renderer.render_frame();
        assert!(
            !diff.is_empty(),
            "the flood's final state reaches the frame"
        );
        assert!(!renderer.needs_frame(), "one frame consumed the flood");
        assert_eq!(
            renderer.render_frame(),
            Vec::new(),
            "the second frame is a no-op — no unbounded redraws"
        );
        // The flood scrolled 1000 lines through 24 rows: the last line
        // written ("line 999" at row 23) moved up one row when its \r\n
        // scrolled the grid, leaving row 22 as the final text row.
        let last = (0..6)
            .map(|c| renderer.buffer[(c, 22)].symbol())
            .collect::<String>();
        assert_eq!(last, "line 9");
    }

    /// Zoomed windows: the Z-flag triple's visible layout is the zoomed
    /// pane alone, full-window; unzooming restores the split and keeps
    /// surviving emulators' grids.
    #[test]
    fn zoomed_triple_renders_one_full_window_pane() {
        let layout = crate::mux::attach::layout::parse_layout_triple(
            TWO_PANE_LAYOUT,
            "0000,80x24,0,0,2",
            "Z",
        )
        .expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.feed_output(2, b"ZOOMED\r\n");
        renderer.render_frame();

        let zoomed = (0..6)
            .map(|c| renderer.buffer[(c, 0)].symbol())
            .collect::<String>();
        assert_eq!(zoomed, "ZOOMED");
        assert_eq!(renderer.layout().len(), 1, "one visible pane while zoomed");

        // Unzoom: the split layout returns and pane 2's grid survives
        // (its emulator was kept, not dropped).
        let split = parse_layout_triple(TWO_PANE_LAYOUT, TWO_PANE_LAYOUT, "").expect("parses");
        renderer.apply_layout(split);
        renderer.feed_output(1, b"LEFT\r\n");
        renderer.render_frame();
        let left = (0..6)
            .map(|c| renderer.buffer[(c, 0)].symbol())
            .collect::<String>();
        assert_eq!(left, "LEFT  ", "pane 1 on the left half");
        let right = (40..46)
            .map(|c| renderer.buffer[(c, 0)].symbol())
            .collect::<String>();
        assert_eq!(right, "ZOOMED", "pane 2 kept its grid through the zoom");
    }

    /// Pane membership changes drop only the gone panes' emulators.
    #[test]
    fn layout_change_drops_departed_panes() {
        let two = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(two);
        renderer.feed_output(1, b"keep\r\n");
        renderer.render_frame();

        // Pane 1 closes: the layout collapses to pane 2 alone.
        let one = parse_layout("0000,80x24,0,0,2").expect("parses");
        renderer.apply_layout(one);
        renderer.render_frame();
        let solo = (0..4)
            .map(|c| renderer.buffer[(c, 0)].symbol())
            .collect::<String>();
        assert_eq!(solo, "    ", "pane 2's grid is empty here");
        assert!(renderer.focused().is_some());

        // Back to two panes: pane 1's emulator was dropped with its
        // membership, so its old content does not return.
        renderer.apply_layout(parse_layout(TWO_PANE_LAYOUT).unwrap());
        renderer.render_frame();
        let back = (0..4)
            .map(|c| renderer.buffer[(c, 0)].symbol())
            .collect::<String>();
        assert_eq!(back, "    ");
    }

    /// Ascii glyphs: the ACS fallback spells dividers with | - +.
    #[test]
    fn ascii_glyphs_fallback() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Ascii);
        renderer.apply_layout(layout);
        renderer.render_frame();
        assert_eq!(renderer.buffer[(39, 0)].symbol(), "|");
    }

    /// Focus: the focused pane's boundary dividers are bold, the others
    /// dim; focusing an absent pane changes nothing. The 3-pane N-ary
    /// layout has two boundaries — at cols 29 and 59 — so the unfocused
    /// one is observable.
    #[test]
    fn focus_highlights_adjacent_dividers() {
        const THREE_PANE: &str = "0000,90x24,0,0{30x24,0,0,1,30x24,30,0,2,30x24,60,0,3}";
        let layout = parse_layout(THREE_PANE).expect("parses");
        let mut renderer = PaneRenderer::new(90, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.focus(1);
        renderer.render_frame();
        assert!(
            renderer.buffer[(29, 0)].modifier.contains(RtModifier::BOLD),
            "focused pane 1's right divider is bold"
        );
        assert!(
            renderer.buffer[(59, 0)].modifier.contains(RtModifier::DIM),
            "the pane2|pane3 divider (no focused neighbor) is dim"
        );

        renderer.focus(2);
        renderer.mark_all_dirty();
        renderer.render_frame();
        assert!(
            renderer.buffer[(29, 0)].modifier.contains(RtModifier::BOLD),
            "moving focus to pane 2: both its dividers highlight"
        );
        assert!(
            renderer.buffer[(59, 0)].modifier.contains(RtModifier::BOLD),
            "moving focus to pane 2: both its dividers highlight"
        );

        // An absent pane is ignored.
        renderer.focus(99);
        assert_eq!(renderer.focused(), Some(2));
    }

    /// Wide characters: a CJK cell occupies its rect cell and marks the
    /// right spacer skip, and the neighbor's content still lands.
    #[test]
    fn wide_chars_mark_their_spacer() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.feed_output(1, "世X\r\n".as_bytes());
        renderer.render_frame();
        assert_eq!(renderer.buffer[(0, 0)].symbol(), "世");
        assert_eq!(
            renderer.buffer[(1, 0)].diff_option,
            CellDiffOption::Skip,
            "wide-char spacer is skip"
        );
        assert_eq!(renderer.buffer[(2, 0)].symbol(), "X");
    }

    /// An empty layout list is a caller contract violation and asserts.
    #[test]
    #[should_panic(expected = "at least one pane")]
    fn empty_layout_asserts() {
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(Vec::new());
    }

    /// Combining marks ride the symbol: the core terminal normalizes
    /// e + U+0301 to the precomposed é, which lands as one cell.
    #[test]
    fn combining_marks_join_the_symbol() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.feed_output(1, "e\u{0301}x\r\n".as_bytes());
        renderer.render_frame();
        assert_eq!(renderer.buffer[(0, 0)].symbol(), "\u{e9}");
        assert_eq!(renderer.buffer[(1, 0)].symbol(), "x");
    }
}
