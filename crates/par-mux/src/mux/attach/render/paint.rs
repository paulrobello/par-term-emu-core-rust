//! Cell painting: a pane's live/scrollback grid into its content view, and the modal overlay.

use super::{PaneRect, PaneRenderer};
use crate::mux::attach::HelpRow;
use par_term_emu_core::cell::CellFlags;
use par_term_emu_core::color::{Color as CoreColor, NamedColor};
use ratatui::buffer::CellDiffOption;
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};

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

impl PaneRenderer {
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
        let max_cols = view_w.min(grid.cols() as u16).min(
            self.width
                .saturating_sub(rect.x + self.geometry.sidebar_w + inset_x),
        );
        for row in 0..max_rows {
            // View row r: live grid row r - S when r >= S; otherwise the
            // scrollback line S_len - S + r (newest history first).
            let scrollback_row: isize = scrollback_len - scroll as isize + row as isize;
            let in_history = (row as usize) < scroll;
            for col in 0..max_cols {
                let core_cell: &par_term_emu_core::cell::Cell = if in_history {
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
                    rect.x + self.geometry.sidebar_w + inset_x + col,
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
        // depth into the history. Without a reserved gutter nothing
        // paints: view_w is not narrowed, so the column right of the
        // content sits outside the rect — a neighbor's first content
        // column, blanked full-height — and the cue must not occlude
        // content the pane does not own.
        if self.scrollbar_gutter && view_h > 0 {
            let gx = rect.x + self.geometry.sidebar_w + inset_x + view_w;
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
            .fg(RtColor::Indexed(14))
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
}
