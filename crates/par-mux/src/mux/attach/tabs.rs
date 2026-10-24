//! The render-mode tab strip: the queried window list, its one-line
//! composition, the ratatui row painter, and the click hit-testing.
//!
//! [`TabStrip`] is the top row's counterpart to the bottom
//! [`super::status::StatusRow`]: it paints one tab per WINDOW of the
//! shown session — the bare NAME, in window order, drawn as herdr's tab
//! bar: each tab a solid block with two columns of padding either side
//! of its name, blocks one bar-background column apart, the active block
//! bright blue with dark bold text, the inactive blocks a lighter navy
//! with muted text, all on a dark navy bar (the `BAR_BG` palette) — into
//! a one-row buffer and diffs it, so a window churn flushes only the
//! changed cells. A ` + ` button sits immediately after the last
//! visible tab (or right `…` marker) — one blank column before its
//! glyph — inside a three-column reservation capping the row's tail
//! whenever the strip can hold tabs at all; a click on it opens the
//! new-tab prompt. The window list comes from the same queried state the
//! status bar uses ([`super::status::StatusState::windows`]), so the
//! strip refreshes with the status re-query over the `%sessions-changed`
//! family: a window add, close, or rename repaints the strip.
//!
//! Truncation (the par-term tab-bar rule: the active tab is always
//! visible) runs in three stages — full names, then a shared name
//! budget, then a contiguous visible run containing the active tab with
//! `…` edge markers. See [`layout_tabs`].
//!
//! [`TabStrip::hit_test`] maps a strip column back to the window index
//! the click lands on — the same layout function paint uses, so what a
//! click highlights is exactly what painted there.

use super::text_input::{char_width, display_width, truncate_to_width};
use ratatui::buffer::{Buffer, Cell as RtCell};
use ratatui::layout::Rect as RtRect;
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};

/// The theme accent: the same indexed color the pane borders and the
/// help modal are themed with.
const ACCENT: RtColor = RtColor::Indexed(14);

/// herdr's tab-bar palette (truecolor), measured from the reference
/// rendering: the bar background every strip column carries.
const BAR_BG: RtColor = RtColor::Rgb(23, 23, 33);
/// An inactive tab block's background.
const INACTIVE_BG: RtColor = RtColor::Rgb(43, 44, 60);
/// Muted text: inactive tab names, the edge markers, and the ` + `.
const MUTED_FG: RtColor = RtColor::Rgb(97, 101, 123);
/// The active tab block's background.
const ACTIVE_BG: RtColor = RtColor::Rgb(126, 171, 249);
/// The active tab's (bold) text: the bar's own dark navy.
const ACTIVE_FG: RtColor = BAR_BG;

/// Columns of block padding either side of a tab's name.
const PAD: usize = 2;
/// Bar-background columns between adjacent blocks (and between an edge
/// marker and its neighboring block).
const GAP: usize = 1;

/// The ellipsis spelling for truncated names and hidden edges.
const ELLIPSIS: char = '\u{2026}';

/// The invalidated baseline's symbol: a single-width private-use glyph
/// no paint writes, so every painted cell diffs against it. (A
/// zero-width control trips ratatui's cell-width assertion.)
const INVALIDATED: &str = "\u{E000}";

/// One visible tab's geometry and text: the column span a click on
/// that tab owns, plus the exact text the painter writes inside its
/// two-column pads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TabCell {
    /// Leftmost column of the cell (pad included).
    pub col: u16,
    /// Cell width in columns (text plus both two-column pads).
    pub width: u16,
    /// The tab's index into the queried window list.
    pub window_index: usize,
    /// The exact text painted inside the pads.
    pub text: String,
}

/// The composed strip layout: the visible tabs' cells plus the columns
/// of the leading/trailing hidden-edge markers, when any tabs are
/// hidden.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct TabLayout {
    /// The visible tabs, left to right.
    pub cells: Vec<TabCell>,
    /// The column of a leading `…` (tabs hidden on the left).
    pub left_marker: Option<u16>,
    /// The column of a trailing `…` (tabs hidden on the right).
    pub right_marker: Option<u16>,
    /// The column the ` + ` button starts on: the blank column right
    /// after the last visible tab or right `…` marker (the glyph paints
    /// at `plus + 1`), when the strip can host the three-column button.
    pub plus: Option<u16>,
    /// The queried window count (the wheel's clamp bound — the visible
    /// run hides how many tabs exist).
    pub total: usize,
}

/// The tab strip's painter: a one-row buffer, its previous frame, and
/// the layout the last paint produced (the hit-test's source of truth).
pub(crate) struct TabStrip {
    cols: u16,
    buffer: Buffer,
    prev: Buffer,
    layout: TabLayout,
    /// The overflowing strip's visible-run seed (the wheel's offset);
    /// `None` keeps the active-centered auto run, `Some(i)` a run
    /// seeded at window `i` — wheeling from the auto run moves to a
    /// real offset in BOTH directions (a plain `usize` could not: 0
    /// meant both "auto" and "leftmost").
    scroll: Option<usize>,
    /// The color theme (config `theme`); `None` = the measured herdr
    /// constants above.
    pub(crate) palette: Option<crate::mux::theme::Palette>,
}

/// The strip's colors: `(bar, inactive block, muted text, active block,
/// active text, accent)` — the theme's panel_bg / surface0 / overlay0 /
/// accent / panel_bg / accent, or the built-in constants.
fn strip_colors(
    palette: Option<crate::mux::theme::Palette>,
) -> (RtColor, RtColor, RtColor, RtColor, RtColor, RtColor) {
    use crate::mux::theme::rt;
    match palette {
        Some(p) => (
            rt(p.panel_bg),
            rt(p.surface0),
            rt(p.overlay0),
            rt(p.accent),
            rt(p.panel_bg),
            rt(p.accent),
        ),
        None => (BAR_BG, INACTIVE_BG, MUTED_FG, ACTIVE_BG, ACTIVE_FG, ACCENT),
    }
}

/// The display text of one tab at a name budget: the bare NAME (the
/// `N:` ordinal prefix is gone — round 8). A name longer than the
/// budget is truncated to it and ellipsized.
fn tab_text(_id: &str, name: &str, budget: Option<usize>) -> String {
    // The reference rendering: the bare NAME (the `N:` ordinal prefix is
    // gone — round 8), truncating to the name budget when over.
    let _ = _id;
    let name = super::chrome_safe(name);
    match budget {
        None => name.to_string(),
        Some(budget) => {
            if display_width(&name) > budget {
                let (head, _) = truncate_to_width(&name, budget);
                format!("{head}{ELLIPSIS}")
            } else {
                name.to_string()
            }
        }
    }
}

/// The columns `widths` occupy laid out with a `GAP` between neighbors.
fn span(widths: &[usize]) -> usize {
    widths.iter().sum::<usize>() + GAP * widths.len().saturating_sub(1)
}

/// Lay the tabs out left to right from `start` with the given texts and
/// widths, one `GAP` apart, when they fit `budget`. `None` on overflow.
fn place(texts: &[String], widths: &[usize], start: u16, budget: usize) -> Option<Vec<TabCell>> {
    if span(widths) > budget {
        return None;
    }
    let mut col = start;
    let cells: Vec<TabCell> = texts
        .iter()
        .zip(widths)
        .enumerate()
        .map(|(index, (text, width))| {
            let cell = TabCell {
                col,
                width: *width as u16,
                window_index: index,
                text: text.clone(),
            };
            col += (*width + GAP) as u16;
            cell
        })
        .collect();
    Some(cells)
}

/// Compose the strip layout for `windows` (the queried `(id, name)`
/// list, window order; `active` is the active window id) into `cols`
/// columns. `lead` is the columns the strip's LEFT segment occupies —
/// the side panel's title when the panel is up, 0 when it is down: the
/// tabs lay out from it onward. `scroll` is the wheel offset: `None`
/// keeps the auto run (the active-centered stage-3 run), `Some(i)`
/// seeds the run at window `i`.
///
/// The ` + ` button reserves the row's last three columns: the tab
/// stages lay out between the lead segment and that reservation, and
/// the button starts on the column right after the last visible tab
/// (or right `…` marker) — its own leading pad is the one blank column
/// between them — so it never runs past the line at any width.
///
/// Each tab is a block `PAD` columns wider than its name on both sides,
/// blocks `GAP` columns apart. Stage 1: full names, left to right, when
/// they all fit. Stage 2: the widest shared name budget that fits —
/// names at or under the budget render whole, longer ones truncate and
/// ellipsize. Stage 3: budget 1 still overflows, so only a contiguous
/// run of tabs is visible, with a `…` marker at each edge where tabs
/// stay hidden (each marker budgets its column plus a `GAP` up front);
/// the run contains the active tab in the auto run, or the
/// scrolled-to tab when `scroll` is set.
pub(crate) fn layout_tabs(
    windows: &[(String, String)],
    active: Option<&str>,
    cols: usize,
    lead: u16,
    scroll: Option<usize>,
) -> TabLayout {
    if windows.is_empty() {
        return TabLayout::default();
    }
    let active_index = windows
        .iter()
        .position(|(id, _)| Some(id.as_str()) == active)
        .unwrap_or(0);
    let lead = (lead as usize).min(cols);

    // The button's three columns cap the tab area at the row's tail; a
    // strip narrower than the button plus one tab column cannot host
    // the reservation (the narrow-strip rule).
    let hostable = cols >= 4 && lead + 3 < cols;
    let tab_cols = if hostable {
        cols - 3 - lead
    } else {
        cols.saturating_sub(lead)
    };
    let start = lead as u16;

    let texts_widths = |budget: Option<usize>| -> (Vec<String>, Vec<usize>) {
        let texts: Vec<String> = windows
            .iter()
            .map(|(id, name)| tab_text(id, name, budget))
            .collect();
        let widths: Vec<usize> = texts
            .iter()
            .map(|text| display_width(text) + 2 * PAD)
            .collect();
        (texts, widths)
    };

    // Stage 1.
    let (texts, widths) = texts_widths(None);
    if let Some(cells) = place(&texts, &widths, start, tab_cols) {
        return TabLayout {
            plus: plus_after(&cells, None, start, cols, hostable),
            cells,
            left_marker: None,
            right_marker: None,
            total: windows.len(),
        };
    }

    // Stage 2.
    let max_name = windows
        .iter()
        .map(|(_, name)| display_width(&super::chrome_safe(name)))
        .max()
        .unwrap_or(1)
        .max(1);
    for budget in (1..=max_name).rev() {
        let (texts, widths) = texts_widths(Some(budget));
        if let Some(cells) = place(&texts, &widths, start, tab_cols) {
            return TabLayout {
                plus: plus_after(&cells, None, start, cols, hostable),
                cells,
                left_marker: None,
                right_marker: None,
                total: windows.len(),
            };
        }
    }

    // Stage 3. The run seeds at the wheel offset when set; `None` is
    // the active-centered auto run (an overflowing strip scrolls to
    // reach its hidden tabs — the audit's R6).
    let (texts, widths) = texts_widths(Some(1));
    // `total` is the run's span (blocks plus the gaps between them): a
    // candidate adds its block and one gap, each hidden-edge marker its
    // column and one gap.
    let marker_cols = 1 + GAP;
    let seed = scroll.unwrap_or(active_index).min(windows.len() - 1);
    let mut run: Vec<usize> = vec![seed];
    let mut total = widths[seed];
    // A scrolled run grows RIGHTWARD first (the user wheeled toward the
    // later windows — filling left would crawl back over what they
    // scrolled past); the auto run keeps its left-first growth.
    let left_first = scroll.is_none();
    loop {
        let first = run[0];
        let last = *run.last().expect("non-empty by construction");
        let mut added = false;
        let left_first_edge = usize::from(first > 0);
        let right_first_edge = usize::from(last < windows.len() - 1);
        let try_left = |run: &mut Vec<usize>, total: &mut usize, candidate: usize| {
            let markers_after = left_first_edge + right_first_edge;
            if *total + widths[candidate] + GAP + markers_after * marker_cols <= tab_cols {
                *total += widths[candidate] + GAP;
                run.insert(0, candidate);
                true
            } else {
                false
            }
        };
        let try_right = |run: &mut Vec<usize>, total: &mut usize, candidate: usize| {
            let markers_after = left_first_edge + right_first_edge;
            if *total + widths[candidate] + GAP + markers_after * marker_cols <= tab_cols {
                *total += widths[candidate] + GAP;
                run.push(candidate);
                true
            } else {
                false
            }
        };
        if left_first {
            if first > 0 {
                added = try_left(&mut run, &mut total, first - 1);
            }
            if !added && last + 1 < windows.len() {
                added = try_right(&mut run, &mut total, last + 1);
            }
        } else {
            if last + 1 < windows.len() {
                added = try_right(&mut run, &mut total, last + 1);
            }
            if !added && first > 0 {
                added = try_left(&mut run, &mut total, first - 1);
            }
        }
        if !added {
            break;
        }
    }
    // The run places left-aligned after the lead segment and the
    // (optional) leading marker; left alignment keeps the hit columns
    // stable for a given window list.
    let mut col = start + if run[0] > 0 { marker_cols as u16 } else { 0 };
    let cells: Vec<TabCell> = run
        .iter()
        .map(|&index| {
            let cell = TabCell {
                col,
                width: widths[index] as u16,
                window_index: index,
                text: texts[index].clone(),
            };
            col += (widths[index] + GAP) as u16;
            cell
        })
        .collect();
    let left_hidden = run[0] > 0;
    let right_hidden = *run.last().expect("non-empty") < windows.len() - 1;
    let right_marker = right_hidden.then(|| {
        cells
            .last()
            .map(|cell| cell.col + cell.width + GAP as u16)
            .unwrap_or_default()
    });
    // The ` + ` button's three reserved columns own the row's tail: when
    // `plus_after` clamps the button to `cols - 3`, a right `…` at or
    // past that column falls inside the button's span — the paint writes
    // the ` + ` over the marker and a click on the marker column hits
    // the button. Degenerate widths reach that clamp, so the button wins
    // the tail: such a marker is suppressed (the left marker still
    // signals hidden tabs).
    let right_marker =
        right_marker.filter(|&marker| !(hostable && marker >= (cols.saturating_sub(3)) as u16));
    let plus = plus_after(&cells, right_marker, start, cols, hostable);
    TabLayout {
        cells,
        left_marker: left_hidden.then_some(start),
        right_marker,
        plus,
        total: windows.len(),
    }
}

/// The ` + ` button's start column: the blank column right after the
/// last painted thing — the run's last block, or the right `…` marker —
/// clamped to the reserved tail (blocks never reach it, so the clamp
/// only binds defensively). `None` when the strip cannot host the
/// three-column button.
fn plus_after(
    cells: &[TabCell],
    right_marker: Option<u16>,
    start: u16,
    cols: usize,
    hostable: bool,
) -> Option<u16> {
    if !hostable {
        return None;
    }
    let base = cells
        .last()
        .map(|cell| cell.col + cell.width)
        .into_iter()
        .chain(right_marker.map(|marker| marker + 1))
        .max()
        .unwrap_or(start);
    Some(base.min((cols - 3) as u16))
}

impl TabStrip {
    /// A strip `cols` wide, initially blank (the first diff paints all
    /// of it).
    pub(crate) fn new(cols: u16) -> Self {
        let area = RtRect::new(0, 0, cols, 1);
        Self {
            cols,
            buffer: Buffer::empty(area),
            prev: Buffer::empty(area),
            layout: TabLayout::default(),
            scroll: None,
            palette: None,
        }
    }

    /// Wheel the overflowing strip's visible run by `delta` tabs
    /// (positive = toward the later windows). The offset becomes the
    /// current run's first visible window moved by `delta`, clamped to
    /// the window list, so wheeling from the auto run moves the run in
    /// both directions. A no-op when the strip fits or the offset
    /// cannot move.
    pub(crate) fn wheel(&mut self, delta: isize) {
        let Some(first) = self.layout.cells.first() else {
            return;
        };
        let max = self.layout.total.saturating_sub(1) as isize;
        let next = Some((first.window_index as isize + delta).clamp(0, max.max(0)) as usize);
        if next != self.scroll {
            self.scroll = next;
            self.invalidate();
        }
    }

    /// Forget the scroll offset (the auto run re-centers on the active
    /// tab; a re-seed lands a fresh window list).
    pub(crate) fn reset_scroll(&mut self) {
        self.scroll = None;
    }

    /// Whether the layout hid tabs at either edge (the wheel's domain).
    pub(crate) fn overflowing(&self) -> bool {
        self.layout.left_marker.is_some() || self.layout.right_marker.is_some()
    }

    /// The strip's width.
    pub(crate) fn cols(&self) -> u16 {
        self.cols
    }

    /// Forget the previous frame: the next diff paints EVERY column of
    /// the row, blank cells included. Callers invalidate when the host
    /// row is not known to match the last frame (a reseed on a workspace
    /// switch, a resize, a sidebar toggle) — the host row is NOT cleared
    /// there, so a blank baseline would let stale tabs survive wherever
    /// the new strip is blank. The baseline is filled with a sentinel
    /// symbol no paint ever writes, so every column differs.
    pub(crate) fn invalidate(&mut self) {
        let mut prev = Buffer::empty(RtRect::new(0, 0, self.cols, 1));
        for x in 0..self.cols {
            prev[(x, 0)].set_symbol(INVALIDATED);
        }
        self.prev = prev;
    }

    /// Paint `windows` over the whole row, every column first filled
    /// with the bar background so the previous frame's tail erases.
    /// `lead` is the side
    /// panel's title when the panel is up — `(title, width)`: the title
    /// paints in the first `width` columns (accent — the panel's
    /// header, per the mock) and the tabs lay out from it onward, the
    /// ` + ` following the last visible tab. Every tab is a solid
    /// block (pads included): the active one bright blue with dark bold
    /// text, the inactive ones lighter navy with muted text. The edge
    /// markers and the ` + ` paint muted on the bar.
    pub(crate) fn paint(
        &mut self,
        windows: &[(String, String)],
        active: Option<&str>,
        lead: Option<(&str, u16)>,
    ) {
        let lead_w = lead.map_or(0, |(_, w)| w);
        let layout = layout_tabs(windows, active, self.cols as usize, lead_w, self.scroll);
        self.layout = layout.clone();
        let (bar_bg, inactive_bg, muted_fg, active_bg, active_fg, accent) =
            strip_colors(self.palette);
        let bar = RtStyle::default().bg(bar_bg);
        let muted = bar.fg(muted_fg);
        for x in 0..self.cols {
            let cell = &mut self.buffer[(x, 0)];
            cell.reset();
            cell.set_style(bar);
        }
        // The lead title: the panel's header — accent, clipped to the
        // lead segment. Clicks on it hit nothing (no cells below).
        if let Some((title, width)) = lead {
            let style = bar.fg(accent);
            for (offset, ch) in format!(" {title} ").chars().enumerate() {
                let x = offset as u16;
                if x >= width || x >= self.cols {
                    break;
                }
                let cell = &mut self.buffer[(x, 0)];
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
            }
        }
        // Edge markers, muted on the bar.
        for marker in [layout.left_marker, layout.right_marker]
            .into_iter()
            .flatten()
        {
            if marker >= self.cols {
                continue;
            }
            let cell = &mut self.buffer[(marker, 0)];
            cell.set_symbol(&ELLIPSIS.to_string());
            cell.set_style(muted);
        }
        // The ` + ` button right after the last visible tab, muted on
        // the bar (its leading pad is the blank column before the
        // glyph).
        if let Some(plus) = layout.plus {
            let cell = &mut self.buffer[(plus + 1, 0)];
            cell.set_symbol("+");
            cell.set_style(muted);
        }
        // Tab texts. The layout carries the exact per-cell text, so the
        // painter and the hit columns cannot disagree.
        let active_cell = layout.cells.iter().position(|cell| {
            windows
                .get(cell.window_index)
                .is_some_and(|(id, _)| Some(id.as_str()) == active)
        });
        for (i, cell) in layout.cells.iter().enumerate() {
            let style = if Some(i) == active_cell {
                // herdr's active tab: a solid bright block with dark bold
                // text — not a foreground tint (a tint read as "hovered",
                // not "current"; the round-5 report).
                RtStyle::default()
                    .fg(active_fg)
                    .bg(active_bg)
                    .add_modifier(RtModifier::BOLD)
            } else {
                RtStyle::default().fg(muted_fg).bg(inactive_bg)
            };
            // Every block covers the tab's full width, pads included.
            for x in cell.col..cell.col + cell.width {
                if x < self.cols {
                    self.buffer[(x, 0)].set_style(style);
                }
            }
            // A wide (CJK) glyph spans two columns: the next glyph starts
            // past its right half, which the buffer diff skips.
            let mut x = cell.col + PAD as u16;
            for ch in cell.text.chars() {
                let width = char_width(ch) as u16;
                if width == 0 {
                    continue;
                }
                if x + width > self.cols {
                    break;
                }
                let buffer_cell = &mut self.buffer[(x, 0)];
                buffer_cell.set_symbol(&ch.to_string());
                buffer_cell.set_style(style);
                x += width;
            }
        }
    }

    /// The window index a click at strip column `col` lands on — the
    /// visible tab whose block spans the column (its pads included).
    /// Gap and marker columns hit nothing.
    pub(crate) fn hit_test(&self, col: u16) -> Option<usize> {
        self.layout
            .cells
            .iter()
            .find(|cell| col >= cell.col && col < cell.col.saturating_add(cell.width))
            .map(|cell| cell.window_index)
    }

    /// Whether a click at strip column `col` lands on the ` + ` button
    /// (any of its three columns: pads included).
    pub(crate) fn plus_hit(&self, col: u16) -> bool {
        self.layout
            .plus
            .is_some_and(|plus| col >= plus && col < plus + 3)
    }

    /// The changed cells against the previous paint, `(x, 0, cell)`;
    /// the painted row becomes the new baseline. Cell `y` is 0 — the
    /// caller flushes it at the host's top row.
    pub(crate) fn diff(&mut self) -> Vec<(u16, u16, RtCell)> {
        let diff = self
            .prev
            .diff(&self.buffer)
            .into_iter()
            .map(|(x, y, cell)| (x, y, cell.clone()))
            .collect::<Vec<_>>();
        self.prev = self.buffer.clone();
        diff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows() -> Vec<(String, String)> {
        vec![
            ("@0".to_string(), "main".to_string()),
            ("@1".to_string(), "vim".to_string()),
            ("@2".to_string(), "build".to_string()),
        ]
    }

    /// Stage 1: short names all fit — every tab, in window order, at
    /// herdr blocks (two-column pads) one gap column apart, no markers,
    /// with the ` + ` button reserved at the strip's right edge.
    #[test]
    fn short_names_place_every_tab_in_order() {
        let ws = windows();
        let layout = layout_tabs(&ws, Some("@1"), 80, 0, None);
        assert_eq!(layout.cells.len(), 3);
        assert_eq!(layout.left_marker, None);
        assert_eq!(layout.right_marker, None);
        let texts: Vec<&str> = layout.cells.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["main", "vim", "build"]);
        assert_eq!(layout.cells[0].col, 0);
        assert_eq!(layout.cells[0].width, 8, "'  main  ' = 8 cols");
        assert_eq!(layout.cells[1].col, 9, "one gap column after main");
        assert_eq!(layout.cells[1].width, 7, "'  vim  ' = 7 cols");
        assert_eq!(layout.cells[2].col, 17, "one gap column after vim");
        assert_eq!(layout.cells[2].width, 9, "'  build  ' = 9 cols");
        assert_eq!(
            layout.plus,
            Some(26),
            "the blank column after the last block"
        );
    }

    /// SEC-212: a bidi override in a window name is dropped before
    /// layout, so the tab is as wide as the visible name.
    #[test]
    fn spoofing_format_chars_do_not_widen_a_tab() {
        let clean = vec![("@0".to_string(), "abc".to_string())];
        let spoofed = vec![("@0".to_string(), "\u{202E}abc".to_string())];
        let a = layout_tabs(&clean, Some("@0"), 80, 0, None);
        let b = layout_tabs(&spoofed, Some("@0"), 80, 0, None);
        assert_eq!(b.cells[0].text, "abc");
        assert_eq!(b.cells[0].width, a.cells[0].width);
    }

    /// Stage 2: long names overflow — every long name truncates to the
    /// widest shared budget that fits BESIDE the ` + ` reservation.
    #[test]
    fn long_names_truncate_to_a_shared_budget() {
        let ws = vec![
            ("@0".to_string(), "development".to_string()),
            ("@1".to_string(), "documentation".to_string()),
        ];
        // The reservation narrows the tab area to 21 (24 - 3). A budget-b
        // block is b + 1 (ellipsis) + 4 (pads) wide, two blocks plus one
        // gap: budget 6 overflows (11 + 1 + 11 = 23 > 21), budget 5 fits
        // exactly ('  devel…  ' = '  docum…  ' = 10, 10 + 1 + 10 = 21).
        let layout = layout_tabs(&ws, Some("@0"), 24, 0, None);
        assert_eq!(layout.cells.len(), 2);
        assert_eq!(layout.cells[0].text, "devel\u{2026}");
        assert_eq!(layout.cells[1].text, "docum\u{2026}");
        assert_eq!(layout.cells[0].width, 10);
        assert_eq!(layout.cells[1].width, 10);
        assert_eq!(layout.cells[1].col, 11, "one gap column between blocks");
        assert_eq!(layout.left_marker, None);
        assert_eq!(layout.right_marker, None);
        assert_eq!(layout.plus, Some(21));
    }

    /// The ` + ` never goes past the line: the reservation caps the tab
    /// area, so a tab that would fill the whole row truncates beside the
    /// button, and the button starts right after the last block — at
    /// most at the reservation edge, where the three columns just reach
    /// the line's end.
    #[test]
    fn plus_clamps_to_the_line_when_tabs_fill_it() {
        // One tab whose name is wider than the row: the reservation
        // truncates it so the button fits — the boundary case, the
        // button exactly at the row's tail.
        let big = vec![("@0".to_string(), "x".repeat(80).to_string())];
        let layout = layout_tabs(&big, Some("@0"), 80, 0, None);
        let plus = layout.plus.expect("an 80-col strip hosts the button");
        assert!(plus + 3 <= 80, "the button stays on the row");
        let last = layout.cells.last().expect("the tab is visible");
        assert_eq!(
            last.col + last.width,
            plus,
            "the button follows the block with one blank column"
        );
        assert_eq!(plus, 77, "the block fills the area up to the reservation");

        // Across every width the button stays on the row, never
        // overlaps a block or a marker, and a strip that cannot host
        // the three-column button carries none.
        let ws: Vec<(String, String)> = (0..12)
            .map(|i| (format!("@{i}"), format!("window{i}")))
            .collect();
        for cols in 0..=80usize {
            let layout = layout_tabs(&ws, Some("@6"), cols, 0, None);
            if let Some(plus) = layout.plus {
                assert!(
                    plus as usize + 3 <= cols,
                    "cols {cols}: the button stays on the row"
                );
                let last = layout.cells.last().expect("stage 3 keeps a run");
                if last.col + last.width <= (cols - 3) as u16 {
                    assert!(
                        last.col + last.width <= plus,
                        "cols {cols}: the button never overlaps the last block"
                    );
                    if let Some(marker) = layout.right_marker {
                        if (marker as usize) < cols - 3 {
                            assert!(marker < plus, "cols {cols}: the marker precedes the button");
                        }
                    }
                }
            } else {
                assert!(cols < 4, "cols {cols}: only narrow strips drop the button");
            }
        }
    }

    /// The right `…` marker never lands inside the ` + ` button's
    /// three-column span: swept across widths, window lists, and active
    /// indices, a visible marker always precedes `plus` (or the layout
    /// suppresses it — the button owns the tail; the left marker still
    /// signals hidden tabs).
    #[test]
    fn right_marker_never_lands_in_the_plus_span() {
        let long_single = vec![(
            "@0".to_string(),
            "an-uncommonly-long-window-name".to_string(),
        )];
        let twelve: Vec<(String, String)> = (0..12)
            .map(|i| (format!("@{i}"), format!("window{i}")))
            .collect();
        let lists: Vec<Vec<(String, String)>> = vec![long_single, windows(), twelve];
        let mut hits: Vec<String> = Vec::new();
        for ws in &lists {
            for &active in [0usize, ws.len() / 2, ws.len() - 1].iter() {
                for cols in 0..=200usize {
                    let layout = layout_tabs(ws, Some(ws[active].0.as_str()), cols, 0, None);
                    if let (Some(plus), Some(marker)) = (layout.plus, layout.right_marker) {
                        if marker >= plus {
                            hits.push(format!(
                                "{} tabs, active {active}, cols {cols}: marker {marker} in span",
                                ws.len()
                            ));
                        }
                    }
                }
            }
        }
        assert!(hits.is_empty(), "markers inside the button span: {hits:?}");
    }

    /// Stage 3: so many tabs that even budget 1 overflows — only a run
    /// containing the active tab is visible, markers mark both hidden
    /// edges, and the run is contiguous and fits with its markers.
    #[test]
    fn too_many_tabs_keeps_the_active_visible() {
        let ws: Vec<(String, String)> = (0..12)
            .map(|i| (format!("@{i}"), format!("window{i}")))
            .collect();
        // Budget-1 width per tab: '  w…  ' = 6 cols; the 30-col strip's
        // tab area beside the plus is 27, and 12 blocks plus 11 gaps far
        // exceed it — even budget 1 overflows, so stage 3 engages. Each
        // hidden-edge marker costs its column plus a gap (2), so a run
        // of k blocks needs 7k - 1 + 4 <= 27: k = 3, the active tab and
        // the two on its left (the run grows leftward first).
        let layout = layout_tabs(&ws, Some("@7"), 30, 0, None);
        let visible: Vec<usize> = layout.cells.iter().map(|c| c.window_index).collect();
        assert_eq!(visible, vec![5, 6, 7], "contiguous run with the active");
        assert_eq!(layout.left_marker, Some(0), "tabs hide on the left");
        let cols: Vec<u16> = layout.cells.iter().map(|c| c.col).collect();
        assert_eq!(
            cols,
            vec![2, 9, 16],
            "marker + gap, then gap-separated blocks"
        );
        assert_eq!(
            layout.right_marker,
            Some(23),
            "tabs hide on the right: one gap after the last block (16 + 6)"
        );
        assert_eq!(layout.plus, Some(24), "the blank column after the marker");
    }

    /// The active tab stays visible even at the very end of the window
    /// list: the run reaches the last tab, so only the left edge hides.
    #[test]
    fn active_at_the_list_end_hides_only_the_left() {
        let ws: Vec<(String, String)> = (0..12)
            .map(|i| (format!("@{i}"), format!("w{i}")))
            .collect();
        let layout = layout_tabs(&ws, Some("@11"), 30, 0, None);
        let visible: Vec<usize> = layout.cells.iter().map(|c| c.window_index).collect();
        assert!(visible.contains(&11));
        assert_eq!(
            *visible.last().expect("non-empty"),
            11,
            "the run reaches the last tab"
        );
        assert!(layout.left_marker.is_some());
        assert_eq!(layout.right_marker, None);
    }

    /// Wheeling the overflowing strip moves the visible run in BOTH
    /// directions from the auto run, and an explicit scroll of 0 seeds
    /// the list's head — not the auto run (the scroll-0 ambiguity: a
    /// plain offset could never leave the auto run leftward). The run
    /// clamps at both edges, and `reset_scroll` returns to the auto run.
    #[test]
    fn wheel_scrolls_both_directions_from_the_auto_run() {
        let ws: Vec<(String, String)> = (0..12)
            .map(|i| (format!("@{i}"), format!("window{i}")))
            .collect();
        let visible = |strip: &TabStrip| -> Vec<usize> {
            strip.layout.cells.iter().map(|c| c.window_index).collect()
        };
        let mut strip = TabStrip::new(30);
        strip.paint(&ws, Some("@7"), None);
        assert_eq!(
            visible(&strip),
            vec![5, 6, 7],
            "the auto run centers the active"
        );

        strip.wheel(-3);
        strip.paint(&ws, Some("@7"), None);
        assert_eq!(
            visible(&strip),
            vec![2, 3, 4],
            "wheel-up reveals earlier tabs"
        );

        strip.wheel(-3);
        strip.paint(&ws, Some("@7"), None);
        assert_eq!(
            visible(&strip),
            vec![0, 1, 2],
            "the run reaches the list head"
        );

        strip.wheel(-3);
        strip.paint(&ws, Some("@7"), None);
        assert_eq!(visible(&strip), vec![0, 1, 2], "clamped at the head");

        strip.wheel(30);
        strip.paint(&ws, Some("@7"), None);
        assert_eq!(
            *visible(&strip).last().expect("non-empty"),
            11,
            "wheel-down reaches the list tail"
        );

        // Explicit scroll 0 seeds the head run; None is the auto run.
        let head = layout_tabs(&ws, Some("@7"), 30, 0, Some(0));
        assert_eq!(
            head.cells
                .iter()
                .map(|c| c.window_index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        let auto = layout_tabs(&ws, Some("@7"), 30, 0, None);
        assert_eq!(
            auto.cells
                .iter()
                .map(|c| c.window_index)
                .collect::<Vec<_>>(),
            vec![5, 6, 7]
        );

        strip.reset_scroll();
        strip.paint(&ws, Some("@7"), None);
        assert_eq!(
            visible(&strip),
            vec![5, 6, 7],
            "reset returns to the auto run"
        );
    }

    /// Hit-testing maps each block's columns (pads included) to its
    /// window index; gap and beyond-the-tabs columns hit nothing.
    #[test]
    fn hit_test_maps_columns_to_windows() {
        let ws = windows();
        let layout = layout_tabs(&ws, Some("@1"), 80, 0, None);
        let mut strip = TabStrip::new(80);
        strip.layout = layout;
        assert_eq!(strip.hit_test(0), Some(0), "the left pad owns the block");
        assert_eq!(strip.hit_test(7), Some(0), "the right pad owns the block");
        assert_eq!(strip.hit_test(8), None, "the gap hits nothing");
        assert_eq!(strip.hit_test(9), Some(1));
        assert_eq!(strip.hit_test(15), Some(1));
        assert_eq!(strip.hit_test(16), None, "the gap hits nothing");
        assert_eq!(strip.hit_test(17), Some(2));
        assert_eq!(strip.hit_test(25), Some(2));
        assert_eq!(strip.hit_test(26), None);
        assert_eq!(strip.hit_test(79), None);
    }

    /// Stage-3 marker columns and the gaps beside them hit nothing.
    #[test]
    fn hit_test_misses_markers_and_their_gaps() {
        let ws: Vec<(String, String)> = (0..12)
            .map(|i| (format!("@{i}"), format!("window{i}")))
            .collect();
        let mut strip = TabStrip::new(30);
        strip.paint(&ws, Some("@7"), None);
        // Left marker 0, gap 1, blocks 2..8 / 9..15 / 16..22, gap 22,
        // right marker 23.
        for col in [0u16, 1, 8, 15, 22, 23, 24] {
            assert_eq!(strip.hit_test(col), None, "col {col} hits nothing");
        }
        assert_eq!(strip.hit_test(2), Some(5));
        assert_eq!(strip.hit_test(21), Some(7));
        assert_eq!(strip.buffer[(0, 0)].symbol(), "\u{2026}");
        assert_eq!(strip.buffer[(23, 0)].symbol(), "\u{2026}");
        assert_eq!(strip.buffer[(23, 0)].fg, MUTED_FG);
        assert_eq!(strip.buffer[(23, 0)].bg, BAR_BG);
    }

    /// CJK tab names lay out and truncate by display width: a wide
    /// glyph is two columns of block, the next glyph starts past its
    /// right half, and stage 2 never splits a wide glyph at the budget.
    #[test]
    fn cjk_names_lay_out_and_truncate_by_display_width() {
        let ws = vec![
            ("@0".to_string(), "日本語".to_string()),
            ("@1".to_string(), "vim".to_string()),
        ];
        let layout = layout_tabs(&ws, Some("@0"), 80, 0, None);
        assert_eq!(layout.cells[0].width, 6 + 2 * PAD as u16);
        assert_eq!(layout.cells[1].col, layout.cells[0].width + GAP as u16);
        let mut strip = TabStrip::new(80);
        strip.paint(&ws, Some("@0"), None);
        let text_start = PAD as u16;
        assert_eq!(strip.buffer[(text_start, 0)].symbol(), "日");
        assert_eq!(strip.buffer[(text_start + 2, 0)].symbol(), "本");
        assert_eq!(strip.buffer[(text_start + 4, 0)].symbol(), "語");

        assert_eq!(tab_text("@0", "日本語", Some(5)), "日本\u{2026}");
        assert_eq!(tab_text("@0", "日本語", Some(6)), "日本語");
    }

    /// The painter draws herdr's blocks: the active one bright blue with
    /// dark bold text (pads included), the inactive ones lighter navy
    /// with muted text (pads included), the gaps and the rest of the row
    /// the bar background.
    #[test]
    fn paint_draws_herdr_blocks_on_the_bar() {
        let mut strip = TabStrip::new(40);
        strip.paint(&windows(), Some("@1"), None);
        // The active block spans 9..16; its text starts after two pads.
        for (col, ch) in [(11u16, 'v'), (12, 'i'), (13, 'm')] {
            let cell = &strip.buffer[(col, 0)];
            assert_eq!(cell.symbol(), ch.to_string(), "col {col}");
            assert_eq!(cell.fg, ACTIVE_FG, "col {col}");
            assert_eq!(cell.bg, ACTIVE_BG, "col {col}");
            assert!(cell.modifier.contains(RtModifier::BOLD), "col {col}");
        }
        for col in [9u16, 10, 14, 15] {
            let cell = &strip.buffer[(col, 0)];
            assert_eq!(cell.symbol(), " ", "pad col {col}");
            assert_eq!(cell.bg, ACTIVE_BG, "pad col {col} carries the block");
        }
        // Inactive: a filled block with muted text, never DIM.
        for col in 0..8u16 {
            let cell = &strip.buffer[(col, 0)];
            assert_eq!(cell.bg, INACTIVE_BG, "inactive col {col}");
            assert_eq!(cell.fg, MUTED_FG, "inactive col {col}");
            assert!(!cell.modifier.contains(RtModifier::DIM));
        }
        assert_eq!(strip.buffer[(2, 0)].symbol(), "m");
        // Gaps and the trailing bar carry the bar background.
        for col in [8u16, 16, 26, 36] {
            assert_eq!(strip.buffer[(col, 0)].bg, BAR_BG, "bar col {col}");
            assert_eq!(strip.buffer[(col, 0)].symbol(), " ", "bar col {col}");
        }
    }

    /// A theme repaints the strip from its palette: the bar panel_bg,
    /// inactive blocks surface0 with overlay0 text, the active block the
    /// accent with panel_bg text.
    #[test]
    fn paint_uses_the_theme_palette() {
        use crate::mux::theme::{palette, rt};
        let p = palette("gruvbox-light").expect("preset");
        let mut strip = TabStrip::new(40);
        strip.palette = Some(p);
        strip.paint(&windows(), Some("@1"), None);
        assert_eq!(strip.buffer[(11, 0)].bg, rt(p.accent));
        assert_eq!(strip.buffer[(11, 0)].fg, rt(p.panel_bg));
        assert_eq!(strip.buffer[(2, 0)].bg, rt(p.surface0));
        assert_eq!(strip.buffer[(2, 0)].fg, rt(p.overlay0));
        assert_eq!(strip.buffer[(36, 0)].bg, rt(p.panel_bg));
    }

    /// The strip diffs: the first paint flushes everything, an
    /// identical repaint flushes nothing, and an accent move flushes
    /// only the changed cells.
    #[test]
    fn strip_diffs_only_changes() {
        let mut strip = TabStrip::new(40);
        strip.paint(&windows(), Some("@0"), None);
        let first = strip.diff();
        // Every column carries at least the bar background, so the first
        // paint differs from the blank buffer everywhere.
        assert_eq!(first.len(), 40, "first paint is the whole row");
        assert_eq!(first[0].1, 0, "cell y is row-relative 0");
        strip.paint(&windows(), Some("@0"), None);
        assert!(strip.diff().is_empty(), "identical repaint diffs empty");
        strip.paint(&windows(), Some("@1"), None);
        let diff = strip.diff();
        let cols: Vec<u16> = diff.iter().map(|(x, _, _)| *x).collect();
        assert_eq!(
            cols,
            (0..8).chain(9..16).collect::<Vec<u16>>(),
            "an active move repaints only the two re-styled blocks"
        );
    }

    /// After `invalidate()` the next diff repaints EVERY column, blank
    /// ones included: the caller invalidates when the host row is not
    /// known to match the previous frame (a workspace switch reseed), so
    /// a 3-tab strip replaced by a 1-tab strip must erase the stale
    /// tabs' columns with blank cells rather than leave them on screen
    /// (the manual-pass stale-tab report).
    #[test]
    fn invalidate_repaints_every_column_including_blanks() {
        let mut strip = TabStrip::new(40);
        strip.paint(&windows(), Some("@0"), None);
        let _ = strip.diff();
        strip.invalidate();
        strip.paint(&windows()[..1], Some("@0"), None);
        let diff = strip.diff();
        let cols: Vec<u16> = diff.iter().map(|(x, _, _)| *x).collect();
        assert_eq!(
            cols,
            (0..40).collect::<Vec<u16>>(),
            "every column repaints after invalidate"
        );
        // The stale second/third tab columns arrive as blank bar cells —
        // except the button's span, which legitimately follows the one
        // remaining tab now (its ` + ` paints where tab two was).
        let mut plus_glyph = None;
        for (x, _, cell) in &diff {
            if (8..26).contains(x) {
                if x == &9 {
                    plus_glyph = Some(cell.symbol().to_string());
                    continue;
                }
                assert_eq!(cell.symbol(), " ", "stale col {x} erases to blank");
                assert_eq!(cell.bg, BAR_BG, "stale col {x} erases to the bar");
            }
        }
        assert_eq!(
            plus_glyph.as_deref(),
            Some("+"),
            "the button paints after the single remaining tab"
        );
        // And the repaint re-baselines: an identical paint diffs empty.
        strip.paint(&windows()[..1], Some("@0"), None);
        assert!(strip.diff().is_empty());
    }

    /// An empty window list paints a blank row and hit-tests nothing.
    #[test]
    fn empty_windows_paint_blank_and_hit_nothing() {
        let mut strip = TabStrip::new(20);
        strip.paint(&[], None, None);
        assert!(strip.diff().iter().all(|(_, _, cell)| cell.symbol() == " "));
        assert_eq!(strip.hit_test(3), None);
    }

    /// A strip narrower than one tab: the degenerate width still keeps
    /// the active tab in the layout (the painter clips the overflow).
    #[test]
    fn degenerate_width_keeps_the_active_tab() {
        let ws = windows();
        let layout = layout_tabs(&ws, Some("@2"), 6, 0, None);
        let visible: Vec<usize> = layout.cells.iter().map(|c| c.window_index).collect();
        assert!(visible.contains(&2), "active visible even tiny");
    }

    /// The active tab's highlight block covers exactly its own cell (text
    /// plus both pads) — with and without the panel's lead segment — and
    /// no other column carries the active background (the manual-pass round-7
    /// report of a highlight block bleeding left of the inactive tab).
    #[test]
    fn active_block_never_bleeds_into_other_tabs() {
        let windows: Vec<(String, String)> =
            vec![("@0".into(), "demo".into()), ("@1".into(), "woot".into())];
        for lead in [0u16, 20u16] {
            let mut strip = TabStrip::new(80);
            strip.paint(
                &windows,
                Some("@1"),
                if lead > 0 {
                    Some(("workspaces", lead))
                } else {
                    None
                },
            );
            let demo_span = lead..lead + 8; // `  demo  `
            let woot_span = lead + 9..lead + 17; // `  woot  ` after the gap
            for x in 0..80u16 {
                let bg_is_active = strip.buffer[(x, 0)].bg == ACTIVE_BG;
                if woot_span.contains(&x) {
                    assert!(
                        bg_is_active,
                        "lead={lead}: the active tab's col {x} must carry the block"
                    );
                } else {
                    assert!(
                        !bg_is_active,
                        "lead={lead}: col {x} outside the active block must not carry it"
                    );
                }
                if demo_span.contains(&x) {
                    assert_eq!(strip.buffer[(x, 0)].bg, INACTIVE_BG, "lead={lead} col {x}");
                }
            }
        }
    }

    /// `tab_text` at a budget: a name at or under the budget renders
    /// whole; a longer one truncates to the budget and ellipsizes.
    #[test]
    fn tab_text_truncates_only_over_budget() {
        assert_eq!(tab_text("@3", "notes", Some(8)), "notes");
        assert_eq!(tab_text("@3", "notes", Some(4)), "note\u{2026}");
        assert_eq!(tab_text("@3", "notes", None), "notes");
    }

    /// The ` + ` button follows the last tab: a muted `+` paints one
    /// blank column after the last block on the bar, its three columns
    /// hit the button, and the tabs lay out beside the button's
    /// three-column reservation (a strip too narrow for the button plus
    /// a tab carries none).
    #[test]
    fn plus_button_lays_out_paints_and_hits() {
        // '  main  ' = 8, '  vim  ' = 7, '  build  ' = 9 plus two gaps →
        // 26 used, the button 26..29 of the 40-col strip.
        let layout = layout_tabs(&windows(), Some("@0"), 40, 0, None);
        assert_eq!(layout.plus, Some(26), "the button follows the last tab");
        let mut strip = TabStrip::new(40);
        strip.layout = layout.clone();
        assert!(strip.plus_hit(26));
        assert!(strip.plus_hit(27));
        assert!(strip.plus_hit(28));
        assert!(!strip.plus_hit(25), "the last tab column is not the plus");
        strip.paint(&windows(), Some("@0"), None);
        assert_eq!(
            strip.buffer[(26, 0)].symbol(),
            " ",
            "the leading pad blanks"
        );
        assert_eq!(strip.buffer[(27, 0)].symbol(), "+");
        assert_eq!(strip.buffer[(27, 0)].fg, MUTED_FG, "the plus paints muted");
        for col in 26..29u16 {
            assert_eq!(
                strip.buffer[(col, 0)].bg,
                BAR_BG,
                "the plus sits on the bar"
            );
        }
        // Degenerate: a strip narrower than the button plus one tab
        // column cannot host the reservation.
        let tiny = layout_tabs(&windows(), Some("@0"), 3, 0, None);
        assert_eq!(tiny.plus, None, "the plus needs 3 spare columns");
    }

    /// The lead segment (the side panel's title when the panel is up):
    /// the tabs lay out FROM the lead columns onward and the ` + `
    /// follows the last tab, so a click maps the RAW host column
    /// straight through — lead columns hit nothing, the first tab
    /// starts at the lead width, and the plus hits right after the
    /// last block.
    #[test]
    fn lead_segment_offsets_the_tabs_and_the_plus_follows() {
        // '  main  ' = 8, '  vim  ' = 7, '  build  ' = 9 one gap apart →
        // blocks at 20..28, 29..36, 37..46 of the 80-col strip, the
        // button 46..49.
        let ws = windows();
        let layout = layout_tabs(&ws, Some("@1"), 80, 20, None);
        assert_eq!(layout.plus, Some(46), "the button follows the last tab");
        assert_eq!(layout.cells[0].col, 20, "tabs start at the lead width");
        assert_eq!(layout.cells[0].text, "main");
        let mut strip = TabStrip::new(80);
        strip.layout = layout;
        assert_eq!(strip.hit_test(0), None, "lead columns hit nothing");
        assert_eq!(strip.hit_test(19), None);
        assert_eq!(strip.hit_test(20), Some(0), "the first tab owns its start");
        assert_eq!(strip.hit_test(27), Some(0));
        assert_eq!(strip.hit_test(28), None, "the gap hits nothing");
        assert_eq!(strip.hit_test(29), Some(1));
        assert!(strip.plus_hit(46));
        assert!(strip.plus_hit(48), "the plus hits through its right pad");
        assert!(!strip.plus_hit(45), "the last tab column is not the plus");
        assert!(!strip.plus_hit(10), "the lead column is not the plus");
    }

    /// The lead title paints accent in the first `width` columns and
    /// the tabs paint from it onward (the mock's shared row 0).
    #[test]
    fn paint_carries_the_lead_title_accent() {
        let mut strip = TabStrip::new(40);
        strip.paint(&windows(), Some("@0"), Some(("workspaces", 14)));
        // " workspaces " = 12 chars paint inside the 14-col lead.
        assert_eq!(strip.buffer[(0, 0)].symbol(), " ");
        assert_eq!(strip.buffer[(1, 0)].symbol(), "w");
        assert_eq!(strip.buffer[(10, 0)].symbol(), "s");
        assert_eq!(
            strip.buffer[(12, 0)].symbol(),
            " ",
            "the lead's pad column stays blank"
        );
        assert_eq!(strip.buffer[(1, 0)].fg, ACCENT);
        assert_eq!(strip.buffer[(1, 0)].bg, BAR_BG, "the lead sits on the bar");
        // The first tab's text starts at the lead width (col 14 + 2 pads).
        assert_eq!(strip.buffer[(16, 0)].symbol(), "m");
        assert_eq!(
            strip.buffer[(14, 0)].bg,
            ACTIVE_BG,
            "the block starts at 14"
        );
        // No lead: the tabs start at col 0 as before.
        let mut plain = TabStrip::new(40);
        plain.paint(&windows(), Some("@0"), None);
        assert_eq!(plain.buffer[(0, 0)].symbol(), " ", "tab pad");
        assert_eq!(plain.buffer[(1, 0)].symbol(), " ", "tab pad");
        assert_eq!(plain.buffer[(2, 0)].symbol(), "m");
        assert_ne!(plain.buffer[(2, 0)].fg, ACCENT);
    }
}
