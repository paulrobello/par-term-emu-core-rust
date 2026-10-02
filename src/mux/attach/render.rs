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
use crate::keyboard::TermKeyEvent;
use crate::mouse::MouseMode;
use crate::mux::attach::input::{InputParser, SgrMouse, Token};
use crate::mux::attach::layout;
use crate::mux::attach::layout::PaneRect;
use crate::mux::attach::status::{self, Segment, StatusRow};
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

    /// The pane rect containing window-relative `(x, y)`, else `None`
    /// (dividers belong to no pane).
    pub fn pane_at(&self, x: u16, y: u16) -> Option<&PaneRect> {
        self.layout
            .iter()
            .find(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
    }

    /// A pane's tracked input-mode state (its emulator's terminal), by
    /// pane id — what the input router reads to re-encode keys and decide
    /// mouse ownership. Unknown pane id → `None`.
    pub fn pane_terminal(&self, pane: u32) -> Option<&Terminal> {
        self.emulators.get(&pane).map(|e| e.terminal())
    }

    /// Reset a pane's client-side scroll offset to live (0). Typing
    /// forwards, so the view snaps back to the bottom.
    pub fn snap_to_live(&mut self, pane: u32) {
        if let Some(emulator) = self.emulators.get_mut(&pane) {
            if emulator.scroll_offset() != 0 {
                emulator.scroll_by(-(emulator.scroll_offset() as isize));
                self.dirty = true;
            }
        }
    }

    /// Enter the keyboard scroll viewport on `pane`: hold the offset
    /// against pane output, jump the view one viewport (the pane's row
    /// count) up from live — the reading position a wheel-up would have
    /// reached — and repaint. `false` when the pane is unknown or has no
    /// scrollback (nothing to scroll).
    pub fn enter_scroll_mode(&mut self, pane: u32) -> bool {
        let rows = {
            let Some(emulator) = self.emulators.get(&pane) else {
                return false;
            };
            if emulator.terminal().active_grid().scrollback_len() == 0 {
                return false;
            }
            emulator.terminal().active_grid().rows().max(1)
        };
        let Some(emulator) = self.emulators.get_mut(&pane) else {
            return false;
        };
        emulator.set_scroll_hold(true);
        emulator.scroll_by(rows as isize);
        self.dirty = true;
        true
    }

    /// Whether `pane`'s scroll viewport is currently held (in scroll
    /// mode). Unknown pane → false.
    pub fn scroll_mode_active(&self, pane: u32) -> bool {
        self.emulators
            .get(&pane)
            .is_some_and(|e| e.scroll_offset() > 0 || e.hold_scroll)
    }

    /// `pane`'s client scroll offset (0 for an unknown pane).
    pub fn scroll_offset_of(&self, pane: u32) -> usize {
        self.emulators
            .get(&pane)
            .map(|e| e.scroll_offset())
            .unwrap_or(0)
    }

    /// Drive the scroll viewport on `pane` by `delta` lines (positive =
    /// up into history). Returns the resulting offset; 0 for an unknown
    /// pane.
    pub fn scroll_viewport(&mut self, pane: u32, delta: isize) -> usize {
        let Some(emulator) = self.emulators.get_mut(&pane) else {
            return 0;
        };
        let after = emulator.scroll_by(delta);
        self.dirty = true;
        after
    }
    /// Leave scroll mode on `pane`: clear the hold and snap to live.
    /// Repaints when anything changes.
    pub fn exit_scroll_mode(&mut self, pane: u32) {
        let Some(emulator) = self.emulators.get_mut(&pane) else {
            return;
        };
        let was_held = emulator.hold_scroll;
        emulator.set_scroll_hold(false);
        if was_held || emulator.scroll_offset() != 0 {
            emulator.scroll_to_live();
            self.dirty = true;
        }
    }

    /// Wheel-scroll the pane under `(x, y)` client-side by `delta` lines
    /// (positive = up into history). Returns `false` when the point is in
    /// no pane or that pane OWNS mouse tracking (its wheel is the pane's
    /// to consume, so the router forwards the event instead).
    pub fn wheel_scroll(&mut self, x: u16, y: u16, delta: isize) -> bool {
        let rect = self.pane_at(x, y).cloned();
        let pane_id = rect.as_ref().map(|r| r.pane);
        let Some(pane_id) = pane_id else {
            return false;
        };
        let Some(emulator) = self.emulators.get_mut(&pane_id) else {
            return false;
        };
        if emulator.owns_mouse() {
            return false;
        }
        let before = emulator.scroll_offset();
        let after = emulator.scroll_by(delta);
        let moved = after != before;
        if moved {
            self.dirty = true;
        }
        moved
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

    /// Copy one pane's visible grid into the buffer at its rect. A pane
    /// scrolled client-side (the wheel path) paints its scrollback: with
    /// offset S, the view shifts up S lines — the rect's TOP S rows show
    /// the newest S scrollback lines (logical index oldest-first:
    /// `len - S + row`), and live grid row `row - S` fills below. The
    /// emulator's grid is the rect's size, so the index math stays exact.
    fn paint_pane(&mut self, rect: &PaneRect) {
        let Some(emulator) = self.emulators.get(&rect.pane) else {
            return;
        };
        let grid = emulator.terminal().active_grid();
        let scroll = emulator.scroll_offset();
        let scrollback_len = grid.scrollback_len() as isize;
        for row in 0..rect.height.min(grid.rows() as u16) {
            // View row r: live grid row r - S when r >= S; otherwise the
            // scrollback line S_len - S + r (newest history first).
            let scrollback_row: isize = scrollback_len - scroll as isize + row as isize;
            let in_history = (row as usize) < scroll;
            for col in 0..rect.width.min(grid.cols() as u16) {
                let core_cell: &crate::cell::Cell = if in_history {
                    let logical = scrollback_row.max(0) as usize;
                    match grid.scrollback_line(logical) {
                        Some(line) if usize::from(col) < line.len() => &line[col as usize],
                        _ => continue,
                    }
                } else {
                    let grid_row = row as isize - scroll as isize;
                    if grid_row < 0 {
                        continue;
                    }
                    match grid.get(col as usize, grid_row as usize) {
                        Some(cell) => cell,
                        None => continue,
                    }
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
    /// The bottom row: queried state + paint/diff pair.
    status: status::StatusState,
    status_row: status::StatusRow,
    /// Whether the status state is stale and needs a re-query before the
    /// next paint (the throttled re-query on agent/sessions churn).
    status_dirty: bool,
    /// Whether scroll mode is up on the focused pane.
    scroll_mode: bool,
    /// The reload chord: the key byte matched after the prefix, and the
    /// detach prefix itself — both live-rebindable by the reload.
    prefix: u8,
    reload_key: u8,
    /// The literal prefix byte to forward when the user types prefix
    /// prefix (rebindable, so it is state, not the C_B constant).
    literal: u8,
    /// A transient confirmation/error cue drawn in place of the status
    /// line's head and cleared after about a second.
    flash: Option<String>,
    /// The flash's remaining lifetime in frame ticks.
    flash_ticks: u32,
}

impl WindowSession {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            window: String::new(),
            renderer: PaneRenderer::new(cols, rows.saturating_sub(1), Glyphs::Unicode),
            pending_layout: None,
            status: status::StatusState::default(),
            status_row: status::StatusRow::new(cols),
            status_dirty: true,
            scroll_mode: false,
            prefix: super::C_B,
            reload_key: 0x12, // C-r
            literal: super::C_B,
            flash: None,
            flash_ticks: 0,
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
        // The status bar seeds with the view.
        let focused = self.renderer.focused().unwrap_or(0);
        if let Err(status::StatusError::SessionGone) =
            self.status.refresh(conn, &self.window, focused)
        {
            return Err("the target's session is gone".to_string());
        }
        self.status_row.invalidate();
        self.draw_status_row();
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

            // 2. Stdin: prefix routing (d detaches, [ enters scroll mode,
            //    n/p/(/) switch windows/sessions), then keys/mouse to the
            //    focused pane.
            if self.pump_stdin(conn, &mut stdin, &mut prefix_pending) {
                return Ok(());
            }

            // 3. Host resize (SIGWINCH): report the new grid, re-fit. The
            //    daemon's window renders into the rows above the status
            //    bar, so the size report carries the content height.
            let (host_cols, host_rows) = super::conn::terminal_grid();
            let content = (host_cols, host_rows.saturating_sub(1));
            if content != current_size {
                current_size = content;
                self.resize_to(conn, content.0, content.1, sink)?;
                // A resize wiped the screen; the row repaints whole.
                self.status_row.invalidate();
                self.draw_status_row();
            }

            // 3b. Status: the throttled re-query. Any %agent-state-changed
            //     / %agent-telemetry-changed / %sessions-changed (and
            //     renames) marked the state stale; one re-query per burst
            //     serves them all. The shown session being gone ends the
            //     view (docs/MUX.md's %sessions-changed client contract).
            if self.status_dirty {
                self.status_dirty = false;
                let focused = self.renderer.focused().unwrap_or(0);
                match self.status.refresh(conn, &self.window, focused) {
                    Ok(()) => {}
                    Err(status::StatusError::SessionGone) => return Ok(()),
                    Err(status::StatusError::Query) => {} // stale state survives; the next mark retries
                }
                self.draw_status_row();
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
            // The status facts: agent churn, session churn, and renames
            // all re-query (the contract's throttled re-query; one burst
            // of events collapses into one refresh in the pump's step 3b).
            TmuxNotification::AgentStateChanged { .. }
            | TmuxNotification::AgentReleased { .. }
            | TmuxNotification::AgentTelemetryChanged { .. }
            | TmuxNotification::SessionsChanged
            | TmuxNotification::WindowRenamed { .. }
            | TmuxNotification::SessionRenamed { .. }
            | TmuxNotification::WindowPaneChanged { .. }
            | TmuxNotification::SessionWindowChanged { .. } => {
                self.status_dirty = true;
                EventOutcome::Continue
            }
            // Everything else — lifecycle elsewhere, paste buffers — is
            // not this window renderer's concern yet.
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

    /// Stdin pump: parse tokens, route the prefix (d detaches, C-b C-b
    /// sends the literal), re-encode keys against the focused pane's
    /// input modes, and route mouse reports (click focus, wheel
    /// scrollback, pane-relative forwarding). Returns true to end the
    /// session.
    fn pump_stdin(
        &mut self,
        conn: &mut super::conn::AttachConn,
        stdin: &mut super::Stdin,
        prefix_pending: &mut bool,
    ) -> bool {
        let mut parser = InputParser::default();
        loop {
            match stdin.read_available() {
                None => return false,
                Some(Ok(bytes)) if bytes.is_empty() => return true, // EOF
                Some(Ok(bytes)) => {
                    for token in parser.feed(&bytes) {
                        match token {
                            Token::Bytes(run) => {
                                if self.route_plain(&run, conn, prefix_pending) {
                                    return true;
                                }
                            }
                            Token::Key(ev) => {
                                if *prefix_pending {
                                    *prefix_pending = false;
                                    // A prefix chord on a functional
                                    // key: unbound in Phase B — consumed.
                                    continue;
                                }
                                if self.scroll_mode {
                                    self.scroll_mode_key(&ev);
                                    continue;
                                }
                                let focused = self.focused_pane();
                                let bytes = self
                                    .renderer
                                    .focused()
                                    .and_then(|id| self.renderer.pane_terminal(id))
                                    .map(|term| crate::keyboard::encode_key(&ev, term))
                                    .unwrap_or_default();
                                if !bytes.is_empty() && !focused.is_empty() {
                                    super::forward_chunked(conn, focused, &bytes);
                                }
                            }
                            Token::Mouse(mouse) => {
                                self.route_mouse(conn, mouse);
                            }
                        }
                    }
                }
                Some(Err(_)) => return true,
            }
        }
    }

    /// A run of plain bytes through the prefix scanner; non-prefix bytes
    /// forward to the focused pane verbatim (the host already encoded
    /// them). Returns true on detach.
    fn route_plain(
        &mut self,
        bytes: &[u8],
        conn: &mut super::conn::AttachConn,
        prefix_pending: &mut bool,
    ) -> bool {
        let mut to_send: Vec<u8> = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if *prefix_pending {
                *prefix_pending = false;
                // The reload chord matches by byte before the fixed
                // table (configurable; the default C-r does not collide
                // with the literal-key arms).
                if byte == self.reload_key && byte != b'd' {
                    self.reload_config(conn);
                    continue;
                }
                match byte {
                    b'd' => return true,
                    b'[' => {
                        // prefix [ — the scroll viewport on the focused
                        // pane. No scrollback means nothing to scroll;
                        // the key is consumed either way.
                        if let Some(id) = self.renderer.focused() {
                            if self.renderer.enter_scroll_mode(id) {
                                self.scroll_mode = true;
                            }
                        }
                    }
                    b'n' | b'p' | b'(' | b')' | b'o' => self.prefix_switch(byte, conn),
                    b if b == self.literal => to_send.push(byte), // literal prefix
                    _ => {}                                       // unbound: consumed
                }
            } else if byte == self.prefix {
                *prefix_pending = true;
            } else if self.scroll_mode {
                // Scroll mode's plain keys: q and Enter exit (the
                // viewport is a modal view — keys do not leak into the
                // pane).
                if byte == b'q' || byte == b'\r' {
                    self.leave_scroll_mode();
                }
            } else {
                to_send.push(byte);
            }
        }
        // Typing snaps this pane's client scroll back to live.
        if !to_send.is_empty() {
            if let Some(id) = self.renderer.focused() {
                self.renderer.snap_to_live(id);
            }
            super::forward_chunked(conn, self.focused_pane(), &to_send);
        }
        false
    }

    /// The prefix commands that move the view through the daemon's tree:
    /// `o` cycles panes of the window, `n`/`p` next/prev window, `(`/`)`
    /// prev/next session — every switch is select-then-refresh, the
    /// daemon-side select + resync passthrough dispatches, with the
    /// renderer rebuilding from the fresh replays.
    fn prefix_switch(&mut self, key: u8, conn: &mut super::conn::AttachConn) {
        match key {
            b'o' => self.cycle_pane(conn),
            b'n' => self.switch_window(conn, 1),
            b'p' => self.switch_window(conn, -1),
            b'(' => self.switch_session(conn, -1),
            b')' => self.switch_session(conn, 1),
            _ => {}
        }
    }

    /// The reload chord in render mode: the same client-side rebind the
    /// passthrough session performs, plus a status-row flash, plus the
    /// daemon's `reload-config` — best-effort either way.
    fn reload_config(&mut self, conn: &mut super::conn::AttachConn) {
        match super::reload_client_chords(crate::mux::config::Chords {
            prefix: self.prefix,
            reload: self.reload_key,
        }) {
            Ok(chords) => {
                self.prefix = chords.prefix;
                self.reload_key = chords.reload;
                self.literal = chords.prefix;
                self.flash = Some("config reloaded".to_string());
            }
            Err(err) => {
                self.flash = Some(format!("reload failed: {err}"));
            }
        }
        let _ = conn.send_checked("reload-config");
    }

    /// prefix o: select the next pane in the window's layout-leaf order,
    /// daemon-side, then re-seed — the render-mode spelling of
    /// select-then-refresh.
    fn cycle_pane(&mut self, conn: &mut super::conn::AttachConn) {
        let order: Vec<u32> = self.renderer.layout().iter().map(|r| r.pane).collect();
        if order.len() < 2 {
            return;
        }
        let current = self
            .renderer
            .focused()
            .and_then(|f| order.iter().position(|p| *p == f))
            .unwrap_or(0);
        let next = order[(current + 1) % order.len()];
        self.renderer.focus(next);
        let _ = conn.send_checked(&format!("select-pane -t %{next}"));
    }

    /// prefix n/p: move to the next/previous window of the shown session
    /// and mirror it (daemon-side select-window, then re-seed from fresh
    /// replays).
    fn switch_window(&mut self, conn: &mut super::conn::AttachConn, direction: i32) {
        // The status state knows the shown session's windows.
        if self.status.session_id.is_none() {
            return;
        }
        // Re-query for the fresh order: the status state may be stale.
        let Ok(reply) = conn.send_checked(&format!(
            "list-windows -t {}",
            self.status.session_id.clone().unwrap_or_default()
        )) else {
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
        let next = windows[(position as i32 + direction).rem_euclid(windows.len() as i32) as usize]
            .clone();
        if !conn
            .send_checked(&format!("select-window -t {next}"))
            .is_ok_and(|reply| reply.ok)
        {
            return;
        }
        self.reseed_window(conn, &next);
    }

    /// prefix ( / ): the previous/next session in list-sessions order;
    /// mirror its active window.
    fn switch_session(&mut self, conn: &mut super::conn::AttachConn, direction: i32) {
        let Ok(reply) = conn.send_checked("list-sessions") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let sessions: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| super::parse_session_line(l).map(|(id, _)| id))
            .collect();
        // Which session owns the shown window right now?
        let Ok(windows) = conn.send_checked(&format!(
            "list-windows -t {}",
            self.status.session_id.clone().unwrap_or_default()
        )) else {
            return;
        };
        let owns_window = windows
            .body
            .iter()
            .any(|l| l.split_whitespace().next() == Some(self.window.as_str()));
        let current = if owns_window {
            sessions
                .iter()
                .position(|s| Some(s.as_str()) == self.status.session_id.as_deref())
        } else {
            // Stale state: fall back to the head so a direction still
            // moves somewhere deterministic.
            Some(0)
        };
        let Some(current) = current else {
            return;
        };
        let next = sessions
            [(current as i32 + direction).rem_euclid(sessions.len() as i32) as usize]
            .clone();
        let Ok(windows) = conn.send_checked(&format!("list-windows -t {next}")) else {
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
        let _ = conn.send_checked(&format!("select-window -t {window}"));
        self.reseed_window(conn, window);
    }

    /// Point the whole view at `window`: daemon-side select already done
    /// (or the window is in the same session), re-fit the renderer from a
    /// fresh layout report, replay every pane, and mark everything dirty
    /// — the render-mode resync.
    fn reseed_window(&mut self, conn: &mut super::conn::AttachConn, window: &str) {
        let (cols, rows) = self.renderer.window_size();
        let pane = self.focused_pane_or_first(conn, window);
        if conn
            .send_checked(&format!("refresh-client -t {pane} -C {cols}x{rows}"))
            .is_err()
        {
            return;
        }
        self.window = window.to_string();
        self.renderer = PaneRenderer::new(cols, rows, Glyphs::Unicode);
        self.scroll_mode = false;
        if let Some((l, v, f)) =
            conn.drain_pending_events()
                .into_iter()
                .find_map(|event| match event {
                    TmuxNotification::LayoutChange {
                        window_id,
                        window_layout,
                        window_visible_layout,
                        window_raw_flags,
                    } if window_id == window => {
                        Some((window_layout, window_visible_layout, window_raw_flags))
                    }
                    _ => None,
                })
        {
            if let Ok(layout) = layout::parse_layout_triple(&l, &v, &f) {
                self.renderer.apply_layout(layout);
            }
        }
        self.replay_all_panes(conn);
        // Fresh facts for the new view; a session gone mid-switch ends
        // the view through the pump's normal path on the next mark.
        let focused = self.renderer.focused().unwrap_or(0);
        self.status_dirty = true;
        if matches!(
            self.status.refresh(conn, window, focused),
            Err(status::StatusError::SessionGone)
        ) {
            // The window vanished between the select and the query: leave
            // the view rendering its last frame; the next %sessions-changed
            // (or this switch's own burst) re-evaluates.
            return;
        }
        self.status_row.invalidate();
        self.draw_status_row();
    }

    /// A pane id of `window` to hang the size report on: the focused pane
    /// when it still belongs there, else the window's first pane.
    fn focused_pane_or_first(&self, conn: &mut super::conn::AttachConn, window: &str) -> String {
        let focused = self.focused_pane();
        if let Ok(reply) = conn.send_checked(&format!("list-panes -t {window}")) {
            if reply.ok {
                let panes: Vec<String> = reply
                    .body
                    .iter()
                    .filter_map(|l| l.split_whitespace().next())
                    .filter(|p| p.starts_with('%'))
                    .map(str::to_string)
                    .collect();
                if panes.contains(&focused) {
                    return focused;
                }
                if let Some(first) = panes.first() {
                    return first.clone();
                }
            }
        }
        focused
    }

    /// Leave scroll mode: clear the hold and snap the focused pane to
    /// live.
    fn leave_scroll_mode(&mut self) {
        self.scroll_mode = false;
        if let Some(id) = self.renderer.focused() {
            self.renderer.exit_scroll_mode(id);
        }
    }

    /// One functional key while scroll mode is up: arrows line-scroll,
    /// PgUp/PgDn page by the pane's height, Home jumps to the top of
    /// history, End and q and Enter exit. Keys never reach the pane
    /// while the viewport is up.
    fn scroll_mode_key(&mut self, ev: &TermKeyEvent) {
        use crate::keyboard::TermKey;
        let Some(id) = self.renderer.focused() else {
            return;
        };
        let rows = self
            .renderer
            .pane_terminal(id)
            .map(|t| t.active_grid().rows().max(1) as isize)
            .unwrap_or(1);
        match (ev.key(), ev.modifiers) {
            (TermKey::Up, 0) => {
                self.renderer.scroll_viewport(id, 1);
            }
            (TermKey::Down, 0) => {
                self.renderer.scroll_viewport(id, -1);
            }
            (TermKey::PageUp, 0) => {
                self.renderer.scroll_viewport(id, rows);
            }
            (TermKey::PageDown, 0) => {
                self.renderer.scroll_viewport(id, -rows);
            }
            (TermKey::Home, 0) => {
                let max = self
                    .renderer
                    .pane_terminal(id)
                    .map(|t| t.active_grid().scrollback_len() as isize)
                    .unwrap_or(0);
                self.renderer.scroll_viewport(id, max);
            }
            (TermKey::End, 0) => self.leave_scroll_mode(),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'q') => self.leave_scroll_mode(),
            _ => {}
        }
    }

    /// Draw the status row: fresh state composed and painted into the row
    /// buffer (the pump flushes the diff with the next frame).
    fn draw_status_row(&mut self) {
        let (cols, _rows) = super::conn::terminal_grid();
        if self.status_row.cols() != cols {
            self.status_row = StatusRow::new(cols);
        }
        let scroll = if self.scroll_mode {
            self.renderer
                .focused()
                .map(|id| self.renderer.scroll_offset_of(id))
        } else {
            None
        };
        let mut segments = self.status.compose(cols, scroll);
        if let Some(flash) = self.flash.clone() {
            segments.insert(
                0,
                Segment {
                    text: format!(" {flash} |"),
                    bold: true,
                },
            );
        }
        self.status_row.paint(&segments);
    }

    /// Flush the status row's changed cells to the host's bottom row.
    fn flush_status_row(&mut self, sink: &mut dyn FlushSink) {
        let (_cols, rows) = super::conn::terminal_grid();
        let bottom = rows.saturating_sub(1);
        let diff = self.status_row.diff();
        if diff.is_empty() {
            return;
        }
        // Rebase the row-relative cells to the host's bottom row and
        // reuse the sink's per-cell spelling.
        let rebased: Vec<(u16, u16, RtCell)> = diff
            .into_iter()
            .map(|(x, _y, cell)| (x, bottom, cell))
            .collect();
        sink.flush(&rebased);
    }

    /// One host mouse report: clicks focus the pane under the pointer
    /// (select-pane daemon-side, so the daemon's own active-pane state
    /// follows); events forward pane-relative SGR when the pane owns
    /// mouse tracking; the wheel scrolls the client's scrollback when it
    /// does not.
    fn route_mouse(&mut self, conn: &mut super::conn::AttachConn, mouse: SgrMouse) {
        // Window-relative, 0-based.
        let Some(x) = mouse.col.checked_sub(1) else {
            return;
        };
        let Some(y) = mouse.row.checked_sub(1) else {
            return;
        };
        let Some(rect) = self.renderer.pane_at(x, y).cloned() else {
            return; // a divider or outside the window
        };
        let owns = self
            .renderer
            .pane_terminal(rect.pane)
            .is_some_and(|t| t.mouse_mode() != crate::mouse::MouseMode::Off);

        if mouse.is_wheel_up() || mouse.is_wheel_down() {
            let delta: isize = if mouse.is_wheel_up() { 3 } else { -3 };
            if !owns && self.renderer.wheel_scroll(x, y, delta) {
                return; // consumed client-side
            }
            if owns {
                self.forward_mouse(conn, &rect, &mouse);
            }
            return;
        }

        if mouse.release || mouse.is_motion() {
            // Drag/release only matter to a pane that owns the mouse;
            // focus follows press only.
            if owns {
                self.forward_mouse(conn, &rect, &mouse);
            }
            return;
        }

        // A press: focus the pane (locally and daemon-side), then forward
        // when it owns mouse tracking.
        self.renderer.focus(rect.pane);
        let _ = conn.send_checked(&format!("select-pane -t %{}", rect.pane));
        if owns {
            self.forward_mouse(conn, &rect, &mouse);
        }
    }

    /// Re-encode one host mouse report pane-relative and send it.
    fn forward_mouse(
        &mut self,
        conn: &mut super::conn::AttachConn,
        rect: &PaneRect,
        mouse: &SgrMouse,
    ) {
        let rel_col = mouse.col.saturating_sub(1).saturating_sub(rect.x);
        let rel_row = mouse.row.saturating_sub(1).saturating_sub(rect.y);
        let bytes = mouse.reencode_sgr(rel_col, rel_row);
        super::forward_chunked(conn, format!("%{}", rect.pane), &bytes);
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
        self.flush_status_row(sink);
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
            // The wire shape is `$N: name`; the id ends at the colon — a
            // whitespace split keeps it (`$0:`), which the daemon's id
            // parser rejects. Newest = highest id (ids are monotonic), the
            // same deterministic newest-stand-in rule passthrough uses.
            let session = sessions
                .body
                .iter()
                .filter_map(|l| super::parse_session_line(l).map(|(id, _)| id))
                .filter(|id| id.starts_with('$'))
                .filter_map(|id| {
                    let n: u64 = id[1..].parse().ok()?;
                    Some((n, id))
                })
                .max_by_key(|(n, _)| *n)
                .map(|(_, id)| id)
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

    // ---- Phase B part 2: the input router (scrollback paint, pane
    // ownership, rect lookup). The parser's own suite lives in
    // `attach::input`.

    /// Criterion 3 (client side): a wheel-up over a pane that does NOT own
    /// the mouse scrolls the pane's client view into its scrollback — the
    /// rect's top rows show the newest history lines, and the live screen
    /// shifts down. A pane that DOES own the mouse leaves the scroll at 0
    /// (its wheel is forwarded instead).
    #[test]
    fn wheel_scrolls_client_scrollback_when_pane_has_no_mouse_mode() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.render_frame(); // settle

        // Pane 1 floods 30 lines through its 24-row pane: the overflow
        // lands in scrollback, the rest stays on screen.
        for i in 0..30 {
            renderer.feed_output(1, format!("hist-{i:02}\r\n").as_bytes());
        }
        renderer.render_frame();

        // The view's authority BEFORE scrolling: a snapshot of the grid.
        let grid = renderer.pane_terminal(1).unwrap().active_grid().clone();
        let len = grid.scrollback_len();
        let offset = 3usize;
        let line_text = |cells: &[crate::cell::Cell]| -> String {
            cells
                .iter()
                .take(8)
                .map(|c| c.c().to_string())
                .collect::<String>()
        };
        let expected_top = line_text(grid.scrollback_line(len - offset).expect("history"));

        // Wheel up 3 over pane 1 (any point of its rect): offset 3.
        assert!(renderer.wheel_scroll(10, 5, 3));
        renderer.render_frame();
        // View row 0 now shows the newest unscrolled-back history line.
        let top: String = (0..8)
            .map(|c| renderer.buffer[(c, 0)].symbol())
            .collect::<String>();
        assert_eq!(top, expected_top, "the top row scrolled into history");

        // Wheel down returns to the live view: row 0 shows live row 0.
        assert!(renderer.wheel_scroll(10, 5, -3));
        renderer.render_frame();
        let painted: String = (0..8)
            .map(|c| renderer.buffer[(c, 0)].symbol())
            .collect::<String>();
        let live = line_text(grid.row(0).expect("live row"));
        assert_eq!(painted, live, "back at the live view");
    }

    /// A pane that owns mouse tracking never scrolls client-side: the
    /// wheel is the pane's.
    #[test]
    fn wheel_does_not_scroll_a_pane_that_owns_the_mouse() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.render_frame();
        for i in 0..30 {
            renderer.feed_output(1, format!("hist-{i:02}\r\n").as_bytes());
        }
        // Pane 1 enables normal mouse tracking (DECSET 1000) through the
        // same %output path the app would use.
        renderer.feed_output(1, b"\x1b[?1000h");
        renderer.render_frame();

        assert!(!renderer.wheel_scroll(10, 5, 3), "owning pane consumes");
    }

    /// pane_at: the rect containing a window-relative point. The daemon's
    /// gap-free tiling means a divider OVERLAYS a content column of one
    /// pane's rect, so a point on the divider resolves to the pane whose
    /// column it is (pane 1 owns cols 0..40 here, divider included);
    /// outside-the-window points find nothing.
    #[test]
    fn pane_at_maps_points_to_their_rects() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        assert_eq!(renderer.pane_at(0, 0).map(|r| r.pane), Some(1));
        assert_eq!(renderer.pane_at(39, 12).map(|r| r.pane), Some(1));
        assert_eq!(renderer.pane_at(40, 12).map(|r| r.pane), Some(2));
        assert_eq!(renderer.pane_at(79, 23).map(|r| r.pane), Some(2));
        assert_eq!(renderer.pane_at(80, 0), None, "outside the window");
        assert_eq!(renderer.pane_at(0, 24), None, "below the window");
    }

    /// Emulator input-mode tracking: DECCKM and mouse mode arrive through
    /// the pane's own output bytes (replay or %output — same stream), and
    /// feed resets the client scroll to live.
    #[test]
    fn emulator_tracks_input_modes_and_feed_resets_scroll() {
        let mut emulator = PaneEmulator::new(7, 80, 24);
        assert!(!emulator.application_cursor());
        assert!(!emulator.owns_mouse());
        emulator.feed(b"\x1b[?1h\x1b[?1000h");
        assert!(emulator.application_cursor());
        assert!(emulator.owns_mouse());
        assert_eq!(
            emulator.mouse_encoding(),
            crate::mouse::MouseEncoding::Default
        );
        emulator.feed(b"\x1b[?1006h");
        assert_eq!(emulator.mouse_encoding(), crate::mouse::MouseEncoding::Sgr);
        // Scroll needs actual history to move into (the offset clamps to
        // the scrollback extent), so overflow the 24-row pane first;
        // feed output afterwards — the output snaps the view back to
        // live.
        for i in 0..30 {
            emulator.feed(format!("line-{i}\r\n").as_bytes());
        }
        assert_eq!(emulator.scroll_by(2), 2);
        emulator.feed(b"x");
        assert_eq!(emulator.scroll_offset(), 0);
    }

    /// The mode-aware key re-encode through the renderer's pane terminal:
    /// the pane's DECCKM state decides the arrow spelling (criterion 1's
    /// unit-level pin; the parser-level one is in `attach::input`).
    #[test]
    fn key_reencode_reads_the_panes_tracked_decckm() {
        use crate::keyboard::{encode_key, TermKey, TermKeyEvent};
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.feed_output(1, b"\x1b[?1h"); // DECCKM on
        let term = renderer.pane_terminal(1).expect("pane 1");
        assert!(term.application_cursor());
        assert_eq!(
            encode_key(&TermKeyEvent::functional(TermKey::Up, 0), term),
            b"\x1bOA"
        );
    }

    // ---- Phase B part 3: the prefix-[ keyboard scroll viewport.

    /// Criterion 2: entering scroll mode holds the view one viewport up
    /// from live and holds it against pane output; arrows move the
    /// viewport; exiting snaps to live. q/Enter are the session's
    /// business (routed in `route_plain`); the renderer only models the
    /// offset.
    #[test]
    fn scroll_mode_holds_view_against_output_and_snaps_on_exit() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        // 30 numbered lines through the 24-row pane: 6+ in scrollback.
        for i in 0..30 {
            renderer.feed_output(1, format!("hist-{i:02}\r\n").as_bytes());
        }
        renderer.render_frame();

        // A pane with no scrollback refuses scroll mode.
        assert!(!renderer.enter_scroll_mode(2), "pane 2 has no history");
        assert!(renderer.enter_scroll_mode(1), "pane 1 has history");
        let history = renderer
            .pane_terminal(1)
            .map(|t| t.active_grid().scrollback_len())
            .unwrap_or(0);
        let viewport = renderer.scroll_offset_of(1);
        assert_eq!(
            viewport,
            24usize.min(history),
            "one viewport up, clamped to the history extent"
        );
        assert!(renderer.scroll_mode_active(1));

        // Pane output while held does NOT snap to live — the view holds.
        renderer.feed_output(1, b"NEW-LINE\r\n");
        assert_eq!(
            renderer.scroll_offset_of(1),
            viewport,
            "the hold survives pane output"
        );
        // The offset clamps when history shrinks relative to the view.
        renderer.scroll_viewport(1, 1);
        assert_eq!(renderer.scroll_offset_of(1), viewport + 1);

        // Arrows-equivalent: viewport down past live clamps to the max
        // (live is reached at offset 0 only via exit).
        renderer.scroll_viewport(1, -(viewport as isize + 10));
        assert_eq!(renderer.scroll_offset_of(1), 0, "clamped at live");

        // Exit: the hold clears and the view is live; fresh output is
        // the pane's business again.
        renderer.exit_scroll_mode(1);
        assert!(!renderer.scroll_mode_active(1));
        renderer.feed_output(1, b"x");
        assert_eq!(renderer.scroll_offset_of(1), 0);
    }

    /// Scroll mode on an unknown pane is a no-op.
    #[test]
    fn scroll_mode_ignores_unknown_panes() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        assert!(!renderer.enter_scroll_mode(99));
        assert!(!renderer.scroll_mode_active(99));
        renderer.exit_scroll_mode(99);
        renderer.scroll_viewport(99, 5);
        assert_eq!(renderer.scroll_offset_of(99), 0);
    }

    /// While the scroll viewport is up, painted rows come from history:
    /// the rect's top row shows a scrollback line, not live row 0.
    #[test]
    fn scroll_viewport_paints_history_rows() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        for i in 0..30 {
            renderer.feed_output(1, format!("hist-{i:02}\r\n").as_bytes());
        }
        renderer.render_frame();
        let grid = renderer.pane_terminal(1).unwrap().active_grid().clone();

        assert!(renderer.enter_scroll_mode(1));
        renderer.render_frame();
        let offset = renderer.scroll_offset_of(1);
        let expected = grid
            .scrollback_line(grid.scrollback_len() - offset)
            .expect("history line");
        let expected: String = expected.iter().take(8).map(|c| c.c().to_string()).collect();
        let top: String = (0..8).map(|c| renderer.buffer[(c, 0)].symbol()).collect();
        assert_eq!(top, expected, "the viewport paints from history");
    }

    /// Regression (render-mode target-less resolution): a `list-sessions`
    /// line is `$N: name`, so a whitespace split keeps the colon and the
    /// daemon's id parser rejects `$0:` — the client exited with "the
    /// session has no windows". The resolution must strip the colon, pick
    /// the NEWEST session (highest id — the documented default), and its
    /// active window's active pane.
    #[test]
    fn target_less_resolution_takes_newest_session_without_colon() {
        use crate::mux::attach::conn;
        // The Listener trait supplies `.accept()` on unix only; the Windows
        // named-pipe listener accepts inherently.
        #[cfg(unix)]
        use interprocess::local_socket::traits::Listener as _;
        use std::io::{BufRead as _, BufReader, Write as _};

        // Scripted daemon: two sessions (id 0 and 3, out of order to prove
        // the newest pick is by id, not line order), each with windows, the
        // newest session's window @9 marked active.
        let mut path = std::env::temp_dir();
        path.push(format!(
            "par-mux-attach-render-resolve-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = crate::mux::bind_local_listener(&path).expect("bind");
        let (tx, rx) = std::sync::mpsc::channel();
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
                number += 1;
                let reply = match trimmed {
                    "version" => "9.9.9+deadbeef".to_string(),
                    "list-commands" => String::new(),
                    "list-sessions" => "$0: old\n$3: new\n".to_string(),
                    "list-windows -t $0" => "@1 - old-win\n".to_string(),
                    "list-windows -t $3" => "@7 - mid\n@9 * new-win\n".to_string(),
                    "list-panes -t @9" => "%5 0 -\n%8 1 *".to_string(),
                    _ => String::new(),
                };
                tx.send(trimmed.to_owned()).ok();
                writer
                    .write_all(crate::mux::emit_block(number, &reply, true).as_bytes())
                    .ok();
                writer.flush().ok();
            }
        });

        let mut conn = conn::AttachConn::connect(&path).expect("connect");
        drop(rx);
        let (window, pane) = resolve_window_and_pane(&mut conn, None).expect("resolve");
        assert_eq!(window, "@9", "the newest session's active window");
        assert_eq!(pane, "%8", "the active pane of that window");
    }
}
