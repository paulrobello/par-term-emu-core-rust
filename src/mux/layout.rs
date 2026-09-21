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
pub enum SplitDirection {
    /// Panes are stacked vertically (split creates top/bottom panes).
    Horizontal,
    /// Panes are side by side (split creates left/right panes).
    Vertical,
}

/// A window's interior pane structure: either a single pane, or a binary
/// split into two sub-trees with a ratio dividing them.
#[derive(Debug, Clone, PartialEq)]
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

    /// Adjust the ratio of the split whose `first` child is the leaf holding
    /// `target`, clamping to `[0.0, 1.0]`.
    ///
    /// tmux's `resize-pane` resizes the split a pane participates in;
    /// finding that split by one of its two sides (rather than requiring a
    /// split-identifying id the tree has no concept of) is the natural
    /// mapping onto this binary shape. Returns [`NoSuchLeaf`] if `target`
    /// does not directly border a split as its `first` child.
    pub fn resize_pane(&mut self, target: PaneId, new_ratio: f32) -> Result<(), NoSuchLeaf> {
        let clamped = new_ratio.clamp(0.0, 1.0);
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
                } else {
                    first
                        .resize_pane(target, new_ratio)
                        .or_else(|_| second.resize_pane(target, new_ratio))
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
        self.geometry_into(x, y, width, height, &mut out);
        out
    }

    fn geometry_into(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        out: &mut Vec<PaneGeometry>,
    ) {
        match self {
            LayoutTree::Pane(pane) => out.push(PaneGeometry {
                pane: *pane,
                x,
                y,
                width,
                height,
            }),
            LayoutTree::Split {
                direction,
                ratio,
                first,
                second,
            } => match direction {
                SplitDirection::Vertical => {
                    let first_width = ((width as f32) * ratio).round() as usize;
                    let first_width = first_width.min(width);
                    let second_width = width.saturating_sub(first_width);
                    first.geometry_into(x, y, first_width, height, out);
                    second.geometry_into(x + first_width, y, second_width, height, out);
                }
                SplitDirection::Horizontal => {
                    let first_height = ((height as f32) * ratio).round() as usize;
                    let first_height = first_height.min(height);
                    let second_height = height.saturating_sub(first_height);
                    first.geometry_into(x, y, width, first_height, out);
                    second.geometry_into(x, y + first_height, width, second_height, out);
                }
            },
        }
    }
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
    fn resize_pane_adjusts_the_bordering_splits_ratio() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.resize_pane(PaneId(0), 0.75)
            .expect("0 borders a split");
        match &tree {
            LayoutTree::Split { ratio, .. } => assert_eq!(*ratio, 0.75),
            other => panic!("expected Split, got {other:?}"),
        }
    }

    #[test]
    fn resize_pane_clamps_out_of_range_ratios() {
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.resize_pane(PaneId(0), 5.0).unwrap();
        match &tree {
            LayoutTree::Split { ratio, .. } => assert_eq!(*ratio, 1.0),
            other => panic!("expected Split, got {other:?}"),
        }
    }

    #[test]
    fn resize_pane_on_a_pane_not_bordering_a_split_is_an_error() {
        // 0 borders the split as `first`; resizing via `1` (the `second`
        // child) is not supported by this mapping — tmux identifies the
        // split by either border in practice, but the simplest binary
        // mapping picks one side deliberately; verify it fails rather than
        // silently doing nothing.
        let mut tree = LayoutTree::leaf(PaneId(0));
        tree.split_pane(PaneId(0), PaneId(1), SplitDirection::Vertical, 0.5)
            .unwrap();
        assert!(tree.resize_pane(PaneId(1), 0.75).is_err());
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
}
