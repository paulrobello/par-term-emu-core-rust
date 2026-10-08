//! The pane renderer's painting and frame diffing: per-pane grids, dividers,
//! pane boxes, the modal overlay, layout geometry, and the stdout flush sink.

use super::*;

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
pub(super) fn active_border_style(active: Option<RtColor>) -> RtStyle {
    let style = match active {
        Some(RtColor::Rgb(r, g, b)) => RtStyle::default().fg(RtColor::Rgb(r, g, b)),
        _ => RtStyle::default().fg(RtColor::Indexed(14)), // bright cyan
    };
    style.add_modifier(RtModifier::BOLD)
}

/// Unfocused divider/border look (config `border-color`, `#rrggbb`;
/// default the dim modifier alone).
pub(super) fn plain_border_style(plain: Option<RtColor>) -> RtStyle {
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
pub(super) fn divider_style(
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

pub(super) fn rows_overlap(a: &PaneRect, b: &PaneRect) -> bool {
    a.y < b.y + b.height && b.y < a.y + a.height
}

/// The prefix+arrow pane navigation's directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaneDir {
    Up,
    Down,
    Left,
    Right,
}

/// The nearest pane to `focused` in `dir` (the prefix+arrow chord):
/// candidates must lie strictly in the direction from the focused
/// pane's center; the nearest center wins. `None` at an edge.
#[must_use]
pub(super) fn pane_in_direction(rects: &[PaneRect], focused: u32, dir: PaneDir) -> Option<u32> {
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

pub(super) fn cols_overlap(a: &PaneRect, b: &PaneRect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width
}

pub(super) fn row_overlap(a: &PaneRect, b: &PaneRect) -> Vec<u16> {
    let start = a.y.max(b.y);
    let end = (a.y + a.height).min(b.y + b.height);
    (start..end).collect()
}

pub(super) fn col_overlap(a: &PaneRect, b: &PaneRect) -> Vec<u16> {
    let start = a.x.max(b.x);
    let end = (a.x + a.width).min(b.x + b.width);
    (start..end).collect()
}

pub(super) fn map_color(color: CoreColor) -> RtColor {
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

pub(super) fn map_flags(flags: &CellFlags) -> RtStyle {
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

/// The stdout sink: absolute CUP + SGR per diff cell — ratatui's diff is
/// exactly the minimal cell set, and each cell carries its full style, so
/// per-cell reset+SGR+CUP+glyph is correct if not maximal-minimal. Frame
/// cadence (16 ms) keeps the volume at TUI-ordinary levels.
pub(super) struct StdoutSink;

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
pub(super) fn push_cursor_shape(out: &mut String, style: CursorStyle) {
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
pub(super) fn write_styled(out: &mut String, cell: &RtCell) {
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

pub(super) fn write_color(out: &mut String, color: RtColor, fg: bool) {
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
            reserved_chrome: false,
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
        let new_ids: Vec<u32> = layout.iter().map(|r| r.pane).collect();
        // Drop emulators for panes that left the layout.
        self.emulators.retain(|id, _| new_ids.contains(id));
        // Create / re-fit the rest to the pane's PTY grid.
        for rect in &layout {
            let (cols, rows) = self.emulator_size(rect);
            let emulator = self
                .emulators
                .entry(rect.pane)
                .or_insert_with(|| PaneEmulator::new(rect.pane, cols, rows));
            if emulator.terminal().size() != (usize::from(cols), usize::from(rows)) {
                emulator.resize(cols, rows);
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
        // The overlay paints at buffer rows y0..y0+height+1, and the
        // frame flushes buffer row b at HOST row b+1 (the strip-row
        // rebase) — so panel row r sits at host y0+2+r. The old mapping
        // dropped that rebase and read every click one row low (the
        // manual-pass round-8 report: menu actions fired the neighbor).
        if y < y0 + 2 || y > y0 + height + 1 {
            return None;
        }
        Some(y - y0 - 2)
    }

    /// Set (or clear) the modal overlay's title, rows, and scroll state.
    /// The overlay paints as the themed modal over the frame; clearing it
    /// lets the next frame's pane repaint restore the covered cells.
    pub(crate) fn set_overlay(&mut self, overlay: Option<Overlay>) {
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
            self.refit_emulators();
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
    pub(super) fn display_rect(&self, rect: &PaneRect) -> PaneRect {
        let gaps_only = PaneChrome {
            gap: self.pane_gaps,
            ..PaneChrome::default()
        };
        let (gap, _, width, height) = gaps_only.interior(rect.width, rect.height);
        let mut r = *rect;
        r.x += gap + self.sidebar_w;
        r.y += gap;
        r.width = width;
        r.height = height;
        r
    }

    /// The per-pane chrome this renderer paints (`pane-borders`,
    /// `pane-gaps`, `scrollbar-gutter`) — what the size report declares
    /// (`refresh-client -I`) so the daemon sizes each pane's PTY to the
    /// same interior [`Self::content_view`] paints.
    pub(crate) fn chrome(&self) -> PaneChrome {
        PaneChrome {
            border: self.pane_borders,
            gap: self.pane_gaps,
            gutter: self.scrollbar_gutter,
        }
    }

    /// The content view of a pane rect: `(inset_x, inset_y, width,
    /// height)`, rect-RELATIVE (the callers add rect.x/rect.y and the
    /// side panel's width themselves) — the gap band inset, then the
    /// interior inside the one-cell border ring when the per-pane-border
    /// option is on (a pane too small to carry a ring keeps its full
    /// rect), narrowed by one more column when the scrollbar gutter is
    /// reserved. The same [`PaneChrome::interior`] the daemon's division
    /// sizes each pane's PTY with, so paint, mouse, cursor, and the PTY
    /// grid agree on one interior.
    pub(super) fn content_view(&self, rect: &PaneRect) -> (u16, u16, u16, u16) {
        self.chrome().interior(rect.width, rect.height)
    }

    /// The daemon reserves the declared chrome (it advertised the
    /// `refresh-client` `chrome` feature): each pane's emulator then
    /// mirrors the PTY grid — the rect less the chrome — instead of the
    /// full rect. Re-fits the existing emulators.
    pub(crate) fn set_reserved_chrome(&mut self, on: bool) {
        if self.reserved_chrome != on {
            self.reserved_chrome = on;
            self.refit_emulators();
        }
    }

    /// The grid a pane's emulator mirrors: the daemon's PTY size for the
    /// rect — [`PaneChrome::pty_size`] under a reserving daemon, the full
    /// rect otherwise.
    fn emulator_size(&self, rect: &PaneRect) -> (u16, u16) {
        if self.reserved_chrome {
            self.chrome().pty_size(rect.width, rect.height)
        } else {
            (rect.width, rect.height)
        }
    }

    /// Re-fit every emulator to [`Self::emulator_size`] after a chrome
    /// change; unchanged sizes are left alone.
    fn refit_emulators(&mut self) {
        let sizes: Vec<(u32, (u16, u16))> = self
            .layout
            .iter()
            .map(|r| (r.pane, self.emulator_size(r)))
            .collect();
        for (pane, (cols, rows)) in sizes {
            if let Some(emulator) = self.emulators.get_mut(&pane) {
                if emulator.terminal().size() != (usize::from(cols), usize::from(rows)) {
                    emulator.resize(cols, rows);
                }
            }
        }
        self.dirty = true;
    }

    /// The gap-band mode (config `pane-gaps`): every pane's chrome and
    /// content paint into the rect inset per side, leaving theme-bg gap
    /// bands between panes. Default 0.
    pub(crate) fn set_pane_gaps(&mut self, gaps: u16) {
        if self.pane_gaps != gaps {
            self.pane_gaps = gaps;
            self.refit_emulators();
        }
    }

    /// The scrollbar-gutter mode (config `scrollbar-gutter`): a
    /// right-edge gutter column is reserved in every pane rect; it
    /// renders a position indicator while the pane's client scroll
    /// offset is > 0. Default off.
    pub(crate) fn set_scrollbar_gutter(&mut self, on: bool) {
        if self.scrollbar_gutter != on {
            self.scrollbar_gutter = on;
            self.refit_emulators();
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
    pub(super) fn border_label(&self, pane: u32) -> String {
        if let Some(title) = self.user_titles.get(&pane) {
            return title.trim().to_string();
        }
        self.emulators
            .get(&pane)
            .map(|e| e.terminal().title().trim().to_string())
            .unwrap_or_default()
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
    pub(super) fn paint_pane(&mut self, rect: &PaneRect) {
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
            let gx = rect.x + self.sidebar_w + inset_x + view_w;
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
    pub(super) fn paint_dividers(&mut self) {
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
                        .any(|r| r.pane == *a && x == self.sidebar_w + r.x + r.width - 1)
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
    pub(super) fn paint_overlay(
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

    /// Paint the per-pane border boxes (config `pane-borders`): a full
    /// ring per rect — the focused pane's border in `border-active-color`
    /// (default the bright-cyan accent), the rest in `border-color`
    /// (default dim), herdr's look; labels follow their ring. When `show-label-in-border` is on, each
    /// pane's non-empty user title breaks the top edge, space-padded and
    /// truncated to fit (herdr's exact treatment); label cells are not
    /// drag handles ([`Self::label_cell_at`]).
    pub(super) fn paint_pane_borders(&mut self) {
        // (x, y, label chars, owning pane) per pane with a label.
        let mut labels: Vec<(u16, u16, Vec<char>, u32)> = Vec::new();
        for layout_rect in &self.layout {
            let rect = self.display_rect(layout_rect);
            let style = if Some(rect.pane) == self.focused {
                active_border_style(self.border_active)
            } else {
                plain_border_style(self.border_plain)
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
                active_border_style(self.border_active)
            } else {
                plain_border_style(self.border_plain)
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
    pub(super) fn label_cell_at(&self, x: u16, y: u16) -> bool {
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
