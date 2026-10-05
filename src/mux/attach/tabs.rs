//! The render-mode tab strip: the queried window list, its one-line
//! composition, the ratatui row painter, and the click hit-testing.
//!
//! [`TabStrip`] is the top row's counterpart to the bottom
//! [`super::status::StatusRow`]: it paints one tab per WINDOW of the
//! shown session — `N:name`, in window order, the active tab a solid
//! highlighted block on the accent, the inactive tabs dim — into a
//! one-row buffer and diffs it, so a window churn flushes only the
//! changed cells. A ` + ` button owns the strip's last three columns
//! whenever the strip can hold tabs at all (the reservation narrows the
//! tab area); a click on it opens the new-tab prompt. The window list comes from the same queried state the
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

use ratatui::buffer::{Buffer, Cell as RtCell};
use ratatui::layout::Rect as RtRect;
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};

/// The theme accent: the same indexed color the pane borders and the
/// help modal are themed with.
const ACCENT: RtColor = RtColor::Indexed(14);

/// The ellipsis spelling for truncated names and hidden edges.
const ELLIPSIS: char = '\u{2026}';

/// One visible tab's geometry and text: the column span a click on
/// that tab owns, plus the exact text the painter writes inside its
/// one-column pads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TabCell {
    /// Leftmost column of the cell (pad included).
    pub col: u16,
    /// Cell width in columns (text plus both pads).
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
    /// The column where the ` + ` button's three-column reservation
    /// starts (the strip's right edge), when the strip is wide enough to
    /// hold tabs plus the button.
    pub plus: Option<u16>,
}

/// The tab strip's painter: a one-row buffer, its previous frame, and
/// the layout the last paint produced (the hit-test's source of truth).
pub(crate) struct TabStrip {
    cols: u16,
    buffer: Buffer,
    prev: Buffer,
    layout: TabLayout,
}

/// The display text of one tab at a name budget: `N:name`, the window's
/// ordinal (the id without its `@`). A name longer than the budget is
/// truncated to it and ellipsized.
fn tab_text(id: &str, name: &str, budget: Option<usize>) -> String {
    let ordinal = id.trim_start_matches('@');
    match budget {
        None => format!("{ordinal}:{name}"),
        Some(budget) => {
            let chars: Vec<char> = name.chars().collect();
            if chars.len() > budget {
                let head: String = chars[..budget].iter().collect();
                format!("{ordinal}:{head}{ELLIPSIS}")
            } else {
                format!("{ordinal}:{name}")
            }
        }
    }
}

/// Lay the tabs out left to right from `start` with the given texts and
/// widths, when their widths fit `budget`. `None` on overflow.
fn place(texts: &[String], widths: &[usize], start: u16, budget: usize) -> Option<Vec<TabCell>> {
    if widths.iter().sum::<usize>() > budget {
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
            col += *width as u16;
            cell
        })
        .collect();
    Some(cells)
}

/// Compose the strip layout for `windows` (the queried `(id, name)`
/// list, window order; `active` is the active window id) into `cols`
/// columns. `lead` is the columns the strip's LEFT segment occupies —
/// the side panel's title when the panel is up, 0 when it is down: the
/// tabs lay out from it onward and the ` + ` stays pinned at the
/// strip's right edge, so a click maps the RAW host column straight
/// through (the panel-up geometry stays coherent).
///
/// Stage 1: full names, left to right, when they all fit. Stage 2: the
/// widest shared name budget that fits — names at or under the budget
/// render whole, longer ones truncate and ellipsize. Stage 3: budget 1
/// still overflows, so only a contiguous run of tabs containing the
/// active one is visible, with a `…` marker at each edge where tabs
/// stay hidden (each marker budgets one column up front).
pub(crate) fn layout_tabs(
    windows: &[(String, String)],
    active: Option<&str>,
    cols: usize,
    lead: u16,
) -> TabLayout {
    if windows.is_empty() {
        return TabLayout::default();
    }
    let active_index = windows
        .iter()
        .position(|(id, _)| Some(id.as_str()) == active)
        .unwrap_or(0);
    let lead = (lead as usize).min(cols);

    // The ` + ` button reserves the strip's last three columns; the tab
    // stages lay out between the lead segment and the reservation. A
    // strip narrower than the button plus one tab column cannot host
    // the reservation.
    let plus_col = u16::try_from(cols.saturating_sub(3))
        .ok()
        .filter(|_| cols >= 4 && lead + 3 < cols);
    let tab_cols = plus_col.map_or(cols.saturating_sub(lead), |p| {
        (p as usize).saturating_sub(lead)
    });
    let start = lead as u16;

    let texts_widths = |budget: Option<usize>| -> (Vec<String>, Vec<usize>) {
        let texts: Vec<String> = windows
            .iter()
            .map(|(id, name)| tab_text(id, name, budget))
            .collect();
        let widths: Vec<usize> = texts.iter().map(|text| text.chars().count() + 2).collect();
        (texts, widths)
    };

    // Stage 1.
    let (texts, widths) = texts_widths(None);
    if let Some(cells) = place(&texts, &widths, start, tab_cols) {
        return TabLayout {
            cells,
            left_marker: None,
            right_marker: None,
            plus: plus_col,
        };
    }

    // Stage 2.
    let max_name = windows
        .iter()
        .map(|(_, name)| name.chars().count())
        .max()
        .unwrap_or(1)
        .max(1);
    for budget in (1..=max_name).rev() {
        let (texts, widths) = texts_widths(Some(budget));
        if let Some(cells) = place(&texts, &widths, start, tab_cols) {
            return TabLayout {
                cells,
                left_marker: None,
                right_marker: None,
                plus: plus_col,
            };
        }
    }

    // Stage 3.
    let (texts, widths) = texts_widths(Some(1));
    let mut run: Vec<usize> = vec![active_index];
    let mut total = widths[active_index];
    loop {
        let first = run[0];
        let last = *run.last().expect("non-empty by construction");
        let mut added = false;
        if first > 0 {
            let candidate = first - 1;
            let markers_after = usize::from(candidate > 0) + usize::from(last < windows.len() - 1);
            if total + widths[candidate] + markers_after <= tab_cols {
                total += widths[candidate];
                run.insert(0, candidate);
                added = true;
            }
        }
        if !added && last + 1 < windows.len() {
            let candidate = last + 1;
            let markers_after = usize::from(first > 0) + usize::from(candidate < windows.len() - 1);
            if total + widths[candidate] + markers_after <= tab_cols {
                total += widths[candidate];
                run.push(candidate);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    // The run places left-aligned after the lead segment and the
    // (optional) leading marker; left alignment keeps the hit columns
    // stable for a given window list.
    let mut col = start + u16::from(run[0] > 0);
    let cells: Vec<TabCell> = run
        .iter()
        .map(|&index| {
            let cell = TabCell {
                col,
                width: widths[index] as u16,
                window_index: index,
                text: texts[index].clone(),
            };
            col += widths[index] as u16;
            cell
        })
        .collect();
    let left_hidden = run[0] > 0;
    let right_hidden = *run.last().expect("non-empty") < windows.len() - 1;
    let right_marker = right_hidden.then(|| {
        cells
            .last()
            .map(|cell| cell.col + cell.width)
            .unwrap_or_default()
    });
    TabLayout {
        cells,
        left_marker: left_hidden.then_some(start),
        right_marker,
        plus: plus_col,
    }
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
        }
    }

    /// The strip's width.
    pub(crate) fn cols(&self) -> u16 {
        self.cols
    }

    /// Forget the previous frame: the next diff paints the whole row
    /// (after a repaint-all or a host resize the caller knows wiped the
    /// row).
    pub(crate) fn invalidate(&mut self) {
        self.prev = Buffer::empty(RtRect::new(0, 0, self.cols, 1));
    }

    /// Paint `windows` over the whole row, padding the remainder with
    /// spaces so the previous frame's tail erases. `lead` is the side
    /// panel's title when the panel is up — `(title, width)`: the title
    /// paints in the first `width` columns (accent — the panel's
    /// header, per the mock), the tabs lay out from it onward, and the
    /// ` + ` stays pinned at the strip's right edge. The active tab is
    /// a solid highlighted block, inactive tabs dim, edge markers dim.
    pub(crate) fn paint(
        &mut self,
        windows: &[(String, String)],
        active: Option<&str>,
        lead: Option<(&str, u16)>,
    ) {
        let lead_w = lead.map_or(0, |(_, w)| w);
        let layout = layout_tabs(windows, active, self.cols as usize, lead_w);
        self.layout = layout.clone();
        for x in 0..self.cols {
            let cell = &mut self.buffer[(x, 0)];
            cell.reset();
        }
        // The lead title: the panel's header — accent, clipped to the
        // lead segment. Clicks on it hit nothing (no cells below).
        if let Some((title, width)) = lead {
            let style = RtStyle::default().fg(ACCENT);
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
        // Edge markers, dim.
        for marker in [layout.left_marker, layout.right_marker]
            .into_iter()
            .flatten()
        {
            let cell = &mut self.buffer[(marker, 0)];
            cell.set_symbol(&ELLIPSIS.to_string());
            cell.set_style(RtStyle::default().add_modifier(RtModifier::DIM));
        }
        // The ` + ` button in its reserved right-edge slot, dim like the
        // inactive tabs.
        if let Some(plus) = layout.plus {
            let cell = &mut self.buffer[(plus + 1, 0)];
            cell.set_symbol("+");
            cell.set_style(RtStyle::default().add_modifier(RtModifier::DIM));
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
                // herdr's active tab: a solid highlighted block — dark on
                // the bright accent — not a foreground tint (a tint read
                // as "hovered", not "current"; the round-5 report).
                RtStyle::default()
                    .fg(RtColor::Indexed(0))
                    .bg(ACCENT)
                    .add_modifier(RtModifier::BOLD)
            } else {
                RtStyle::default().add_modifier(RtModifier::DIM)
            };
            if Some(i) == active_cell {
                // The block covers the tab's full width, pads included.
                for x in cell.col..cell.col + cell.width {
                    if x < self.cols {
                        self.buffer[(x, 0)].set_style(style);
                    }
                }
            }
            for (x, ch) in (cell.col + 1..).zip(cell.text.chars()) {
                if x >= self.cols {
                    break;
                }
                let buffer_cell = &mut self.buffer[(x, 0)];
                buffer_cell.set_symbol(&ch.to_string());
                buffer_cell.set_style(style);
            }
        }
    }

    /// The window index a click at strip column `col` lands on — the
    /// visible tab whose cell spans the column. Marker and pad columns
    /// hit nothing.
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
    /// consecutive offsets, no markers, with the ` + ` button reserved
    /// at the strip's right edge.
    #[test]
    fn short_names_place_every_tab_in_order() {
        let ws = windows();
        let layout = layout_tabs(&ws, Some("@1"), 80, 0);
        assert_eq!(layout.cells.len(), 3);
        assert_eq!(layout.left_marker, None);
        assert_eq!(layout.right_marker, None);
        let texts: Vec<&str> = layout.cells.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["0:main", "1:vim", "2:build"]);
        assert_eq!(layout.cells[0].col, 0);
        assert_eq!(layout.cells[0].width, 8, "' 0:main ' = 8 cols");
        assert_eq!(layout.cells[1].col, 8);
        assert_eq!(layout.cells[1].width, 7, "' 1:vim ' = 7 cols");
        assert_eq!(layout.cells[2].col, 15);
        assert_eq!(layout.cells[2].width, 9, "' 2:build ' = 9 cols");
        assert_eq!(layout.plus, Some(77), "the plus reserves the right edge");
    }

    /// Stage 2: long names overflow — every long name truncates to the
    /// widest shared budget that fits BESIDE the ` + ` reservation.
    #[test]
    fn long_names_truncate_to_a_shared_budget() {
        let ws = vec![
            ("@0".to_string(), "development".to_string()),
            ("@1".to_string(), "documentation".to_string()),
        ];
        // The reservation narrows the tab area to 21 (24 - 3): budget 8
        // overflows (13 + 13 = 26 > 21), budget 6 too (11 + 11 = 22),
        // budget 5 fits: ' 0:devel… ' = ' 1:docum… ' = 10 each.
        let layout = layout_tabs(&ws, Some("@0"), 24, 0);
        assert_eq!(layout.cells.len(), 2);
        assert_eq!(layout.cells[0].text, "0:devel\u{2026}");
        assert_eq!(layout.cells[1].text, "1:docum\u{2026}");
        assert_eq!(layout.cells[0].width, 10);
        assert_eq!(layout.cells[1].width, 10);
        assert_eq!(layout.left_marker, None);
        assert_eq!(layout.right_marker, None);
        assert_eq!(layout.plus, Some(21));
    }

    /// Stage 3: so many tabs that even budget 1 overflows — only a run
    /// containing the active tab is visible, markers mark both hidden
    /// edges, and the run is contiguous and fits with its markers.
    #[test]
    fn too_many_tabs_keeps_the_active_visible() {
        let ws: Vec<(String, String)> = (0..12)
            .map(|i| (format!("@{i}"), format!("window{i}")))
            .collect();
        // Budget-1 width per tab: ' Ni:w… ' = 7 cols; 12 * 7 = 84 > 40.
        let layout = layout_tabs(&ws, Some("@7"), 40, 0);
        let visible: Vec<usize> = layout.cells.iter().map(|c| c.window_index).collect();
        assert!(visible.contains(&7), "the active tab is visible");
        for pair in visible.windows(2) {
            assert_eq!(pair[1], pair[0] + 1, "the run is contiguous");
        }
        assert!(layout.left_marker.is_some(), "tabs hide on the left");
        assert!(layout.right_marker.is_some(), "tabs hide on the right");
        let total: usize = layout.cells.iter().map(|c| c.width as usize).sum();
        assert!(total <= 35, "run fits beside the plus reservation: {total}");
        assert_eq!(layout.plus, Some(37));
    }

    /// The active tab stays visible even at the very end of the window
    /// list: the run reaches the last tab, so only the left edge hides.
    #[test]
    fn active_at_the_list_end_hides_only_the_left() {
        let ws: Vec<(String, String)> = (0..12)
            .map(|i| (format!("@{i}"), format!("w{i}")))
            .collect();
        let layout = layout_tabs(&ws, Some("@11"), 30, 0);
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

    /// Hit-testing maps each tab's columns to its window index; pad and
    /// beyond-the-tabs columns hit nothing.
    #[test]
    fn hit_test_maps_columns_to_windows() {
        let ws = windows();
        let layout = layout_tabs(&ws, Some("@1"), 80, 0);
        let mut strip = TabStrip::new(80);
        strip.layout = layout;
        assert_eq!(strip.hit_test(0), Some(0), "pad still owns the cell");
        assert_eq!(strip.hit_test(7), Some(0));
        assert_eq!(strip.hit_test(8), Some(1));
        assert_eq!(strip.hit_test(14), Some(1));
        assert_eq!(strip.hit_test(15), Some(2));
        assert_eq!(strip.hit_test(23), Some(2));
        assert_eq!(strip.hit_test(24), None);
        assert_eq!(strip.hit_test(79), None);
    }

    /// The painter styles the active tab as the solid highlighted block
    /// (dark on the accent, pads included) and the inactive tabs dim.
    #[test]
    fn paint_highlights_the_active_tab_as_a_block_and_dims_the_rest() {
        let mut strip = TabStrip::new(40);
        strip.paint(&windows(), Some("@1"), None);
        // The active cell's text starts at col 8 + 1 pad.
        for (col, ch) in [(9u16, '1'), (10, ':'), (11, 'v'), (12, 'i'), (13, 'm')] {
            let cell = &strip.buffer[(col, 0)];
            assert_eq!(cell.symbol(), ch.to_string(), "col {col}");
            assert_eq!(cell.fg, RtColor::Indexed(0), "col {col}");
            assert_eq!(cell.bg, ACCENT, "col {col}");
            assert!(cell.modifier.contains(RtModifier::BOLD), "col {col}");
        }
        // The block covers the pads too: the active cell spans cols 8..14.
        for col in [8u16, 14] {
            let cell = &strip.buffer[(col, 0)];
            assert_eq!(cell.bg, ACCENT, "pad col {col} carries the block");
            assert!(cell.modifier.contains(RtModifier::BOLD), "pad col {col}");
        }
        // Inactive: dim, not the block.
        let cell = &strip.buffer[(1, 0)];
        assert_eq!(cell.symbol(), "0");
        assert!(cell.modifier.contains(RtModifier::DIM));
        assert_ne!(cell.bg, ACCENT);
    }

    /// The strip diffs: the first paint flushes everything, an
    /// identical repaint flushes nothing, and an accent move flushes
    /// only the changed cells.
    #[test]
    fn strip_diffs_only_changes() {
        let mut strip = TabStrip::new(40);
        strip.paint(&windows(), Some("@0"), None);
        let first = strip.diff();
        // Only the painted cells differ from the blank buffer: 18 text
        // cells across the three tabs, the active tab's two pads (the
        // block background), and the ` + ` glyph.
        assert_eq!(
            first.len(),
            21,
            "first paint is the text cells + block pads + plus"
        );
        assert_eq!(first[0].1, 0, "cell y is row-relative 0");
        strip.paint(&windows(), Some("@0"), None);
        assert!(strip.diff().is_empty(), "identical repaint diffs empty");
        strip.paint(&windows(), Some("@1"), None);
        let diff = strip.diff();
        assert!(!diff.is_empty());
        assert!(
            diff.len() <= first.len(),
            "an accent move repaints only the re-styled tabs"
        );
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
        let layout = layout_tabs(&ws, Some("@2"), 6, 0);
        let visible: Vec<usize> = layout.cells.iter().map(|c| c.window_index).collect();
        assert!(visible.contains(&2), "active visible even tiny");
    }

    /// The active tab's highlight block covers exactly its own cell (text
    /// plus both pads) — with and without the panel's lead segment — and
    /// no other tab's columns carry the accent (the manual-pass round-7
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
            let demo_span = lead..lead + 8; // ` 0:demo `
            let woot_span = lead + 8..lead + 16; // ` 1:woot `
            for x in 0..80u16 {
                let bg_is_accent = strip.buffer[(x, 0)].bg == ACCENT;
                if demo_span.contains(&x) {
                    assert!(
                        !bg_is_accent,
                        "lead={lead}: the inactive tab's col {x} must not carry the accent"
                    );
                }
                if woot_span.contains(&x) {
                    assert!(
                        bg_is_accent,
                        "lead={lead}: the active tab's col {x} must carry the block"
                    );
                }
            }
        }
    }

    /// `tab_text` at a budget: a name at or under the budget renders
    /// whole; a longer one truncates to the budget and ellipsizes.
    #[test]
    fn tab_text_truncates_only_over_budget() {
        assert_eq!(tab_text("@3", "notes", Some(8)), "3:notes");
        assert_eq!(tab_text("@3", "notes", Some(4)), "3:note\u{2026}");
        assert_eq!(tab_text("@3", "notes", None), "3:notes");
    }

    /// The ` + ` button owns the strip's reserved right-edge slot: a
    /// dim `+` paints at its middle column, its three columns hit the
    /// button, and the tabs are laid out beside the reservation (a
    /// strip too narrow for the button plus a tab carries none).
    #[test]
    fn plus_button_lays_out_paints_and_hits() {
        // ' 0:main ' = 8, ' 1:vim ' = 7, ' 2:build ' = 9 → 24 used, the
        // plus reservation at 37..40 of the 40-col strip.
        let layout = layout_tabs(&windows(), Some("@0"), 40, 0);
        assert_eq!(layout.plus, Some(37), "the plus reserves the right edge");
        let mut strip = TabStrip::new(40);
        strip.layout = layout.clone();
        assert!(strip.plus_hit(37));
        assert!(strip.plus_hit(38));
        assert!(strip.plus_hit(39));
        assert!(!strip.plus_hit(36), "the last tab column is not the plus");
        strip.paint(&windows(), Some("@0"), None);
        assert_eq!(strip.buffer[(38, 0)].symbol(), "+");
        assert!(
            strip.buffer[(38, 0)].modifier.contains(RtModifier::DIM),
            "the plus paints dim"
        );
        // Degenerate: a strip narrower than the button plus one tab
        // column cannot host the reservation.
        let tiny = layout_tabs(&windows(), Some("@0"), 3, 0);
        assert_eq!(tiny.plus, None, "the plus needs 3 spare columns");
    }

    /// The lead segment (the side panel's title when the panel is up):
    /// the tabs lay out FROM the lead columns onward, the ` + ` stays
    /// pinned at the strip's right edge, and a click maps the RAW host
    /// column straight through — lead columns hit nothing, the first
    /// tab starts at the lead width, the plus hits at the right edge.
    /// This is the plus-defect fix's unit half.
    #[test]
    fn lead_segment_offsets_the_tabs_and_keeps_the_plus_pinned() {
        // ' 0:main ' = 8, ' 1:vim ' = 7, ' 2:build ' = 9 → the three
        // tabs span 20..44 of the 80-col strip, plus reservation 77..80.
        let ws = windows();
        let layout = layout_tabs(&ws, Some("@1"), 80, 20);
        assert_eq!(layout.plus, Some(77), "the plus stays pinned right");
        assert_eq!(layout.cells[0].col, 20, "tabs start at the lead width");
        assert_eq!(layout.cells[0].text, "0:main");
        let mut strip = TabStrip::new(80);
        strip.layout = layout;
        assert_eq!(strip.hit_test(0), None, "lead columns hit nothing");
        assert_eq!(strip.hit_test(19), None);
        assert_eq!(strip.hit_test(20), Some(0), "the first tab owns its start");
        assert_eq!(strip.hit_test(27), Some(0));
        assert_eq!(strip.hit_test(28), Some(1));
        assert!(strip.plus_hit(78), "the plus hits at the raw right edge");
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
        // The first tab's text starts at the lead width (col 14 + 1 pad).
        assert_eq!(strip.buffer[(15, 0)].symbol(), "0");
        // No lead: the tabs start at col 0 as before.
        let mut plain = TabStrip::new(40);
        plain.paint(&windows(), Some("@0"), None);
        assert_eq!(plain.buffer[(0, 0)].symbol(), " ", "tab pad");
        assert_eq!(plain.buffer[(1, 0)].symbol(), "0");
        assert_ne!(plain.buffer[(1, 0)].fg, ACCENT);
    }
}
