//! The modal overlay painted over the frame: state, anchor and geometry
//! math, hit-tests, and painting.

use super::{Overlay, OverlayAnchor, PaneRenderer};
use crate::mux::attach::HelpRow;
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};

impl PaneRenderer {
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
            // Anchored: the box's left ring at the anchor column, pulled
            // back inside the frame when it would overflow. `Above`
            // (the command menu beside its chip) puts the bottom ring on
            // the anchor row — the box opens upward; `Below` (the tab
            // menu under its tab) puts the top ring on the anchor row —
            // it opens downward, clamped back up when the rows below
            // cannot hold it.
            Some(anchor) => {
                let (ax, ay, below) = match anchor {
                    OverlayAnchor::Above(ax, ay) => (usize::from(ax), usize::from(ay), false),
                    OverlayAnchor::Below(ax, ay) => (usize::from(ax), usize::from(ay), true),
                };
                let w = inner + 2;
                let h = height + 2;
                let x0 = ax.min((self.width as usize).saturating_sub(w));
                let y0 = if below {
                    ay.min((self.height as usize).saturating_sub(h))
                } else {
                    (ay + 1)
                        .saturating_sub(h)
                        .min((self.height as usize).saturating_sub(h))
                };
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

    /// Anchor the NEXT overlay instead of centering it:
    /// [`OverlayAnchor::Above`] hangs the box's bottom ring on the
    /// anchor row, [`OverlayAnchor::Below`] its top ring; a new overlay
    /// without an anchor clears it.
    pub(crate) fn set_overlay_anchor(&mut self, anchor: Option<OverlayAnchor>) {
        if self.overlay_anchor != anchor {
            self.overlay_anchor = anchor;
            self.dirty = true;
        }
    }

    /// The command menu chip's frame cell `(x_end, y)`: the column just
    /// right of the chip and its row — wrap it in
    /// [`OverlayAnchor::Above`] where the command menu anchors.
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

    /// Paint the help overlay as the themed modal (the round-3 restyle):
    /// every cell carries the resolved theme bg — the same rule the frame
    /// fill applies, so no default-style (light) cell ever shows — ringed
    /// by a rounded box-drawing border in the accent color, title
    /// `keybinds` left and `esc close` badge top-right embedded in the
    /// top border, category headers in the accent, and the footer hints
    /// line inside the box. Clamped to the window; paints last in
    /// `render_frame`, covering panes and dividers; dismissal lets the
    /// next frame's pane repaint restore the covered cells.
    /// Reverse the overlay's highlight spans (the settings dialog's tab
    /// and selection bands), clipped to its inner box.
    pub(super) fn paint_overlay_highlights(&mut self) {
        let Some((x0, y0, inner, height)) = self.overlay_geometry() else {
            return;
        };
        for &(row, from, to) in &self.overlay_highlights {
            if row >= height {
                continue;
            }
            for col in from..to.min(inner) {
                let cell = &mut self.buffer[((x0 + 1 + col) as u16, (y0 + 1 + row) as u16)];
                let style = cell.style().add_modifier(RtModifier::REVERSED);
                cell.set_style(style);
            }
        }
    }

    pub(super) fn paint_overlay(
        &mut self,
        title: &str,
        rows: &[HelpRow],
        scroll: Option<(usize, usize, usize)>,
    ) {
        let accent_style = RtStyle::default()
            .fg(self.accent())
            .add_modifier(RtModifier::BOLD);
        // The box: the widest row clamped so the ring fits, centered or
        // anchored — the same geometry every overlay hit-test reads.
        let Some((x0, y0, inner, height)) = self.overlay_geometry() else {
            return;
        };
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
            // A wide (CJK) glyph takes two columns; the next glyph starts
            // past its right half.
            let mut j = 0;
            for ch in row.text.chars() {
                let width = crate::mux::attach::text_input::char_width(ch);
                if width == 0 {
                    continue;
                }
                if j + width > inner {
                    break;
                }
                let cell = &mut self.buffer[((x0 + 1 + j) as u16, (y0 + 1 + i) as u16)];
                j += width;
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
}
