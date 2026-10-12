//! Pane chrome: shared dividers (tmux half rule), per-pane border boxes, and their embedded labels.

use super::{PaneRect, PaneRenderer};
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};

/// The focused boundary half's highlight (config `border-active-color`,
/// `#rrggbb`; default the bright-cyan accent), always bold. Dividers
/// follow the tmux half rule (see [`divider_style`]): on a divider shared
/// by two panes, the half nearer the focused pane takes this style and
/// the other half takes [`plain_border_style`], so the indication flips
/// sides when focus moves. A live drag renders the boundary reversed.
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

impl PaneRenderer {
    /// Draw the dividers between adjacent layout rects. The daemon's
    /// geometry tiles exactly (no reserved gap), so a divider overlays the
    /// boundary column/row of the pane content — tmux's look on a grid the
    /// daemon divided gap-free. Vertical boundaries first, horizontals
    /// second, then crossing cells become the junction glyph.
    pub(super) fn paint_dividers(&mut self) {
        if self.layout.len() < 2 {
            return;
        }
        let (vertical, horizontal) = collect_boundaries(&self.layout, self.geometry.sidebar_w);
        for group in group_boundaries(&vertical) {
            self.paint_boundary_group(Axis::Vertical, &group, &vertical);
        }
        for group in group_boundaries(&horizontal) {
            self.paint_boundary_group(Axis::Horizontal, &group, &vertical);
        }
    }

    /// Paint one divider: each cell's glyph and the focus-half / drag
    /// style by its position along the divider. `vertical` is every
    /// vertical boundary cell — a horizontal cell on one becomes the
    /// junction glyph.
    fn paint_boundary_group(
        &mut self,
        axis: Axis,
        (boundary, cells): &BoundaryGroup,
        vertical: &[BoundaryCell],
    ) {
        let len = cells.len() as u16;
        for (index, &(along, a, b)) in cells.iter().enumerate() {
            let (x, y) = axis.cell(*boundary, along);
            if x >= self.width || y >= self.height {
                continue; // a stale layout racing a shrink can overflow
            }
            if self.gutter_owns(axis, x, y, a) {
                continue;
            }
            let is_vertical = axis == Axis::Vertical;
            if !is_vertical && vertical.iter().any(|&(vx, vy, _, _)| vx == x && vy == y) {
                self.buffer[(x, y)].set_symbol(self.glyphs.cross());
                continue;
            }
            let glyph = if is_vertical {
                self.glyphs.vertical()
            } else {
                self.glyphs.horizontal()
            };
            let style = divider_style(
                self.focused,
                self.drag_divider,
                is_vertical,
                a,
                b,
                index as u16,
                len,
                self.border_active,
                self.border_plain,
            );
            let cell = &mut self.buffer[(x, y)];
            cell.reset();
            if let Some(bg) = self.bg {
                cell.set_bg(bg);
            }
            cell.set_symbol(glyph);
            cell.set_style(style);
        }
    }

    /// The scrollbar gutter owns the left/top pane's last column/row when
    /// reserved (config `scrollbar-gutter`): the boundary divider yields
    /// the cell so the gutter's indicator stays visible. The boundary
    /// stays a drag handle (`divider_near` reads the layout geometry, not
    /// the paint).
    fn gutter_owns(&self, axis: Axis, x: u16, y: u16, a: u32) -> bool {
        match axis {
            Axis::Vertical => {
                self.scrollbar_gutter
                    && self
                        .layout
                        .iter()
                        .any(|r| r.pane == a && x == self.geometry.sidebar_w + r.x + r.width - 1)
            }
            Axis::Horizontal => {
                self.scrollbar_gutter
                    && self
                        .layout
                        .iter()
                        .any(|r| r.pane == a && y == r.y + r.height - 1)
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

/// A divider's orientation: `Vertical` boundaries are columns between
/// side-by-side panes, `Horizontal` ones rows between stacked panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    /// The frame cell of a `(boundary, along)` pair: a vertical
    /// boundary is a column, a horizontal one a row.
    fn cell(self, boundary: u16, along: u16) -> (u16, u16) {
        match self {
            Axis::Vertical => (boundary, along),
            Axis::Horizontal => (along, boundary),
        }
    }
}

/// One boundary cell: `(boundary, along, a, b)` — the divider's
/// column (vertical) or row (horizontal), the cell's position along it,
/// and the left/top and right/bottom pane ids (they drive the
/// focus-side style and the drag highlight). Frame coordinates.
pub(super) type BoundaryCell = (u16, u16, u32, u32);

/// One divider: its boundary coordinate and its `(along, a, b)` cells
/// sorted along its length.
pub(super) type BoundaryGroup = (u16, Vec<(u16, u32, u32)>);

/// Every boundary cell of `layout`, `(vertical, horizontal)`. Dividers
/// draw on the left/top pane's last column/row; content x shifts right
/// by the side panel's `sidebar_w` columns.
pub(super) fn collect_boundaries(
    layout: &[PaneRect],
    sidebar_w: u16,
) -> (Vec<BoundaryCell>, Vec<BoundaryCell>) {
    let mut vertical = Vec::new();
    let mut horizontal = Vec::new();
    for (i, first) in layout.iter().enumerate() {
        for second in layout.iter().skip(i + 1) {
            for (l, r) in [(first, second), (second, first)] {
                if l.x + l.width == r.x && rows_overlap(l, r) {
                    let boundary = r.x.saturating_sub(1) + sidebar_w;
                    vertical.extend(
                        row_overlap(l, r)
                            .into_iter()
                            .map(|y| (boundary, y, l.pane, r.pane)),
                    );
                }
                if l.y + l.height == r.y && cols_overlap(l, r) {
                    let boundary = r.y.saturating_sub(1);
                    horizontal.extend(
                        col_overlap(l, r)
                            .into_iter()
                            .map(|x| (boundary, x + sidebar_w, l.pane, r.pane)),
                    );
                }
            }
        }
    }
    (vertical, horizontal)
}

/// Group boundary cells per divider (first-seen order), each sorted
/// along its length — the position the tmux half rule reads (the active
/// pane's half of a shared divider carries the highlight).
pub(super) fn group_boundaries(cells: &[BoundaryCell]) -> Vec<BoundaryGroup> {
    let mut groups: Vec<BoundaryGroup> = Vec::new();
    for &(boundary, along, a, b) in cells {
        match groups.iter_mut().find(|(k, _)| *k == boundary) {
            Some((_, entries)) => entries.push((along, a, b)),
            None => groups.push((boundary, vec![(along, a, b)])),
        }
    }
    for (_, entries) in &mut groups {
        entries.sort_by_key(|&(along, _, _)| along);
    }
    groups
}
