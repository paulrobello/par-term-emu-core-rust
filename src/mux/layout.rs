//! Layout tree: a window's interior pane split structure.
//!
//! Binary split tree matching `par-term`'s own `PaneNode` shape exactly
//! (`par-term/src/pane/types/pane_node.rs`), stored per-window in place of a
//! flat pane list. Every `split-window` command creates exactly one new
//! binary division of an existing pane, so the tree never needs to construct
//! an N-way split directly — only nested binary ones. Rendering to the tmux
//! wire grammar (Task 2.2) collapses a run of same-direction sibling splits
//! into one N-ary group, mirroring the inverse of `par-term`'s
//! `rebuild_multi_split_to_binary`. See `par-mux.md` Phase 2 Decision 1.

use crate::mux::ids::PaneId;

/// Split orientation, matching `par-term`'s `SplitDirection` naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SplitDirection {
    /// Panes are stacked vertically (split creates top/bottom panes).
    Horizontal,
    /// Panes are side by side (split creates left/right panes).
    Vertical,
}

/// Which way a resize adjustment pushes the pane's border, matching tmux's
/// `resize-pane -L`/`-R`/`-U`/`-D` flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeDirection {
    /// `-L`: grow toward the left.
    Left,
    /// `-R`: grow toward the right.
    Right,
    /// `-U`: grow upward.
    Up,
    /// `-D`: grow downward.
    Down,
}

/// A window's interior pane structure: either a single pane, or a binary
/// split into two sub-trees with a ratio dividing them.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LayoutTree {
    /// A leaf holding one pane.
    Pane(PaneId),
    /// A binary split. `ratio` is the fraction of the split's extent
    /// (width for `Vertical`, height for `Horizontal`) given to `first`;
    /// `second` gets the remainder.
    Split {
        /// Split orientation.
        direction: SplitDirection,
        /// Fraction (0.0–1.0) of the split's extent given to `first`.
        ratio: f32,
        /// First child (left for `Vertical`, top for `Horizontal`).
        first: Box<LayoutTree>,
        /// Second child (right for `Vertical`, bottom for `Horizontal`).
        second: Box<LayoutTree>,
    },
}

/// Absolute geometry of one node, computed by [`LayoutTree::geometry`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneGeometry {
    /// The pane this geometry describes.
    pub pane: PaneId,
    /// Column offset from the window's left edge.
    pub x: usize,
    /// Row offset from the window's top edge.
    pub y: usize,
    /// Width in columns.
    pub width: usize,
    /// Height in rows.
    pub height: usize,
}

/// Error returned when a tree mutation targets a pane the tree does not
/// contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoSuchLeaf(pub PaneId);

/// The outcome of an absolute extent request ([`LayoutTree::set_leaf_extent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtentOutcome {
    /// An enclosing split's divider moved; the target's geometry changed.
    Adjusted,
    /// The target spans the axis outright — no divider of that orientation
    /// encloses it, so its extent is the window's, not the pane's to set.
    SpansAxis,
}

impl std::fmt::Display for NoSuchLeaf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no such pane in layout: {}", self.0)
    }
}

impl std::error::Error for NoSuchLeaf {}

impl LayoutTree {
    /// A tree holding a single pane, the shape every new window starts with.
    pub fn leaf(pane: PaneId) -> Self {
        LayoutTree::Pane(pane)
    }

    /// Every pane id in the tree, in left-to-right / top-to-bottom order.
    ///
    /// Existing call sites (`server.rs`'s `ListPanes`/`NewSession` handlers)
    /// consume a flat `Vec<PaneId>`; this is the drop-in replacement for the
    /// flat `MuxWindow.panes` field those sites read today.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::new();
        self.collect_pane_ids(&mut ids);
        ids
    }

    fn collect_pane_ids(&self, out: &mut Vec<PaneId>) {
        match self {
            LayoutTree::Pane(id) => out.push(*id),
            LayoutTree::Split { first, second, .. } => {
                first.collect_pane_ids(out);
                second.collect_pane_ids(out);
            }
        }
    }

    /// Split the leaf holding `target` into two panes, `target` on one side
    /// and `new_pane` on the other, dividing the leaf's extent by `ratio`
    /// (the fraction given to `target`'s side).
    ///
    /// Returns [`NoSuchLeaf`] if `target` is not a leaf in this tree —
    /// mirrors [`crate::mux::tree::MuxTree::kill_pane`]'s
    /// error-not-panic convention for an unknown pane.
    pub fn split_pane(
        &mut self,
        target: PaneId,
        new_pane: PaneId,
        direction: SplitDirection,
        ratio: f32,
    ) -> Result<(), NoSuchLeaf> {
        match self {
            LayoutTree::Pane(id) if *id == target => {
                *self = LayoutTree::Split {
                    direction,
                    ratio,
                    first: Box::new(LayoutTree::Pane(target)),
                    second: Box::new(LayoutTree::Pane(new_pane)),
                };
                Ok(())
            }
            LayoutTree::Pane(_) => Err(NoSuchLeaf(target)),
            LayoutTree::Split { first, second, .. } => first
                .split_pane(target, new_pane, direction, ratio)
                .or_else(|_| second.split_pane(target, new_pane, direction, ratio)),
        }
    }

    /// Swap the positions of two leaf panes in the tree, leaving the split
    /// structure and ratios unchanged.
    ///
    /// Returns [`NoSuchLeaf`] naming whichever of `a`/`b` was not found; if
    /// neither is present the first (`a`) is reported.
    pub fn swap_pane(&mut self, a: PaneId, b: PaneId) -> Result<(), NoSuchLeaf> {
        if a == b {
            return if self.contains(a) {
                Ok(())
            } else {
                Err(NoSuchLeaf(a))
            };
        }
        let has_a = self.contains(a);
        let has_b = self.contains(b);
        if !has_a {
            return Err(NoSuchLeaf(a));
        }
        if !has_b {
            return Err(NoSuchLeaf(b));
        }
        self.swap_pane_unchecked(a, b);
        Ok(())
    }

    fn contains(&self, target: PaneId) -> bool {
        match self {
            LayoutTree::Pane(id) => *id == target,
            LayoutTree::Split { first, second, .. } => {
                first.contains(target) || second.contains(target)
            }
        }
    }

    fn swap_pane_unchecked(&mut self, a: PaneId, b: PaneId) {
        match self {
            LayoutTree::Pane(id) if *id == a => *id = b,
            LayoutTree::Pane(id) if *id == b => *id = a,
            LayoutTree::Pane(_) => {}
            LayoutTree::Split { first, second, .. } => {
                first.swap_pane_unchecked(a, b);
                second.swap_pane_unchecked(a, b);
            }
        }
    }

    /// Remove `target`, collapsing its parent split into the sibling.
    ///
    /// Required to preserve [`crate::mux::tree::MuxTree::kill_pane`]'s
    /// existing cascading-close behavior once a window holds a tree instead
    /// of a flat pane list: removing one side of a split promotes the other
    /// side into the parent's place, exactly as removing an element from
    /// the middle of the old `Vec<PaneId>` did.
    ///
    /// Returns [`NoSuchLeaf`] if `target` is not in the tree, or if `self`
    /// is the single-pane tree holding `target` — a tree with no panes left
    /// has no valid `LayoutTree` value, so the caller (the window's owner)
    /// must detect and remove the whole window in that case, exactly as it
    /// already does when the old flat `panes` list became empty.
    pub fn remove_pane(&mut self, target: PaneId) -> Result<(), NoSuchLeaf> {
        let placeholder = LayoutTree::Pane(target);
        match std::mem::replace(self, placeholder) {
            LayoutTree::Pane(id) => {
                *self = LayoutTree::Pane(id);
                Err(NoSuchLeaf(target))
            }
            LayoutTree::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if matches!(*first, LayoutTree::Pane(id) if id == target) {
                    *self = *second;
                    Ok(())
                } else if matches!(*second, LayoutTree::Pane(id) if id == target) {
                    *self = *first;
                    Ok(())
                } else {
                    let mut first = first;
                    let mut second = second;
                    let result = if first.contains(target) {
                        first.remove_pane(target)
                    } else if second.contains(target) {
                        second.remove_pane(target)
                    } else {
                        Err(NoSuchLeaf(target))
                    };
                    *self = LayoutTree::Split {
                        direction,
                        ratio,
                        first,
                        second,
                    };
                    result
                }
            }
        }
    }

    /// Set the fraction of its bordering split's extent that `target`
    /// receives, clamping to `[0.0, 1.0]`; the sibling keeps the complement.
    ///
    /// The replacement for a first-child-only ratio setter: a pane on
    /// either side of its bordering split can be resized, and the share is
    /// stated from the TARGET's perspective regardless of which side it
    /// occupies. Returns [`NoSuchLeaf`] if `target` is absent or borders no
    /// split directly (a single-pane tree).
    pub fn set_bordering_share(&mut self, target: PaneId, share: f32) -> Result<(), NoSuchLeaf> {
        let clamped = share.clamp(0.0, 1.0);
        match self {
            LayoutTree::Pane(_) => Err(NoSuchLeaf(target)),
            LayoutTree::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if matches!(first.as_ref(), LayoutTree::Pane(id) if *id == target) {
                    *ratio = clamped;
                    Ok(())
                } else if matches!(second.as_ref(), LayoutTree::Pane(id) if *id == target) {
                    *ratio = 1.0 - clamped;
                    Ok(())
                } else {
                    first
                        .set_bordering_share(target, share)
                        .or_else(|_| second.set_bordering_share(target, share))
                }
            }
        }
    }

    /// The orientation, current ratio, and side of the split the leaf
    /// `target` directly borders — the split [`Self::set_bordering_share`]
    /// adjusts.
    ///
    /// The third tuple element is `true` when the target is the split's
    /// `first` child, `false` when it is the `second`. `None` when `target`
    /// does not border a split directly (including a single-pane tree), so
    /// a caller can distinguish "nothing to resize" before computing a new
    /// share.
    pub fn bordering_split(&self, target: PaneId) -> Option<(SplitDirection, f32, bool)> {
        match self {
            LayoutTree::Pane(_) => None,
            LayoutTree::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if matches!(first.as_ref(), LayoutTree::Pane(id) if *id == target) {
                    Some((*direction, *ratio, true))
                } else if matches!(second.as_ref(), LayoutTree::Pane(id) if *id == target) {
                    Some((*direction, *ratio, false))
                } else {
                    first
                        .bordering_split(target)
                        .or_else(|| second.bordering_split(target))
                }
            }
        }
    }

    /// Set the leaf `target`'s extent along `axis` (width for
    /// [`SplitDirection::Vertical`], height for
    /// [`SplitDirection::Horizontal`]) to `cells`, by adjusting its
    /// innermost enclosing split of that orientation.
    ///
    /// `axis_extent` is the whole layout's extent along `axis` (the
    /// window's width or height — the caller owns that fact, the tree
    /// does not), used both to locate shares and to convert `cells` to a
    /// ratio. Nested cross-orientation layouts are handled: a pane whose
    /// direct parent splits the other way still has its width set by the
    /// nearest same-axis ancestor. Returns [`NoSuchLeaf`] when `target` is
    /// absent; [`ExtentOutcome::SpansAxis`] when no ancestor split of the
    /// axis encloses the target — a pane that already spans that axis has
    /// no divider to move.
    pub fn set_leaf_extent(
        &mut self,
        target: PaneId,
        axis: SplitDirection,
        cells: u16,
        axis_extent: usize,
    ) -> Result<ExtentOutcome, NoSuchLeaf> {
        match self {
            LayoutTree::Pane(id) => {
                if *id == target {
                    Ok(ExtentOutcome::SpansAxis)
                } else {
                    Err(NoSuchLeaf(target))
                }
            }
            LayoutTree::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                // Each child's extent along `axis`: divided when this split
                // runs along the axis, inherited unchanged otherwise.
                let split_ratio = *ratio;
                let (first_extent, second_extent) = match direction {
                    SplitDirection::Vertical => {
                        let first_width = (((axis_extent as f32) * split_ratio).round() as usize)
                            .min(axis_extent);
                        if axis == SplitDirection::Vertical {
                            (first_width, axis_extent - first_width)
                        } else {
                            (axis_extent, axis_extent)
                        }
                    }
                    SplitDirection::Horizontal => {
                        let first_height = (((axis_extent as f32) * split_ratio).round() as usize)
                            .min(axis_extent);
                        if axis == SplitDirection::Horizontal {
                            (first_height, axis_extent - first_height)
                        } else {
                            (axis_extent, axis_extent)
                        }
                    }
                };
                if !first.contains(target) && !second.contains(target) {
                    return Err(NoSuchLeaf(target));
                }
                let (child, child_extent, target_is_first) = if first.contains(target) {
                    (first, first_extent, true)
                } else {
                    (second, second_extent, false)
                };
                match child.set_leaf_extent(target, axis, cells, child_extent)? {
                    ExtentOutcome::Adjusted => Ok(ExtentOutcome::Adjusted),
                    ExtentOutcome::SpansAxis => {
                        if *direction == axis {
                            let share = (cells as f32 / axis_extent as f32).clamp(0.0, 1.0);
                            *ratio = if target_is_first { share } else { 1.0 - share };
                            Ok(ExtentOutcome::Adjusted)
                        } else {
                            // This split divides the other axis; keep
                            // bubbling until a same-axis ancestor — or the
                            // root — answers.
                            Ok(ExtentOutcome::SpansAxis)
                        }
                    }
                }
            }
        }
    }

    /// Compute the absolute geometry of every pane in the tree, given the
    /// window's overall top-left corner and extent.
    ///
    /// Mirrors the algorithm `par-term`'s `PaneManager::recalculate_bounds`
    /// already computes for the client's own tree: each split divides its
    /// rect by `ratio` along its direction's axis, recursing into both
    /// children with their sub-rects.
    pub fn geometry(&self, x: usize, y: usize, width: usize, height: usize) -> Vec<PaneGeometry> {
        let mut out = Vec::new();
        self.geometry_into(
            Rect {
                x,
                y,
                width,
                height,
            },
            &mut out,
        );
        out
    }

    fn geometry_into(&self, rect: Rect, out: &mut Vec<PaneGeometry>) {
        match self {
            LayoutTree::Pane(pane) => out.push(PaneGeometry {
                pane: *pane,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            }),
            LayoutTree::Split { .. } => {
                let (first, first_rect, second, second_rect) = self.split_rects(rect);
                first.geometry_into(first_rect, out);
                second.geometry_into(second_rect, out);
            }
        }
    }

    /// Divide `rect` between this split's two children along its
    /// direction's axis, by `ratio`. Panics if `self` is not
    /// [`LayoutTree::Split`] — an internal helper shared by [`Self::geometry`]
    /// and [`Self::render`], both of which only call it on a `Split`.
    fn split_rects(&self, rect: Rect) -> (&LayoutTree, Rect, &LayoutTree, Rect) {
        let LayoutTree::Split {
            direction,
            ratio,
            first,
            second,
        } = self
        else {
            unreachable!("split_rects called on a non-Split node");
        };
        match direction {
            SplitDirection::Vertical => {
                let first_width = (((rect.width as f32) * ratio).round() as usize).min(rect.width);
                let second_width = rect.width.saturating_sub(first_width);
                (
                    first,
                    Rect {
                        width: first_width,
                        ..rect
                    },
                    second,
                    Rect {
                        x: rect.x + first_width,
                        width: second_width,
                        ..rect
                    },
                )
            }
            SplitDirection::Horizontal => {
                let first_height =
                    (((rect.height as f32) * ratio).round() as usize).min(rect.height);
                let second_height = rect.height.saturating_sub(first_height);
                (
                    first,
                    Rect {
                        height: first_height,
                        ..rect
                    },
                    second,
                    Rect {
                        y: rect.y + first_height,
                        height: second_height,
                        ..rect
                    },
                )
            }
        }
    }

    /// Render the tmux wire layout grammar (`WIDTHxHEIGHT,X,Y{...}`/`[...]`/
    /// `,ID`) for this tree, given the window's top-left corner and extent —
    /// the same parameters [`Self::geometry`] takes.
    ///
    /// The string is prefixed with a placeholder 4-hex-digit checksum
    /// (`0000,...`): `TmuxLayout::parse` never validates it (only checks the
    /// shape), and tmux itself only uses it to bust a client-side cache, so
    /// there is nothing for a real checksum to protect here. See Decision 1
    /// in `par-mux.md`.
    ///
    /// A run of sibling splits sharing direction — `Split(Split(A,B,d),C,d)`
    /// — collapses into one N-ary `{...}`/`[...]` group with 3+ children,
    /// the inverse of what `par-term`'s `rebuild_multi_split_to_binary`
    /// already does converting an incoming wire layout to nested binary.
    pub fn render(&self, x: usize, y: usize, width: usize, height: usize) -> String {
        format!(
            "0000,{}",
            self.render_node(Rect {
                x,
                y,
                width,
                height
            })
        )
    }

    fn render_node(&self, rect: Rect) -> String {
        match self {
            LayoutTree::Pane(pane) => format!(
                "{}x{},{},{},{}",
                rect.width, rect.height, rect.x, rect.y, pane.0
            ),
            LayoutTree::Split { direction, .. } => {
                let mut children = Vec::new();
                self.collect_same_direction(rect, *direction, &mut children);
                let (open, close) = match direction {
                    SplitDirection::Vertical => ('{', '}'),
                    SplitDirection::Horizontal => ('[', ']'),
                };
                format!(
                    "{}x{},{},{}{open}{}{close}",
                    rect.width,
                    rect.height,
                    rect.x,
                    rect.y,
                    children.join(",")
                )
            }
        }
    }

    /// Collect the rendered strings of every child in this node's
    /// same-`direction` split chain, recursing through nested splits that
    /// share `direction` and rendering (via [`Self::render_node`], which can
    /// itself collapse an inner chain of a *different* direction) any child
    /// that does not.
    fn collect_same_direction(&self, rect: Rect, direction: SplitDirection, out: &mut Vec<String>) {
        match self {
            LayoutTree::Split { direction: d, .. } if *d == direction => {
                let (first, first_rect, second, second_rect) = self.split_rects(rect);
                first.collect_same_direction(first_rect, direction, out);
                second.collect_same_direction(second_rect, direction, out);
            }
            other => out.push(other.render_node(rect)),
        }
    }
}

/// An absolute window-relative rectangle, in columns/rows — the internal
/// unit [`LayoutTree::geometry`] and [`LayoutTree::render`] recurse over.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_pane_ids_returns_the_single_pane() {
        let tree = LayoutTree::leaf(PaneId(0));
        assert_eq!(tree.pane_ids(), vec![PaneId(0)]);
    }

    #[test]
    fn split_pane_turns_a_leaf_into_a_binary_split() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .expect("splits the only leaf");
        assert_eq!(tree.pane_ids(), vec![PaneId(0), PaneId(1)]);
        match &tree {
            LayoutTree::Split {
                direction, ratio, ..
            } => {
                assert_eq!(*direction, SplitDirection::Vertical);
                assert_eq!(*ratio, 0.5);
            }
            other => panic!("expected Split, got {other:?}"),
        }
    }

    #[test]
    fn split_pane_on_an_unknown_target_is_an_error_not_a_panic() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        let err = tree
            .split_pane(PaneId(9), PaneId(1), SplitDirection::Vertical, 0.5)
            .expect_err("target does not exist");
        assert_eq!(err, NoSuchLeaf(PaneId(9)));
        // The tree is unchanged on failure.
        assert_eq!(tree.pane_ids(), vec![PaneId(0)]);
    }

    #[test]
    fn nested_split_creates_three_panes_side_by_side() {
        // Three side-by-side panes is Split(Split(A,B), C), not a 3-element
        // Vec — the binary-only shape the design commits to.
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.split_pane(PaneId(1), PaneId(2), SplitDirection::Vertical, 0.5)
            .unwrap();
        assert_eq!(tree.pane_ids(), vec![PaneId(0), PaneId(1), PaneId(2)]);
    }

    #[test]
    fn remove_pane_collapses_a_split_into_its_sibling() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.remove_pane(PaneId(0)).expect("0 is a leaf");
        assert_eq!(tree, LayoutTree::Pane(PaneId(1)));
    }

    #[test]
    fn remove_pane_from_a_nested_split_promotes_the_sibling_in_place() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.split_pane(PaneId(1), PaneId(2), SplitDirection::Vertical, 0.5)
            .unwrap();
        // Tree is Split(Pane(0), Split(Pane(1), Pane(2))). Removing 1 should
        // leave Split(Pane(0), Pane(2)) — the nested split collapses without
        // disturbing the outer one.
        tree.remove_pane(PaneId(1)).expect("1 is a leaf");
        assert_eq!(tree.pane_ids(), vec![PaneId(0), PaneId(2)]);
    }

    #[test]
    fn remove_pane_on_the_sole_remaining_pane_is_an_error() {
        // A tree with no panes left has no valid LayoutTree value — the
        // caller (the window's owner) must detect and remove the whole
        // window instead, exactly as it did when the old flat `panes` list
        // became empty.
        let mut tree = LayoutTree::leaf(PaneId(0));
        let err = tree.remove_pane(PaneId(0)).expect_err("last pane");
        assert_eq!(err, NoSuchLeaf(PaneId(0)));
        assert_eq!(
            tree,
            LayoutTree::Pane(PaneId(0)),
            "tree unchanged on failure"
        );
    }

    #[test]
    fn remove_pane_on_an_unknown_target_is_an_error() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        let err = tree.remove_pane(PaneId(9)).expect_err("9 absent");
        assert_eq!(err, NoSuchLeaf(PaneId(9)));
        assert_eq!(
            tree.pane_ids(),
            vec![PaneId(0), PaneId(1)],
            "tree unchanged"
        );
    }

    #[test]
    fn swap_pane_exchanges_two_leaves() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.swap_pane(PaneId(0), PaneId(1)).expect("both exist");
        // Order in the tree traversal is now reversed: 1 (was first) then 0.
        assert_eq!(tree.pane_ids(), vec![PaneId(1), PaneId(0)]);
    }

    #[test]
    fn swap_pane_with_an_unknown_id_is_an_error() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        let err = tree.swap_pane(PaneId(0), PaneId(9)).expect_err("9 absent");
        assert_eq!(err, NoSuchLeaf(PaneId(9)));
    }

    #[test]
    fn set_bordering_share_adjusts_the_split_from_either_side() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        // The `first` child's share is the split's ratio directly.
        tree.set_bordering_share(PaneId(0), 0.75)
            .expect("0 borders a split");
        match &tree {
            LayoutTree::Split { ratio, .. } => assert_eq!(*ratio, 0.75),
            other => panic!("expected Split, got {other:?}"),
        }
        // The `second` child's share is the complement — 0.25 means the
        // ratio keeps 0.75 for the first child.
        tree.set_bordering_share(PaneId(1), 0.25)
            .expect("1 borders the same split");
        match &tree {
            LayoutTree::Split { ratio, .. } => assert_eq!(*ratio, 0.75),
            other => panic!("expected Split, got {other:?}"),
        }
    }

    #[test]
    fn set_bordering_share_clamps_out_of_range_shares() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.set_bordering_share(PaneId(0), 5.0).unwrap();
        match &tree {
            LayoutTree::Split { ratio, .. } => assert_eq!(*ratio, 1.0),
            other => panic!("expected Split, got {other:?}"),
        }
    }

    #[test]
    fn set_bordering_share_on_an_absent_or_lone_pane_is_an_error() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        assert_eq!(
            tree.set_bordering_share(PaneId(9), 0.75),
            Err(NoSuchLeaf(PaneId(9)))
        );
        assert_eq!(
            LayoutTree::leaf(PaneId(0)).set_bordering_share(PaneId(0), 0.75),
            Err(NoSuchLeaf(PaneId(0)))
        );
    }

    #[test]
    fn bordering_split_reports_the_split_and_side_the_target_borders() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Horizontal, 0.25)
            .unwrap();
        assert_eq!(
            tree.bordering_split(PaneId(0)),
            Some((SplitDirection::Horizontal, 0.25, true))
        );
        assert_eq!(
            tree.bordering_split(PaneId(1)),
            Some((SplitDirection::Horizontal, 0.25, false))
        );
        // An absent pane and a lone leaf border nothing.
        assert_eq!(tree.bordering_split(PaneId(9)), None);
        assert_eq!(LayoutTree::leaf(PaneId(0)).bordering_split(PaneId(0)), None);
    }

    #[test]
    fn set_leaf_extent_sets_a_direct_childs_width() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        assert_eq!(
            tree.set_leaf_extent(PaneId(0), SplitDirection::Vertical, 20, 80),
            Ok(ExtentOutcome::Adjusted)
        );
        let geo = tree.geometry(0, 0, 80, 24);
        assert_eq!(geo[0].width, 20);
        assert_eq!(geo[1].width, 60);
    }

    #[test]
    fn set_leaf_extent_works_for_the_second_child() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        assert_eq!(
            tree.set_leaf_extent(PaneId(1), SplitDirection::Vertical, 30, 80),
            Ok(ExtentOutcome::Adjusted)
        );
        let geo = tree.geometry(0, 0, 80, 24);
        assert_eq!(geo[1].width, 30);
        assert_eq!(geo[0].width, 50);
    }

    #[test]
    fn set_leaf_extent_reaches_a_cross_orientation_ancestor() {
        // Split(V){0, Split(H){1,2}}: pane 1's width is governed by the
        // outer vertical split even though its direct parent is horizontal.
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.split_pane(PaneId(1), PaneId(2), SplitDirection::Horizontal, 0.5)
            .unwrap();
        assert_eq!(
            tree.set_leaf_extent(PaneId(1), SplitDirection::Vertical, 20, 80),
            Ok(ExtentOutcome::Adjusted)
        );
        let geo = tree.geometry(0, 0, 80, 24);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(width_of(PaneId(0)), 60);
        assert_eq!(width_of(PaneId(1)), 20);
        assert_eq!(width_of(PaneId(2)), 20);
    }

    #[test]
    fn set_leaf_extent_on_a_spanning_pane_reports_no_divider() {
        // A lone pane spans both axes; a stacked pane spans the width.
        let mut lone = LayoutTree::leaf(PaneId(0));
        assert_eq!(
            lone.set_leaf_extent(PaneId(0), SplitDirection::Vertical, 40, 80),
            Ok(ExtentOutcome::SpansAxis)
        );
        let mut stacked = LayoutTree::leaf(PaneId(0));
        stacked
            .split_pane(PaneId(0), PaneId(1), SplitDirection::Horizontal, 0.5)
            .unwrap();
        assert_eq!(
            stacked.set_leaf_extent(PaneId(0), SplitDirection::Vertical, 40, 80),
            Ok(ExtentOutcome::SpansAxis),
            "a horizontal split leaves the width undivided"
        );
    }

    #[test]
    fn set_leaf_extent_on_an_absent_pane_is_an_error() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        assert_eq!(
            tree.set_leaf_extent(PaneId(9), SplitDirection::Vertical, 40, 80),
            Err(NoSuchLeaf(PaneId(9)))
        );
    }

    #[test]
    fn geometry_of_a_single_pane_fills_the_window() {
        let tree = LayoutTree::leaf(PaneId(0));
        let geo = tree.geometry(0, 0, 80, 24);
        assert_eq!(geo.len(), 1);
        assert_eq!(
            geo[0],
            PaneGeometry {
                pane: PaneId(0),
                x: 0,
                y: 0,
                width: 80,
                height: 24
            }
        );
    }

    #[test]
    fn geometry_of_a_vertical_split_divides_width() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        let geo = tree.geometry(0, 0, 80, 24);
        assert_eq!(geo.len(), 2);
        assert_eq!(geo[0].pane, PaneId(0));
        assert_eq!(geo[0].x, 0);
        assert_eq!(geo[0].width, 40);
        assert_eq!(geo[0].height, 24);
        assert_eq!(geo[1].pane, PaneId(1));
        assert_eq!(geo[1].x, 40);
        assert_eq!(geo[1].width, 40);
        assert_eq!(geo[1].height, 24);
    }

    #[test]
    fn geometry_of_a_horizontal_split_divides_height() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Horizontal, 0.25)
            .unwrap();
        let geo = tree.geometry(0, 0, 80, 24);
        assert_eq!(geo[0].pane, PaneId(0));
        assert_eq!(geo[0].y, 0);
        assert_eq!(geo[0].height, 6); // 24 * 0.25 = 6
        assert_eq!(geo[1].pane, PaneId(1));
        assert_eq!(geo[1].y, 6);
        assert_eq!(geo[1].height, 18);
    }

    #[test]
    fn geometry_of_nested_splits_produces_three_non_overlapping_panes() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.split_pane(PaneId(1), PaneId(2), SplitDirection::Vertical, 0.5)
            .unwrap();
        let geo = tree.geometry(0, 0, 80, 24);
        assert_eq!(geo.len(), 3);
        // Widths sum to the total; no overlap (each starts where the
        // previous ended).
        let total_width: usize = geo.iter().map(|g| g.width).sum();
        assert_eq!(total_width, 80);
        assert_eq!(geo[0].x, 0);
        assert_eq!(geo[1].x, geo[0].x + geo[0].width);
        assert_eq!(geo[2].x, geo[1].x + geo[1].width);
    }

    #[test]
    fn render_of_a_single_pane_has_a_checksum_prefix_and_no_group() {
        let tree = LayoutTree::leaf(PaneId(0));
        assert_eq!(tree.render(0, 0, 89, 24), "0000,89x24,0,0,0");
    }

    #[test]
    fn render_of_a_vertical_split_uses_curly_braces() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        let rendered = tree.render(0, 0, 89, 24);
        assert!(rendered.starts_with("0000,89x24,0,0{"));
        assert!(rendered.ends_with('}'));
        assert_eq!(rendered, "0000,89x24,0,0{45x24,0,0,0,44x24,45,0,1}");
    }

    #[test]
    fn render_of_a_horizontal_split_uses_square_brackets() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Horizontal, 0.5)
            .unwrap();
        let rendered = tree.render(0, 0, 89, 24);
        assert!(rendered.starts_with("0000,89x24,0,0["));
        assert!(rendered.ends_with(']'));
    }

    #[test]
    fn render_of_three_same_direction_splits_collapses_to_one_group() {
        // Split(Split(A,B,Vertical),C,Vertical) — three side-by-side panes —
        // must render as ONE {...} group with 3 children, not a nested
        // {...{...}} shape. This is the inverse of par-term's
        // rebuild_multi_split_to_binary.
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.split_pane(PaneId(1), PaneId(2), SplitDirection::Vertical, 0.5)
            .unwrap();
        let rendered = tree.render(0, 0, 90, 24);

        // Exactly one '{' / one '}' — a nested rendering would have two of each.
        assert_eq!(rendered.matches('{').count(), 1, "rendered: {rendered}");
        assert_eq!(rendered.matches('}').count(), 1, "rendered: {rendered}");
        // Exact string: proves the collapse produced the right geometry, not
        // just the right punctuation — a comma count alone is satisfiable by
        // a flat-but-wrong rendering (bad offsets, bad widths, bad IDs).
        assert_eq!(
            rendered,
            "0000,90x24,0,0{45x24,0,0,0,23x24,45,0,1,22x24,68,0,2}"
        );
    }

    #[test]
    fn render_round_trips_through_a_ported_grammar_parser() {
        for tree in [
            LayoutTree::leaf(PaneId(0)),
            {
                let mut t = LayoutTree::leaf(PaneId(0));
                t.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
                    .unwrap();
                t
            },
            {
                let mut t = LayoutTree::leaf(PaneId(0));
                t.split_pane(PaneId(0), PaneId(1), SplitDirection::Horizontal, 0.25)
                    .unwrap();
                t
            },
            {
                // Three-way collapse: the case Decision 1 exists for.
                let mut t = LayoutTree::leaf(PaneId(0));
                t.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
                    .unwrap();
                t.split_pane(PaneId(1), PaneId(2), SplitDirection::Vertical, 0.5)
                    .unwrap();
                t
            },
            {
                // Mixed directions: an outer vertical split whose second
                // child is itself a horizontal split — no collapsing across
                // the direction change.
                let mut t = LayoutTree::leaf(PaneId(0));
                t.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
                    .unwrap();
                t.split_pane(PaneId(1), PaneId(2), SplitDirection::Horizontal, 0.5)
                    .unwrap();
                t
            },
        ] {
            let expected_geo = tree.geometry(0, 0, 90, 24);
            let rendered = tree.render(0, 0, 90, 24);

            let parsed =
                ported_parser::TmuxLayout::parse(&rendered).expect("render() output parses");
            let mut parsed_panes = Vec::new();
            ported_parser::collect_panes(&parsed.root, &mut parsed_panes);
            parsed_panes.sort_by_key(|p| p.id);

            let mut expected: Vec<_> = expected_geo
                .iter()
                .map(|g| (g.pane.0, g.x, g.y, g.width, g.height))
                .collect();
            expected.sort_by_key(|g| g.0);

            assert_eq!(
                parsed_panes
                    .iter()
                    .map(|p| (p.id, p.x, p.y, p.width, p.height))
                    .collect::<Vec<_>>(),
                expected,
                "rendered: {rendered}"
            );
        }
    }

    /// A test-only port of `par-term-tmux`'s `TmuxLayout::parse`
    /// (`par-term-tmux/src/types.rs:166-284`), used ONLY to prove
    /// [`LayoutTree::render`] is self-consistent with a grammar parser that
    /// is not this crate's own code, without a reverse dependency on
    /// `par-term-tmux` (par-mux.md Phase 2 Task 2.2: the core cannot depend
    /// on `par-term-tmux`, which already depends on the core).
    ///
    /// This is a FIXTURE, not a second implementation: it never leaves
    /// `#[cfg(test)]`, and does not prove conformance with the real parser —
    /// a copy that drifts from `par-term-tmux/src/types.rs` still agrees
    /// with itself. The test that actually proves conformance is the
    /// cross-repo integration test tracked on the `par-term` project
    /// (kanban `01a0c5c74485730188f8a798fce521a1`), which calls this crate's
    /// real `LayoutTree::render()` into the real `TmuxLayout::parse`.
    mod ported_parser {
        #[derive(Debug)]
        pub struct TmuxLayout {
            pub root: LayoutNode,
        }

        #[derive(Debug)]
        pub enum LayoutNode {
            Pane {
                id: u32,
                width: usize,
                height: usize,
                x: usize,
                y: usize,
            },
            HorizontalSplit {
                children: Vec<LayoutNode>,
            },
            VerticalSplit {
                children: Vec<LayoutNode>,
            },
        }

        pub struct ParsedPane {
            pub id: u32,
            pub x: usize,
            pub y: usize,
            pub width: usize,
            pub height: usize,
        }

        pub fn collect_panes(node: &LayoutNode, out: &mut Vec<ParsedPane>) {
            match node {
                LayoutNode::Pane {
                    id,
                    width,
                    height,
                    x,
                    y,
                } => out.push(ParsedPane {
                    id: *id,
                    x: *x,
                    y: *y,
                    width: *width,
                    height: *height,
                }),
                LayoutNode::HorizontalSplit { children }
                | LayoutNode::VerticalSplit { children } => {
                    for child in children {
                        collect_panes(child, out);
                    }
                }
            }
        }

        impl TmuxLayout {
            pub fn parse(layout_str: &str) -> Option<Self> {
                let layout_str = layout_str.trim();
                let layout_str = if let Some(comma_idx) = layout_str.find(',') {
                    if comma_idx == 4 && layout_str[..4].chars().all(|c| c.is_ascii_hexdigit()) {
                        &layout_str[5..]
                    } else {
                        layout_str
                    }
                } else {
                    layout_str
                };
                if layout_str.is_empty() {
                    return None;
                }
                let (node, _) = Self::parse_node(layout_str)?;
                Some(Self { root: node })
            }

            fn parse_node(s: &str) -> Option<(LayoutNode, &str)> {
                let (width, s) = Self::parse_number(s)?;
                let s = s.strip_prefix('x')?;
                let (height, s) = Self::parse_number(s)?;
                let s = s.strip_prefix(',')?;
                let (x, s) = Self::parse_number(s)?;
                let s = s.strip_prefix(',')?;
                let (y, s) = Self::parse_number(s)?;

                if let Some(rest) = s.strip_prefix('{') {
                    let (children, rest) = Self::parse_children(rest, '}')?;
                    Some((LayoutNode::VerticalSplit { children }, rest))
                } else if let Some(rest) = s.strip_prefix('[') {
                    let (children, rest) = Self::parse_children(rest, ']')?;
                    Some((LayoutNode::HorizontalSplit { children }, rest))
                } else if let Some(rest) = s.strip_prefix(',') {
                    let (id, rest) = Self::parse_number(rest)?;
                    Some((
                        LayoutNode::Pane {
                            id: id as u32,
                            width,
                            height,
                            x,
                            y,
                        },
                        rest,
                    ))
                } else {
                    None
                }
            }

            fn parse_children(s: &str, end_char: char) -> Option<(Vec<LayoutNode>, &str)> {
                let mut children = Vec::new();
                let mut remaining = s;
                loop {
                    let (child, rest) = Self::parse_node(remaining)?;
                    children.push(child);
                    remaining = rest;
                    if remaining.starts_with(end_char) {
                        return Some((children, &remaining[1..]));
                    } else if remaining.starts_with(',') {
                        remaining = &remaining[1..];
                    } else {
                        return None;
                    }
                }
            }

            fn parse_number(s: &str) -> Option<(usize, &str)> {
                let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
                if end == 0 {
                    return None;
                }
                let num = s[..end].parse().ok()?;
                Some((num, &s[end..]))
            }
        }
    }
}
