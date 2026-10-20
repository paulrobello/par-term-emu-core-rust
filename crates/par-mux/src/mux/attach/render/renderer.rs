//! The pane renderer's state: layout, focus, emulators, titles, scroll, display
//! options, and the frame orchestrator.

use super::{DividerHit, Glyphs, Overlay, PaneChrome, PaneEmulator, PaneRect, PaneRenderer};
use par_term_emu_core::cursor::CursorStyle;
use par_term_emu_core::terminal::Terminal;
use ratatui::buffer::{Buffer, Cell as RtCell};
use ratatui::layout::Rect as RtRect;
use ratatui::style::{Color as RtColor, Style as RtStyle};
use std::collections::HashMap;

pub(super) use super::{pane_chrome::*, sink::*};
// Reached only by the render tests (`map_color`).
#[cfg(test)]
pub(super) use super::paint::*;

/// Every display option a [`PaneRenderer`] paints with, as one value.
/// The session owns the configured value and [`PaneRenderer::with_options`]
/// applies it whole. Host resizes, sidebar refits, and window re-seeds
/// re-fit the live renderer in place ([`PaneRenderer::resize`]), so no
/// option is re-applied per site.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RenderOptions {
    /// The host background fill (the OSC 11 probe; `None` = terminal
    /// default).
    pub bg: Option<RtColor>,
    /// The divider/border glyph set (config `border-lines`).
    pub glyphs: Glyphs,
    /// Per-pane border boxes (config `pane-borders`).
    pub pane_borders: bool,
    /// The pane title embedded in its top border (config
    /// `show-label-in-border`).
    pub show_label_in_border: bool,
    /// Theme-bg gap bands per pane side (config `pane-gaps`).
    pub pane_gaps: u16,
    /// A reserved right-edge gutter column (config `scrollbar-gutter`).
    pub scrollbar_gutter: bool,
    /// The side panel's width. In the session's stored value this is
    /// the CONFIGURED width (config `sidebar-width`); the session's
    /// effective options zero it while the panel is hidden, and the
    /// renderer paints exactly what it is given.
    pub sidebar_width: u16,
    /// The focused border color (config `border-active-color`; `None`
    /// = the bright-cyan built-in).
    pub border_active: Option<RtColor>,
    /// The unfocused border color (config `border-color`; `None` = the
    /// dim built-in).
    pub border_plain: Option<RtColor>,
}

impl Default for RenderOptions {
    /// The chrome-free baseline a session renders with before its config
    /// applies: unicode dividers, no borders or labels, a 20-column side
    /// panel when shown, built-in colors. NOT the configured defaults:
    /// those live once in `Chords::with_defaults` (herdr glyphs, labels
    /// on) and reach the session through `render_session_inner`.
    fn default() -> Self {
        Self {
            bg: None,
            glyphs: Glyphs::Unicode,
            pane_borders: false,
            show_label_in_border: false,
            pane_gaps: 0,
            scrollbar_gutter: false,
            sidebar_width: 20,
            border_active: None,
            border_plain: None,
        }
    }
}

impl PaneRenderer {
    /// A renderer over a `width` x `height` window with no layout yet,
    /// default display options, and no side panel.
    pub fn new(width: u16, height: u16, glyphs: Glyphs) -> Self {
        Self::with_options(
            width,
            height,
            &RenderOptions {
                glyphs,
                sidebar_width: 0,
                ..RenderOptions::default()
            },
        )
    }

    /// A renderer over a `width` x `height` window with no layout yet,
    /// painting with `options` (its `sidebar_width` taken as the
    /// effective panel width).
    pub(crate) fn with_options(width: u16, height: u16, options: &RenderOptions) -> Self {
        let area = RtRect::new(0, 0, width, height);
        Self {
            emulators: HashMap::new(),
            layout: Vec::new(),
            focused: None,
            width,
            height,
            // A session overrides this with its own (`set_geometry`).
            geometry: super::geometry::FrameGeometry::standalone(
                width,
                height,
                options.sidebar_width,
            ),
            sidebar_sections: None,
            sidebar_split: crate::mux::attach::panels::SIDEBAR_SPLIT_DEFAULT,
            sidebar_collapsed: false,
            user_titles: HashMap::new(),
            glyphs: options.glyphs,
            bg: options.bg,
            drag_divider: None,
            overlay: None,
            overlay_anchor: None,
            toast: None,
            popup: None,
            selection: None,
            overlay_highlights: Vec::new(),
            pane_borders: options.pane_borders,
            show_label_in_border: options.show_label_in_border,
            pane_gaps: options.pane_gaps,
            scrollbar_gutter: options.scrollbar_gutter,
            reserved_chrome: false,
            buffer: Buffer::empty(area),
            prev_buffer: Buffer::empty(area),
            border_active: options.border_active,
            border_plain: options.border_plain,
            palette: None,
            cell_px: (10, 20),
            dirty: true,
        }
    }

    /// The display options this renderer currently paints with (the
    /// sidebar width is the effective one).
    #[cfg(test)]
    pub(crate) fn options(&self) -> RenderOptions {
        RenderOptions {
            bg: self.bg,
            glyphs: self.glyphs,
            pane_borders: self.pane_borders,
            show_label_in_border: self.show_label_in_border,
            pane_gaps: self.pane_gaps,
            scrollbar_gutter: self.scrollbar_gutter,
            sidebar_width: self.geometry.sidebar_w,
            border_active: self.border_active,
            border_plain: self.border_plain,
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

    /// Set the color theme's palette (`None` = the built-in look). A
    /// change dirties the frame.
    pub(crate) fn set_palette(&mut self, palette: Option<crate::mux::theme::Palette>) {
        if self.palette != palette {
            self.palette = palette;
            self.dirty = true;
        }
    }

    /// The theme palette in force, if any.
    pub(crate) fn palette(&self) -> Option<crate::mux::theme::Palette> {
        self.palette
    }

    /// The focused and unfocused border colors in force: the explicit
    /// config colors, else the theme's accent / overlay0, else `None`
    /// (the built-in bright-cyan accent and DIM).
    pub(super) fn border_colors(&self) -> (Option<RtColor>, Option<RtColor>) {
        let theme = |pick: fn(&crate::mux::theme::Palette) -> crate::mux::theme::Rgb| {
            self.palette
                .as_ref()
                .map(|p| crate::mux::theme::rt(pick(p)))
        };
        (
            self.border_active.or_else(|| theme(|p| p.accent)),
            self.border_plain.or_else(|| theme(|p| p.overlay0)),
        )
    }

    /// The overlay/indicator accent: the theme's, else bright cyan.
    pub(super) fn accent(&self) -> RtColor {
        self.palette
            .map_or(RtColor::Indexed(14), |p| crate::mux::theme::rt(p.accent))
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

    /// Re-size the frame to `cols` x `rows` in place (ENH-044): the frame
    /// buffers rebuild at the new extent, while the pane emulators, the
    /// layout, focus, titles, the side panel's sections, any open overlay,
    /// and every display option survive. Emulators are NOT re-fit here:
    /// their size follows the daemon's layout, which the caller installs
    /// next with [`Self::apply_layout`]. An unchanged size is a no-op.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if (cols, rows) == (self.width, self.height) {
            return;
        }
        let area = RtRect::new(0, 0, cols, rows);
        self.width = cols;
        self.height = rows;
        self.buffer = Buffer::empty(area);
        self.prev_buffer = Buffer::empty(area);
        // A divider mark is geometry from the old frame.
        self.drag_divider = None;
        self.dirty = true;
    }

    /// Forget the last painted frame so the next frame's diff emits every
    /// cell: the host screen no longer holds that frame (a `repaint_all`
    /// cleared it, or the view switched windows).
    /// `render_frame` swaps the buffers first, so the baseline the next
    /// diff reads is the current `buffer`; both are cleared.
    pub fn invalidate_frame(&mut self) {
        let area = RtRect::new(0, 0, self.width, self.height);
        self.buffer = Buffer::empty(area);
        self.prev_buffer = Buffer::empty(area);
        self.dirty = true;
    }

    /// The window extent the daemon divides: the renderer's full width
    /// less the side panel's strip (the panel is a client-only overlay —
    /// the daemon never learns of it), full height.
    pub fn window_size(&self) -> (u16, u16) {
        (
            self.width.saturating_sub(self.geometry.sidebar_w),
            self.height,
        )
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
    /// dropped for gone ones, and re-fit to their rect's geometry. The
    /// re-fit reflows the emulator exactly as the daemon's pane terminal
    /// reflowed, so a surviving pane needs no replay; a pane new to the
    /// renderer starts empty and the caller replays it. A focus on a pane
    /// that left the layout moves to the first pane.
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
        // Drop emulators and titles for panes that left the layout: a
        // title held across a window switch could not see a rename made
        // while the pane was off screen.
        self.emulators.retain(|id, _| new_ids.contains(id));
        self.user_titles.retain(|id, _| new_ids.contains(id));
        // Create / re-fit the rest to the pane's PTY grid.
        for rect in &layout {
            let (cols, rows) = self.emulator_size(rect);
            let emulator = self
                .emulators
                .entry(rect.pane)
                .or_insert_with(|| PaneEmulator::with_cell_px(cols, rows, self.cell_px));
            if emulator.terminal().size() != (usize::from(cols), usize::from(rows)) {
                emulator.resize(cols, rows);
            }
        }
        if !self.focused.is_some_and(|f| new_ids.contains(&f)) {
            self.focused = Some(layout[0].pane);
        }
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
            .map(|r| crate::mux::attach::text_input::display_width(&r.text))
            .max()
            .unwrap_or(0)
            .min(self.width.saturating_sub(2) as usize)
            .max(1);
        let height = rows
            .len()
            .min(self.height.saturating_sub(2) as usize)
            .max(1);
        let (x0, y0) = match self.overlay_anchor {
            // Anchored (the command menu beside its chip): the box's left
            // ring at the anchor column, its bottom ring on the anchor
            // row — pulled back inside the frame when it would overflow.
            Some((ax, ay)) => {
                let w = inner + 2;
                let h = height + 2;
                let x0 = usize::from(ax).min((self.width as usize).saturating_sub(w));
                let y0 = (usize::from(ay) + 1)
                    .saturating_sub(h)
                    .min((self.height as usize).saturating_sub(h));
                (x0, y0)
            }
            None => (
                (self.width as usize).saturating_sub(inner + 2) / 2,
                (self.height as usize).saturating_sub(height + 2) / 2,
            ),
        };
        Some((x0, y0, inner, height))
    }

    /// Whether HOST cell `(x, y)` lies on the overlay's box, ring
    /// included.
    pub(crate) fn overlay_contains(&self, x: u16, y: u16) -> bool {
        let Some((x0, y0, inner, height)) = self.overlay_geometry() else {
            return false;
        };
        let Some((_, frame_y)) = self.geometry.host_to_frame(0, y) else {
            return false;
        };
        let (x, frame_y) = (usize::from(x), usize::from(frame_y));
        x >= x0 && x <= x0 + inner + 1 && frame_y >= y0 && frame_y <= y0 + height + 1
    }

    /// Anchor the NEXT overlay at frame cell `(x, y)` (its left ring at
    /// `x`, bottom ring on row `y`) instead of centering it; a new
    /// overlay without an anchor clears it.
    pub(crate) fn set_overlay_anchor(&mut self, anchor: Option<(u16, u16)>) {
        if self.overlay_anchor != anchor {
            self.overlay_anchor = anchor;
            self.dirty = true;
        }
    }

    /// The command menu chip's frame cell `(x_end, y)`: the column just
    /// right of the chip and its row — where the command menu anchors.
    pub(crate) fn sidebar_menu_chip_anchor(&self) -> Option<(u16, u16)> {
        self.sidebar_lines()
            .into_iter()
            .find(|line| line.id == Some(crate::mux::attach::panels::SidebarRowId::Menu))
            .map(|line| (line.x_end, line.y))
    }

    /// The overlay's composed panel row (index into the panel's rows:
    /// 0 is the filter line, the last the footer) at window-relative
    /// `(x, y)`, or `None` when no overlay is up or the point is off the
    /// box.
    pub(crate) fn overlay_row_at(&self, x: u16, y: u16) -> Option<usize> {
        let (x0, y0, inner, height) = self.overlay_geometry()?;
        let x = usize::from(x);
        if x < x0 + 1 || x > x0 + inner {
            return None;
        }
        // The overlay paints at buffer rows y0..y0+height+1 (its top
        // border at y0, panel row r at y0+1+r); the host row maps into
        // the buffer through the geometry's strip rebase. The old mapping
        // dropped that rebase and read every click one row low (the
        // manual-pass round-8 report: menu actions fired the neighbor).
        let (_, frame_y) = self.geometry.host_to_frame(0, y)?;
        let frame_y = usize::from(frame_y);
        if frame_y < y0 + 1 || frame_y > y0 + height {
            return None;
        }
        Some(frame_y - y0 - 1)
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
            if overlay.is_none() {
                self.overlay_anchor = None;
            }
            self.overlay = overlay;
            self.overlay_highlights.clear();
            self.dirty = true;
        }
    }

    /// Highlight spans `(row, from, to)` in the overlay's inner columns,
    /// painted reversed (the settings dialog's tab and selection). A new
    /// overlay clears them.
    pub(crate) fn set_overlay_highlights(&mut self, highlights: Vec<(usize, usize, usize)>) {
        if self.overlay_highlights != highlights {
            self.overlay_highlights = highlights;
            self.dirty = true;
        }
    }

    /// The overlay's inner column at window-relative `x`, while one is up
    /// and `x` is inside its box.
    pub(crate) fn overlay_col_at(&self, x: u16) -> Option<usize> {
        let (x0, _, inner, _) = self.overlay_geometry()?;
        let x = usize::from(x);
        (x > x0 && x <= x0 + inner).then(|| x - x0 - 1)
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
        self.focused_cell(true)
    }

    /// Where the host cursor PARKS while the focused pane hides its
    /// cursor (DECTCEM off): the same tracked cell as
    /// [`Self::focused_cursor`], minus the visibility test. The host
    /// terminal anchors IME preedit and the candidate window at its
    /// cursor even when hidden, so a TUI that paints its own cursor
    /// (agent CLIs) keeps the composition at the input line instead of
    /// at whatever cell the last diff wrote. `None` for every other
    /// hide reason (scrolled view, nothing focused, cell outside view).
    pub fn focused_ime_anchor(&self) -> Option<(u16, u16)> {
        self.focused_cell(false).map(|(x, y, _)| (x, y))
    }

    fn focused_cell(&self, require_visible: bool) -> Option<(u16, u16, CursorStyle)> {
        let Some(focus) = self.focused else {
            return super::trace_none(format_args!("no focused pane"));
        };
        let Some(rect) = self.layout.iter().find(|r| r.pane == focus) else {
            return super::trace_none(format_args!("focused pane has no layout rect"));
        };
        let Some(emulator) = self.emulators.get(&focus) else {
            return super::trace_none(format_args!("no emulator for focused pane"));
        };
        if emulator.scroll_offset() > 0 {
            return super::trace_none(format_args!(
                "view scrolled: offset {}",
                emulator.scroll_offset()
            ));
        }
        let cursor = emulator.terminal().cursor();
        if require_visible && !cursor.visible {
            return super::trace_none(format_args!("pane tracked cursor hidden by DECTCEM"));
        }
        let (col, row) = (cursor.col, cursor.row);
        // The placement path shares the paint inset: with the per-pane
        // border option the cursor maps inside the border ring, hidden
        // when the tracked cell falls in the cropped perimeter band.
        let (inset_x, inset_y, view_w, view_h) = self.content_view(rect);
        if col >= usize::from(view_w) || row >= usize::from(view_h) {
            return super::trace_none(format_args!(
                "tracked cell ({col},{row}) outside content view {view_w}x{view_h}"
            ));
        }
        let (x, y) = self
            .geometry
            .content_to_frame(rect.x + inset_x + col as u16, rect.y + inset_y + row as u16);
        Some((x, y, cursor.style))
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
        let (x, y) = self.geometry.content_to_frame(rect.x + gap, rect.y + gap);
        PaneRect {
            x,
            y,
            width,
            height,
            ..*rect
        }
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
        let osc = self.osc_title(pane);
        match self.user_titles.get_mut(&pane) {
            Some((known, seen_osc)) => {
                *seen_osc = osc;
                if known != title {
                    *known = title.to_string();
                    self.dirty = true;
                }
            }
            None => {
                self.user_titles.insert(pane, (title.to_string(), osc));
                self.dirty = true;
            }
        }
    }

    /// Whether a pane's recorded title needs a `pane-title` re-query:
    /// never queried, invalidated by a clearing `%pane-title-changed`, or
    /// the pane set a new OSC title since (the effective title may follow
    /// it).
    pub(crate) fn title_stale(&self, pane: u32) -> bool {
        match self.user_titles.get(&pane) {
            Some((_, seen_osc)) => *seen_osc != self.osc_title(pane),
            None => true,
        }
    }

    /// Forget a pane's recorded title (a `%pane-title-changed` cleared the
    /// user title, so the effective title falls back to an OSC title the
    /// payload lacks), so the next status refresh re-queries it.
    pub(crate) fn invalidate_title(&mut self, pane: u32) {
        self.user_titles.remove(&pane);
    }

    /// The pane emulator's own OSC 0/2 title.
    fn osc_title(&self, pane: u32) -> String {
        self.emulators
            .get(&pane)
            .map(|e| e.terminal().title().to_string())
            .unwrap_or_default()
    }

    /// The pane's border label: the daemon's effective title (user label
    /// first), or the emulator's own OSC title when never queried.
    pub(super) fn border_label(&self, pane: u32) -> String {
        let raw = match self.user_titles.get(&pane) {
            Some((title, _)) => title.clone(),
            None => self
                .emulators
                .get(&pane)
                .map(|e| e.terminal().title().to_string())
                .unwrap_or_default(),
        };
        crate::mux::attach::chrome_safe(&raw).trim().to_string()
    }

    /// Give `pane` a fresh emulator at its current size — a respawned
    /// process starts on a clean grid, and the dead one's frozen screen
    /// must not mix with the replay. A pane the layout does not hold is
    /// ignored.
    pub fn reset_pane(&mut self, pane: u32) {
        let Some(rect) = self.layout.iter().find(|r| r.pane == pane).copied() else {
            return;
        };
        let (cols, rows) = self.emulator_size(&rect);
        self.emulators
            .insert(pane, PaneEmulator::with_cell_px(cols, rows, self.cell_px));
        self.dirty = true;
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

    /// Fill the content area past the window's extent with dim `.` cells
    /// (tmux's padding): the daemon sizes a window to its smallest viewer,
    /// so a larger client shows the window in its top-left corner and the
    /// rest marks where the other client's screen ends.
    fn paint_window_padding(&mut self) {
        let window_w = self.layout.iter().map(|r| r.x + r.width).max().unwrap_or(0);
        let window_h = self
            .layout
            .iter()
            .map(|r| r.y + r.height)
            .max()
            .unwrap_or(0);
        let content = self.geometry.content_rect();
        if self.layout.is_empty() || (window_w >= content.width && window_h >= content.height) {
            return;
        }
        let mut style = RtStyle::default().add_modifier(ratatui::style::Modifier::DIM);
        if let Some(bg) = self.bg {
            style = style.bg(bg);
        }
        for row in 0..content.height {
            for col in 0..content.width {
                if col < window_w && row < window_h {
                    continue;
                }
                let (x, y) = self.geometry.content_to_frame(col, row);
                if x >= self.width || y >= self.height {
                    continue;
                }
                let cell = &mut self.buffer[(x, y)];
                cell.reset();
                cell.set_symbol(".");
                cell.set_style(style);
            }
        }
    }

    /// The pane rect containing window-relative `(x, y)`, else `None`
    /// (dividers belong to no pane).
    pub fn pane_at(&self, x: u16, y: u16) -> Option<&PaneRect> {
        self.layout
            .iter()
            .find(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
    }

    /// The frame's row count (the pane renderer's grid height — the
    /// sidebar composes against it).
    pub(crate) fn frame_height(&self) -> u16 {
        self.height
    }

    /// A pane's tracked input-mode state (its emulator's terminal), by
    /// pane id — what the input router reads to re-encode keys and decide
    /// mouse ownership. Unknown pane id → `None`.
    pub fn pane_terminal(&self, pane: u32) -> Option<&Terminal> {
        self.emulators.get(&pane).map(|e| e.terminal())
    }

    /// Reset a pane's client-side scroll offset to live (0). Typing
    /// forwards, so the view snaps back to the bottom — and the scroll
    /// hold must drop with it, or the pane's next output tick would push
    /// the freshly-live view back up into history.
    pub fn snap_to_live(&mut self, pane: u32) {
        if let Some(emulator) = self.emulators.get_mut(&pane) {
            if emulator.scroll_offset() != 0 {
                emulator.scroll_to_live();
                emulator.set_scroll_hold(false);
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

    /// Set (or clear) the line selection. Repaints on change.
    pub(crate) fn set_selection(&mut self, sel: Option<(u32, usize, usize)>) {
        if self.selection != sel {
            self.selection = sel;
            self.dirty = true;
        }
    }

    /// `pane`'s viewport-top absolute line: scrollback lines first, then
    /// live rows (the selection's coordinate).
    pub(crate) fn viewport_top_line(&self, pane: u32) -> Option<usize> {
        let emulator = self.emulators.get(&pane)?;
        let len = emulator.terminal().active_grid().scrollback_len();
        Some(len.saturating_sub(emulator.scroll_offset()))
    }

    /// The selected lines' text — one trimmed line per row, newline
    /// joined (the OSC 52 copy payload).
    pub(crate) fn selection_text(&self) -> Option<String> {
        let (pane, a, b) = self.selection?;
        let emulator = self.emulators.get(&pane)?;
        let grid = emulator.terminal().active_grid();
        let len = grid.scrollback_len();
        let mut text = String::new();
        for line in a..=b {
            let line_text = if line < len {
                grid.scrollback_line(line).map(|cells| {
                    let row: String = cells.iter().map(|cell| cell.c()).collect();
                    row.trim_end().to_string()
                })
            } else {
                Some(grid.row_text(line - len).trim_end().to_string())
            };
            text.push_str(&line_text.unwrap_or_default());
            text.push('\n');
        }
        Some(text)
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
    ///
    /// A view scrolled off live ARMS the scroll hold: pane output keeps
    /// pushing lines into history under a pinned view instead of snapping
    /// it back to the bottom (without this, a pane that keeps writing —
    /// omp's streaming chat, a log tail — swallows the scroll within one
    /// output tick, and the wheel looks dead). Scrolling back to the
    /// bottom (offset 0) releases the hold and the view follows live
    /// again; typing snaps it back as before.
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
        emulator.set_scroll_hold(after > 0);
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

        // The last painted frame becomes this frame's diff baseline by a
        // swap, not a clone (ARC-132): both buffers share the renderer's
        // area (`resize` replaces both at once), and the reset below
        // clears the recycled buffer before painting. After the frame,
        // `self.buffer` holds it for tests and callers to read.
        std::mem::swap(&mut self.buffer, &mut self.prev_buffer);

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
        self.paint_window_padding();
        self.paint_sidebar();
        self.paint_popup();
        self.paint_toast();
        if let Some((title, rows, scroll)) = self.overlay.clone() {
            self.paint_overlay(title, &rows, scroll);
            self.paint_overlay_highlights();
        }

        let diff = self
            .prev_buffer
            .diff(&self.buffer)
            .into_iter()
            .map(|(x, y, cell)| (x, y, cell.clone()))
            .collect::<Vec<_>>();
        diff
    }
}
