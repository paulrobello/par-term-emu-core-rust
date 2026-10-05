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
use crate::mux::attach::{layout, HelpRow, ManagementKey};
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
    overlay: Option<(&'static str, Vec<HelpRow>, Option<(usize, usize, usize)>)>,
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
            sidebar_w: 0,
            sidebar_sections: None,
            user_titles: HashMap::new(),
            glyphs,
            bg: None,
            drag_divider: None,
            overlay: None,
            pane_borders: false,
            show_label_in_border: false,
            pane_gaps: 0,
            scrollbar_gutter: false,
            buffer: Buffer::empty(area),
            prev_buffer: Buffer::empty(area),
            border_active: None,
            border_plain: None,
            dirty: true,
        }
    }

    /// Set the border colors from the resolved config (`#rrggbb` hex;
    /// `None` keeps the built-ins). A change dirties the frame.
    pub fn set_border_colors(&mut self, active: Option<RtColor>, plain: Option<RtColor>) {
        if self.border_active != active || self.border_plain != plain {
            self.border_active = active;
            self.border_plain = plain;
            self.dirty = true;
        }
    }

    /// Set the background the next frame fills every cell with — the
    /// host terminal's resolved background color. `None` (the probe
    /// failed) leaves the fill at the terminal default: bg-fill and
    /// divider cells carry no painted color, so a failed probe can never
    /// mismatch the theme. `Some` paints the probed value — including
    /// black, when the host truly is black.
    pub fn set_background(&mut self, bg: Option<RtColor>) {
        if self.bg != bg {
            self.bg = bg;
            self.dirty = true;
        }
    }

    /// The renderer's resolved background, if the probe reported one.
    pub fn background(&self) -> Option<RtColor> {
        self.bg
    }

    /// The current layout rects (window-relative), in leaf order.
    pub fn layout(&self) -> &[PaneRect] {
        &self.layout
    }

    /// The renderer's full frame extent — the host grid the seed and
    /// `resize_to` derived, strip/status included. `window_size` reports
    /// the pane extent; this is the buffer's own.
    pub fn frame_size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// The window extent the daemon divides: the renderer's full width
    /// less the side panel's strip (the panel is a client-only overlay —
    /// the daemon never learns of it), full height.
    pub fn window_size(&self) -> (u16, u16) {
        (self.width.saturating_sub(self.sidebar_w), self.height)
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

    /// Mark the divider being dragged (orientation + pane ids) or clear
    /// the mark. The dragged divider renders reversed while the drag
    /// lasts.
    pub fn set_drag_divider(&mut self, divider: Option<(bool, u32, u32)>) {
        if self.drag_divider != divider {
            self.drag_divider = divider;
            self.dirty = true;
        }
    }

    /// The painted overlay's geometry (the same math `paint_overlay`
    /// runs): `(x0, y0, inner_width, height)` while one is up — the read
    /// the picker's click hit-test makes.
    pub(crate) fn overlay_geometry(&self) -> Option<(usize, usize, usize, usize)> {
        let rows = &self.overlay.as_ref()?.1;
        let inner = rows
            .iter()
            .map(|r| r.text.chars().count())
            .max()
            .unwrap_or(0)
            .min(self.width.saturating_sub(2) as usize)
            .max(1);
        let height = rows
            .len()
            .min(self.height.saturating_sub(2) as usize)
            .max(1);
        let x0 = (self.width as usize).saturating_sub(inner + 2) / 2;
        let y0 = (self.height as usize).saturating_sub(height + 2) / 2;
        Some((x0, y0, inner, height))
    }

    /// The overlay's composed panel row (index into the panel's rows:
    /// 0 is the filter line, the last the footer) at window-relative
    /// `(x, y)`, or `None` when no overlay is up or the point is off the
    /// box.
    pub(crate) fn overlay_row_at(&self, x: u16, y: u16) -> Option<usize> {
        let (x0, y0, inner, height) = self.overlay_geometry()?;
        let x = usize::from(x);
        let y = usize::from(y);
        if x < x0 + 1 || x > x0 + inner {
            return None;
        }
        if y < y0 + 1 || y > y0 + height {
            return None;
        }
        Some(y - y0 - 1)
    }

    /// Set (or clear) the modal overlay's title, rows, and scroll state.
    /// The overlay paints as the themed modal over the frame; clearing it
    /// lets the next frame's pane repaint restore the covered cells.
    pub(crate) fn set_overlay(
        &mut self,
        overlay: Option<(&'static str, Vec<HelpRow>, Option<(usize, usize, usize)>)>,
    ) {
        let same = match (&self.overlay, &overlay) {
            (Some((a, ra, sa)), Some((b, rb, sb))) => a == b && ra == rb && sa == sb,
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.overlay = overlay;
            self.dirty = true;
        }
    }

    /// Swap the divider/border glyph set (the border-cycle chord and
    /// the `border-lines` config): the frame marks dirty so the
    /// dividers and pane boxes repaint at the new glyphs.
    pub(crate) fn set_glyphs(&mut self, glyphs: Glyphs) {
        if self.glyphs != glyphs {
            self.glyphs = glyphs;
            self.dirty = true;
        }
    }

    /// The pane-border display mode (config `pane-borders`): each pane
    /// renders its own complete box with the content inset by the border
    /// cells, replacing the shared-divider look. Default off.
    pub(crate) fn set_pane_borders(&mut self, on: bool) {
        if self.pane_borders != on {
            self.pane_borders = on;
            self.dirty = true;
        }
    }

    /// The label-in-border display mode (config `show-label-in-border`,
    /// only meaningful with `pane_borders`): each pane's user title
    /// renders embedded in its top border edge. Default off.
    pub(crate) fn set_show_label_in_border(&mut self, on: bool) {
        if self.show_label_in_border != on {
            self.show_label_in_border = on;
            self.dirty = true;
        }
    }

    /// The divider within `tol` cells of window-relative `(x, y)`, if any
    /// — the shared edge of the two rects it separates. Dividers draw on
    /// the left/top pane's last column/row (the daemon's geometry tiles
    /// exactly), so the test measures against that cell line.
    pub fn divider_near(&self, x: u16, y: u16, tol: i32) -> Option<DividerHit> {
        let px = i32::from(x);
        let py = i32::from(y);
        for (i, first) in self.layout.iter().enumerate() {
            for second in self.layout.iter().skip(i + 1) {
                // Order the pair along each axis; only the side-by-side
                // (or stacked) case is a boundary.
                let (l, r) = if first.x <= second.x {
                    (first, second)
                } else {
                    (second, first)
                };
                let (t, b) = if first.y <= second.y {
                    (first, second)
                } else {
                    (second, first)
                };
                if l.x + l.width == r.x && rows_overlap(l, r) {
                    let div = i32::from(r.x) - 1;
                    if (px - div).abs() <= tol {
                        return Some(DividerHit {
                            vertical: true,
                            a: l.pane,
                            b: r.pane,
                        });
                    }
                }
                if t.y + t.height == b.y && cols_overlap(t, b) {
                    let div = i32::from(b.y) - 1;
                    if (py - div).abs() <= tol {
                        return Some(DividerHit {
                            vertical: false,
                            a: t.pane,
                            b: b.pane,
                        });
                    }
                }
            }
        }
        None
    }

    /// The terminal cursor position for the focused pane, mapped through
    /// its rect origin and the client scroll offset: `Some((x, y, style))`
    /// where `(x, y)` is the window-relative cell to place the host
    /// cursor at. `None` — the cursor hides — when nothing is focused,
    /// the pane's emulator tracks a hidden cursor (DECTCEM), the client
    /// view is scrolled off live (the wheel/prefix-[ viewport: the live
    /// cell is not on screen), or the tracked cell is somehow outside
    /// the pane's rect.
    ///
    /// The style is the pane's tracked DECSCUSR shape, for the sink to
    /// re-emit (`CSI Ps SP q`).
    pub fn focused_cursor(&self) -> Option<(u16, u16, CursorStyle)> {
        let focus = self.focused?;
        let rect = self.layout.iter().find(|r| r.pane == focus)?;
        let emulator = self.emulators.get(&focus)?;
        if emulator.scroll_offset() > 0 {
            return None;
        }
        let cursor = emulator.terminal().cursor();
        if !cursor.visible {
            return None;
        }
        let (col, row) = (cursor.col, cursor.row);
        // The placement path shares the paint inset: with the per-pane
        // border option the cursor maps inside the border ring, hidden
        // when the tracked cell falls in the cropped perimeter band.
        let (inset_x, inset_y, view_w, view_h) = self.content_view(rect);
        if col >= usize::from(view_w) || row >= usize::from(view_h) {
            return None;
        }
        Some((
            rect.x + self.sidebar_w + inset_x + col as u16,
            rect.y + inset_y + row as u16,
            cursor.style,
        ))
    }

    /// The rect the pane's chrome and content paint into: the layout
    /// rect inset by the `pane_gaps` band per side (clamped so at least
    /// one cell of edge survives). The band cells keep the frame's
    /// theme-bg fill.
    fn display_rect(&self, rect: &PaneRect) -> PaneRect {
        let gap = self
            .pane_gaps
            .min(rect.width.saturating_sub(1) / 2)
            .min(rect.height.saturating_sub(1) / 2);
        let mut r = *rect;
        r.x += gap + self.sidebar_w;
        r.y += gap;
        r.width = r.width.saturating_sub(2 * gap);
        r.height = r.height.saturating_sub(2 * gap);
        r
    }

    /// The content view of a pane rect: `(inset_x, inset_y, width,
    /// height)` — the display rect's full area, or the interior inside
    /// the one-cell border ring when the per-pane-border option is on (a
    /// pane too small to carry a ring keeps its full rect), narrowed by
    /// one more column when the scrollbar gutter is reserved.
    fn content_view(&self, rect: &PaneRect) -> (u16, u16, u16, u16) {
        let d = self.display_rect(rect);
        let (inset_x, inset_y, mut w, h) = if self.pane_borders && d.width > 2 && d.height > 2 {
            (1, 1, d.width - 2, d.height - 2)
        } else {
            (0, 0, d.width, d.height)
        };
        if self.scrollbar_gutter {
            w = w.saturating_sub(1);
        }
        // Rect-RELATIVE insets: the callers add rect.x/rect.y (and the
        // side panel's width) themselves. display_rect already carries
        // the panel's offset in d.x, so it is subtracted back out here —
        // otherwise the content paints the panel offset twice (the
        // manual-pass double-push report).
        (
            d.x - rect.x - self.sidebar_w + inset_x,
            d.y - rect.y + inset_y,
            w,
            h,
        )
    }

    /// The gap-band mode (config `pane-gaps`): every pane's chrome and
    /// content paint into the rect inset per side, leaving theme-bg gap
    /// bands between panes. Default 0.
    pub(crate) fn set_pane_gaps(&mut self, gaps: u16) {
        if self.pane_gaps != gaps {
            self.pane_gaps = gaps;
            self.dirty = true;
        }
    }

    /// The scrollbar-gutter mode (config `scrollbar-gutter`): a
    /// right-edge gutter column is reserved in every pane rect; it
    /// renders a position indicator while the pane's client scroll
    /// offset is > 0. Default off.
    pub(crate) fn set_scrollbar_gutter(&mut self, on: bool) {
        if self.scrollbar_gutter != on {
            self.scrollbar_gutter = on;
            self.dirty = true;
        }
    }

    /// The side panel's width (0 = hidden).
    pub(crate) fn sidebar_width(&self) -> u16 {
        self.sidebar_w
    }

    /// Show the side panel at `width` columns (0 hides it). The daemon
    /// never learns of the strip — window_size reports the reduced grid,
    /// and the next refresh-client report re-divides the panes around it.
    pub(crate) fn set_sidebar_width(&mut self, width: u16) {
        if self.sidebar_w != width {
            self.sidebar_w = width;
            self.sidebar_sections = None;
            self.dirty = true;
        }
    }

    /// Replace the panel's sections (the workspace roster query's result)
    /// and mark dirty.
    pub(crate) fn set_sidebar_sections(&mut self, sections: Option<Vec<super::SidebarSection>>) {
        if self.sidebar_sections != sections {
            self.sidebar_sections = sections;
            self.dirty = true;
        }
    }

    /// Record a pane's effective title (the `pane-title` reply); marks
    /// dirty when it changed so the border label repaints.
    pub(crate) fn set_user_title(&mut self, pane: u32, title: &str) {
        if self.user_titles.get(&pane).map(String::as_str) != Some(title) {
            self.user_titles.insert(pane, title.to_string());
            self.dirty = true;
        }
    }

    /// The pane's border label: the daemon's effective title (user label
    /// first), or the emulator's own OSC title when never queried.
    fn border_label(&self, pane: u32) -> String {
        if let Some(title) = self.user_titles.get(&pane) {
            return title.trim().to_string();
        }
        self.emulators
            .get(&pane)
            .map(|e| e.terminal().title().trim().to_string())
            .unwrap_or_default()
    }

    /// The clickable side-panel id under the HOST cell `(x, y)`, if any.
    /// Host row 0 is the tab-strip row (no panel there), and a panel
    /// line composed at row `y` paints at HOST row `y + 1` — the frame
    /// rebases the renderer's rows +1 — so the lookup shifts the host
    /// row down one before matching.
    pub(crate) fn sidebar_row_at(&self, x: u16, y: u16) -> Option<String> {
        if x >= self.sidebar_w || y == 0 {
            return None;
        }
        let sections = self.sidebar_sections.as_ref()?;
        let lines = super::compose_sidebar(sections, self.sidebar_w, self.height);
        lines
            .into_iter()
            .find(|line| line.y == y - 1 && line.id.is_some() && x >= line.x && x < line.x_end)
            .and_then(|line| line.id)
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

        // Clear the frame to the host background BEFORE painting: every
        // cell that paint_pane skips (a short history line, the wide-char
        // spacer, past the grid edge) and every cell no pane rect covers
        // then carries the resolved bg instead of the ratatui default the
        // buffer was born with — the grey top/bottom bands the host
        // showed. When the probe failed (`bg: None`), the fill paints NO
        // color — cells stay terminal-default — so a failed probe can
        // never mismatch the host theme. Blank cells keep a blank symbol
        // but a real bg.
        if let Some(bg) = self.bg {
            let bg_style = RtStyle::default().bg(bg);
            self.buffer.reset();
            self.buffer
                .set_style(RtRect::new(0, 0, self.width, self.height), bg_style);
        } else {
            self.buffer.reset();
        }

        // Paint every pane's grid into its rect, then the chrome on top
        // (per-pane border boxes when the `pane-borders` option is on,
        // shared dividers otherwise), then the help overlay last (it
        // covers both).
        let layout = self.layout.clone();
        for rect in &layout {
            self.paint_pane(rect);
        }
        if self.pane_borders {
            self.paint_pane_borders();
        } else {
            self.paint_dividers();
        }
        self.paint_sidebar();
        if let Some((title, rows, scroll)) = self.overlay.clone() {
            self.paint_overlay(title, &rows, scroll);
        }

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
        // The content paints into the content view: inside the border
        // ring when the per-pane-border option is on (the grid's outer
        // columns/rows are cropped — the daemon's pane is the full rect;
        // the border overlays its perimeter), inside the gap band, and
        // beside the reserved gutter column.
        let (inset_x, inset_y, view_w, view_h) = self.content_view(rect);
        let grid = emulator.terminal().active_grid();
        let scroll = emulator.scroll_offset();
        let scrollback_len = grid.scrollback_len() as isize;
        // Clamped to the frame: a layout broadcast racing a host shrink
        // (or a test fixture) can carry rects past the buffer's edge,
        // and painting would panic on the index.
        let max_rows = view_h
            .min(grid.rows() as u16)
            .min(self.height.saturating_sub(rect.y.saturating_add(inset_y)));
        // The clamp counts the side panel's offset: a layout broadcast
        // racing the toggle still carries full-width rects, and painting
        // them offset would run past the buffer's right edge.
        let max_cols = view_w
            .min(grid.cols() as u16)
            .min(self.width.saturating_sub(rect.x + self.sidebar_w + inset_x));
        for row in 0..max_rows {
            // View row r: live grid row r - S when r >= S; otherwise the
            // scrollback line S_len - S + r (newest history first).
            let scrollback_row: isize = scrollback_len - scroll as isize + row as isize;
            let in_history = (row as usize) < scroll;
            for col in 0..max_cols {
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
                let (x, y) = (
                    rect.x + self.sidebar_w + inset_x + col,
                    rect.y + inset_y + row,
                );
                // The wide base already marked this spacer skip; painting
                // it would clear the mark (reset() clears diff_option).
                if core_cell.flags().wide_char_spacer() {
                    continue;
                }
                let cell = &mut self.buffer[(x, y)];
                cell.reset();
                if let Some(bg) = self.bg {
                    cell.set_bg(bg);
                }
                // Grapheme cluster: base char plus combining marks.
                let mut symbol = String::from(core_cell.c());
                for comb in core_cell.combining() {
                    symbol.push(*comb);
                }
                cell.set_symbol(&symbol);
                cell.set_fg(map_color(core_cell.fg()));
                // Blank-cell background semantics: the core grid marks
                // unwritten cells bg-black. A blank carries the probed
                // theme bg when the probe succeeded (the frame fill rule)
                // and terminal-default when it failed — palette black is
                // never painted for blank cells.
                if matches!(core_cell.bg(), CoreColor::Named(NamedColor::Black)) {
                    if let Some(bg) = self.bg {
                        cell.set_bg(bg);
                    }
                } else {
                    cell.set_bg(map_color(core_cell.bg()));
                }
                cell.set_style(map_flags(core_cell.flags()));
                // A double-width base marks its right-hand spacer skip so
                // the diff's flush never draws into it.
                if core_cell.width() == 2 && col + 1 < view_w && x + 1 < self.width {
                    self.buffer[(x + 1, y)].set_diff_option(CellDiffOption::Skip);
                }
            }
        }
        // The reserved gutter column (config `scrollbar-gutter`): theme-bg
        // fill, with a minimal position indicator while the pane's client
        // scroll offset is > 0 — one `▐` at the view top's proportional
        // depth into the history.
        if (self.scrollbar_gutter || scroll > 0) && view_h > 0 {
            let gx = rect.x + inset_x + view_w;
            if gx < self.width {
                let indicator_row = (scroll.min(u16::MAX as usize) as u32 * u32::from(view_h))
                    / (scroll.min(u16::MAX as usize) as u32 + u32::from(view_h));
                for gy in 0..view_h {
                    let y = rect.y + inset_y + gy;
                    if y >= self.height {
                        break;
                    }
                    let cell = &mut self.buffer[(gx, y)];
                    cell.reset();
                    if let Some(bg) = self.bg {
                        cell.set_bg(bg);
                    }
                    if scroll > 0 && u32::from(gy) == indicator_row {
                        cell.set_symbol("\u{2590}");
                        cell.set_fg(RtColor::Indexed(14));
                    }
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
        // (x, y, left/top pane, right/bottom pane) per boundary cell; the
        // pane ids drive the focus-side style and the drag highlight.
        let mut vertical: Vec<(u16, u16, u32, u32)> = Vec::new();
        let mut horizontal: Vec<(u16, u16, u32, u32)> = Vec::new();
        for (i, a) in self.layout.iter().enumerate() {
            for b in self.layout.iter().skip(i + 1) {
                // `a` ends where `b` starts along x, with row overlap: a
                // vertical boundary (divider cell in a's last column).
                if a.x + a.width == b.x && rows_overlap(a, b) {
                    for y in row_overlap(a, b) {
                        vertical.push((b.x.saturating_sub(1) + self.sidebar_w, y, a.pane, b.pane));
                    }
                }
                if b.x + b.width == a.x && rows_overlap(a, b) {
                    for y in row_overlap(a, b) {
                        vertical.push((a.x.saturating_sub(1) + self.sidebar_w, y, b.pane, a.pane));
                    }
                }
                // Same along y for a horizontal boundary. (boundary, along)
                // — the same order vertical pushes, so the grouping below
                // keys on the boundary coordinate for both.
                if a.y + a.height == b.y && cols_overlap(a, b) {
                    for x in col_overlap(a, b) {
                        horizontal.push((
                            b.y.saturating_sub(1),
                            x + self.sidebar_w,
                            a.pane,
                            b.pane,
                        ));
                    }
                }
                if b.y + b.height == a.y && cols_overlap(a, b) {
                    for x in col_overlap(a, b) {
                        horizontal.push((
                            a.y.saturating_sub(1),
                            x + self.sidebar_w,
                            b.pane,
                            a.pane,
                        ));
                    }
                }
            }
        }
        // Group the flat boundary cells per divider so each cell knows its
        // position along the divider's length — the tmux half rule (the
        // active pane's half of the shared divider carries the highlight).
        type BoundaryCell = (u16, u16, u32, u32);
        type BoundaryGroup = (u16, Vec<(u16, u32, u32)>);
        let group = |cells: &[BoundaryCell]| -> Vec<BoundaryGroup> {
            let mut groups: Vec<BoundaryGroup> = Vec::new();
            for (coord, along, a, b) in cells {
                match groups.iter_mut().find(|(k, _)| *k == *coord) {
                    Some((_, entries)) => entries.push((*along, *a, *b)),
                    None => groups.push((*coord, vec![(*along, *a, *b)])),
                }
            }
            groups
        };
        for (x, mut cells) in group(&vertical) {
            cells.sort_by_key(|(y, _, _)| *y);
            let len = cells.len() as u16;
            for (index, (y, a, b)) in cells.iter().enumerate() {
                if x >= self.width || *y >= self.height {
                    continue; // a stale layout racing a shrink can overflow
                }
                // The scrollbar gutter owns the left/top pane's last column
                // when reserved (config `scrollbar-gutter`): the boundary
                // divider yields the cell so the gutter's indicator stays
                // visible. The boundary stays a drag handle (`divider_near`
                // reads the layout geometry, not the paint).
                let a_scrolled = self.emulators.get(a).is_some_and(|e| e.scroll_offset() > 0);
                if (self.scrollbar_gutter || a_scrolled)
                    && self
                        .layout
                        .iter()
                        .any(|r| r.pane == *a && x == r.x + r.width - 1)
                {
                    continue;
                }
                let cell = &mut self.buffer[(x, *y)];
                cell.reset();
                if let Some(bg) = self.bg {
                    cell.set_bg(bg);
                }
                cell.set_symbol(self.glyphs.vertical());
                cell.set_style(divider_style(
                    self.focused,
                    self.drag_divider,
                    true,
                    *a,
                    *b,
                    index as u16,
                    len,
                    self.border_active,
                    self.border_plain,
                ));
            }
        }
        for (y, mut cells) in group(&horizontal) {
            cells.sort_by_key(|(x, _, _)| *x);
            let len = cells.len() as u16;
            for (index, (x, a, b)) in cells.iter().enumerate() {
                if *x >= self.width || y >= self.height {
                    continue; // a stale layout racing a shrink can overflow
                }
                // Same yield for the top pane's last ROW.
                if self.scrollbar_gutter
                    && self
                        .layout
                        .iter()
                        .any(|r| r.pane == *a && y == r.y + r.height - 1)
                {
                    continue;
                }
                // A cell that is also a vertical boundary becomes the junction.
                if vertical.iter().any(|(vx, vy, _, _)| *vx == *x && *vy == y) {
                    let cell = &mut self.buffer[(*x, y)];
                    cell.set_symbol(self.glyphs.cross());
                } else {
                    let cell = &mut self.buffer[(*x, y)];
                    cell.reset();
                    if let Some(bg) = self.bg {
                        cell.set_bg(bg);
                    }
                    cell.set_symbol(self.glyphs.horizontal());
                    cell.set_style(divider_style(
                        self.focused,
                        self.drag_divider,
                        false,
                        *a,
                        *b,
                        index as u16,
                        len,
                        self.border_active,
                        self.border_plain,
                    ));
                }
            }
        }
    }

    /// Paint the help overlay as the themed modal (the round-3 restyle):
    /// every cell carries the resolved theme bg — the same rule the frame
    /// fill applies, so no default-style (light) cell ever shows — ringed
    /// by a rounded box-drawing border in the accent color, title
    /// `keybinds` left and `esc close` badge top-right embedded in the
    /// top border, category headers in the accent, and the footer hints
    /// line inside the box. Clamped to the window; paints last in
    /// `render_frame`, covering panes and dividers; dismissal lets the
    /// next frame's pane repaint restore the covered cells.
    fn paint_overlay(
        &mut self,
        title: &str,
        rows: &[HelpRow],
        scroll: Option<(usize, usize, usize)>,
    ) {
        let accent_style = RtStyle::default()
            .fg(RtColor::Indexed(14))
            .add_modifier(RtModifier::BOLD);
        // Inner width: the widest row, clamped so the ring fits.
        let inner = rows
            .iter()
            .map(|r| r.text.chars().count())
            .max()
            .unwrap_or(0)
            .min(self.width.saturating_sub(2) as usize)
            .max(1);
        let height = rows
            .len()
            .min(self.height.saturating_sub(2) as usize)
            .max(1);
        let x0 = (self.width as usize).saturating_sub(inner + 2) / 2;
        let y0 = (self.height as usize).saturating_sub(height + 2) / 2;
        // Fill every cell of the box with the theme background first.
        for y in y0..y0 + height + 2 {
            for x in x0..x0 + inner + 2 {
                let cell = &mut self.buffer[(x as u16, y as u16)];
                cell.reset();
                if let Some(bg) = self.bg {
                    cell.set_bg(bg);
                }
            }
        }
        // Border ring in the accent color (ASCII fallback: `+` corners).
        let mut ring = |x: usize, y: usize, symbol: &str| {
            let cell = &mut self.buffer[(x as u16, y as u16)];
            cell.set_symbol(symbol);
            cell.set_style(accent_style);
        };
        for x in x0 + 1..x0 + inner + 1 {
            ring(x, y0, self.glyphs.horizontal());
            ring(x, y0 + height + 1, self.glyphs.horizontal());
        }
        for y in y0 + 1..y0 + height + 1 {
            ring(x0, y, self.glyphs.vertical());
            ring(x0 + inner + 1, y, self.glyphs.vertical());
        }
        ring(x0, y0, self.glyphs.corner_top_left());
        ring(x0 + inner + 1, y0, self.glyphs.corner_top_right());
        ring(x0, y0 + height + 1, self.glyphs.corner_bottom_left());
        ring(
            x0 + inner + 1,
            y0 + height + 1,
            self.glyphs.corner_bottom_right(),
        );
        // Title and badge embedded in the top border (herdr's label
        // treatment): `keybinds` after the top-left corner, `esc close`
        // flush right before the top-right corner. Both accent.
        let mut embed = |text: &str, start: usize| {
            for (j, ch) in text.chars().enumerate() {
                let x = start + j;
                if x > x0 + inner {
                    break;
                }
                let cell = &mut self.buffer[(x as u16, y0 as u16)];
                cell.set_symbol(&ch.to_string());
                cell.set_style(accent_style);
            }
        };
        embed(title, x0 + 1);
        let badge = " esc close ";
        let badge_start = x0 + 1 + inner - badge.chars().count().min(inner);
        embed(badge, badge_start);
        // Content rows: accent rows (category headers) in the accent
        // color, footer rows on a dark-grey band across the inner width,
        // the rest in the default foreground on the theme bg.
        for (i, row) in rows.iter().take(height).enumerate() {
            if row.footer {
                let band = RtStyle::default().bg(RtColor::Rgb(64, 64, 64));
                for x in x0 + 1..x0 + inner + 1 {
                    let cell = &mut self.buffer[(x as u16, (y0 + 1 + i) as u16)];
                    cell.reset();
                    cell.set_style(band);
                }
            }
            for (j, ch) in row.text.chars().take(inner).enumerate() {
                let cell = &mut self.buffer[((x0 + 1 + j) as u16, (y0 + 1 + i) as u16)];
                cell.set_symbol(&ch.to_string());
                if row.accent {
                    cell.set_style(accent_style);
                } else if row.footer {
                    let band = RtStyle::default().bg(RtColor::Rgb(64, 64, 64));
                    cell.set_style(band);
                }
            }
        }
        // The overflow scrollbar: a thumb on the ring's right column,
        // sized and positioned from the panel's scroll state — nothing
        // painted while the content fits.
        if let Some((start, visible, total)) = scroll {
            if total > visible && visible > 0 {
                let track = height;
                let thumb_len = (track * visible / total).max(1);
                let max_start = total.saturating_sub(visible).max(1);
                let thumb_pos = y0 + 1 + start.min(max_start) * (track - thumb_len) / max_start;
                for y in thumb_pos..thumb_pos + thumb_len {
                    let cell = &mut self.buffer[((x0 + inner + 1) as u16, y as u16)];
                    cell.set_symbol("┃");
                    cell.set_style(accent_style);
                }
            }
        }
    }

    /// Paint the workspace side panel over the strip columns: theme-bg
    /// fill, a dim divider on the strip's right edge, then the sections —
    /// each header in the accent, its rows dim (the active workspace
    /// accent + bold, herdr's emphasis). Clipped to the strip; composed
    /// by [`super::compose_sidebar`].
    fn paint_sidebar(&mut self) {
        let w = self.sidebar_w;
        if w == 0 {
            return;
        }
        let dim = RtStyle::default().add_modifier(RtModifier::DIM);
        for y in 0..self.height {
            for x in 0..w {
                let cell = &mut self.buffer[(x, y)];
                cell.reset();
                if let Some(bg) = self.bg {
                    cell.set_bg(bg);
                }
                if x + 1 == w {
                    cell.set_symbol(self.glyphs.vertical());
                    cell.set_style(dim);
                }
            }
        }
        let Some(sections) = self.sidebar_sections.clone() else {
            return;
        };
        for line in super::compose_sidebar(&sections, w, self.height) {
            // herdr's active treatment: the ACTIVE WORKSPACE's row is a
            // full-width inverted block (the round-6 panel lists only
            // workspaces). The footer chips paint at their own columns
            // (line.x), body rows at col 0.
            let workspace_row = line.id.as_deref().is_some_and(|id| id.starts_with("ws:"));
            if line.active && workspace_row {
                let block = RtStyle::default()
                    .add_modifier(RtModifier::REVERSED)
                    .add_modifier(RtModifier::BOLD);
                for x in 0..w - 1 {
                    let cell = &mut self.buffer[(x, line.y)];
                    cell.reset();
                    if let Some(bg) = self.bg {
                        cell.set_bg(bg);
                    }
                    cell.set_style(block);
                }
            }
            let style = if line.active && workspace_row {
                RtStyle::default()
                    .add_modifier(RtModifier::REVERSED)
                    .add_modifier(RtModifier::BOLD)
            } else {
                dim
            };
            for (j, ch) in line.text.chars().enumerate() {
                let x = line.x + j as u16;
                if x >= w - 1 {
                    break;
                }
                let cell = &mut self.buffer[(x, line.y)];
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
            }
        }
    }

    /// Paint the per-pane border boxes (config `pane-borders`): a full
    /// ring per rect — the focused pane's border in the accent color, the
    /// rest dim, herdr's look. When `show-label-in-border` is on, each
    /// pane's non-empty user title breaks the top edge, space-padded and
    /// truncated to fit (herdr's exact treatment); label cells are not
    /// drag handles ([`Self::label_cell_at`]).
    fn paint_pane_borders(&mut self) {
        // (x, y, label chars, owning pane) per pane with a label.
        let mut labels: Vec<(u16, u16, Vec<char>, u32)> = Vec::new();
        for layout_rect in &self.layout {
            let rect = self.display_rect(layout_rect);
            let style = if Some(rect.pane) == self.focused {
                RtStyle::default()
                    .fg(RtColor::Indexed(14))
                    .add_modifier(RtModifier::BOLD)
            } else {
                RtStyle::default().add_modifier(RtModifier::DIM)
            };
            let x1 = rect.x + rect.width.saturating_sub(1);
            let y1 = rect.y + rect.height.saturating_sub(1);
            for y in rect.y..=y1 {
                for x in rect.x..=x1 {
                    if x > rect.x && x < x1 && y > rect.y && y < y1 {
                        continue; // interior: not a border cell
                    }
                    if x >= self.width || y >= self.height {
                        continue; // a stale layout racing a shrink can overflow
                    }
                    let symbol = if x == rect.x && y == rect.y {
                        self.glyphs.corner_top_left()
                    } else if x == x1 && y == rect.y {
                        self.glyphs.corner_top_right()
                    } else if x == rect.x && y == y1 {
                        self.glyphs.corner_bottom_left()
                    } else if x == x1 && y == y1 {
                        self.glyphs.corner_bottom_right()
                    } else if y == rect.y || y == y1 {
                        self.glyphs.horizontal()
                    } else {
                        self.glyphs.vertical()
                    };
                    let cell = &mut self.buffer[(x, y)];
                    cell.reset();
                    if let Some(bg) = self.bg {
                        cell.set_bg(bg);
                    }
                    cell.set_symbol(symbol);
                    cell.set_style(style);
                }
            }
            if self.show_label_in_border && rect.width > 4 {
                let title = self.border_label(rect.pane);
                if !title.is_empty() {
                    let max = rect.width.saturating_sub(4) as usize;
                    let chars: Vec<char> =
                        format!(" {} ", title.chars().take(max).collect::<String>())
                            .chars()
                            .collect();
                    labels.push((rect.x + 1, rect.y, chars, rect.pane));
                }
            }
        }
        for (x, y, chars, pane) in labels {
            let style = if Some(pane) == self.focused {
                RtStyle::default()
                    .fg(RtColor::Indexed(14))
                    .add_modifier(RtModifier::BOLD)
            } else {
                RtStyle::default().add_modifier(RtModifier::DIM)
            };
            for (j, ch) in chars.into_iter().enumerate() {
                let cell = &mut self.buffer[(x + j as u16, y)];
                cell.reset();
                if let Some(bg) = self.bg {
                    cell.set_bg(bg);
                }
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
            }
        }
    }

    /// Whether window-relative `(x, y)` sits on a label's text cells (the
    /// space-padded title embedded in a pane's top border). Label cells
    /// are not drag handles: a press there must focus, not resize. The
    /// plain border segments around a label stay draggable.
    fn label_cell_at(&self, x: u16, y: u16) -> bool {
        if !self.show_label_in_border || !self.pane_borders {
            return false;
        }
        for layout_rect in &self.layout {
            let rect = self.display_rect(layout_rect);
            if y != rect.y || rect.width <= 4 {
                continue;
            }
            let title = self.border_label(rect.pane);
            if title.is_empty() {
                continue;
            }
            let max = rect.width.saturating_sub(4) as usize;
            let len = format!(" {} ", title.chars().take(max).collect::<String>())
                .chars()
                .count();
            if x > rect.x && x < rect.x + 1 + len as u16 {
                return true;
            }
        }
        false
    }
}

/// A boundary's style. The focus indication must FLIP when focus moves in
/// a two-pane split — both panes share one divider, so a single accent
/// color read identically from either side (the owner's manual pass: "the
/// border still does not change color"). The boundary carries the accent
/// on the side it names: bright cyan when the focused pane is the
/// boundary's left/top pane (`a`), bright magenta when it is the
/// right/bottom pane (`b`), dim when the boundary does not touch the
/// focus. While a drag is live on the boundary, the style renders
/// reversed so the edge being moved stands out.
/// The focused boundary half's highlight (config `border-active-color`,
/// `#rrggbb`; default the bright-cyan accent), always bold.
fn active_border_style(active: Option<RtColor>) -> RtStyle {
    let style = match active {
        Some(RtColor::Rgb(r, g, b)) => RtStyle::default().fg(RtColor::Rgb(r, g, b)),
        _ => RtStyle::default().fg(RtColor::Indexed(14)), // bright cyan
    };
    style.add_modifier(RtModifier::BOLD)
}

/// Unfocused divider/border look (config `border-color`, `#rrggbb`;
/// default the dim modifier alone).
fn plain_border_style(plain: Option<RtColor>) -> RtStyle {
    match plain {
        Some(RtColor::Rgb(r, g, b)) => RtStyle::default().fg(RtColor::Rgb(r, g, b)),
        _ => RtStyle::default().add_modifier(RtModifier::DIM),
    }
}

/// One divider boundary's style at cell `index` of `len`: the tmux
/// convention — the HALF of the divider nearer the active pane's side
/// carries the highlight. A vertical divider (panes `a`|`b` side by
/// side) highlights its TOP half while `a` (left) is focused and its
/// BOTTOM half while `b` (right) is; a horizontal divider mirrors it
/// (left half for the top pane, right half for the bottom pane).
#[allow(
    clippy::too_many_arguments,
    reason = "the tmux half rule reads as a flat (focus, drag, side, pair, half, colors) tuple"
)]
fn divider_style(
    focused: Option<u32>,
    drag: Option<(bool, u32, u32)>,
    vertical: bool,
    a: u32,
    b: u32,
    index: u16,
    len: u16,
    active: Option<RtColor>,
    plain: Option<RtColor>,
) -> RtStyle {
    let mut style = match focused {
        Some(f) if f == a && index * 2 < len => active_border_style(active),
        Some(f) if f == b && index * 2 >= len => active_border_style(active),
        _ => plain_border_style(plain),
    };
    if drag == Some((vertical, a, b)) {
        style = style.add_modifier(RtModifier::REVERSED);
    }
    style
}

fn rows_overlap(a: &PaneRect, b: &PaneRect) -> bool {
    a.y < b.y + b.height && b.y < a.y + a.height
}

/// The prefix+arrow pane navigation's directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneDir {
    Up,
    Down,
    Left,
    Right,
}

/// The nearest pane to `focused` in `dir` (the prefix+arrow chord):
/// candidates must lie strictly in the direction from the focused
/// pane's center; the nearest center wins. `None` at an edge.
#[must_use]
fn pane_in_direction(rects: &[PaneRect], focused: u32, dir: PaneDir) -> Option<u32> {
    let from = rects.iter().find(|r| r.pane == focused)?;
    let fc = (
        i32::from(from.x) + i32::from(from.width) / 2,
        i32::from(from.y) + i32::from(from.height) / 2,
    );
    let mut best: Option<(u32, i64)> = None;
    for r in rects {
        if r.pane == focused {
            continue;
        }
        let c = (
            i32::from(r.x) + i32::from(r.width) / 2,
            i32::from(r.y) + i32::from(r.height) / 2,
        );
        let (dx, dy) = (c.0 - fc.0, c.1 - fc.1);
        let in_dir = match dir {
            PaneDir::Up => dy < 0,
            PaneDir::Down => dy > 0,
            PaneDir::Left => dx < 0,
            PaneDir::Right => dx > 0,
        };
        if !in_dir {
            continue;
        }
        let dist = i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy);
        if best.is_none() || dist < best.map(|(_, d)| d).unwrap_or(i64::MAX) {
            best = Some((r.pane, dist));
        }
    }
    best.map(|(pane, _)| pane)
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
    /// Position (and shape) the host cursor for the frame just flushed:
    /// `Some((x, y, style))` places it at the window-relative 0-based
    /// cell with the DECSCUSR shape, `None` hides it. Default: nothing
    /// (the cursor state the repaint_all/flush left stands).
    fn place_cursor(&mut self, _cursor: Option<(u16, u16, CursorStyle)>) {
        let _ = _cursor;
    }
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

    fn place_cursor(&mut self, cursor: Option<(u16, u16, CursorStyle)>) {
        let mut out = String::with_capacity(24);
        match cursor {
            Some((x, y, style)) => {
                // 0-based window cell -> 1-based CUP, then the DECSCUSR
                // shape, then show.
                let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
                push_cursor_shape(&mut out, style);
                out.push_str("\x1b[?25h");
            }
            None => out.push_str("\x1b[?25l"),
        }
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }
}

/// The DECSCUSR spelling for one core cursor shape (`CSI Ps SP q`, xterm
/// ctlseqs "Set cursor style"), mapped from the pane's tracked DECSCUSR
/// state. Blink variants keep blinking: the blink rate is the host's.
fn push_cursor_shape(out: &mut String, style: CursorStyle) {
    let ps = match style {
        CursorStyle::BlinkingBlock => 0,
        CursorStyle::SteadyBlock => 2,
        CursorStyle::BlinkingUnderline => 3,
        CursorStyle::SteadyUnderline => 4,
        CursorStyle::BlinkingBar => 5,
        CursorStyle::SteadyBar => 6,
    };
    let _ = write!(out, "\x1b[{ps} q");
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
    // The management chords resolve here against the canonical config
    // file — the SAME pure parser the live reload runs (main.rs hands
    // prefix/reload over explicitly; the management keys ride the file).
    // A malformed chord fails the attach like a malformed prefix; a
    // broken FILE stays the lenient startup rule (warn-and-defaults —
    // the strict error is the reload's, per docs/MUX.md).
    let file = crate::mux::config::load_canonical();
    let chords = crate::mux::config::reload_client_chords(
        &file,
        &crate::mux::config::Chords {
            prefix: session.prefix,
            reload: session.reload_key,
            management: crate::mux::config::Management::default(),
            ..crate::mux::config::Chords::with_defaults()
        },
    )
    .map_err(|err| format!("config: {err}"))?;
    session.management = chords.management;
    session.resize_step = chords.resize_step;
    session.sidebar_width = chords.sidebar_width;
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
        session.set_pane_borders(matches!(session.border_glyphs, Glyphs::Herdr));
    }
    session.set_show_label_in_border(chords.show_label_in_border);
    session.set_pane_gaps(chords.pane_gaps);
    session.set_scrollbar_gutter(chords.scrollbar_gutter);
    session.drag_cursor_shape = chords.drag_cursor_shape;
    let eff = crate::mux::config::resolve(
        &crate::mux::config::load_canonical(),
        &crate::mux::config::Overrides::default(),
    );
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
    /// The session's resolved background (the OSC 11 probe result), kept
    /// OUTSIDE the renderer so a re-seed or resize re-fit — which rebuild
    /// the renderer — re-applies it instead of silently dropping back to
    /// the terminal-default fill (the round-3 prefix n/p black band).
    bg: Option<RtColor>,
    /// The session's pane-border display options (config), session-level
    /// like the background so renderer reconstruction re-applies them.
    pane_borders: bool,
    show_label_in_border: bool,
    /// The herdr-parity display options (config), session-level like the
    /// background so renderer reconstruction re-applies them.
    pane_gaps: u16,
    scrollbar_gutter: bool,
    /// The side panel's state (config `sidebar-width`, toggled by the
    /// `sidebar` chord), session-level like the background so renderer
    /// reconstruction re-applies it.
    sidebar_on: bool,
    sidebar_width: u16,
    /// The pump refits the grid (resize_to + full repaint) on the next
    /// loop — set by the sidebar toggle, whose chord has no sink.
    pending_grid_refit: bool,
    /// While a divider drag is live, the host cursor carries the resize
    /// shape (best-effort DECSCUSR steady block; restored on drag end).
    drag_cursor_shape: bool,
    /// The divider/border glyph set (config `border-lines`, cycled live
    /// by the border chord), session-level like the background so
    /// renderer reconstruction re-applies it.
    border_glyphs: Glyphs,
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
}

/// The context menus' action vocabulary: rename/close per target, the
/// tab menu's add-tab, the workspace menu's new-workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Rename,
    Close,
    AddTab,
    NewWorkspace,
}

/// The open menu's state: its target plus the action dispatched per
/// overlay row (the parallel mapping `compose_menu_panel` returns; row
/// `None` entries — the header and the footer — consume the click).
#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuState {
    target: MenuTarget,
    actions: Vec<Option<MenuAction>>,
}

/// The context menus' footer controls line.
const MENU_FOOTER: &str = " action click · close esc/q ";

/// The context menus' overlay title (the target's name rides the panel's
/// accent header row).
const MENU_TITLE: &str = " menu ";

/// Compose the context menu's overlay panel for `target`: the target's
/// name as the accent header, one row per action, the footer controls
/// line — [`super::compose_prompt_panel`]'s row/footer shape. Returns
/// the panel rows AND the action per row (`None` for the header and
/// footer) — the click dispatch's pure mapping, so what a click
/// highlights is exactly what painted there.
fn compose_menu_panel(target: &MenuTarget, name: &str) -> (Vec<HelpRow>, Vec<Option<MenuAction>>) {
    let actions: &[MenuAction] = match target {
        MenuTarget::Tab(_) => &[MenuAction::Rename, MenuAction::Close, MenuAction::AddTab],
        MenuTarget::Workspace(_) => &[
            MenuAction::Rename,
            MenuAction::Close,
            MenuAction::NewWorkspace,
        ],
    };
    let labels: &[&str] = match target {
        MenuTarget::Tab(_) => &["rename", "close", "add tab"],
        MenuTarget::Workspace(_) => &["rename", "close", "new"],
    };
    let mut rows = vec![HelpRow {
        text: format!(" {name} "),
        accent: true,
        footer: false,
    }];
    let mut mapping = vec![None];
    for (action, label) in actions.iter().zip(labels.iter()) {
        rows.push(HelpRow {
            text: format!(" {label} "),
            accent: false,
            footer: false,
        });
        mapping.push(Some(*action));
    }
    rows.push(HelpRow {
        text: MENU_FOOTER.to_string(),
        accent: false,
        footer: true,
    });
    mapping.push(None);
    (rows, mapping)
}

impl WindowSession {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            window: String::new(),
            renderer: PaneRenderer::new(cols, rows.saturating_sub(2), Glyphs::Unicode),
            pending_layout: None,
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
            bg: None,
            pane_borders: false,
            show_label_in_border: false,
            pane_gaps: 0,
            scrollbar_gutter: false,
            sidebar_on: false,
            sidebar_width: 20,
            pending_grid_refit: false,
            drag_cursor_shape: false,
            border_glyphs: Glyphs::Unicode,
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
        }
    }

    /// Record the session's resolved background: the renderer carries it
    /// AND the session keeps a copy so renderer reconstruction (re-seed,
    /// resize re-fit) re-applies at the next paint. `None` = the probe
    /// failed; the fill stays terminal-default.
    fn set_background(&mut self, bg: Option<RtColor>) {
        self.bg = bg;
        self.renderer.set_background(bg);
    }

    /// Apply the config `border-lines` spelling to the session's glyph
    /// set; reports whether the spelling was recognized (an unknown
    /// value keeps the current set — the caller warns).
    fn set_border_lines(&mut self, spec: &str) -> bool {
        let glyphs = match spec {
            "" | "unicode" => Glyphs::Unicode,
            "double" => Glyphs::Double,
            "heavy" => Glyphs::Heavy,
            "ascii" => Glyphs::Ascii,
            "herdr" => Glyphs::Herdr,
            _ => return false,
        };
        self.border_glyphs = glyphs;
        self.renderer.set_glyphs(glyphs);
        // The style owns the paint mode: herdr draws every pane its own
        // complete box, the rest the shared dividers. Applied before the
        // config's explicit `pane-borders` at the init/reload sites, so
        // that key still overrides for any glyph set.
        self.set_pane_borders(matches!(glyphs, Glyphs::Herdr));
        true
    }

    /// The session's pane-border mode (config `pane-borders`): each pane
    /// renders its own complete box with the content inset by the border
    /// cells, replacing the shared-divider look. Session-level like the
    /// background so renderer reconstruction re-applies it.
    fn set_pane_borders(&mut self, on: bool) {
        self.pane_borders = on;
        self.renderer.set_pane_borders(on);
    }

    /// The session's border colors (config `border-active-color` /
    /// `border-color`, `#rrggbb` hex; `None` keeps the built-ins).
    /// Session-level like the background so renderer reconstruction
    /// re-applies it.
    fn set_border_colors(&mut self, active: Option<RtColor>, plain: Option<RtColor>) {
        self.renderer.set_border_colors(active, plain);
    }

    /// The session's label-in-border mode (config `show-label-in-border`):
    /// each pane's user title renders embedded in its top border edge.
    fn set_show_label_in_border(&mut self, on: bool) {
        self.show_label_in_border = on;
        self.renderer.set_show_label_in_border(on);
    }

    /// The session's gap-band mode (config `pane-gaps`): theme-bg gap
    /// bands between panes, the renderer insetting each pane's rect.
    fn set_pane_gaps(&mut self, gaps: u16) {
        self.pane_gaps = gaps;
        self.renderer.set_pane_gaps(gaps);
    }

    /// The session's scrollbar-gutter mode (config `scrollbar-gutter`):
    /// a right-edge gutter column reserved in every pane rect.
    fn set_scrollbar_gutter(&mut self, on: bool) {
        self.scrollbar_gutter = on;
        self.renderer.set_scrollbar_gutter(on);
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

    /// Seed the window: resolve it from the target (a pane/window/session
    /// target narrows as in passthrough; none = the newest session's
    /// active window), report the renderer size against one of its panes
    /// (the `%layout-change` reply carries the layout triple), parse the
    /// layout, then replay every pane's screen into its emulator.
    fn seed(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
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
        self.tab_strip.invalidate();
        self.draw_tab_strip();
        // First frame: clear + full paint. repaint_all hides the host
        // cursor, so the frame's place_cursor must fire even if the
        // mapped state matches the hidden default.
        self.cursor_placed = Some(None);
        sink.repaint_all();
        self.frame(sink);
        Ok(())
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
            //     view (docs/MUX.md's %sessions-changed client contract).
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
                    Err(status::StatusError::SessionGone) => return Ok(()),
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
            | TmuxNotification::WorkspacesChanged
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
    /// the post-resize re-seed share this). The reply body's lines join
    /// back EXACTLY: the wire's body lines split on `\n` (BufRead::lines)
    /// and joining restores the byte stream — appending another `\n`
    /// would add a line feed the pane's restore stream never contained,
    /// driving the freshly positioned cursor one row below the tracked
    /// cell (the round-3 cursor off-by-one) or scrolling the grid when
    /// the cursor sat on the bottom row.
    fn replay_all_panes(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        for rect in self.renderer.layout().to_vec() {
            let pane = format!("%{}", rect.pane);
            if let Ok(reply) = conn.send_checked(&format!("refresh-client -t {pane}")) {
                if reply.ok {
                    let bytes = reply.body.join("\n").into_bytes();
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
        conn: &mut crate::mux::attach::conn::AttachConn,
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
                                    // key: the arrows navigate panes
                                    // directionally, shift+arrows swap
                                    // with the neighbor; anything else
                                    // is unbound — consumed either way.
                                    self.prefix_pane_arrow(conn, &ev);
                                    continue;
                                }
                                if self.scroll_mode {
                                    self.scroll_mode_key(&ev);
                                    continue;
                                }
                                if self.prompt_mode {
                                    self.prompt_key(conn, &ev);
                                    continue;
                                }
                                if self.menu.is_some() {
                                    self.menu_key(&ev);
                                    continue;
                                }
                                if self.help_mode {
                                    self.help_key(&ev);
                                    continue;
                                }
                                if self.picker_mode {
                                    self.picker_key(conn, &ev);
                                    continue;
                                }
                                if self.resize_mode {
                                    self.resize_mode_key(conn, &ev);
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
        conn: &mut crate::mux::attach::conn::AttachConn,
        prefix_pending: &mut bool,
    ) -> bool {
        if self.prompt_mode {
            // The prompt owns the run whole: every byte feeds the input
            // line until Enter commits or Escape cancels; the closing
            // byte and everything after in the run is dropped (the
            // picker's modal-run shape).
            for &byte in bytes {
                if !self.prompt_byte(conn, byte) {
                    return false;
                }
            }
            return false;
        }
        if self.menu.is_some() {
            for &byte in bytes {
                if !self.menu_byte(byte) {
                    return false;
                }
            }
            return false;
        }
        if self.help_mode {
            for &byte in bytes {
                if !self.help_byte(byte) {
                    return false;
                }
            }
            return false;
        }
        if self.picker_mode {
            for &byte in bytes {
                if !self.picker_byte(conn, byte) {
                    return false;
                }
            }
            return false;
        }
        if self.resize_mode {
            // Resize mode owns plain runs: the bytes are typed keys that
            // would leak into the pane, so the run exits the mode and is
            // consumed whole (arrows arrive as Key tokens, not bytes).
            self.resize_mode = false;
            return false;
        }
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
                // The management chords match by byte before the fixed
                // table (configurable; the defaults `%`, `"`, `x`, `c`
                // are consumed unbound by the table today).
                let management = match byte {
                    b if b == self.management.split_right => Some(ManagementKey::SplitRight),
                    b if b == self.management.split_down => Some(ManagementKey::SplitDown),
                    b if b == self.management.kill_pane => Some(ManagementKey::KillPane),
                    b if b == self.management.new_window => Some(ManagementKey::NewWindow),
                    b if b == self.management.swap_prev => Some(ManagementKey::SwapPrev),
                    b if b == self.management.swap_next => Some(ManagementKey::SwapNext),
                    b if b == self.management.workspace_next => Some(ManagementKey::WorkspaceNext),
                    b if b == self.management.workspace_prev => Some(ManagementKey::WorkspacePrev),
                    b if b == self.management.zoom => Some(ManagementKey::Zoom),
                    b if b == self.management.rename_window => Some(ManagementKey::RenameWindow),
                    b if b == self.management.rename_pane => Some(ManagementKey::RenamePane),
                    b if b == self.management.border_cycle => Some(ManagementKey::BorderCycle),
                    b if b == self.management.label_toggle => Some(ManagementKey::Labels),
                    b if b == self.management.workspace_picker => {
                        Some(ManagementKey::WorkspacePicker)
                    }
                    b if b == self.management.sidebar => Some(ManagementKey::Sidebar),
                    b if b == self.management.status_bar => Some(ManagementKey::StatusBar),
                    _ => None,
                };
                if let Some(key) = management {
                    self.management_chord(key, conn);
                    continue;
                }
                // The resize chord: a sticky mode — arrows adjust the
                // focused pane's edges until Enter/Escape/q.
                if byte == self.management.resize {
                    self.enter_resize_mode();
                    continue;
                }
                // The help chord: the bindings overlay over the frame.
                if byte == self.management.help {
                    self.enter_help();
                    continue;
                }
                // The picker chord: the session/window modal over the
                // frame.
                if byte == self.management.picker {
                    self.enter_picker(conn);
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

    /// Enter the sticky resize mode (the `resize` chord): arrows adjust
    /// the focused pane's edges until Enter/Escape/`q` — tmux's resize
    /// step with an explicit mode instead of repeat-time. The flash cue
    /// rides the frame cadence on the status row.
    fn enter_resize_mode(&mut self) {
        self.resize_mode = true;
        self.flash = Some(format!(
            "resize — arrows move the edge by {}, Enter/q exits",
            self.resize_step
        ));
    }

    /// Open the bindings help panel (the `help` chord): the live chord
    /// state's categories composed into the bordered modal (title
    /// `keybinds`, `esc close` badge, footer controls line) painted
    /// centered over the frame. `/` opens a filter-as-you-type box, j/k
    /// and pgup/pgdn scroll, esc/Enter/q close and the prior frame
    /// repaints. Keys never reach the pane while it is up.
    fn enter_help(&mut self) {
        self.help_mode = true;
        self.help_filter.clear();
        self.help_filtering = false;
        self.help_scroll = 0;
        self.refresh_help();
    }

    /// Dismiss the help panel: the next frame's pane repaint restores the
    /// covered cells (the frame buffer resets, then panes repaint).
    fn leave_help(&mut self) {
        self.help_mode = false;
        self.renderer.set_overlay(None);
    }

    /// Re-compose the overlay from the live panel state (filter/scroll).
    fn refresh_help(&mut self) {
        let rows = super::help_rows(
            self.prefix,
            self.reload_key,
            self.management,
            self.resize_step,
        );
        // The modal's own chrome (2 border rows) and the footer surround
        // the content window; the filter line joins them only while a
        // filter is open or set (the footer already advertises
        // `search /` — no idle placeholder row).
        let filtering = self.help_filtering || !self.help_filter.is_empty();
        let filter_lines = usize::from(filtering);
        let visible = usize::from(
            self.renderer
                .window_size()
                .1
                .saturating_sub(3 + filter_lines as u16),
        )
        .max(1);
        let content_len = super::help_content(&rows, &self.help_filter).len();
        let start = super::help_window_start(content_len, visible, self.help_scroll);
        let lines = super::compose_help_panel(
            &rows,
            &self.help_filter,
            self.help_filtering,
            visible,
            self.help_scroll,
        );
        self.renderer.set_overlay(Some((
            super::HELP_OVERLAY_TITLE,
            lines,
            Some((start, visible, content_len)),
        )));
    }

    /// One key while the help panel is up: `/` opens the filter box
    /// (typing edits it, Enter commits and keeps the filter, Esc closes
    /// the panel), j/k/arrows/pgup/pgdn scroll, esc/Enter/q close. Every
    /// key is consumed — nothing leaks into the pane.
    fn help_key(&mut self, ev: &TermKeyEvent) {
        use crate::keyboard::TermKey;
        if self.help_filtering {
            match ev.key() {
                TermKey::Char => {
                    if let Some(ch) = char::from_u32(ev.codepoint) {
                        self.help_filter.push(ch);
                    }
                }
                TermKey::Escape => {
                    self.leave_help();
                    return;
                }
                _ => self.help_filtering = false,
            }
            self.refresh_help();
            return;
        }
        match (ev.key(), ev.modifiers) {
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'/') => {
                self.help_filtering = true;
                self.refresh_help();
            }
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'j') => self.help_scroll_by(1),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'k') => self.help_scroll_by(-1),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'q') => self.leave_help(),
            (TermKey::Up, 0) => self.help_scroll_by(-1),
            (TermKey::Down, 0) => self.help_scroll_by(1),
            (TermKey::PageUp, 0) => self.help_scroll_by(-10),
            (TermKey::PageDown, 0) => self.help_scroll_by(10),
            // Enter (arrives as a byte via route_plain) and everything
            // else close the panel.
            _ => self.leave_help(),
        }
    }

    /// Scroll the help panel's content window (clamped by the compose).
    fn help_scroll_by(&mut self, delta: isize) {
        self.help_scroll = (self.help_scroll as isize + delta).max(0) as usize;
        self.refresh_help();
    }

    /// One plain byte while the help panel is up — the same controls the
    /// key path takes, for the byte spellings (Backspace pops the filter,
    /// `/` opens it, Enter commits or closes, `q` closes).
    fn help_byte(&mut self, byte: u8) -> bool {
        if self.help_filtering {
            match byte {
                0x7f => {
                    self.help_filter.pop();
                    self.refresh_help();
                }
                b'\r' => self.help_filtering = false,
                b if byte != 0x1b && (b.is_ascii_graphic() || b == b' ') => {
                    self.help_filter.push(b as char);
                    self.refresh_help();
                }
                _ => {}
            }
            return self.help_mode;
        }
        match byte {
            b'/' => {
                self.help_filtering = true;
                self.refresh_help();
            }
            b'q' | b'\r' => self.leave_help(),
            _ => {}
        }
        self.help_mode
    }

    /// Open the session/window picker (the `picker` chord): the daemon\'s
    /// session roster with each session\'s windows nested, composed into
    /// the same themed modal the help panel uses (title `picker`, `esc
    /// close` badge, footer controls line), the current session/window
    /// `>`-marked. `/` opens the filter box; arrows/j/k move the
    /// selection; Enter selects through [`Self::picker_activate`];
    /// esc/q dismiss and the prior frame repaints. Keys and the mouse
    /// never reach the pane while it is up.
    fn enter_picker(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Ok(reply) = conn.send_checked("list-sessions") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let mut entries: Vec<super::PickerEntry> = Vec::new();
        for line in &reply.body {
            let Some((sid, name)) = super::parse_session_line(line) else {
                continue;
            };
            let mut windows: Vec<(String, String)> = Vec::new();
            let mut active_window: Option<String> = None;
            if let Ok(r) = conn.send_checked(&format!("list-windows -t {sid}")) {
                if r.ok {
                    for wl in &r.body {
                        let mut fields = wl.split_whitespace();
                        let Some(id) = fields.next().filter(|id| id.starts_with('@')) else {
                            continue;
                        };
                        let marker = fields.next().unwrap_or("-");
                        let rest = fields.collect::<Vec<_>>().join(" ");
                        let wname = if rest.is_empty() { id } else { &rest };
                        if marker == "*" {
                            active_window = Some(id.to_string());
                        }
                        windows.push((id.to_string(), wname.to_string()));
                    }
                }
            }
            let current = self.status.session_id.as_deref() == Some(sid.as_str());
            entries.push(super::PickerEntry {
                session_id: sid,
                session_name: name,
                windows,
                active_window,
                current,
            });
        }
        if entries.is_empty() {
            return;
        }
        self.picker_entries = entries;
        self.picker_filter.clear();
        self.picker_filtering = false;
        self.picker_start = 0;
        self.picker_mode = true;
        // The selection opens on the current session\'s header row (or the
        // first row when the view\'s session is unknown to the roster).
        let (_rows, refs) = super::picker_rows(&self.picker_entries, Some(&self.window));
        self.picker_selected = refs
            .iter()
            .position(
                |r| matches!(r, super::PickerRef::Session(i) if self.picker_entries[*i].current),
            )
            .unwrap_or(0);
        self.refresh_picker();
    }

    /// Open the workspace picker (the workspace-picker chord): the
    /// daemon's workspace roster — id, name, the active one `>`-marked —
    /// in the same themed modal, filter, cursor, and click handling the
    /// session/window picker runs. Enter lands on the workspace through
    /// the same select-then-resync every switch follows.
    fn enter_ws_picker(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Ok(reply) = conn.send_checked("list-workspaces") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let workspaces: Vec<(String, String, bool)> = reply
            .body
            .iter()
            .filter_map(|l| super::parse_workspace_line(l))
            .collect();
        if workspaces.is_empty() {
            return;
        }
        let open_on = workspaces.iter().position(|(_, _, active)| *active);
        self.picker_workspaces = Some(workspaces);
        self.picker_filter.clear();
        self.picker_filtering = false;
        self.picker_start = 0;
        self.picker_mode = true;
        self.picker_selected = open_on.unwrap_or(0);
        self.refresh_ws_picker();
    }

    /// Dismiss the picker: the next frame\'s pane repaint restores the
    /// covered cells.
    fn leave_picker(&mut self) {
        self.picker_mode = false;
        self.picker_workspaces = None;
        self.renderer.set_overlay(None);
    }

    /// Open the prompt modal for `target`: the rename flows seed the
    /// target's current name — the window's or workspace's own name (a
    /// context menu may target a window/workspace the view is not
    /// showing) — and the create flows seed the next free index.
    fn enter_prompt(&mut self, target: PromptTarget) {
        let seed = match &target {
            PromptTarget::Pane => self.status.pane_title().to_string(),
            PromptTarget::Window(id) => self
                .status
                .windows()
                .iter()
                .find(|(wid, _)| wid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_default(),
            PromptTarget::NewWindow => super::next_window_name(self.status.windows()),
            PromptTarget::Workspace(id) => self
                .status
                .workspaces()
                .iter()
                .find(|(wsid, _)| wsid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_default(),
            PromptTarget::NewWorkspace => super::next_workspace_name(self.status.workspaces()),
        };
        self.prompt_target = target;
        self.prompt_text = seed;
        self.prompt_mode = true;
        self.refresh_prompt();
    }

    /// Dismiss the prompt: the next frame's pane repaint restores the
    /// covered cells.
    fn leave_prompt(&mut self) {
        self.prompt_mode = false;
        self.renderer.set_overlay(None);
    }

    /// Open the context menu for `target`: the tab menu (right-press on
    /// a tab) or the workspace menu (the panel's ` menu ` chip, a
    /// right-press on a workspace row). The overlay rides the same
    /// modal machinery the picker uses.
    fn open_menu(&mut self, target: MenuTarget) {
        let name = match &target {
            MenuTarget::Tab(id) => self
                .status
                .windows()
                .iter()
                .find(|(wid, _)| wid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| id.clone()),
            MenuTarget::Workspace(id) => self
                .status
                .workspaces()
                .iter()
                .find(|(wsid, _)| wsid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| id.clone()),
        };
        let (rows, actions) = compose_menu_panel(&target, &name);
        self.menu = Some(MenuState { target, actions });
        self.renderer.set_overlay(Some((MENU_TITLE, rows, None)));
    }

    /// Dismiss the menu: the next frame's pane repaint restores the
    /// covered cells.
    fn leave_menu(&mut self) {
        self.menu = None;
        self.renderer.set_overlay(None);
    }

    /// One key while the menu is up: Escape closes, everything else is
    /// consumed (a modal owns the keyboard).
    fn menu_key(&mut self, ev: &TermKeyEvent) {
        use crate::keyboard::TermKey;
        if ev.key() == TermKey::Escape {
            self.leave_menu();
        }
    }

    /// One plain byte while the menu is up: `q`/Escape close, the rest
    /// consumed.
    fn menu_byte(&mut self, byte: u8) -> bool {
        if byte == b'q' || byte == 0x1b {
            self.leave_menu();
        }
        self.menu.is_some()
    }

    /// One press click while the menu is up, at overlay row `row` (the
    /// `overlay_row_at` mapping): an action row dispatches, the header
    /// and footer consume. The menu closes first so an action's own
    /// overlay (the rename prompt, the new-tab prompt) replaces it.
    fn menu_click(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, row: usize) {
        let Some(state) = self.menu.as_ref() else {
            return;
        };
        let Some(action) = state.actions.get(row).copied().flatten() else {
            return; // header/footer/off-panel: consumed
        };
        let target = state.target.clone();
        self.leave_menu();
        match (target, action) {
            (MenuTarget::Tab(window), MenuAction::Rename) => {
                self.enter_prompt(PromptTarget::Window(window));
            }
            (MenuTarget::Tab(_), MenuAction::AddTab) => {
                self.enter_prompt(PromptTarget::NewWindow);
            }
            (MenuTarget::Tab(window), MenuAction::Close) => {
                self.close_window_from_menu(conn, &window);
            }
            (MenuTarget::Workspace(workspace), MenuAction::Rename) => {
                self.enter_prompt(PromptTarget::Workspace(workspace));
            }
            (MenuTarget::Workspace(_), MenuAction::NewWorkspace) => {
                self.enter_prompt(PromptTarget::NewWorkspace);
            }
            (MenuTarget::Workspace(workspace), MenuAction::Close) => {
                self.close_workspace_from_menu(conn, &workspace);
            }
            // The cross pair never composes (each menu carries only its
            // own third action).
            _ => {}
        }
    }

    /// The tab menu's close: `kill-window -t {id}`; when the killed
    /// window is the SHOWN one, land on the session's surviving
    /// daemon-active (or first) window through the select+resync — a
    /// session that died with its last window ends the view through the
    /// next status refresh's SessionGone. Any other target just marks
    /// the status stale.
    fn close_window_from_menu(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) {
        let shown = self.window == window;
        let _ = conn.send_checked(&format!("kill-window -t {window}"));
        if !shown {
            self.status_dirty = true;
            return;
        }
        let Some(session) = self.status.session_id.clone() else {
            self.status_dirty = true;
            return;
        };
        let Ok(reply) = conn.send_checked(&format!("list-windows -t {session}")) else {
            return;
        };
        if !reply.ok {
            return;
        }
        let survivor = reply
            .body
            .iter()
            .find(|l| l.split_whitespace().nth(1) == Some("*"))
            .or_else(|| reply.body.first())
            .and_then(|l| l.split_whitespace().next())
            .map(str::to_string);
        let Some(survivor) = survivor else {
            // The session died with the window: the next status refresh
            // reports SessionGone and the view ends cleanly.
            self.status_dirty = true;
            return;
        };
        let _ = conn.send_checked(&format!("select-window -t {survivor}"));
        self.reseed_window(conn, &survivor);
    }

    /// The workspace menu's close: `kill-workspace -t {id}`; when the
    /// killed workspace was the SHOWN one (the view's session lives in
    /// it), land on the first surviving workspace through the
    /// select+land contract — none remaining ends the view cleanly
    /// (the daemon kills the session, the refresh's SessionGone fires).
    /// Any other target just marks the status stale.
    fn close_workspace_from_menu(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        workspace: &str,
    ) {
        // Was the view's session inside this workspace?
        let shown = self.status.session_id.as_ref().is_some_and(|session| {
            conn.send_checked(&format!("list-sessions -t {workspace}"))
                .ok()
                .filter(|reply| reply.ok)
                .is_some_and(|reply| {
                    reply
                        .body
                        .iter()
                        .filter_map(|l| super::parse_session_line(l))
                        .any(|(sid, _)| sid == *session)
                })
        });
        let _ = conn.send_checked(&format!("kill-workspace -t {workspace}"));
        if !shown {
            self.status_dirty = true;
            return;
        }
        let Ok(reply) = conn.send_checked("list-workspaces") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let survivor = reply
            .body
            .iter()
            .filter_map(|l| super::parse_workspace_line(l))
            .map(|(id, _, _)| id)
            .next();
        let Some(survivor) = survivor else {
            // No workspace remains: the view ends cleanly through the
            // next status refresh.
            self.status_dirty = true;
            return;
        };
        let _ = conn.send_checked(&format!("select-workspace -t {survivor}"));
        self.land_on_workspace(conn, &survivor);
    }

    /// Re-compose the prompt overlay from the edit buffer.
    fn refresh_prompt(&mut self) {
        let title = match &self.prompt_target {
            PromptTarget::Pane => super::PROMPT_PANE_OVERLAY_TITLE,
            PromptTarget::Window(_) => super::PROMPT_WINDOW_OVERLAY_TITLE,
            PromptTarget::NewWindow => super::PROMPT_NEW_WINDOW_OVERLAY_TITLE,
            PromptTarget::Workspace(_) => super::PROMPT_WORKSPACE_OVERLAY_TITLE,
            PromptTarget::NewWorkspace => super::PROMPT_NEW_WORKSPACE_OVERLAY_TITLE,
        };
        let footer = match self.prompt_target {
            PromptTarget::NewWindow | PromptTarget::NewWorkspace => super::NEW_PROMPT_FOOTER,
            _ => super::PROMPT_FOOTER,
        };
        self.renderer.set_overlay(Some((
            title,
            super::compose_prompt_panel(&self.prompt_text, footer),
            None,
        )));
    }

    /// One plain byte while the prompt is up: printable bytes and
    /// spaces append, Backspace pops, ^C clears the input (herdr's
    /// footer), Enter commits (the daemon command below), Escape
    /// cancels. Returns whether the mode is still up.
    fn prompt_byte(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, byte: u8) -> bool {
        match byte {
            0x1b => self.leave_prompt(),
            b'\r' => self.commit_prompt(conn),
            0x03 => {
                self.prompt_text.clear();
                self.refresh_prompt();
            }
            0x7f => {
                self.prompt_text.pop();
                self.refresh_prompt();
            }
            b if b.is_ascii_graphic() || b == b' ' => {
                self.prompt_text.push(b as char);
                self.refresh_prompt();
            }
            _ => {}
        }
        self.prompt_mode
    }

    /// One key event while the prompt is up: characters append, Escape
    /// cancels, everything else is consumed (the byte path carries the
    /// controls).
    fn prompt_key(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, ev: &TermKeyEvent) {
        let _ = conn;
        use crate::keyboard::TermKey;
        match ev.key() {
            TermKey::Char => {
                if let Some(ch) = char::from_u32(ev.codepoint) {
                    self.prompt_text.push(ch);
                }
            }
            TermKey::Escape => {
                self.leave_prompt();
                return;
            }
            _ => {}
        }
        if self.prompt_mode {
            self.refresh_prompt();
        }
    }

    /// Commit the prompt: the pane spelling sets the sticky user title
    /// (`select-pane -T`), the window and workspace spellings rename
    /// their target (`rename-window`/`rename-workspace`, quoted), the
    /// create flows make the window/workspace and land the view on it;
    /// an empty input cancels (an empty name is not expressible on the
    /// wire — the parses require one).
    fn commit_prompt(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let text = self.prompt_text.trim().to_string();
        if text.is_empty() {
            self.leave_prompt();
            return;
        }
        let target = self.prompt_target.clone();
        match target {
            PromptTarget::Pane => {
                let _ = conn.send_checked(&format!(
                    "select-pane -t {} -T {}",
                    self.focused_pane(),
                    super::wire_quote(&text)
                ));
                self.leave_prompt();
                self.status_dirty = true;
            }
            PromptTarget::Window(id) => {
                let _ = conn.send_checked(&format!(
                    "rename-window -t {id} {}",
                    super::wire_quote(&text)
                ));
                self.leave_prompt();
                self.status_dirty = true;
            }
            PromptTarget::Workspace(id) => {
                let _ = conn.send_checked(&format!(
                    "rename-workspace -t {id} {}",
                    super::wire_quote(&text)
                ));
                self.leave_prompt();
                self.status_dirty = true;
            }
            PromptTarget::NewWindow => {
                let Some(session) = self.status.session_id.clone() else {
                    self.leave_prompt();
                    return;
                };
                let Ok(reply) = conn.send_checked(&format!(
                    "new-window -t {session} -n {}",
                    super::wire_quote(&text)
                )) else {
                    self.leave_prompt();
                    return;
                };
                self.leave_prompt();
                if !reply.ok {
                    return;
                }
                if let Some(window) = reply.body.first().map(|w| w.trim().to_string()) {
                    let _ = conn.send_checked(&format!("select-window -t {window}"));
                    self.reseed_window(conn, &window);
                }
            }
            PromptTarget::NewWorkspace => {
                let Ok(reply) =
                    conn.send_checked(&format!("new-workspace -n {}", super::wire_quote(&text)))
                else {
                    self.leave_prompt();
                    return;
                };
                self.leave_prompt();
                if !reply.ok {
                    return;
                }
                if let Some(workspace) = reply.body.first().map(|w| w.trim().to_string()) {
                    let _ = conn.send_checked(&format!("select-workspace -t {workspace}"));
                    self.land_on_workspace(conn, &workspace);
                }
            }
        }
    }

    /// The zoom state's effective layout — the daemon's zoom re-lays the
    /// window itself, so the client applies what the broadcast carries;
    /// the cache of the unzoomed tree geometry exists only so the
    /// prefix+arrow navigation can leave a zoom (the daemon unzooms on
    /// the select it issues).
    fn tree_layout(&self) -> Vec<PaneRect> {
        if self.zoomed && !self.daemon_layout.is_empty() {
            self.daemon_layout.clone()
        } else {
            self.renderer.layout().to_vec()
        }
    }

    /// Re-compose the picker overlay from the live state (filter,
    /// selection, pan). The workspace picker composes its own rows and
    /// rides the same filter/window/footer machinery.
    fn refresh_picker(&mut self) {
        if self.picker_workspaces.is_some() {
            self.refresh_ws_picker();
            return;
        }
        let (rows, refs) = super::picker_rows(&self.picker_entries, Some(&self.window));
        let visible = self.picker_visible();
        let (lines, filtered_refs, start) = super::compose_picker_panel(
            &rows,
            &refs,
            &self.picker_filter,
            self.picker_filtering,
            self.picker_selected,
            visible,
            self.picker_start,
        );
        self.picker_refs = filtered_refs;
        self.picker_start = start;
        self.picker_panel_len = lines.len();
        self.renderer
            .set_overlay(Some((super::PICKER_OVERLAY_TITLE, lines, None)));
    }

    /// The picker's visible content-row count (the same window the
    /// compose uses; the filter line joins the chrome only while a
    /// filter is open or set).
    fn picker_visible(&self) -> usize {
        let filtering = self.picker_filtering || !self.picker_filter.is_empty();
        let filter_lines = usize::from(filtering);
        usize::from(
            self.renderer
                .window_size()
                .1
                .saturating_sub(3 + filter_lines as u16),
        )
        .max(1)
    }

    /// Move the picker\'s selection cursor by `delta` content rows (the
    /// cursor wraps at the ends of the filtered list) and re-compose.
    fn picker_move(&mut self, delta: isize) {
        let count = self.picker_refs.len();
        if count == 0 {
            return;
        }
        self.picker_selected =
            (self.picker_selected as isize + delta).rem_euclid(count as isize) as usize;
        self.refresh_picker();
    }

    /// Activate the selected picker row: a session header lands on the
    /// session\'s active window (the select-session-equivalent), a window
    /// row lands on that window — daemon-side `select-window`, then the
    /// select+resync re-seed every switch follows.
    fn picker_activate(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let idx = self
            .picker_selected
            .min(self.picker_refs.len().saturating_sub(1));
        let Some(item) = self.picker_refs.get(idx).copied() else {
            return;
        };
        let super::PickerRef::Workspace(index) = item else {
            return self.ws_picker_activate(conn, item);
        };
        let Some(workspaces) = self.picker_workspaces.clone() else {
            return;
        };
        let Some((id, _, _)) = workspaces.get(index) else {
            self.leave_picker();
            return;
        };
        let id = id.clone();
        self.leave_picker();
        self.land_on_workspace(conn, &id);
    }

    /// Activate a session/window picker row: a session header lands on
    /// the session's active window, a window row on that window.
    fn ws_picker_activate(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        item: super::PickerRef,
    ) {
        let entry_index = match item {
            super::PickerRef::Session(i) | super::PickerRef::Window(i, _) => i,
            super::PickerRef::Workspace(_) => return,
        };
        let entry = &self.picker_entries[entry_index];
        let window = match item {
            // Unreachable: ws_picker_activate returned early on Workspace.
            super::PickerRef::Workspace(_) => None,
            super::PickerRef::Session(_) => entry
                .active_window
                .clone()
                .or_else(|| entry.windows.first().map(|(id, _)| id.clone())),
            super::PickerRef::Window(_, w) => entry.windows.get(w).map(|(id, _)| id.clone()),
        };
        let Some(window) = window else {
            // A session with no windows: nothing to land on; dismiss.
            self.leave_picker();
            return;
        };
        self.leave_picker();
        let _ = conn.send_checked(&format!("select-window -t {window}"));
        self.reseed_window(conn, &window);
    }

    /// Compose the workspace picker's overlay: one row per workspace
    /// (`>`- and `*`-marked when active), through the same filter,
    /// cursor, windowing, and footer machinery as the session picker.
    fn refresh_ws_picker(&mut self) {
        let Some(workspaces) = self.picker_workspaces.clone() else {
            return;
        };
        let mut rows: Vec<super::HelpRow> = Vec::new();
        let mut refs: Vec<super::PickerRef> = Vec::new();
        for (i, (id, name, active)) in workspaces.iter().enumerate() {
            let marker = if *active { ">" } else { " " };
            let star = if *active { " *" } else { "" };
            rows.push(super::HelpRow {
                text: format!(" {marker}{id}  {name}{star}"),
                accent: false,
                footer: false,
            });
            refs.push(super::PickerRef::Workspace(i));
        }
        let visible = self.picker_visible();
        let (lines, filtered_refs, start) = super::compose_picker_panel(
            &rows,
            &refs,
            &self.picker_filter,
            self.picker_filtering,
            self.picker_selected,
            visible,
            self.picker_start,
        );
        self.picker_refs = filtered_refs;
        self.picker_start = start;
        self.picker_panel_len = lines.len();
        self.renderer
            .set_overlay(Some((super::WORKSPACE_PICKER_OVERLAY_TITLE, lines, None)));
    }

    /// One key while the picker is up: the same modal-key shape the help
    /// panel runs, plus the selection cursor. Every key is consumed —
    /// nothing leaks into the pane.
    fn picker_key(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, ev: &TermKeyEvent) {
        let _ = conn;
        use crate::keyboard::TermKey;
        if self.picker_filtering {
            match ev.key() {
                TermKey::Char => {
                    if let Some(ch) = char::from_u32(ev.codepoint) {
                        self.picker_filter.push(ch);
                    }
                }
                TermKey::Escape => {
                    self.leave_picker();
                    return;
                }
                _ => self.picker_filtering = false,
            }
            if self.picker_mode {
                self.refresh_picker();
            }
            return;
        }
        match (ev.key(), ev.modifiers) {
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'/') => {
                self.picker_filtering = true;
                self.refresh_picker();
            }
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'j') => self.picker_move(1),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'k') => self.picker_move(-1),
            (TermKey::Up, 0) => self.picker_move(-1),
            (TermKey::Down, 0) => self.picker_move(1),
            _ => self.leave_picker(),
        }
    }

    /// One plain byte while the picker is up — the byte spellings of the
    /// same controls (Backspace pops the filter, `/` opens it, Enter
    /// selects, `q`/Escape close).
    fn picker_byte(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, byte: u8) -> bool {
        if self.picker_filtering {
            match byte {
                0x7f => {
                    self.picker_filter.pop();
                    self.refresh_picker();
                }
                0x1b => {
                    self.leave_picker();
                    return false;
                }
                b'\r' => self.picker_filtering = false,
                b if b.is_ascii_graphic() || b == b' ' => {
                    self.picker_filter.push(b as char);
                    self.refresh_picker();
                }
                _ => {}
            }
            return self.picker_mode;
        }
        match byte {
            b'/' => {
                self.picker_filtering = true;
                self.refresh_picker();
            }
            b'j' => self.picker_move(1),
            b'k' => self.picker_move(-1),
            b'\r' => self.picker_activate(conn),
            b'q' | 0x1b => self.leave_picker(),
            _ => {}
        }
        self.picker_mode
    }

    /// One key while resize mode is up: arrows send one resize step for
    /// the focused pane (`resize-pane -t <pane> -L|-R|-U|-D <step>`, the
    /// wire's relative form); Escape and `q` exit; every other key exits
    /// the mode and is consumed (keys must not leak into the pane during
    /// a modal chord).
    fn resize_mode_key(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        use crate::keyboard::TermKey;
        match ev.key() {
            TermKey::Up if ev.modifiers == 0 => self.send_resize(conn, "-U"),
            TermKey::Down if ev.modifiers == 0 => self.send_resize(conn, "-D"),
            TermKey::Right if ev.modifiers == 0 => self.send_resize(conn, "-R"),
            TermKey::Left if ev.modifiers == 0 => self.send_resize(conn, "-L"),
            // Escape, q, Enter (as bytes via route_plain), or anything
            // else: the mode exits and the key is consumed.
            _ => self.resize_mode = false,
        }
    }

    /// One resize step for the focused pane, the wire's relative form.
    /// Best-effort — the %layout-change broadcast the resize queues
    /// re-seeds the window through the pump's pending_layout path.
    fn send_resize(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, flag: &str) {
        let focused = self.focused_pane();
        if focused.is_empty() {
            return;
        }
        // Divider-direction semantics: the arrows move the SHARED divider
        // in the pressed direction regardless of which side is focused
        // (the manual-pass report — the right pane used to invert). The
        // wire's resize-pane GROWS the focused pane, so when the focused
        // pane sits on the far side of its boundary (a neighbor to its
        // left, or above), the wire direction inverts.
        let focused_id: u32 = focused[1..].parse().unwrap_or(u32::MAX);
        let layout = self.renderer.layout();
        let invert = match layout.iter().find(|r| r.pane == focused_id) {
            Some(f) => match flag {
                "-L" | "-R" => layout
                    .iter()
                    .any(|r| r.pane != f.pane && r.x + r.width == f.x && rows_overlap(r, f)),
                "-U" | "-D" => layout
                    .iter()
                    .any(|r| r.pane != f.pane && r.y + r.height == f.y && cols_overlap(r, f)),
                _ => false,
            },
            None => false,
        };
        let flag = match (flag, invert) {
            ("-L", true) => "-R",
            ("-R", true) => "-L",
            ("-U", true) => "-D",
            ("-D", true) => "-U",
            (other, _) => other,
        };
        let _ = conn.send_checked(&format!(
            "resize-pane -t {focused} {flag} {}",
            self.resize_step
        ));
    }

    /// The prefix commands that move the view through the daemon's tree:
    /// `o` cycles panes of the window, `n`/`p` next/prev window, `(`/`)`
    /// prev/next session — every switch is select-then-refresh, the
    /// daemon-side select + resync passthrough dispatches, with the
    /// renderer rebuilding from the fresh replays.
    fn prefix_switch(&mut self, key: u8, conn: &mut crate::mux::attach::conn::AttachConn) {
        match key {
            b'o' => self.cycle_pane(conn),
            b'n' => self.switch_window(conn, 1),
            b'p' => self.switch_window(conn, -1),
            b'(' => self.switch_session(conn, -1),
            b')' => self.switch_session(conn, 1),
            _ => {}
        }
    }

    /// prefix + arrow: select the nearest pane in the arrow's direction
    /// (tmux's directional pane navigation). While zoomed the geometry
    /// is the cached daemon layout, so the arrow can leave the zoom the
    /// way tmux's does — unzoom, then land on the neighbor. A fixed
    /// binding (no config key): the management chords are byte-matched,
    /// and arrows arrive as key events.
    fn prefix_pane_arrow(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        use crate::keyboard::TermKey;
        // Shift+arrow: swap with the pane in that direction. tmux's
        // swap-pane keeps focus following the pane's content, so no
        // re-select is needed — the %layout-change broadcast re-seeds the
        // view with the focused pane in the new cell. The shift bit is
        // matched loosely (a terminal may co-report other modifiers), and
        // both outcomes flash on the status row so a no-op at an edge is
        // never silent (the manual-pass report: the chord felt dead).
        if ev.modifiers & crate::keyboard::modifiers::SHIFT != 0 {
            let dir = match ev.key() {
                TermKey::Up => PaneDir::Up,
                TermKey::Down => PaneDir::Down,
                TermKey::Left => PaneDir::Left,
                TermKey::Right => PaneDir::Right,
                _ => return,
            };
            let Some(focused_id) = self.renderer.focused() else {
                return;
            };
            let tree = self.tree_layout();
            let Some(next) = pane_in_direction(&tree, focused_id, dir) else {
                self.flash = Some("no pane in that direction".to_string());
                self.draw_status_row();
                return;
            };
            let _ = conn.send_checked(&format!("swap-pane -s %{focused_id} -t %{next}"));
            self.flash = Some(format!("swapped %{focused_id} with %{next}"));
            self.draw_status_row();
            return;
        }
        if ev.modifiers != 0 {
            return;
        }
        let dir = match ev.key() {
            TermKey::Up => PaneDir::Up,
            TermKey::Down => PaneDir::Down,
            TermKey::Left => PaneDir::Left,
            TermKey::Right => PaneDir::Right,
            _ => return,
        };
        let Some(focused) = self.renderer.focused() else {
            return;
        };
        // The full tree geometry: while zoomed, the renderer only holds
        // the zoomed pane's expanded rect.
        let tree = self.tree_layout();
        let Some(next) = pane_in_direction(&tree, focused, dir) else {
            return;
        };
        // The daemon unzooms on the select of a different pane; the
        // broadcast re-seeds the view.
        self.renderer.focus(next);
        let _ = conn.send_checked(&format!("select-pane -t %{next}"));
    }

    /// The management chords in render mode: issue the daemon command for
    /// the focused pane/session, then land the view through the same
    /// re-seed contract `switch_window` follows — split lands on the new
    /// pane (the reply body IS its id, the daemon focuses it), kill lands
    /// on the window's survivor, new-window re-seeds the fresh window.
    fn management_chord(
        &mut self,
        key: super::ManagementKey,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) {
        let focused = self.focused_pane();
        match key {
            super::ManagementKey::SplitRight | super::ManagementKey::SplitDown => {
                let flag = if matches!(key, super::ManagementKey::SplitRight) {
                    " -h"
                } else {
                    ""
                };
                let Ok(reply) = conn.send_checked(&format!("split-window -t {focused}{flag}"))
                else {
                    return;
                };
                if !reply.ok {
                    return;
                }
                // The reply body is the new pane id; re-seed the window
                // from the fresh layout (the split broadcast rides the
                // reply), THEN focus the fresh pane — reseed_window
                // rebuilds the renderer, which resets focus to the first
                // leaf, so the focus must come after.
                let new_pane = reply
                    .body
                    .first()
                    .and_then(|id| id.trim().strip_prefix('%'))
                    .and_then(|n| n.parse::<u32>().ok());
                let window = self.window.clone();
                self.reseed_window(conn, &window);
                if let Some(new_pane) = new_pane {
                    self.renderer.focus(new_pane);
                    let _ = conn.send_checked(&format!("select-pane -t %{new_pane}"));
                }
            }
            super::ManagementKey::KillPane => {
                let Ok(reply) = conn.send_checked(&format!("kill-pane -t {focused}")) else {
                    return;
                };
                if !reply.ok {
                    return;
                }
                // The window survives with a new active pane, or the view
                // is over (the last pane died; %window-close's session
                // contract ends the view through the status refresh). The
                // survivor focus lands AFTER the re-seed (reseed_window
                // resets focus to the first leaf).
                let window = self.window.clone();
                let survivor = conn
                    .send_checked(&format!("list-panes -t {window}"))
                    .ok()
                    .filter(|reply| reply.ok)
                    .and_then(|reply| super::marked_pane(&reply.body, &window).ok());
                if let Some(pane) = survivor {
                    self.reseed_window(conn, &window);
                    if let Some(n) = pane.trim().strip_prefix('%').and_then(|n| n.parse().ok()) {
                        self.renderer.focus(n);
                        let _ = conn.send_checked(&format!("select-pane -t %{n}"));
                    }
                    return;
                }
                // No survivor: end the view like %sessions-changed's
                // contract does — the pump's next status refresh observes
                // the session gone and exits cleanly. Ending NOW would
                // skip the terminal restore; flag it and let the pump's
                // normal path tear down.
                self.status_dirty = true;
            }
            super::ManagementKey::SwapPrev | super::ManagementKey::SwapNext => {
                // Swap with the layout-order neighbor; the %layout-change
                // broadcast the swap queues re-seeds the window through
                // the pump's pending_layout path. Fewer than two panes is
                // a no-op.
                let order: Vec<u32> = self.renderer.layout().iter().map(|r| r.pane).collect();
                if order.len() < 2 {
                    return;
                }
                let Some(position) = self
                    .renderer
                    .focused()
                    .and_then(|f| order.iter().position(|p| *p == f))
                else {
                    return;
                };
                let dir = if matches!(key, super::ManagementKey::SwapPrev) {
                    -1
                } else {
                    1
                };
                let next = order[((position as i32 + dir).rem_euclid(order.len() as i32)) as usize];
                let focused = self.focused_pane();
                let _ = conn.send_checked(&format!("swap-pane -s {focused} -t %{next}"));
            }
            super::ManagementKey::NewWindow => {
                let Some(session) = self.status.session_id.clone() else {
                    return;
                };
                let Ok(reply) = conn.send_checked(&format!("new-window -t {session}")) else {
                    return;
                };
                if !reply.ok {
                    return;
                }
                let Some(window) = reply.body.first().map(|w| w.trim().to_string()) else {
                    return;
                };
                let _ = conn.send_checked(&format!("select-window -t {window}"));
                self.reseed_window(conn, &window);
            }
            super::ManagementKey::WorkspaceNext | super::ManagementKey::WorkspacePrev => {
                let direction = if matches!(key, super::ManagementKey::WorkspaceNext) {
                    1
                } else {
                    -1
                };
                self.switch_workspace(conn, direction);
            }
            super::ManagementKey::Zoom => {
                // tmux's zoom: the daemon's `resize-pane -Z` toggle. The
                // daemon re-lays the window to the single pane full-grid
                // (children resize), selects away unzooms, and any layout
                // mutation ends the zoom — the %layout-change broadcast
                // re-seeds this client through the pump's pending path.
                // The bool only drives the status row's Z cue; the daemon
                // holds the truth.
                let Ok(reply) = conn.send_checked(&format!("resize-pane -t {focused} -Z")) else {
                    return;
                };
                if !reply.ok {
                    return;
                }
                self.zoomed = !self.zoomed;
                self.flash = Some(if self.zoomed { "zoomed" } else { "unzoomed" }.to_string());
                self.draw_status_row();
            }
            super::ManagementKey::RenameWindow | super::ManagementKey::RenamePane => {
                self.enter_prompt(if matches!(key, super::ManagementKey::RenamePane) {
                    PromptTarget::Pane
                } else {
                    PromptTarget::Window(self.window.clone())
                });
            }
            super::ManagementKey::StatusBar => {
                self.status_bar_on = !self.status_bar_on;
                // No flash cue: the flash paints ON the status row, so a
                // hide would flash invisibly — the row's vanishing (or
                // returning) is the feedback. The bottom row's presence
                // changes the content height: park a refit so the report
                // and re-fit follow (the sidebar toggle's contract).
                self.pending_grid_refit = true;
                self.draw_status_row();
            }
            super::ManagementKey::BorderCycle => {
                self.border_glyphs = self.border_glyphs.next();
                self.renderer.set_glyphs(self.border_glyphs);
                // herdr style: every pane draws its own complete box, not
                // the shared dividers — the style owns the paint mode.
                // Through the session-level setter: the chord used to
                // flip the renderer's flag only, so every re-seed (a
                // split is one) restored the session flag and reverted
                // the boxes to shared dividers (the manual-pass report).
                self.set_pane_borders(matches!(self.border_glyphs, Glyphs::Herdr));
                self.flash = Some(format!("border style: {}", self.border_glyphs.name()));
                self.draw_status_row();
            }
            super::ManagementKey::WorkspacePicker => {
                self.enter_ws_picker(conn);
            }
            super::ManagementKey::Sidebar => {
                self.toggle_sidebar(conn);
            }
            super::ManagementKey::Labels => {
                // Toggle each pane's title in its border: the session's
                // bool flips, the flash cue confirms the new state, and
                // the next frame repaints the borders.
                let on = !self.show_label_in_border;
                self.set_show_label_in_border(on);
                self.flash = Some(if on { "labels on" } else { "labels off" }.to_string());
                self.draw_status_row();
            }
        }
    }

    /// prefix W / C-w: the next/previous workspace in id order —
    /// `select-workspace -t +N`, then land the view on the workspace's
    /// session (its active window, re-seeded from fresh replays) through
    /// the same select-then-refresh contract every switch follows. A
    /// workspace with no sessions cannot be landed on: the selection
    /// still moves and the status refresh carries the new active marker.
    fn switch_workspace(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        direction: i32,
    ) {
        let Ok(reply) = conn.send_checked("list-workspaces") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let rows: Vec<(String, String, bool)> = reply
            .body
            .iter()
            .filter_map(|l| super::parse_workspace_line(l))
            .collect();
        if rows.is_empty() {
            return;
        }
        let Some(current) = rows.iter().position(|(_, _, active)| *active) else {
            return;
        };
        let next = (current as i32 + direction).rem_euclid(rows.len() as i32) as usize;
        let (ws_id, _, _) = &rows[next];
        let _ = conn.send_checked(&format!("select-workspace -t {ws_id}"));
        self.land_on_workspace(conn, ws_id);
    }

    /// Land the view on `ws_id`: its first listed session's active
    /// window, re-seeded (the select+resync contract every switch
    /// follows). An empty workspace only marks the status stale — the
    /// %workspaces-changed the select queued moves the strip's active
    /// marker on the next frame.
    /// Toggle the side panel: flip the strip width on the renderer, then
    /// report the REDUCED/RESTORED grid — `refresh-client -C` — so the
    /// daemon re-divides the panes around it; the `%layout-change`
    /// re-seed repaints. Opening queries the workspace roster for the
    /// section rows.
    fn toggle_sidebar(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        self.sidebar_on = !self.sidebar_on;
        let width = if self.sidebar_on {
            self.sidebar_width
        } else {
            0
        };
        self.renderer.set_sidebar_width(width);
        // The refit runs on the pump (it owns the flush sink): resize_to
        // reports the new grid, then repaint_all erases the region the
        // old layout vacated — the plain %layout-change re-seed never
        // clears those host cells (the manual-pass ghost-pane report).
        self.pending_grid_refit = true;
        if self.sidebar_on {
            self.refresh_sidebar(conn);
        }
        self.flash = Some(
            if self.sidebar_on {
                "sidebar on"
            } else {
                "sidebar off"
            }
            .to_string(),
        );
        self.draw_status_row();
    }

    /// Re-query every pane's effective title into the renderer's label
    /// store — the border labels paint the user `-T` label when set (the
    /// manual-pass report: prefix `$` labels never showed because the
    /// painter read the shell's OSC title only). Called on seed and on
    /// the throttled status refresh; a changed title marks dirty.
    fn refresh_pane_titles(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let panes: Vec<u32> = self.renderer.layout().iter().map(|r| r.pane).collect();
        for pane in panes {
            if let Ok(reply) = conn.send_checked(&format!("pane-title -t %{pane}")) {
                if reply.ok {
                    let title = reply.body.join(" ");
                    self.renderer.set_user_title(pane, title.trim());
                }
            }
        }
    }

    /// Re-query the workspace roster into the panel's sections — on
    /// open, and on every status refresh while the panel is up (the
    /// `%workspaces-changed` mark rides the same throttle).
    fn refresh_sidebar(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let rows: Vec<(String, String, bool)> = conn
            .send_checked("list-workspaces")
            .ok()
            .filter(|reply| reply.ok)
            .map(|reply| {
                reply
                    .body
                    .iter()
                    .filter_map(|l| super::parse_workspace_line(l))
                    .map(|(id, name, active)| (format!("ws:{id}"), name, active))
                    .collect()
            })
            .unwrap_or_default();
        if rows.is_empty() {
            self.renderer.set_sidebar_sections(None);
            return;
        }
        self.renderer
            .set_sidebar_sections(Some(vec![super::SidebarSection { rows }]));
    }

    fn land_on_workspace(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, ws_id: &str) {
        let Ok(sessions) = conn.send_checked(&format!("list-sessions -t {ws_id}")) else {
            return;
        };
        if !sessions.ok {
            return;
        }
        let Some((session, _)) = sessions
            .body
            .iter()
            .filter_map(|l| super::parse_session_line(l))
            .next()
        else {
            self.status_dirty = true;
            return;
        };
        let Ok(windows) = conn.send_checked(&format!("list-windows -t {session}")) else {
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

    /// The reload chord in render mode: the same client-side rebind the
    /// passthrough session performs, plus a status-row flash, plus the
    /// daemon's `reload-config` — best-effort either way.
    fn reload_config(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        // The file drives both the chord rebind and the pane-borders
        // resolution below, so it loads once, here.
        let file = match crate::mux::config::load_canonical_checked() {
            Ok(file) => file,
            Err(err) => {
                self.flash = Some(format!("reload failed: {err}"));
                let _ = conn.send_checked("reload-config");
                return;
            }
        };
        match crate::mux::config::reload_client_chords(
            &file,
            &crate::mux::config::Chords {
                prefix: self.prefix,
                reload: self.reload_key,
                management: self.management,
                resize_step: self.resize_step,
                pane_borders: self.pane_borders,
                show_label_in_border: self.show_label_in_border,
                pane_gaps: self.pane_gaps,
                scrollbar_gutter: self.scrollbar_gutter,
                drag_cursor_shape: self.drag_cursor_shape,
                border_lines: self.border_glyphs.name().to_string(),
                sidebar_width: self.sidebar_width,
            },
        ) {
            Ok(chords) => {
                self.prefix = chords.prefix;
                self.reload_key = chords.reload;
                self.management = chords.management;
                self.resize_step = chords.resize_step;
                if !self.set_border_lines(&chords.border_lines) {
                    self.flash = Some(format!(
                        "border-lines {:?} unknown — using {}",
                        chords.border_lines,
                        self.border_glyphs.name()
                    ));
                }
                // An explicit `pane-borders` key survives every reload;
                // an absent key follows the (possibly reloaded) border
                // style — herdr implies per-pane boxes. The chords' own
                // bool cannot tell explicit from absent, so the raw file
                // key decides.
                match file.client.pane_borders {
                    Some(on) => self.set_pane_borders(on),
                    None => self.set_pane_borders(matches!(self.border_glyphs, Glyphs::Herdr)),
                }
                self.set_pane_gaps(chords.pane_gaps);
                self.set_scrollbar_gutter(chords.scrollbar_gutter);
                self.set_show_label_in_border(chords.show_label_in_border);
                self.drag_cursor_shape = chords.drag_cursor_shape;
                let eff =
                    crate::mux::config::resolve(&file, &crate::mux::config::Overrides::default());
                self.set_border_colors(
                    crate::mux::config::parse_hex_color(&eff.border_active_color)
                        .map(|(r, g, b)| RtColor::Rgb(r, g, b)),
                    crate::mux::config::parse_hex_color(&eff.border_color)
                        .map(|(r, g, b)| RtColor::Rgb(r, g, b)),
                );
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
    fn cycle_pane(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
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
    fn switch_window(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, direction: i32) {
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
    fn switch_session(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, direction: i32) {
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
    fn reseed_window(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, window: &str) {
        let pane = self.focused_pane_or_first(conn, window);
        // Rebuild at the CURRENT renderer's frame extent — the host grid
        // the seed and resize_to derived — then report the fresh
        // renderer's window_size: that size is the reported extent (the
        // grid less the side panel), not the buffer's width, and
        // constructing from it ratcheted the view one panel-width
        // narrower on every re-seed while the panel was up, each report
        // then shrinking again (the manual-pass click-shrinks-the-panes
        // report).
        let (host_cols, host_rows) = self.renderer.frame_size();
        self.renderer = PaneRenderer::new(host_cols, host_rows, self.border_glyphs);
        // Renderer reconstruction resets the theme and display options:
        // re-apply the session's resolved background and pane-border/
        // label flags so a re-seed does not paint a black band or drop
        // the border mode (the round-3 prefix n/p defects).
        self.renderer.set_background(self.bg);
        self.renderer.set_pane_borders(self.pane_borders);
        self.renderer
            .set_show_label_in_border(self.show_label_in_border);
        self.renderer.set_pane_gaps(self.pane_gaps);
        self.renderer.set_scrollbar_gutter(self.scrollbar_gutter);
        // The side panel is renderer state too: without its width the
        // fresh renderer reports a FULL-width grid on the next size
        // report and paints panes unshifted under the panel (the
        // manual-pass split-while-panel-open reports).
        self.renderer.set_sidebar_width(if self.sidebar_on {
            self.sidebar_width
        } else {
            0
        });
        if self.sidebar_on {
            self.refresh_sidebar(conn);
        }
        let (cols, rows) = self.renderer.window_size();
        if conn
            .send_checked(&format!("refresh-client -t {pane} -C {cols}x{rows}"))
            .is_err()
        {
            return;
        }
        self.window = window.to_string();
        // A view rebuild unzooms the cue (the daemon unzooms on layout
        // mutations and select-away; a fresh window starts unzoomed).
        self.zoomed = false;
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
        // The re-seed replaced the renderer's buffers: the next frame's
        // diff repaints every cell, and the cursor guard must reset so a
        // changed position/shape re-emits even when the recorded state
        // coincidentally matches the old window's.
        self.cursor_placed = Some(None);
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
        self.tab_strip.invalidate();
        self.draw_tab_strip();
    }

    /// A pane id of `window` to hang the size report on: the focused pane
    /// when it still belongs there, else the window's first pane.
    fn focused_pane_or_first(
        &self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) -> String {
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
    /// buffer (the pump flushes the diff with the next frame). Hidden,
    /// nothing paints: the row belongs to the pane grid, and the refit's
    /// full repaint is what erased the bar exactly once.
    fn draw_status_row(&mut self) {
        let (cols, _rows) = super::conn::terminal_grid();
        if self.status_row.cols() != cols {
            self.status_row = StatusRow::new(cols);
        }
        if !self.status_bar_on {
            return;
        }
        let scroll = if self.scroll_mode {
            self.renderer
                .focused()
                .map(|id| self.renderer.scroll_offset_of(id))
        } else {
            None
        };
        // The flash cue and the zoom marker lead the line and the Help
        // chip RIGHT-ALIGNS at the row's end: all three reserve their
        // widths up front so the composed state truncates into what
        // remains, and the paint clips the content at the chip's left
        // column — the chip can never be pushed off the row (round 6:
        // the chip used to ride at the end of the composed segments,
        // and a full line pushed it off).
        let mut head: Vec<Segment> = Vec::new();
        if self.zoomed {
            head.push(Segment {
                text: " Z |".to_string(),
                bold: true,
                dim: false,
                shaded: false,
            });
        }
        if let Some(flash) = self.flash.clone() {
            head.push(Segment {
                text: format!(" {flash} |"),
                bold: true,
                dim: false,
                shaded: false,
            });
        }
        let chip = status::help_chip(self.prefix, self.management.help);
        let reserved: usize = head
            .iter()
            .chain(std::iter::once(&chip))
            .map(|s| s.text.chars().count())
            .sum();
        let segments = self.status.compose(
            cols.saturating_sub(reserved as u16),
            scroll,
            self.sidebar_on,
        );
        let mut with_head = head;
        with_head.extend(segments);
        self.status_row.paint(&with_head, Some(&chip));
    }

    /// Flush the status row's changed cells to the host's bottom row.
    /// Returns whether anything flushed (its per-cell CUPs move the host
    /// cursor, so the caller must re-place it). While the bar is hidden
    /// nothing flushes at all: the bottom row is pane real estate, and a
    /// blank-cell flush would erase the pane's freshly painted bottom
    /// row after the pane frame.
    fn flush_status_row(&mut self, sink: &mut dyn FlushSink) -> bool {
        if !self.status_bar_on {
            return false;
        }
        let (_cols, rows) = super::conn::terminal_grid();
        let bottom = rows.saturating_sub(1);
        let diff = self.status_row.diff();
        if diff.is_empty() {
            return false;
        }
        // Rebase the row-relative cells to the host's bottom row and
        // reuse the sink's per-cell spelling.
        let rebased: Vec<(u16, u16, RtCell)> = diff
            .into_iter()
            .map(|(x, _y, cell)| (x, bottom, cell))
            .collect();
        sink.flush(&rebased);
        true
    }

    /// One host mouse report. Clicks on the tab strip switch windows;
    /// clicks in the content area focus the pane under the pointer
    /// (select-pane daemon-side, so the daemon's own active-pane state
    /// follows); events forward pane-relative SGR when the pane owns
    /// mouse tracking; the wheel scrolls the client's scrollback when it
    /// does not. A press within one cell of a divider starts a drag: it
    /// must not focus or forward (a drag on a divider is not a
    /// click-through), motion adjusts the adjacent split via the wire's
    /// relative `resize-pane`, and release without any motion falls
    /// through as a click (focus, plus the pane's release when owned).
    fn route_mouse(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, mouse: SgrMouse) {
        // Window-relative, 0-based host coordinates.
        let Some(x) = mouse.col.checked_sub(1) else {
            return;
        };
        let Some(y) = mouse.row.checked_sub(1) else {
            return;
        };
        let strip = self.renderer.sidebar_width();

        // The context menu is modal for the pointer: a press on an
        // action row dispatches it, everything else is consumed while
        // it is up.
        if self.menu.is_some() {
            if !mouse.release
                && !mouse.is_motion()
                && !mouse.is_wheel_up()
                && !mouse.is_wheel_down()
            {
                let content_x = x.saturating_sub(strip);
                if let Some(row) = self.renderer.overlay_row_at(content_x, y) {
                    self.menu_click(conn, row);
                }
            }
            return;
        }

        // The side panel owns its strip below the tab-strip row: a
        // press on a workspace row lands on it (the select+resync every
        // switch follows), a right-press opens the workspace menu, the
        // footer chips open their prompts/menu, and every other event
        // in the strip is consumed — no pane, divider, or drag lives in
        // the panel. The TOP row stays the tab strip's (the panel
        // begins under it), so a click there falls through to the tab
        // strip even inside the strip's width — the sidebar path used
        // to consume it (the round-5 report).
        if strip > 0 && y > 0 && x < strip {
            if !mouse.release && !mouse.is_motion() {
                if let Some(id) = self.renderer.sidebar_row_at(x, y) {
                    if id == super::SIDEBAR_NEW_ID {
                        self.enter_prompt(PromptTarget::NewWorkspace);
                    } else if id == super::SIDEBAR_MENU_ID {
                        // The workspace menu for the daemon-active
                        // workspace (the first when none is marked).
                        let active = self
                            .status
                            .active_workspace()
                            .map(str::to_string)
                            .or_else(|| self.status.workspaces().first().map(|(id, _)| id.clone()));
                        if let Some(ws) = active {
                            self.open_menu(MenuTarget::Workspace(ws));
                        }
                    } else if let Some(ws_id) = id.strip_prefix("ws:") {
                        if mouse.is_right_press() {
                            self.open_menu(MenuTarget::Workspace(ws_id.to_string()));
                        } else {
                            self.land_on_workspace(conn, ws_id);
                        }
                    }
                }
            }
            return;
        }
        // Pane coordinates are layout coordinates: the host x less the
        // side panel's strip width. The TOP row is exempt — the strip
        // paints the panel's title at 0..strip when the panel is up and
        // lays the tabs out from it onward in ABSOLUTE host columns, so
        // the tab hit-test takes the RAW host column (the round-6 fix:
        // the rebase used to run before this branch and shift every
        // panel-up top-row click one panel width left, killing the +
        // button's clickability).
        let rx = x;
        let x = x.saturating_sub(strip);

        // A drag in flight continues over motion/release regardless of
        // where the pointer is (a pointer that wanders onto the strip
        // row must not strand it); the drag's press stored content
        // coordinates, so the strip row saturates to content row 0.
        if self.drag.is_some() && (mouse.release || mouse.is_motion()) {
            self.drag_event(conn, x, y.saturating_sub(1), mouse.release);
            return;
        }

        // The rename prompt is modal for the pointer too: every event is
        // consumed while it is up.
        if self.prompt_mode {
            return;
        }

        // The picker is modal for the pointer too: wheels move the
        // selection, a click on a content row selects AND activates it
        // (switch through the select+resync), a click on the filter line
        // opens the filter box, and everything else is consumed.
        if self.picker_mode {
            if mouse.is_wheel_up() {
                self.picker_move(-1);
            } else if mouse.is_wheel_down() {
                self.picker_move(1);
            } else if mouse.release || mouse.is_motion() {
                // Drags do nothing in the modal; only a press click.
            } else if let Some(row) = self.renderer.overlay_row_at(x, y) {
                // The panel's row 0 is the filter line only while a
                // filter is open or set; idle, content starts at row 0
                // (the idle placeholder row the old mapping assumed is
                // gone). The footer — the panel's last row — does
                // nothing.
                let filtering = self.picker_filtering || !self.picker_filter.is_empty();
                let filter_lines = usize::from(filtering);
                if filtering && row == 0 {
                    self.picker_filtering = true;
                    self.refresh_picker();
                } else if let Some(content) = row.checked_sub(filter_lines) {
                    let window_len = self.picker_panel_len.saturating_sub(filter_lines + 1);
                    if content < window_len {
                        // Move the cursor to the clicked row and activate
                        // it (start maps the windowed index onto the
                        // filtered list).
                        self.picker_selected = self.picker_start + content;
                        self.refresh_picker();
                        self.picker_activate(conn);
                    }
                }
            }
            return;
        }

        // The help panel is modal for the pointer too: while it is up
        // every mouse event is consumed — wheels scroll the PANEL (the
        // round-3 defect: they fell through to the pane scrollback /
        // pane forwarding), clicks and drags do nothing.
        if self.help_mode {
            if mouse.is_wheel_up() {
                self.help_scroll_by(-3);
            } else if mouse.is_wheel_down() {
                self.help_scroll_by(3);
            }
            return;
        }

        // The tab strip owns the top row: a left press hit-tests the
        // tabs and switches windows through the select+resync contract;
        // a RIGHT press on a tab opens that window's context menu; no
        // pane focus, no pane forwarding, and no drag ever starts
        // there. The hit-test takes the RAW host column (see above).
        if y == 0 {
            let is_press = !mouse.release
                && !mouse.is_motion()
                && !mouse.is_wheel_up()
                && !mouse.is_wheel_down();
            if mouse.is_right_press() {
                if let Some((id, _)) = self
                    .tab_strip
                    .hit_test(rx)
                    .and_then(|index| self.status.windows().get(index))
                {
                    let id = id.clone();
                    self.open_menu(MenuTarget::Tab(id));
                }
            } else if is_press {
                self.tab_click(conn, rx);
            }
            return;
        }
        // Below the strip, content coordinates are host rows minus the
        // strip row.
        let Some(cy) = y.checked_sub(1) else {
            return;
        };

        if mouse.is_wheel_up() || mouse.is_wheel_down() {
            if self.drag.is_some() {
                return; // the held button owns the pointer; wheels wait
            }
            let Some(rect) = self.renderer.pane_at(x, cy).cloned() else {
                return;
            };
            let owns = self.pane_owns_mouse(rect.pane);
            let delta: isize = if mouse.is_wheel_up() { 3 } else { -3 };
            if !owns && self.renderer.wheel_scroll(x, cy, delta) {
                return; // consumed client-side
            }
            if owns {
                self.forward_mouse(conn, &rect, &mouse);
            }
            return;
        }

        if mouse.release || mouse.is_motion() {
            if self.drag.is_some() {
                self.drag_event(conn, x, cy, mouse.release);
                return;
            }
            // Drag/release only matter to a pane that owns the mouse;
            // focus follows press only.
            let Some(rect) = self.renderer.pane_at(x, cy).cloned() else {
                return;
            };
            if self.pane_owns_mouse(rect.pane) {
                self.forward_mouse(conn, &rect, &mouse);
            }
            return;
        }

        // A press: a divider hit starts a drag (never a click-through) —
        // UNLESS the cell is an embedded border label (text cells are not
        // drag handles, herdr's semantics); then it falls through to the
        // focus path below. The plain border segments around a label stay
        // draggable (divider_near still matches the boundary line).
        if let Some(divider) = self.renderer.divider_near(x, cy, 1) {
            if !self.renderer.label_cell_at(x, cy) {
                self.drag = Some(DragState::Pending { divider, x, y: cy });
                return;
            }
        }
        let Some(rect) = self.renderer.pane_at(x, cy).cloned() else {
            return;
        };
        self.renderer.focus(rect.pane);
        let _ = conn.send_checked(&format!("select-pane -t %{}", rect.pane));
        if self.pane_owns_mouse(rect.pane) {
            self.forward_mouse(conn, &rect, &mouse);
        }
    }

    /// A press on the tab strip: the ` + ` button opens the new-tab
    /// prompt; any tab column switches to that window through the
    /// existing select+resync contract (daemon-side `select-window`,
    /// then a full re-seed). A click on the already-shown window, or on
    /// a pad/marker column, does nothing. No pane focus, no pane
    /// forwarding.
    fn tab_click(&mut self, conn: &mut crate::mux::attach::conn::AttachConn, x: u16) {
        if self.tab_strip.plus_hit(x) {
            self.enter_prompt(PromptTarget::NewWindow);
            return;
        }
        let Some(index) = self.tab_strip.hit_test(x) else {
            return;
        };
        let Some((id, _)) = self.status.windows().get(index) else {
            return;
        };
        let id = id.clone();
        if id == self.window {
            return; // the shown window: a no-op click
        }
        if !conn
            .send_checked(&format!("select-window -t {id}"))
            .is_ok_and(|reply| reply.ok)
        {
            return;
        }
        self.reseed_window(conn, &id);
    }

    /// Whether `pane`'s emulator tracks the mouse (the forwarding gate).
    fn pane_owns_mouse(&self, pane: u32) -> bool {
        self.renderer
            .pane_terminal(pane)
            .is_some_and(|t| t.mouse_mode() != crate::mouse::MouseMode::Off)
    }

    /// One motion or release while a drag is in flight: motion promotes a
    /// pending press to an active drag and records the pointer's signed
    /// delta from the press point (cells along the divider's axis);
    /// release ends the drag and clears the highlight — a release that
    /// never moved falls through as a click (focus the pane under the
    /// pointer, forward the release when the pane owns the mouse).
    fn drag_event(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        x: u16,
        y: u16,
        release: bool,
    ) {
        if release {
            let state = self.drag.take();
            self.renderer.set_drag_divider(None);
            if let Some(DragState::Pending { .. }) = state {
                if let Some(rect) = self.renderer.pane_at(x, y).cloned() {
                    self.renderer.focus(rect.pane);
                    let _ = conn.send_checked(&format!("select-pane -t %{}", rect.pane));
                    let mouse = SgrMouse {
                        cb: 0,
                        col: x + 1,
                        row: y + 1,
                        release: true,
                    };
                    if self.pane_owns_mouse(rect.pane) {
                        self.forward_mouse(conn, &rect, &mouse);
                    }
                }
            }
            return;
        }
        // Motion.
        // Promote only a PENDING press: an already-active drag must stay
        // in place — `take()` here on an Active drag would drop the whole
        // drag state, starving every resize after the first motion (the
        // round-3 "highlight engages, drag does not resize" defect).
        if matches!(&self.drag, Some(DragState::Pending { .. })) {
            if let Some(DragState::Pending { divider, x, y }) = self.drag.take() {
                self.drag = Some(DragState::Active {
                    divider,
                    x,
                    y,
                    applied: 0,
                    pending: 0,
                });
                self.renderer
                    .set_drag_divider(Some((divider.vertical, divider.a, divider.b)));
            }
        }
        if let Some(DragState::Active {
            divider,
            x: px,
            y: py,
            pending,
            ..
        }) = &mut self.drag
        {
            *pending = if divider.vertical {
                i32::from(x) - i32::from(*px)
            } else {
                i32::from(y) - i32::from(*py)
            };
        }
    }

    /// The pump's frame-cadence drag application: one relative
    /// `resize-pane` per unapplied cell of delta, aimed at the boundary's
    /// left/top pane (the daemon re-divides the neighbor). Best-effort —
    /// the %layout-change broadcast re-seeds the window through the
    /// pump's pending_layout path.
    fn apply_drag(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Some(DragState::Active {
            divider,
            pending,
            applied,
            ..
        }) = &mut self.drag
        else {
            return;
        };
        let step = *pending - *applied;
        if step == 0 {
            return;
        }
        let flag = if divider.vertical {
            if step > 0 {
                "-R"
            } else {
                "-L"
            }
        } else if step > 0 {
            "-D"
        } else {
            "-U"
        };
        let _ = conn.send_checked(&format!(
            "resize-pane -t %{a} {flag} {n}",
            a = divider.a,
            n = step.abs()
        ));
        *applied = *pending;
    }

    /// Re-encode one host mouse report pane-relative and send it. The
    /// report's row is host 1-based; the pane rects live in content
    /// coordinates (below the strip row), so the strip comes off
    /// first.
    fn forward_mouse(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        rect: &PaneRect,
        mouse: &SgrMouse,
    ) {
        let rel_col = mouse.col.saturating_sub(1).saturating_sub(rect.x);
        let rel_row = mouse.row.saturating_sub(2).saturating_sub(rect.y);
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
        conn: &mut crate::mux::attach::conn::AttachConn,
        cols: u16,
        rows: u16,
        sink: &mut dyn FlushSink,
    ) -> Result<(), String> {
        let pane = self.focused_pane();
        if pane.is_empty() {
            return Ok(());
        }
        // Reconstruct FIRST: the report below reads the fresh renderer's
        // window_size (the new host grid less the side panel), and the
        // reconstruction re-applies the panel width it depends on.
        self.renderer = PaneRenderer::new(cols, rows, self.border_glyphs);
        // Same reconstruction reset as reseed_window: re-apply the theme
        // and display options the fresh renderer dropped.
        self.renderer.set_background(self.bg);
        self.renderer.set_pane_borders(self.pane_borders);
        self.renderer
            .set_show_label_in_border(self.show_label_in_border);
        self.renderer.set_pane_gaps(self.pane_gaps);
        self.renderer.set_scrollbar_gutter(self.scrollbar_gutter);
        self.renderer.set_sidebar_width(if self.sidebar_on {
            self.sidebar_width
        } else {
            0
        });
        let (report_cols, report_rows) = self.renderer.window_size();
        conn.send_checked(&format!(
            "refresh-client -t {pane} -C {report_cols}x{report_rows}"
        ))
        .map_err(|err| format!("resize report failed: {err}"))?;
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
                self.daemon_layout = layout.clone();
                self.renderer.apply_layout(layout);
                for rect in self.renderer.layout().to_vec() {
                    let pane = format!("%{}", rect.pane);
                    if let Ok(reply) = conn.send_checked(&format!("refresh-client -t {pane}")) {
                        if reply.ok {
                            // Join only: an extra trailing `\n` here would
                            // push the replayed cursor one row down (the
                            // replay_all_panes note).
                            let bytes = reply.body.join("\n").into_bytes();
                            self.renderer.feed_output(rect.pane, &bytes);
                        }
                    }
                }
            }
        }
        // Same as the seed: repaint_all hid the cursor, so force the
        // next place_cursor.
        self.cursor_placed = Some(None);
        // repaint_all clears the WHOLE screen — the strip rows too — and
        // the frame below flushes diff-based, so both strips forget
        // their previous frames here (the sidebar-toggle path parks the
        // refit into the pump, which repaints nothing else on those
        // rows).
        self.status_row.invalidate();
        self.draw_status_row();
        self.tab_strip.invalidate();
        self.draw_tab_strip();
        sink.repaint_all();
        self.frame(sink);
        Ok(())
    }

    /// Frame the pending output at cadence: panes, the tab strip, the
    /// status row, then the cursor — positioned at the focused pane's
    /// tracked cell (mapped through rect origin + scroll offset; hidden
    /// when the view is scrolled off live or the pane hid its cursor via
    /// DECTCEM). A flushed frame always repositions (the diff's last
    /// per-cell CUP left the terminal cursor wherever the last diff cell
    /// sits), while a quiet pump re-emits only on a state change.
    ///
    /// Coordinate spaces: the renderer's frame covers the rows BETWEEN
    /// the strip and the status row, so its diff and cursor rebase +1
    /// host row; the strip flushes at host row 0 and the status row at
    /// the host's bottom row.
    fn frame(&mut self, sink: &mut dyn FlushSink) {
        const STRIP_ROWS: u16 = 1;
        let mut flushed = false;
        if self.renderer.needs_frame() {
            let diff = self.renderer.render_frame();
            if !diff.is_empty() {
                let rebased: Vec<(u16, u16, RtCell)> = diff
                    .into_iter()
                    .map(|(x, y, cell)| (x, y + STRIP_ROWS, cell))
                    .collect();
                sink.flush(&rebased);
                flushed = true;
            }
        }
        if self.flush_tab_strip(sink) {
            flushed = true;
        }
        if self.flush_status_row(sink) {
            flushed = true;
        }
        let mut cursor = self
            .renderer
            .focused_cursor()
            .map(|(x, y, style)| (x, y + STRIP_ROWS, style));
        // The drag cursor shape (config `drag-cursor-shape`): while a
        // divider drag is live the host cursor carries the resize shape
        // (best-effort DECSCUSR steady block — DECSCUSR has no
        // column-resize value; every drag-end path clears `drag`, so the
        // next frame restores the pane's own shape).
        if self.drag_cursor_shape && matches!(&self.drag, Some(DragState::Active { .. })) {
            cursor = cursor.map(|(x, y, _)| (x, y, CursorStyle::SteadyBlock));
        }
        // A modal overlay (help, picker) covers the focused pane: placing
        // the pane's cursor would draw it through the panel (the
        // manual-pass report). Hide the host cursor while a modal is up;
        // the next unmodaled frame restores placement.
        if self.renderer.overlay.is_some() || self.picker_mode {
            cursor = None;
        }
        if flushed || self.cursor_placed.as_ref() != Some(&cursor) {
            sink.place_cursor(cursor);
            self.cursor_placed = Some(cursor);
        }
    }

    /// Paint the tab strip from the queried window state (the same
    /// state the status row composes from): the shown session's windows
    /// in order, the active one accented. With the side panel up the
    /// strip leads with the panel's title (the lead segment the hit-test
    /// offsets through).
    fn draw_tab_strip(&mut self) {
        let (cols, _rows) = super::conn::terminal_grid();
        if self.tab_strip.cols() != cols {
            self.tab_strip = TabStrip::new(cols);
        }
        let strip = self.renderer.sidebar_width();
        let lead = (strip > 0).then_some(("workspaces", strip));
        self.tab_strip.paint(
            self.status.windows(),
            self.status.active_window.as_deref(),
            lead,
        );
    }

    /// Flush the tab strip's changed cells to the host's top row.
    /// Returns whether anything flushed (its per-cell CUPs move the host
    /// cursor, so the caller must re-place it).
    fn flush_tab_strip(&mut self, sink: &mut dyn FlushSink) -> bool {
        let diff = self.tab_strip.diff();
        if diff.is_empty() {
            return false;
        }
        sink.flush(&diff);
        true
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
    conn: &mut crate::mux::attach::conn::AttachConn,
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
fn marked_pane_of(
    conn: &mut crate::mux::attach::conn::AttachConn,
    window: &str,
) -> Result<String, String> {
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
    use crate::keyboard::TermKey;
    use crate::mux::attach::layout::{parse_layout, parse_layout_triple};
    use std::io::BufRead as _;

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
    /// The stacked-split regression: a horizontal boundary paints the
    /// horizontal glyph along the overlap (the halves wave grouped
    /// horizontal cells by the along-axis and the bounds check then
    /// skipped them — the manual-pass report).
    #[test]
    fn horizontal_dividers_paint_on_stacked_splits() {
        const STACKED: &str = "0000,80x24,0,0[80x12,0,0,1,80x12,0,12,2]";
        let layout = parse_layout(STACKED).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.render_frame();
        let y = 11u16; // the top pane's last row — the boundary cell
        for x in (0..80u16).step_by(7) {
            assert_eq!(
                renderer.buffer[(x, y)].symbol(),
                "─",
                "the stacked split's horizontal divider paints at column {x}"
            );
        }
        // And nothing transposed: a mid-pane column keeps its content.
        assert_ne!(
            renderer.buffer[(11, 3)].symbol(),
            "─",
            "no transposed vertical streak at the boundary row's coordinate"
        );
    }

    /// The prefix+arrow navigation: the nearest pane strictly in the
    /// direction wins; an edge is `None`.
    #[test]
    fn pane_in_direction_picks_the_nearest_pane_in_the_direction() {
        const MIXED: &str = "0000,90x24,0,0{30x24,0,0,1,60x24,30,0[60x12,30,0,2,60x12,30,12,3]}";
        let rects = parse_layout(MIXED).expect("parses");
        assert_eq!(pane_in_direction(&rects, 1, PaneDir::Right), Some(2));
        assert_eq!(pane_in_direction(&rects, 3, PaneDir::Up), Some(2));
        assert_eq!(pane_in_direction(&rects, 2, PaneDir::Down), Some(3));
        assert_eq!(pane_in_direction(&rects, 2, PaneDir::Left), Some(1));
        assert_eq!(pane_in_direction(&rects, 1, PaneDir::Left), None);
    }

    /// The tmux half rule across a three-pane side-by-side row: the
    /// shared divider's half nearer the focus highlights, the other
    /// stays dim.
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
        // The middle pane is the RIGHT side of the 1|2 boundary (bottom
        // half highlights) and the LEFT side of the 2|3 boundary (top
        // half highlights).
        assert!(
            renderer.buffer[(29, 23)]
                .modifier
                .contains(RtModifier::BOLD),
            "moving focus to pane 2: the 1|2 divider's bottom half highlights"
        );
        assert!(
            renderer.buffer[(29, 0)].modifier.contains(RtModifier::DIM),
            "moving focus to pane 2: the 1|2 divider's top half stays dim"
        );
        assert!(
            renderer.buffer[(59, 0)].modifier.contains(RtModifier::BOLD),
            "moving focus to pane 2: the 2|3 divider's top half highlights"
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

    // ---- The manual-pass card: cursor mapping, focus accent, bg fill.

    /// Fix 1 (cursor mapping): the focused pane's tracked cursor cell +
    /// rect origin maps to the window-relative cell the host cursor is
    /// placed at. Hidden while the view is scrolled off live, when the
    /// pane hid its cursor (DECTCEM), and for an absent focus.
    #[test]
    fn focused_cursor_maps_cell_plus_origin_and_hides_when_scrolled() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.focus(2);

        // Pane 2's emulator sits at rect (40, 0); move its tracked cursor
        // to (5, 3) through the same %output path the pane writes.
        renderer.feed_output(2, b"\r\n\r\n\r\n     ");
        let tracked = renderer.pane_terminal(2).expect("pane 2").cursor();
        assert_eq!((tracked.col, tracked.row), (5, 3), "precondition");
        let origin_x = renderer
            .layout()
            .iter()
            .find(|r| r.pane == 2)
            .map(|r| r.x)
            .unwrap_or(0);
        assert_eq!(
            renderer.focused_cursor(),
            Some((origin_x + 5, 3, tracked.style)),
            "tracked cell + rect origin"
        );

        // Scroll pane 2's client view off live: the live cell is not on
        // screen, so the cursor hides. Pane 2 needs scrollback first (the
        // offset clamps to the history extent); 30 lines through its
        // 24-row pane leaves 6+ in history and parks the tracked cursor
        // at the bottom-left of its grid.
        for i in 0..30 {
            renderer.feed_output(2, format!("hist-{i:02}\r\n").as_bytes());
        }
        assert!(
            renderer.wheel_scroll(45, 0, 3),
            "pane 2 scrolls client-side"
        );
        assert_eq!(renderer.scroll_offset_of(2), 3, "view off live");
        assert_eq!(renderer.focused_cursor(), None, "scrolled view hides");

        // Back to live: the cursor reappears at the tracked cell —
        // bottom-left of pane 2's grid after the history flood.
        assert!(renderer.wheel_scroll(45, 0, -3));
        let tracked = renderer.pane_terminal(2).expect("pane 2").cursor();
        assert_eq!(
            (tracked.col, tracked.row),
            (0, 23),
            "precondition: flood parked it"
        );
        assert_eq!(
            renderer.focused_cursor(),
            Some((40, 23, tracked.style)),
            "live again: tracked cell + origin"
        );

        // DECTCEM hide/show through the pane's own bytes.
        renderer.feed_output(2, b"\x1b[?25l");
        assert_eq!(renderer.focused_cursor(), None, "pane hid its cursor");
        renderer.feed_output(2, b"\x1b[?25h");
        assert!(renderer.focused_cursor().is_some(), "pane re-showed");

        // A fresh renderer still maps — its emulators' cursors are
        // visible at the rect origin, which is exactly right for a pane
        // whose shell sits at home.
        let mut fresh = PaneRenderer::new(80, 24, Glyphs::Unicode);
        fresh.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        assert_eq!(
            fresh.focused_cursor(),
            Some((0, 0, CursorStyle::default())),
            "fresh emulator: tracked cell + origin"
        );
    }
    /// The DECSCUSR shape rides `focused_cursor` for the sink to re-emit:
    /// the pane's `CSI 4 SP q` (steady underline) is tracked and mapped.
    #[test]
    fn focused_cursor_carries_the_tracked_decscusr_shape() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.focus(1);
        renderer.feed_output(1, b"\x1b[4 q");
        let (x, _y, style) = renderer.focused_cursor().expect("cursor");
        assert_eq!(x, 0, "pane 1's origin col");
        assert_eq!(style, CursorStyle::SteadyUnderline, "DECSCUSR 4 tracked");
    }

    /// Fix 2 (focus accent): the focused pane's boundary dividers carry
    /// the bright-cyan accent fg (indexed 14) plus bold, clearly
    /// distinguishable from the unfocused dividers (dim, default fg) —
    /// the shipped bold/dim-only highlight read as identical dividers.
    #[test]
    fn focused_divider_carries_accent_vs_dim_unfocused() {
        const THREE_PANE: &str = "0000,90x24,0,0{30x24,0,0,1,30x24,30,0,2,30x24,60,0,3}";
        let layout = parse_layout(THREE_PANE).expect("parses");
        let mut renderer = PaneRenderer::new(90, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);
        renderer.focus(1);
        renderer.render_frame();

        let focused_div = &renderer.buffer[(29, 0)];
        assert_eq!(
            focused_div.fg,
            RtColor::Indexed(14),
            "accent fg on the focused divider"
        );
        assert!(focused_div.modifier.contains(RtModifier::BOLD));

        let unfocused_div = &renderer.buffer[(59, 0)];
        assert_eq!(
            unfocused_div.fg,
            RtColor::Reset,
            "no accent on the unfocused divider"
        );
        assert!(unfocused_div.modifier.contains(RtModifier::DIM));
    }

    /// Fix 3 (background fill): after a frame, NO cell carries the
    /// ratatui default (`Reset`) background — every cell the pane grids
    /// do not cover (and the skipped cells inside them: short history
    /// lines, wide-char spacers) is filled with the renderer's
    /// configured background, which `set_background` can point at the
    /// host's resolved value.
    #[test]
    fn frame_fills_every_cell_with_the_configured_background() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_background(Some(RtColor::Rgb(16, 24, 40)));
        renderer.apply_layout(layout);
        // A short history line leaves the rest of its row unpainted, and
        // a wide char marks a spacer that painting skips.
        renderer.feed_output(1, "世\r\n".as_bytes());
        renderer.render_frame();

        let default_bg_cells = (0..80u16)
            .flat_map(|x| (0..24u16).map(move |y| (x, y)))
            .filter(|(x, y)| renderer.cell(*x, *y).map(|c| c.bg == RtColor::Reset) == Some(true))
            .count();
        assert_eq!(default_bg_cells, 0, "no Reset-style cell survives a frame");

        // The skipped cells specifically: the wide-char spacer (painting
        // skipped it, so the fill's bg stands) and an unpainted tail cell.
        // Painted BLANK cells also carry the probed bg (the round-3
        // contract: the resolved background fills every cell, so a pane
        // area never shows palette black where the theme bg belongs);
        // only cells the core painted with a real color keep it.
        assert_eq!(
            renderer.cell(1, 0).expect("spacer").bg,
            RtColor::Rgb(16, 24, 40)
        );
        assert_eq!(
            renderer.cell(0, 0).expect("painted").bg,
            RtColor::Rgb(16, 24, 40),
            "blank painted cells carry the probed bg"
        );

        // The grey-band case: rows BELOW the tiled layout (a renderer
        // taller than the layout rects) are painted by nothing — they
        // must still carry the fill, not the ratatui default.
        let mut banded = PaneRenderer::new(80, 30, Glyphs::Unicode);
        banded.set_background(Some(RtColor::Rgb(16, 24, 40)));
        banded.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        banded.render_frame();
        for y in 24..30u16 {
            assert_eq!(
                banded.cell(10, y).expect("band").bg,
                RtColor::Rgb(16, 24, 40),
                "row {y} below the layout carries the fill"
            );
        }

        // Changing the background dirties and repaints with the new fill.
        renderer.set_background(Some(RtColor::Rgb(1, 2, 3)));
        let diff = renderer.render_frame();
        assert!(!diff.is_empty(), "a bg change repaints");
        assert_eq!(
            renderer.cell(1, 0).expect("spacer").bg,
            RtColor::Rgb(1, 2, 3)
        );
    }

    /// The status row's cells carry the frame background too — the
    /// `REVERSED` style the painter uses means fg/bg swap, so the row's
    /// recorded bg must be the host bg for the reversed band to read as
    /// the host's foreground on the host's background.
    #[test]
    fn status_row_cells_carry_no_reset_bg_after_paint() {
        // The status painter sets only a REVERSED modifier, leaving bg at
        // the buffer default; the sink's write_styled skips a Reset bg, so
        // the host's own bg shows through the reversed cells' unswapped
        // half. This is the documented v1 behavior; the assertion pins it
        // so a future bg-aware status painter updates both sides.
        let segments = vec![status::Segment {
            text: "x".to_string(),
            bold: false,
            dim: false,
            shaded: false,
        }];
        let mut row = status::StatusRow::new(4);
        row.paint(&segments, None);
        let diff = row.diff();
        assert!(diff.iter().all(|(_, _, cell)| cell.bg == RtColor::Reset));
    }

    /// The cursor-emitting sink: the recorded place_cursor calls.
    struct CursorSink {
        placements: Vec<Option<(u16, u16, CursorStyle)>>,
    }

    impl FlushSink for CursorSink {
        fn flush(&mut self, _diff: &[(u16, u16, RtCell)]) {}
        fn repaint_all(&mut self) {}
        fn place_cursor(&mut self, cursor: Option<(u16, u16, CursorStyle)>) {
            self.placements.push(cursor);
        }
    }

    /// A modal overlay (help, picker) covers the focused pane: the frame
    /// hides the host cursor instead of placing the pane's cell through
    /// the panel; dismissal restores placement (the manual-pass report —
    /// the block cursor drew through the help panel).
    #[test]
    fn modal_overlay_hides_the_pane_cursor() {
        let mut session = WindowSession::new(80, 25);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        session.renderer.feed_output(1, b"hello\r\n");

        let mut sink = CursorSink {
            placements: Vec::new(),
        };
        session
            .renderer
            .set_overlay(Some(("keybinds", vec![], None)));
        session.frame(&mut sink);
        assert_eq!(
            sink.placements.last(),
            Some(&None),
            "an open modal hides the host cursor: {:?}",
            sink.placements
        );

        session.renderer.set_overlay(None);
        session.frame(&mut sink);
        let expected = session
            .renderer
            .focused_cursor()
            .map(|(x, y, style)| (x, y + 1, style)); // +1: the strip row
        assert_eq!(
            sink.placements.last(),
            Some(&expected),
            "dismissal restores the pane cursor placement"
        );
    }

    /// End-to-end through the session's real frame path: a flushed frame
    /// always re-places the cursor (the diff's per-cell CUPs moved the
    /// host cursor), and a quiet pump emits nothing new. The session is
    /// constructed headless — `WindowSession::new` touches no daemon —
    /// and its renderer seeded directly.
    #[test]
    fn frame_replaces_cursor_after_flush_and_quiets_when_idle() {
        let mut session = WindowSession::new(80, 25);
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        session.renderer.apply_layout(layout);
        session.renderer.feed_output(1, b"hello\r\n");

        let mut sink = CursorSink {
            placements: Vec::new(),
        };
        // First frame: the flush must place the cursor (repaint_all's
        // hide is simulated by the initial cursor_placed = Some(None)
        // state — a placement must appear regardless, because flushed).
        session.frame(&mut sink);
        let expected = session
            .renderer
            .focused_cursor()
            .map(|(x, y, style)| (x, y + 1, style)); // +1: the strip row
        assert_eq!(sink.placements.len(), 1, "flushed frame places once");
        assert_eq!(sink.placements[0], expected);

        // A quiet pump: no flush, unchanged cursor state — no emission.
        session.frame(&mut sink);
        assert_eq!(sink.placements.len(), 1, "quiet pump emits nothing");

        // Pane output: a flush — the cursor re-places even though the
        // mapped cell did not move (the diff's CUPs moved the host one).
        // The "x" feed moved the tracked cursor to (1, 1); recompute.
        session.renderer.feed_output(1, b"x");
        let moved = session
            .renderer
            .focused_cursor()
            .map(|(x, y, style)| (x, y + 1, style)); // +1: the strip row
        session.frame(&mut sink);
        assert_eq!(sink.placements.len(), 2, "flush re-places");
        assert_eq!(sink.placements[1], moved);

        // Scroll the focused pane off live: state change to hidden even
        // without a flush. Pane 1 needs scrollback to scroll into first.
        for i in 0..30 {
            session
                .renderer
                .feed_output(1, format!("h{i}\r\n").as_bytes());
        }
        assert!(session.renderer.wheel_scroll(10, 5, 3));
        session.frame(&mut sink);
        assert_eq!(sink.placements.len(), 3, "hide emits");
        assert_eq!(sink.placements[2], None);
    }

    /// `recording_conn` with per-command canned replies: the fake daemon
    /// answers the exact command line with the body, everything else
    /// with an ok empty reply.
    fn scripted_conn(
        tag: &str,
        replies: std::collections::HashMap<String, String>,
    ) -> (
        std::sync::mpsc::Receiver<(String, String)>,
        crate::mux::attach::conn::AttachConn,
    ) {
        let mut path = std::env::temp_dir();
        path.push(format!("par-mux-render-{tag}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = crate::mux::bind_local_listener(&path).expect("bind");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            #[cfg(unix)]
            use interprocess::local_socket::traits::Listener as _;
            let Ok(stream) = listener.accept() else {
                return;
            };
            use interprocess::TryClone as _;
            let mut writer = stream.try_clone().expect("clone");
            let mut reader = std::io::BufReader::new(stream);
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
                    _ => replies.get(trimmed).cloned().unwrap_or_default(),
                };
                number += 1;
                tx.send((name, trimmed.to_owned())).ok();
                writer
                    .write_all(crate::mux::emit_block(number, &reply, true).as_bytes())
                    .ok();
                writer.flush().ok();
            }
        });
        let conn = crate::mux::attach::conn::AttachConn::connect(&path).expect("connect");
        (rx, conn)
    }

    /// A live `AttachConn` over a recording listener: every command rides
    /// the wire and gets an ok empty reply, and the recorded (name, line)
    /// pairs land on the returned receiver. The resize/swap affordance
    /// tests read the exact wire spellings off it.
    fn recording_conn(
        tag: &str,
    ) -> (
        std::sync::mpsc::Receiver<(String, String)>,
        crate::mux::attach::conn::AttachConn,
    ) {
        scripted_conn(tag, std::collections::HashMap::new())
    }

    /// The next recorded line named `name`, bounding the wait.
    fn wait_recorded(rx: &std::sync::mpsc::Receiver<(String, String)>, name: &str) -> String {
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
                    panic!("no {name} line: the fake daemon is gone");
                }
            }
        }
    }

    /// A sink recording every flushed cell and cursor placement — the
    /// frame-path read for the strip and height assertions.
    #[derive(Default)]
    struct RecordingSink {
        cells: Vec<(u16, u16, RtCell)>,
        placements: Vec<Option<(u16, u16, CursorStyle)>>,
    }

    impl FlushSink for RecordingSink {
        fn flush(&mut self, diff: &[(u16, u16, RtCell)]) {
            self.cells.extend_from_slice(diff);
        }
        fn repaint_all(&mut self) {}
        fn place_cursor(&mut self, cursor: Option<(u16, u16, CursorStyle)>) {
            self.placements.push(cursor);
        }
    }

    /// The tab strip costs one content row everywhere the status row
    /// does: the renderer is the host grid minus strip minus status
    /// (80x25 host → an 80x23 content renderer).
    #[test]
    fn session_height_carves_the_strip_and_status_out() {
        let session = WindowSession::new(80, 25);
        assert_eq!(session.renderer.window_size(), (80, 23));
    }

    /// The context menus' dispatch mapping is pure: each overlay row
    /// carries exactly one action (or `None` for the header/footer), so
    /// what a click highlights is exactly what dispatches — and the two
    /// menus' vocabularies differ in the third action only.
    #[test]
    fn compose_menu_panel_maps_rows_to_actions() {
        let (rows, mapping) = compose_menu_panel(&MenuTarget::Tab("@1".to_string()), "vim");
        assert_eq!(rows.len(), mapping.len(), "rows and mapping stay parallel");
        assert_eq!(rows[0].text, " vim ", "the header carries the name");
        assert_eq!(mapping[0], None, "the header consumes its clicks");
        assert_eq!(mapping[1], Some(MenuAction::Rename));
        assert_eq!(rows[1].text, " rename ");
        assert_eq!(mapping[2], Some(MenuAction::Close));
        assert_eq!(rows[2].text, " close ");
        assert_eq!(mapping[3], Some(MenuAction::AddTab));
        assert_eq!(rows[3].text, " add tab ");
        assert_eq!(mapping[4], None, "the footer consumes its clicks");

        let (_, ws_mapping) = compose_menu_panel(&MenuTarget::Workspace("+0".to_string()), "main");
        assert_eq!(
            ws_mapping,
            vec![
                None,
                Some(MenuAction::Rename),
                Some(MenuAction::Close),
                Some(MenuAction::NewWorkspace),
                None
            ]
        );
    }

    /// A flushed frame lands the strip at host row 0, rebases the pane
    /// diff one host row down, and places the cursor one host row below
    /// its renderer-mapped cell — the strip's height consumers, pinned.
    /// (`terminal_grid` is 80x24 headless, so the session is built for a
    /// 24-row host: strip 0, panes 1..=22, status 23.) Unix-only: the
    /// assertion pins the non-tty (80, 24) fallback — an interactive
    /// Windows console session reports its real grid and the bottom row
    /// lands elsewhere (measured on the Windows VM, 2026-10-04).
    #[cfg(unix)]
    #[test]
    fn frame_flushes_the_strip_at_row_zero_and_rebases_the_panes() {
        let mut session = WindowSession::new(80, 24);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        session.renderer.feed_output(1, b"pane-one\r\n");
        // Paint the strip and the status row so both rows flush.
        let windows = vec![
            ("@0".to_string(), "main".to_string()),
            ("@1".to_string(), "vim".to_string()),
        ];
        session.tab_strip.paint(&windows, Some("@0"), None);
        session.draw_status_row();

        let mut sink = RecordingSink::default();
        session.frame(&mut sink);

        // The strip's text cells flush at the host's top row...
        let strip_cells: Vec<&(u16, u16, RtCell)> =
            sink.cells.iter().filter(|(_, y, _)| *y == 0).collect();
        assert!(!strip_cells.is_empty(), "the strip flushes at row 0");
        // ...the pane diff lands strictly between the strip and the
        // status row (rebased one host row down)...
        assert!(sink.cells.iter().any(|(_, y, _)| (1..=22).contains(y)));
        // ...and the status row stays at the bottom (23).
        assert!(sink.cells.iter().any(|(x, y, _)| *y == 23 && *x < 80));
        // The cursor maps through the strip: renderer row + 1.
        let expected = session
            .renderer
            .focused_cursor()
            .map(|(x, y, style)| (x, y + 1, style));
        assert_eq!(sink.placements.last(), Some(&expected));
    }

    /// A press on a tab switches to that window through the existing
    /// select+resync contract — the wire carries `select-window` and the
    /// re-seed's size report (content height = rows - strip - status =
    /// 23 on an 80x25 host) — and never focuses or forwards into a pane.
    #[test]
    fn strip_click_switches_windows_without_forwarding() {
        let mut replies = std::collections::HashMap::new();
        replies.insert("list-sessions".to_string(), "$0: work".to_string());
        replies.insert(
            "list-windows -t $0".to_string(),
            "@0 - main\n@1 * vim".to_string(),
        );
        replies.insert("list-panes -t @1".to_string(), "%1".to_string());
        let (rx, mut conn) = scripted_conn("tabclick", replies);
        let mut session = WindowSession::new(80, 25);
        session.window = "@0".to_string();

        // The queried state fills through the real refresh path; the
        // strip paints from it.
        session.status.refresh(&mut conn, "@0", 1).expect("refresh");
        session.draw_tab_strip();
        // Drain the refresh traffic so the click's wire assertions see
        // only the click's lines.
        while rx
            .recv_timeout(std::time::Duration::from_millis(150))
            .is_ok()
        {}

        // The second tab's cell (" 1:vim ") spans cols 8..15; the click
        // is a 1-based host col 12 (strip col 11).
        session.tab_click(&mut conn, 11);

        assert_eq!(session.window, "@1", "the view moved to the clicked tab");
        assert_eq!(session.status.active_window.as_deref(), Some("@1"));
        // The wire contract: select-window + re-seed; nothing pane-level.
        let switch = wait_recorded(&rx, "select-window");
        assert!(
            switch.contains("-t @1"),
            "the click selects the clicked window: {switch}"
        );
        let size_report = wait_recorded(&rx, "refresh-client");
        assert!(
            size_report.contains("80x23"),
            "the size report carries content height rows-strip-status: {size_report}"
        );
        // No focus, no forwarding: drain the residual resync traffic
        // (status re-queries) and assert nothing pane-level rode it.
        while rx
            .recv_timeout(std::time::Duration::from_millis(150))
            .is_ok()
        {}
        // (The receiver is dropped with the conn at scope end; the
        // assertions above ran over every recorded line already.)
        assert_eq!(
            session.tab_strip.hit_test(11),
            Some(1),
            "the strip layout still maps the clicked column"
        );
    }

    /// The drag affordance, headless end-to-end: a press within one cell
    /// of the two-pane divider starts a PENDING drag (no focus, no
    /// forward), motion promotes it and the pump's apply_drag sends one
    /// relative resize-pane for the boundary's left pane at the full
    /// delta, the divider highlights while dragging, and release clears
    /// the state without forwarding anything to either pane.
    #[test]
    fn drag_on_divider_resizes_without_clicking_through() {
        let (rx, mut conn) = recording_conn("drag");
        let mut session = WindowSession::new(80, 25);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));

        // Drain the handshake's recorded lines first, so the quiet wire
        // assertions below see only the drag's traffic.
        while rx
            .recv_timeout(std::time::Duration::from_millis(150))
            .is_ok()
        {}

        // Press on the divider column (x=39, 1-based col 40): pending,
        // and the press must NOT focus or forward.
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 0,
                col: 40,
                row: 6,
                release: false,
            },
        );
        assert!(
            matches!(session.drag, Some(DragState::Pending { .. })),
            "the press starts a pending drag"
        );
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "a divider press forwards nothing"
        );

        // Motion +5 cells: the drag activates and highlights the divider.
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 32,
                col: 45,
                row: 6,
                release: false,
            },
        );
        assert_eq!(
            session.renderer.drag_divider,
            Some((true, 1, 2)),
            "the dragged divider highlights"
        );

        // The pump's frame-cadence application: one resize-pane at the
        // full delta, aimed at the boundary's left/top pane.
        session.apply_drag(&mut conn);
        assert_eq!(
            wait_recorded(&rx, "resize-pane"),
            "resize-pane -t %1 -R 5",
            "the drag maps to the wire's relative resize for pane 1"
        );
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "a drag forwards no mouse bytes to the pane"
        );

        // Release: the drag ends, the highlight clears, still no
        // click-through.
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 0,
                col: 45,
                row: 6,
                release: true,
            },
        );
        assert!(session.drag.is_none());
        assert_eq!(session.renderer.drag_divider, None);
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "the drag's release forwards nothing"
        );
    }

    /// A click NEAR a divider without any drag still focuses: press and
    /// release at the same point land the focus on the pane under the
    /// pointer (select-pane rides the wire) even though the press itself
    /// was captured by the drag's pending state.
    #[test]
    fn click_near_divider_without_drag_still_focuses() {
        let (rx, mut conn) = recording_conn("click-div");
        let mut session = WindowSession::new(80, 25);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        session.renderer.focus(2);

        // Press on the divider, release without motion: the focus flips
        // to the pane under the pointer (the divider column sits in pane
        // 1's last column).
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 0,
                col: 40,
                row: 6,
                release: false,
            },
        );
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 0,
                col: 40,
                row: 6,
                release: true,
            },
        );
        assert_eq!(
            session.renderer.focused(),
            Some(1),
            "the bare click focuses the pane under the pointer"
        );
        assert_eq!(
            wait_recorded(&rx, "select-pane"),
            "select-pane -t %1",
            "the focus rides select-pane"
        );
    }

    /// The tmux divider convention (the owner's manual pass): the HALF of
    /// the shared divider nearer the active pane carries the highlight —
    /// a side-by-side split's divider highlights its top half when the
    /// left pane is focused and its bottom half when the right one is.
    #[test]
    fn focus_flip_changes_the_shared_dividers_color() {
        let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(layout);

        renderer.focus(1);
        renderer.render_frame();
        assert_eq!(
            renderer.buffer[(39, 0)].fg,
            RtColor::Indexed(14),
            "pane 1 (left) focused: the divider's TOP half is the accent"
        );
        assert!(
            renderer.buffer[(39, 23)].modifier.contains(RtModifier::DIM),
            "pane 1 focused: the divider's bottom half stays dim"
        );
        renderer.focus(2);
        renderer.mark_all_dirty();
        renderer.render_frame();
        assert_eq!(
            renderer.buffer[(39, 23)].fg,
            RtColor::Indexed(14),
            "pane 2 (right) focused: the divider's BOTTOM half is the accent"
        );
        assert!(
            renderer.buffer[(39, 0)].modifier.contains(RtModifier::DIM),
            "pane 2 focused: the divider's top half stays dim"
        );
    }

    /// Render-mode resize chords: prefix R enters the mode and arrows
    /// send the relative resize-pane for the FOCUSED pane; a non-arrow
    /// key leaves the mode with nothing leaked into the pane.
    #[test]
    fn render_resize_chord_sends_for_the_focused_pane() {
        let (rx, conn) = recording_conn("render-resize");
        let mut session = WindowSession::new(80, 25);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        let mut conn = conn;
        let mut prefix_pending = false;

        assert!(
            !session.route_plain(
                &[crate::mux::attach::C_B, b'R'],
                &mut conn,
                &mut prefix_pending
            ),
            "the resize chord enters the mode"
        );
        assert!(session.resize_mode);
        session.resize_mode_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
        assert_eq!(
            wait_recorded(&rx, "resize-pane"),
            "resize-pane -t %1 -R 1",
            "the arrow resizes the focused pane (the layout's first leaf)"
        );
        // Any other key leaves the mode, consumed.
        session.resize_mode_key(&mut conn, &TermKeyEvent::functional(TermKey::Escape, 0));
        assert!(!session.resize_mode);

        // The RIGHT pane focused: the arrows move the shared divider in
        // the pressed direction, so the wire direction inverts (the wire
        // grows the focused pane; growing the right pane would move the
        // divider left — the manual-pass report).
        session.renderer.focus(2);
        assert!(
            !session.route_plain(
                &[crate::mux::attach::C_B, b'R'],
                &mut conn,
                &mut prefix_pending
            ),
            "the resize chord re-enters the mode"
        );
        session.resize_mode_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
        assert_eq!(
            wait_recorded(&rx, "resize-pane"),
            "resize-pane -t %2 -L 1",
            "Right with the right pane focused shrinks it: the divider moves right"
        );
    }

    /// The help chord: prefix ? opens the panel (categories and effective
    /// bindings in the overlay), / narrows it, and dismissal (q) restores
    /// the prior frame — the pane's content cells repaint.
    #[test]
    fn help_chord_opens_filters_and_dismissal_restores_the_frame() {
        let (_rx, conn) = recording_conn("help");
        let mut session = WindowSession::new(80, 25);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        session.renderer.feed_output(1, b"LEFT\r\n");
        let mut sink = CursorSink {
            placements: Vec::new(),
        };
        session.frame(&mut sink); // the settled prior frame

        let mut conn = conn;
        let mut prefix_pending = false;
        assert!(
            !session.route_plain(
                &[crate::mux::attach::C_B, b'?'],
                &mut conn,
                &mut prefix_pending
            ),
            "the help chord opens the panel"
        );
        assert!(session.help_mode);
        let overlay = session.renderer.overlay.clone().expect("the overlay is up");
        let joined = overlay
            .1
            .iter()
            .map(|r| r.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        // The compose carries content rows only; the ring/title/badge are
        // paint_overlay's (asserted in the paint-level tests below). The
        // idle panel has no placeholder row — the footer advertises
        // `search /`.
        assert!(
            !joined.contains("press / to filter"),
            "no idle placeholder row: {joined}"
        );
        assert!(joined.contains(" global "), "a category header: {joined}");
        assert!(
            joined.contains("close esc/enter"),
            "the footer names the controls: {joined}"
        );
        // The filter: '/' then "swap" narrows; a non-matching row drops.
        assert!(session.help_byte(b'/'));
        for byte in b"swap" {
            assert!(session.help_byte(*byte));
        }
        let filtered = session.renderer.overlay.clone().expect("overlay");
        let ftext = filtered
            .1
            .iter()
            .map(|r| r.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(ftext.contains("swap"), "matching rows survive: {ftext}");
        assert!(!ftext.contains("detach"), "others drop: {ftext}");
        // Commit the filter (Enter), then dismiss (q): the prior frame's
        // cells repaint.
        assert!(session.help_byte(b'\r'));
        assert!(
            !session.help_byte(b'q'),
            "q closed the panel (the return is still-open)"
        );
        assert!(!session.help_mode);
        assert_eq!(session.renderer.overlay, None);
        session.frame(&mut sink);
        let left: String = (0..4)
            .map(|c| session.renderer.buffer[(c, 0)].symbol())
            .collect();
        assert_eq!(left, "LEFT", "the prior frame's cells restore");
    }

    /// The picker chord: prefix w opens the modal (sessions with nested
    /// windows, the current one emphasized), j moves the cursor, Enter
    /// activates — the select+resync contract's select-window rides the
    /// wire — and `q` dismisses without selecting.
    #[test]
    fn picker_chord_opens_navigates_and_selects_with_resync() {
        let replies = std::collections::HashMap::from([
            (
                "list-sessions".to_string(),
                "+0: main: $0: work\n+1: lab: $1: build\n".to_string(),
            ),
            (
                "list-windows -t $0".to_string(),
                "@0 - main\n@1 * vim\n".to_string(),
            ),
            ("list-windows -t $1".to_string(), "@2 * logs\n".to_string()),
        ]);
        let (rx, conn) = scripted_conn("picker", replies);
        let mut conn = conn;
        let mut session = WindowSession::new(80, 25);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        // The view shows $0's window @0.
        session.status.session_id = Some("$0".to_string());
        session.window = "@0".to_string();

        let mut prefix_pending = false;
        assert!(
            !session.route_plain(
                &[crate::mux::attach::C_B, b'w'],
                &mut conn,
                &mut prefix_pending
            ),
            "the picker chord opens the modal"
        );
        assert!(session.picker_mode);
        let (overlay, _, _) = session.renderer.overlay.clone().expect("the overlay is up");
        assert_eq!(overlay, crate::mux::attach::PICKER_OVERLAY_TITLE);
        let joined = overlay_text(&session).join("\n");
        assert!(
            joined.contains(">$0: work"),
            "the current session is >-marked: {joined}"
        );
        assert!(
            joined.contains("@1: vim *"),
            "the active window carries *: {joined}"
        );

        // j j moves the cursor to the second window row; Enter activates
        // — the wire sees the select-window and the re-seed's queries
        // (the resync).
        session.picker_byte(&mut conn, b'j');
        session.picker_byte(&mut conn, b'j');
        session.picker_byte(&mut conn, b'\r');
        assert!(!session.picker_mode, "activation dismisses the modal");
        assert_eq!(
            wait_recorded(&rx, "select-window"),
            "select-window -t @1",
            "the session header's active window is the landing target"
        );
        let _ = wait_recorded(&rx, "list-panes");
        let _ = wait_recorded(&rx, "refresh-client");
        assert_eq!(session.window, "@1", "the view re-seeded onto @1");

        // Re-open and dismiss with q: no select goes out.
        assert!(!session.route_plain(
            &[crate::mux::attach::C_B, b'w'],
            &mut conn,
            &mut prefix_pending
        ));
        assert!(session.picker_mode);
        assert!(!session.picker_byte(&mut conn, b'q'));
        assert!(!session.picker_mode);
    }

    /// The overlay's rows joined (the test-side read of the modal's
    /// content).
    fn overlay_text(session: &WindowSession) -> Vec<String> {
        session
            .renderer
            .overlay
            .as_ref()
            .expect("the overlay is up")
            .1
            .iter()
            .map(|r| r.text.clone())
            .collect()
    }

    /// The `pane-gaps` option: each pane's content insets by the gap and
    /// the band cells carry the theme background — pane A's first content
    /// column moves to x=1 and the band at x=0 keeps the fill.
    #[test]
    fn pane_gaps_option_insets_pane_content_and_fills_the_band() {
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_background(Some(RtColor::Rgb(16, 24, 40)));
        renderer.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        renderer.feed_output(1, b"LEFT\r\n");
        renderer.feed_output(2, b"RIGHT\r\n");
        renderer.set_pane_gaps(1);
        renderer.render_frame();

        let bg = RtColor::Rgb(16, 24, 40);
        // The band rows/cols carry no content and keep the fill; the
        // content starts one cell in on each axis.
        for x in [0u16, 40] {
            let band = renderer.cell(x, 0).expect("band cell");
            assert_eq!(band.symbol(), " ", "the band cell carries no content");
            assert_eq!(band.bg, bg, "the band cell carries the theme bg");
        }
        let a1 = renderer.cell(1, 1).expect("content cell");
        assert_eq!(a1.symbol(), "L", "pane A's content starts inside the gap");
        let b1 = renderer.cell(41, 1).expect("right content cell");
        assert_eq!(b1.symbol(), "R", "pane B's content starts inside the gap");
        // The cursor maps through the same inset (content_view drives
        // the placement path too): the feed's trailing newline left the
        // pane cursor at content col 0 / row 1.
        let cursor = renderer.focused_cursor().expect("cursor");
        assert_eq!(cursor.0, 1, "the cursor maps inside the left band");
        assert_eq!(cursor.1, 2, "the cursor maps below the top band");
    }

    /// Options OFF render identical to a default renderer: the
    /// pins-the-default contract for pane-gaps / scrollbar-gutter.
    #[test]
    fn display_options_off_render_identical_to_defaults() {
        let build = |gaps: u16, gutter: bool| {
            let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
            renderer.set_background(Some(RtColor::Rgb(16, 24, 40)));
            renderer.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
            renderer.feed_output(1, b"LEFT\r\nsecond\r\n");
            renderer.feed_output(2, b"RIGHT\r\n");
            renderer.set_pane_gaps(gaps);
            renderer.set_scrollbar_gutter(gutter);
            renderer.render_frame();
            renderer
        };
        let baseline = build(0, false);
        let explicit = build(0, false);
        for y in 0..24u16 {
            for x in 0..80u16 {
                let (a, b) = (
                    baseline.cell(x, y).expect("cell"),
                    explicit.cell(x, y).expect("cell"),
                );
                assert_eq!(a.symbol(), b.symbol(), "symbol differs at {x},{y}");
                assert_eq!(a.bg, b.bg, "bg differs at {x},{y}");
                assert_eq!(a.fg, b.fg, "fg differs at {x},{y}");
            }
        }
    }

    /// The `scrollbar-gutter` option: the content narrows by one column,
    /// the gutter fills with the theme bg, and a scrolled pane shows the
    /// position indicator; an unscrolled pane's gutter stays blank and
    /// gutter OFF paints no indicator at all.
    #[test]
    fn scrollbar_gutter_option_reserves_a_column_and_indicates() {
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_background(Some(RtColor::Rgb(16, 24, 40)));
        renderer.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        // 40 lines of scrollback on pane 1.
        let mut flood = Vec::new();
        for i in 0..40 {
            flood.extend(format!("line{i:02}\r\n").as_bytes());
        }
        renderer.feed_output(1, &flood);
        renderer.feed_output(2, b"R\r\n");
        renderer.set_scrollbar_gutter(true);
        renderer.render_frame();

        // Unscrolled: the gutter column is theme-bg blank; the content
        // beside it is the pane's line (the last full line — the feed's
        // trailing newline scrolled one more row).
        let gutter = renderer.cell(39, 23).expect("gutter cell");
        assert_eq!(gutter.symbol(), " ", "an unscrolled gutter stays blank");
        let content = renderer.cell(0, 22).expect("narrowed content");
        assert_eq!(
            content.symbol(),
            "l",
            "content paints through the narrowed view"
        );

        renderer.scroll_viewport(1, 10);
        renderer.render_frame();
        // The indicator sits at the view top's proportional depth:
        // 10 * 24 / (10 + 24) = row 7.
        let indicator = renderer.cell(39, 7).expect("indicator cell");
        assert_eq!(
            indicator.symbol(),
            "\u{2590}",
            "the scrolled pane shows the indicator"
        );
        assert_eq!(
            indicator.fg,
            RtColor::Indexed(14),
            "the indicator is accent"
        );

        // Gutter OFF: no indicator paints at the pane's edge.
        let mut plain = PaneRenderer::new(80, 24, Glyphs::Unicode);
        plain.set_background(Some(RtColor::Rgb(16, 24, 40)));
        plain.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        plain.feed_output(1, &flood);
        plain.render_frame();
        plain.scroll_viewport(1, 10);
        plain.render_frame();
        assert_ne!(
            plain.cell(39, 0).expect("plain").symbol(),
            "\u{2590}",
            "gutter off paints no indicator"
        );
    }

    /// The `drag-cursor-shape` option: while a divider drag is live the
    /// frame's cursor placement carries the resize shape (steady block),
    /// overriding the pane's tracked DECSCUSR shape; option off (and
    /// drag end) keeps the pane's own shape.
    #[test]
    fn drag_cursor_shape_option_shapes_the_cursor_during_a_drag() {
        let mut session = WindowSession::new(80, 24);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        // A distinctive tracked shape: the pane set DECSCUSR blinking bar.
        session.renderer.feed_output(1, b"hi\x1b[5 q\r\n");
        let divider = DividerHit {
            vertical: true,
            a: 1,
            b: 2,
        };
        session.drag = Some(DragState::Active {
            divider,
            x: 39,
            y: 0,
            applied: 0,
            pending: 0,
        });

        // Option off: the pane's tracked shape survives a drag frame.
        let mut sink = RecordingSink::default();
        session.frame(&mut sink);
        let last = sink
            .placements
            .last()
            .copied()
            .flatten()
            .expect("placement");
        assert_eq!(
            last.2,
            CursorStyle::BlinkingBar,
            "off keeps the pane's shape"
        );

        // Option on: the drag frame shapes the cursor.
        session.drag_cursor_shape = true;
        session.renderer.mark_all_dirty();
        let mut sink = RecordingSink::default();
        session.frame(&mut sink);
        let last = sink
            .placements
            .last()
            .copied()
            .flatten()
            .expect("placement");
        assert_eq!(last.2, CursorStyle::SteadyBlock, "on shapes the cursor");

        // Drag end restores: the next frame carries the pane's shape.
        session.drag = None;
        session.renderer.mark_all_dirty();
        let mut sink = RecordingSink::default();
        session.frame(&mut sink);
        let last = sink
            .placements
            .last()
            .copied()
            .flatten()
            .expect("placement");
        assert_eq!(last.2, CursorStyle::BlinkingBar, "drag end restores");
    }

    /// Two side-by-side panes (a vertical divider at x=39).
    const TWO_PANE: &str = "0000,80x24,0,0{40x24,0,0,1,40x24,40,0,2}";
    /// The same window re-divided for a 20-col side panel: the 60-col
    /// extent split in two 30-col panes.
    const TWO_PANE_SIDEBAR: &str = "0000,60x24,0,0{30x24,0,0,1,30x24,30,0,2}";
    /// Two stacked panes (a horizontal divider at y=11).
    const STACKED: &str = "0000,80x24,0,0{80x12,0,0,1,80x12,0,12,2}";

    /// Sidebar paint geometry: with the panel up, the re-divided
    /// layout's right pane paints flush against the panel — content and
    /// dividers carry the panel offset exactly once. (The manual-pass
    /// double-push report: content shifted twice, dividers once.)
    #[test]
    fn sidebar_offset_paints_content_and_dividers_together() {
        let mut session = WindowSession::new(80, 25);
        // The pre-toggle layout: two 40-col panes; the right pane's
        // marker row lands at x=40 (no panel, no offset).
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE).expect("parses"));
        session.renderer.feed_output(2, b"GEO-MARK-42\r\n");
        session.renderer.render_frame();
        assert_eq!(
            session.renderer.cell(40, 0).map(|c| c.symbol()),
            Some("G"),
            "pre-toggle the right pane starts at 40"
        );

        // The toggle: panel width on, the daemon's re-divided layout in.
        session.renderer.set_sidebar_width(20);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE_SIDEBAR).expect("parses"));
        session.renderer.render_frame();
        let symbol = |x: u16, y: u16| session.renderer.cell(x, y).map(|c| c.symbol());
        assert_eq!(
            symbol(50, 0),
            Some("G"),
            "content offset once by the panel: the right pane at 30+20"
        );
        assert_eq!(symbol(60, 0), Some("2"), "the marker paints unwrapped");
        assert_eq!(
            symbol(49, 5),
            Some("│"),
            "the pane divider carries the same offset"
        );
        assert_eq!(
            symbol(40, 0),
            Some(" "),
            "the vacated pre-toggle position is gone"
        );
    }

    /// A sink that records flushed cells (the flush-gate test's probe).
    struct CellSink {
        cells: Vec<(u16, u16, RtCell)>,
    }

    impl FlushSink for CellSink {
        fn flush(&mut self, diff: &[(u16, u16, RtCell)]) {
            self.cells.extend(diff.iter().cloned());
        }
        fn repaint_all(&mut self) {}
        fn place_cursor(&mut self, _cursor: Option<(u16, u16, CursorStyle)>) {}
    }

    /// While the status bar is hidden the row is NEVER flushed: the
    /// bottom row is pane real estate, and a blank-cell diff flush would
    /// erase the pane's freshly painted bottom row right after the pane
    /// frame. Shown, the row flushes as usual.
    #[test]
    fn hidden_status_bar_flushes_nothing() {
        let mut session = WindowSession::new(80, 24);
        session.draw_status_row();
        let mut sink = CellSink { cells: Vec::new() };
        assert!(
            session.flush_status_row(&mut sink),
            "shown, the row flushes"
        );
        assert!(!sink.cells.is_empty());

        session.status_bar_on = false;
        session.draw_status_row();
        let mut sink = CellSink { cells: Vec::new() };
        assert!(
            !session.flush_status_row(&mut sink),
            "hidden, nothing flushes"
        );
        assert!(sink.cells.is_empty());
    }

    /// Side-panel click geometry after round 6: the lookup takes the
    /// HOST row (row 0 is the tab strip's — nothing there), shifted one
    /// row so a click lands on the row as painted — the FIRST workspace
    /// row at host row 1 (no section header above it), the footer chips
    /// at the panel's last row, each chip only its own cell.
    #[test]
    fn sidebar_clicks_land_on_the_painted_row() {
        let mut session = WindowSession::new(80, 24);
        session.renderer.set_sidebar_width(20);
        session
            .renderer
            .set_sidebar_sections(Some(vec![super::super::SidebarSection {
                rows: vec![
                    ("ws:+0".to_string(), "alpha".to_string(), true),
                    ("ws:+1".to_string(), "beta".to_string(), false),
                ],
            }]));
        // Host row 0 (the tab strip row): nothing.
        assert_eq!(session.renderer.sidebar_row_at(2, 0), None);
        // Host row 1: the first workspace row, as painted (no header
        // above it since round 6).
        assert_eq!(
            session.renderer.sidebar_row_at(2, 1),
            Some("ws:+0".to_string()),
            "the click lands on the row as painted"
        );
        // Host row 2: the second workspace row.
        assert_eq!(
            session.renderer.sidebar_row_at(2, 2),
            Some("ws:+1".to_string())
        );
        // The footer row (host row = renderer height - 1 + 1 = 24... the
        // 22-row renderer's LAST row at host row 22): the chips answer
        // only within their own cells — ` new ` spans cols 0..5, ` menu
        // ` the content's right edge.
        assert_eq!(
            session.renderer.sidebar_row_at(2, 22),
            Some("panel:new".to_string()),
            "the new chip's cell"
        );
        assert_eq!(
            session.renderer.sidebar_row_at(16, 22),
            Some("panel:menu".to_string()),
            "the menu chip's cell"
        );
        assert_eq!(
            session.renderer.sidebar_row_at(10, 22),
            None,
            "the gap between the chips hits nothing"
        );
        // A body row answers across its full content width (the active
        // row's inverted block is clickable anywhere on it).
        assert_eq!(
            session.renderer.sidebar_row_at(17, 1),
            Some("ws:+0".to_string())
        );
        // Past the strip's width: nothing.
        assert_eq!(session.renderer.sidebar_row_at(20, 2), None);
    }

    /// The probe FAILED (`bg: None`): the frame fill paints NO color -
    /// every cell stays terminal-default, so a failed probe can never
    /// mismatch the theme. Default-to-black only appears when the probe
    /// SUCCEEDS with black.
    #[test]
    fn probe_failure_fill_leaves_terminal_default_cells() {
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
        let diff = renderer.render_frame();
        assert!(!diff.is_empty());
        assert_eq!(
            renderer.cell(0, 0).expect("cell").bg,
            RtColor::Reset,
            "the fill paints no color"
        );
        let mid = renderer.cell(70, 10).expect("in-window cell");
        assert_eq!(mid.bg, RtColor::Reset);

        // A probe that SUCCEEDS with black still paints black.
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_background(Some(RtColor::Rgb(0, 0, 0)));
        renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
        renderer.render_frame();
        let cell = renderer.cell(70, 10).expect("in-window cell");
        assert_eq!(cell.bg, RtColor::Rgb(0, 0, 0));
    }

    /// The help modal's paint: every cell carries the resolved theme bg,
    /// the border ring draws the rounded box-drawing glyphs in the accent
    /// color, and the title/badge sit in the top border.
    #[test]
    fn help_modal_paints_theme_bg_accent_ring_and_title() {
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        let bg = RtColor::Rgb(30, 30, 30);
        renderer.set_background(Some(bg));
        renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
        let rows = crate::mux::attach::compose_help_panel(
            &crate::mux::attach::help_rows(0x02, 0x12, Default::default(), 1),
            "",
            false,
            6,
            0,
        );
        renderer.set_overlay(Some((crate::mux::attach::HELP_OVERLAY_TITLE, rows, None)));
        renderer.render_frame();
        // Locate the modal's top-left corner.
        let mut origin = None;
        for y in 0..24u16 {
            for x in 0..80u16 {
                if renderer.cell(x, y).expect("cell").symbol() == "\u{256d}" {
                    origin = Some((x, y));
                }
            }
        }
        let (x0, y0) = origin.expect("the modal's rounded corner draws");
        // The title rides the top border (" keybinds " from x0+1).
        assert_eq!(renderer.cell(x0 + 2, y0).expect("cell").symbol(), "k");
        // The top edge between title and badge runs in the accent color.
        let edge = renderer.cell(x0 + 20, y0).expect("cell");
        assert_eq!(edge.symbol(), "\u{2500}");
        assert_eq!(edge.fg, RtColor::Indexed(14), "the ring is accent");
        // Interior cells carry the theme bg (no default/light cells).
        let inside = renderer.cell(x0 + 5, y0 + 2).expect("cell");
        assert_eq!(inside.bg, bg, "modal cells carry the theme bg");
        // The modal's height: rows.len() clamped to the window; with the
        // composed panel (filter + 6 window rows + footer) the box is 10
        // rows tall - find the bottom corner on this column.
        let bottom = (y0..24u16)
            .map(|y| (y, renderer.cell(x0, y).expect("cell").symbol().to_string()))
            .find(|(_, sym)| sym == "\u{2570}");
        assert!(bottom.is_some(), "bottom-left rounded corner draws");
    }

    /// With the help panel open, wheels scroll the PANEL and reach
    /// nothing else: the pane's client scrollback does not move and no
    /// wheel falls through to forwarding.
    #[test]
    fn help_open_consumes_wheel_events() {
        let (_, mut conn) = recording_conn("help-wheel");
        let mut session = WindowSession::new(80, 25);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE).expect("parses"));
        for i in 0..30 {
            session
                .renderer
                .feed_output(1, format!("h{i}\r\n").as_bytes());
        }
        session.enter_help();
        // Wheel down (cb 65) scrolls the panel down three rows; wheel up
        // (cb 64) back. The events never reach the pane paths.
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 65,
                col: 10,
                row: 10,
                release: false,
            },
        );
        assert_eq!(session.help_scroll, 3, "the wheel scrolls the panel");
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 64,
                col: 10,
                row: 10,
                release: false,
            },
        );
        assert_eq!(session.help_scroll, 0, "wheel up scrolls back");
        assert_eq!(
            session.renderer.scroll_offset_of(1),
            0,
            "the pane scrollback never moved"
        );
    }

    /// A drag survives MOTION WHILE ACTIVE: every additional motion
    /// extends the delta and the frame-cadence application sends one
    /// resize per unapplied cell. (The round-3 defect: the second motion
    /// destroyed the active drag, so only one resize ever fired while the
    /// highlight stayed up.)
    #[test]
    fn drag_survives_active_motion_and_resizes_per_cell() {
        let (rx, mut conn) = recording_conn("drag-move");
        let mut session = WindowSession::new(80, 25);
        session
            .renderer
            .apply_layout(parse_layout(TWO_PANE).expect("parses"));
        while rx
            .recv_timeout(std::time::Duration::from_millis(120))
            .is_ok()
        {}
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 0,
                col: 40,
                row: 6,
                release: false,
            },
        );
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 32,
                col: 41,
                row: 6,
                release: false,
            },
        );
        session.apply_drag(&mut conn);
        assert_eq!(
            wait_recorded(&rx, "resize-pane"),
            "resize-pane -t %1 -R 1",
            "the first motion resizes one cell"
        );
        // A SECOND motion while active must extend the drag, not kill it.
        session.route_mouse(
            &mut conn,
            SgrMouse {
                cb: 32,
                col: 43,
                row: 6,
                release: false,
            },
        );
        session.apply_drag(&mut conn);
        assert_eq!(
            wait_recorded(&rx, "resize-pane"),
            "resize-pane -t %1 -R 2",
            "the second motion resizes its unapplied delta"
        );
    }

    /// `pane-borders = on`: each pane renders a complete ring (corners on
    /// every rect), the focused pane's border in the accent, the other
    /// dim - and the option OFF renders no corner glyphs (today's
    /// shared-divider look, byte for byte the default).
    #[test]
    fn pane_borders_option_replaces_dividers_with_full_boxes() {
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_background(Some(RtColor::Rgb(0, 0, 0)));
        renderer.set_pane_borders(true);
        renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
        renderer.focus(2);
        renderer.render_frame();
        assert_eq!(renderer.cell(0, 0).expect("c").symbol(), "\u{256d}");
        assert_eq!(renderer.cell(39, 0).expect("c").symbol(), "\u{256e}");
        assert_eq!(renderer.cell(40, 0).expect("c").symbol(), "\u{256d}");
        assert_eq!(renderer.cell(79, 23).expect("c").symbol(), "\u{256f}");
        // Focused pane 2's right border carries the accent; pane 1's is
        // dim.
        let focused = renderer.cell(79, 10).expect("c");
        assert_eq!(focused.fg, RtColor::Indexed(14));
        let unfocused = renderer.cell(39, 10).expect("c");
        assert!(
            unfocused.modifier.contains(RtModifier::DIM) && unfocused.fg != RtColor::Indexed(14),
            "the unfocused border is dim"
        );

        // Option OFF: no corner glyphs anywhere (the default look).
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
        renderer.render_frame();
        assert_ne!(renderer.cell(0, 0).expect("c").symbol(), "\u{256d}");
    }

    /// `show-label-in-border = on`: the pane's user title embeds in the
    /// top edge space-padded, the label cells are not drag handles, and
    /// the option OFF leaves plain borders.
    #[test]
    fn label_in_border_embeds_title_and_blocks_drag_on_label_cells() {
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_background(Some(RtColor::Rgb(0, 0, 0)));
        renderer.set_pane_borders(true);
        renderer.set_show_label_in_border(true);
        renderer.apply_layout(parse_layout(STACKED).expect("parses"));
        renderer.feed_output(2, b"\x1b]2;dbug\x1b\\");
        renderer.render_frame();
        // The label " dbug " starts one cell in from the rect corner.
        assert_eq!(renderer.cell(2, 12).expect("c").symbol(), "d");
        assert_eq!(renderer.cell(1, 12).expect("c").symbol(), " ");
        assert!(renderer.label_cell_at(2, 12), "a label cell knows itself");
        assert!(
            !renderer.label_cell_at(70, 12),
            "plain border cells stay drag handles"
        );
        assert!(
            !renderer.label_cell_at(2, 13),
            "interior cells are not label cells"
        );

        // A press ON the label cell does not start a drag even though the
        // top border row sits within the divider tolerance.
        let (_, mut conn) = recording_conn("label-drag");
        let mut session = WindowSession::new(80, 25);
        session.renderer.set_pane_borders(true);
        session.renderer.set_show_label_in_border(true);
        session
            .renderer
            .apply_layout(parse_layout(STACKED).expect("parses"));
        session.renderer.feed_output(2, b"\x1b]2;dbug\x1b\\");
        session.route_mouse(
            &mut conn,
            SgrMouse {
                // Host row 14 = content row 12 (the strip row shifts the
                // content down one).
                cb: 0,
                col: 3,
                row: 14,
                release: false,
            },
        );
        assert!(
            session.drag.is_none(),
            "a label press focuses, it never drags"
        );

        // Option OFF (the default): the title does not render.
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_pane_borders(true);
        renderer.apply_layout(parse_layout(STACKED).expect("parses"));
        renderer.feed_output(2, b"\x1b]2;dbug\x1b\\");
        renderer.render_frame();
        assert_ne!(renderer.cell(2, 12).expect("c").symbol(), "d");
    }

    /// The cursor placement path shares the border inset: with the
    /// per-pane-border option on, the tracked cell maps inside the ring
    /// (hidden when it falls in the cropped perimeter band); with the
    /// option off the mapping is the plain rect origin - the round-3
    /// pin for the cell-to-screen cursor math.
    #[test]
    fn focused_cursor_maps_through_the_rect_and_the_border_inset() {
        // Off: plain rect-origin mapping.
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
        renderer.feed_output(1, b"hi");
        assert_eq!(
            renderer.focused_cursor(),
            Some((2, 0, CursorStyle::BlinkingBlock)),
            "plain mapping: rect origin plus the tracked cell"
        );

        // On: inset by the ring.
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_pane_borders(true);
        renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
        renderer.feed_output(1, b"hi");
        assert_eq!(
            renderer.focused_cursor(),
            Some((3, 1, CursorStyle::BlinkingBlock)),
            "inset mapping: ring offset plus the tracked cell"
        );
    }

    /// The btop-exit artifact (kanban card: stale solid-background cells
    /// after an alt-screen app exits), pinned headlessly. A pane draws an
    /// alt-screen btop-like frame — rows of solid-background cells, wide
    /// chars included — the renderer frames it, then the app exits
    /// (CSI ? 1049 l) back to the primary screen's shell prompt and the
    /// frame re-renders. The composed frame must show the primary screen
    /// with NO btop background remnants anywhere: both the live diff
    /// path (the same renderer fed the exit bytes in a second frame) and
    /// the re-seed path (a fresh renderer replaying the full scripted
    /// session, as reseed_window does) are checked.
    #[test]
    fn alt_screen_exit_leaves_no_stale_background_at_the_prompt_rows() {
        let theme_bg = RtColor::Rgb(16, 16, 16);
        let red = map_color(CoreColor::Named(NamedColor::Red));
        let blue = map_color(CoreColor::Named(NamedColor::Blue));
        let green = map_color(CoreColor::Named(NamedColor::Green));

        // The scripted session: alt-screen enter, three btop-like rows
        // (a red bar, a blue row of double-width chars filling all 80
        // columns, a green bar), then the app exits to the restored
        // primary screen and the shell draws its prompt on row 10.
        let wide_row: String = "\u{6f22}".repeat(40); // 40 wide chars = 80 cols
        let session = format!(
            "\x1b[?1049h\x1b[H\x1b[41m{}\x1b[0m\r\n\x1b[44m{}\x1b[0m\r\n\x1b[42m{}\x1b[0m\x1b[?1049l\x1b[10;1H$ ",
            "R".repeat(80),
            wide_row,
            "G".repeat(80),
        );

        // The exit marker is pure ASCII, so its byte offset is a char
        // boundary; the session is fed in byte slices around it.
        let bytes = session.as_bytes();
        let split = session.find("\x1b[?1049l").expect("exit marker");

        // The alt-screen frame really is on screen when framed alone.
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_background(Some(theme_bg));
        renderer.apply_layout(parse_layout("0000,80x24,0,0,2").expect("parses"));
        renderer.feed_output(2, &bytes[..split]);
        let first = renderer.render_frame();
        assert!(!first.is_empty(), "the btop frame paints");
        assert_eq!(renderer.cell(0, 0).expect("c").symbol(), "R");
        assert_eq!(renderer.cell(0, 0).expect("c").bg, red);
        assert_eq!(renderer.cell(0, 1).expect("c").bg, blue);

        // Live path: the same renderer, the exit bytes landing in a
        // second frame.
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_background(Some(theme_bg));
        renderer.apply_layout(parse_layout("0000,80x24,0,0,2").expect("parses"));
        renderer.feed_output(2, &bytes[..split]);
        renderer.render_frame();
        renderer.feed_output(2, &bytes[split..]);
        let second = renderer.render_frame();
        assert!(!second.is_empty(), "the exit repaints");
        for y in 0..24u16 {
            for x in 0..80u16 {
                let cell = renderer.cell(x, y).expect("in-bounds");
                assert_ne!(
                    cell.bg,
                    red,
                    "stale red bg at ({x},{y}): {:?}",
                    cell.symbol()
                );
                assert_ne!(
                    cell.bg,
                    blue,
                    "stale blue bg at ({x},{y}): {:?}",
                    cell.symbol()
                );
                assert_ne!(
                    cell.bg,
                    green,
                    "stale green bg at ({x},{y}): {:?}",
                    cell.symbol()
                );
            }
        }
        // The prompt landed where the app put it.
        assert_eq!(renderer.cell(0, 9).expect("c").symbol(), "$");

        // Re-seed path: a fresh renderer replaying the session (the
        // reseed_window reconstruction), framed once.
        let mut reseeded = PaneRenderer::new(80, 24, Glyphs::Unicode);
        reseeded.set_background(Some(theme_bg));
        reseeded.apply_layout(parse_layout("0000,80x24,0,0,2").expect("parses"));
        reseeded.feed_output(2, session.as_bytes());
        reseeded.render_frame();
        for y in 0..24u16 {
            for x in 0..80u16 {
                let cell = reseeded.cell(x, y).expect("in-bounds");
                assert_ne!(cell.bg, red, "re-seed stale red at ({x},{y})");
                assert_ne!(cell.bg, blue, "re-seed stale blue at ({x},{y})");
                assert_ne!(cell.bg, green, "re-seed stale green at ({x},{y})");
            }
        }
    }

    /// The new-tab prompt's default name: one past the highest window
    /// ordinal, bumped past any window NAME that already claims the
    /// number (the manual-pass ask: the next non-conflicting index).
    #[test]
    fn next_window_name_bumps_past_conflicts() {
        // One past the highest window ordinal.
        assert_eq!(
            super::super::next_window_name(&[
                ("@0".to_string(), "0".to_string()),
                ("@1".to_string(), "1".to_string()),
            ]),
            "2"
        );
        // A rename claiming the next index bumps the default past it.
        assert_eq!(
            super::super::next_window_name(&[
                ("@0".to_string(), "0".to_string()),
                ("@1".to_string(), "2".to_string()),
            ]),
            "3"
        );
        // No numeric names in use: still the ordinal successor.
        assert_eq!(
            super::super::next_window_name(&[("@0".to_string(), "demo".to_string())]),
            "1"
        );
    }

    /// A name crossing the wire spelled so the daemon's quoting grammar
    /// keeps it one word: unconditional single quotes, embedded quotes
    /// via the `'\''` idiom.
    #[test]
    fn wire_quote_survives_spaces_and_quotes() {
        assert_eq!(super::super::wire_quote("notes"), "'notes'");
        assert_eq!(super::super::wire_quote("my notes"), "'my notes'");
        assert_eq!(super::super::wire_quote("it's"), "'it'\\''s'");
        assert_eq!(super::super::wire_quote(""), "''");
    }

    /// The help overlay's overflow scrollbar: a `┃` thumb on the ring's
    /// right column, sized/positioned from the scroll state; a panel
    /// that fits paints nothing on the ring.
    #[test]
    fn help_overlay_paints_a_scrollbar_thumb_when_it_overflows() {
        use super::super::{compose_help_panel, help_rows};
        let rows = help_rows(0x02, 0x12, Default::default(), 1);
        let total = super::super::help_content(&rows, "").len();
        let mut renderer = PaneRenderer::new(80, 40, Glyphs::Unicode);
        renderer.apply_layout(parse_layout("0000,80x40,0,0,1").expect("parses"));
        // A window small enough that the panel must scroll: thumb at the
        // top border column.
        renderer.set_overlay(Some((
            crate::mux::attach::HELP_OVERLAY_TITLE,
            compose_help_panel(&rows, "", false, 6, 0),
            Some((0, 6, total)),
        )));
        renderer.render_frame();
        let (x0, y0, inner, height) = renderer.overlay_geometry().expect("geometry");
        let ring_x = (x0 + inner + 1) as u16;
        let thumbs: Vec<u16> = (y0 as u16 + 1..y0 as u16 + 1 + height as u16)
            .filter(|y| renderer.cell(ring_x, *y).is_some_and(|c| c.symbol() == "┃"))
            .collect();
        assert!(
            !thumbs.is_empty(),
            "the thumb paints on the ring: rows {thumbs:?}"
        );
        assert_eq!(
            thumbs.len(),
            (height * 6 / total).max(1),
            "the thumb spans its proportional share"
        );
        // Scroll to the bottom: the thumb parks at the track's end.
        renderer.set_overlay(Some((
            crate::mux::attach::HELP_OVERLAY_TITLE,
            compose_help_panel(&rows, "", false, 6, total),
            Some((total.saturating_sub(6), 6, total)),
        )));
        renderer.render_frame();
        let last_thumb = (y0 as u16 + 1..y0 as u16 + 1 + height as u16)
            .filter(|y| renderer.cell(ring_x, *y).is_some_and(|c| c.symbol() == "┃"))
            .max()
            .expect("a bottom thumb");
        assert_eq!(
            last_thumb,
            y0 as u16 + height as u16,
            "the thumb parks at the track's end"
        );
        // A panel that fits: nothing on the ring.
        let (x0s, y0s, inners, heights) = {
            let short: Vec<crate::mux::attach::HelpRow> = (0..4)
                .map(|i| crate::mux::attach::HelpRow {
                    text: format!("row {i}"),
                    accent: false,
                    footer: false,
                })
                .chain(std::iter::once(crate::mux::attach::HelpRow {
                    text: "footer".to_string(),
                    accent: false,
                    footer: true,
                }))
                .collect();
            renderer.set_overlay(Some((" t ", short, Some((0, 6, 4)))));
            renderer.render_frame();
            renderer.overlay_geometry().expect("geometry")
        };
        assert!(
            (y0s as u16 + 1..y0s as u16 + 1 + heights as u16).all(|y| renderer
                .cell((x0s + inners + 1) as u16, y)
                .is_some_and(|c| c.symbol() != "┃"))
        );
    }
}
