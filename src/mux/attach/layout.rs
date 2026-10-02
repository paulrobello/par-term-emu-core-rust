//! Client-side parser for the tmux layout-string grammar.
//!
//! The daemon pushes `%layout-change @W <layout> <visible-layout> <flags>`
//! (docs/MUX.md Broadcasts); each layout string is the tmux wire grammar
//! `LAYOUT` renders: an optional 4-hex-digit checksum prefix, then either a
//! leaf `WIDTHxHEIGHT,X,Y,PANE` or an N-ary same-direction group
//! `{children}` (side-by-side) / `[children]` (stacked). The parser walks
//! the string and collects every leaf's absolute window-relative rect —
//! the same geometry the daemon's own [`crate::mux::layout::LayoutTree::
//! geometry`] computes, in the same leaf order its `render` emits (which is
//! the leaf index `list-panes -t` reports).
//!
//! Zoomed windows: the flags field carries `Z` and the *visible* layout is
//! the zoomed pane alone filling the window — [`parse_layout_triple`]
//! selects the visible string when the flag is set, so callers render what
//! a real tmux client would see.

/// One parsed leaf: a pane's absolute, window-relative rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneRect {
    /// The pane id from the layout string (the `%N` number without the
    /// sigil).
    pub pane: u32,
    /// Column offset from the window's left edge.
    pub x: u16,
    /// Row offset from the window's top edge.
    pub y: u16,
    /// Width in columns.
    pub width: u16,
    /// Height in rows.
    pub height: u16,
}

/// The error a malformed layout string produces. The message names the byte
/// offset where parsing stopped; layout strings are daemon-controlled state
/// mirrored verbatim from real tmux, so a parse failure is logged, not a
/// panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutParseError {
    /// Character offset into the input where the grammar broke.
    pub offset: usize,
    /// What the parser expected at that offset.
    pub expected: &'static str,
}

impl std::fmt::Display for LayoutParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "malformed layout string at byte {}: expected {}",
            self.offset, self.expected
        )
    }
}

impl std::error::Error for LayoutParseError {}

/// Parse one layout string into its panes' absolute rects, in leaf order.
///
/// Accepts the checksum-prefixed and bare forms (`0000,80x24,0,0,1` and
/// `80x24,0,0,1`); the checksum itself is a cache-buster, not a value the
/// grammar constrains (the daemon's own renderer always emits `0000`).
pub fn parse_layout(s: &str) -> Result<Vec<PaneRect>, LayoutParseError> {
    let mut parser = Parser {
        bytes: s.as_bytes(),
        pos: 0,
    };
    // Optional checksum prefix: exactly 4 hex digits followed by a comma.
    if parser.peek_prefix_checksum() {
        parser.pos += 5;
    }
    let mut out = Vec::new();
    parser.parse_node(&mut out)?;
    if parser.pos != parser.bytes.len() {
        return Err(parser.err("end of string"));
    }
    Ok(out)
}

/// Parse a `%layout-change` (or `%window-add`) layout triple into rects.
///
/// `flags` containing `Z` marks a zoomed window: the *visible* layout is
/// what renders (the zoomed pane filling the window), while `layout` still
/// names the underlying split — tmux's own client contract. Either string
/// may be empty on a bare `%window-add` line (docs/MUX.md); that parses as
/// no panes.
pub fn parse_layout_triple(
    layout: &str,
    visible_layout: &str,
    flags: &str,
) -> Result<Vec<PaneRect>, LayoutParseError> {
    let source = if flags.contains('Z') && !visible_layout.is_empty() {
        visible_layout
    } else {
        layout
    };
    if source.is_empty() {
        return Ok(Vec::new());
    }
    parse_layout(source)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, expected: &'static str) -> LayoutParseError {
        LayoutParseError {
            offset: self.pos,
            expected,
        }
    }

    /// Whether the remaining input starts with the `HHHH,` checksum prefix.
    fn peek_prefix_checksum(&self) -> bool {
        self.bytes.len() >= self.pos + 5
            && self.bytes[self.pos..self.pos + 4]
                .iter()
                .all(u8::is_ascii_hexdigit)
            && self.bytes[self.pos + 4] == b','
    }

    fn parse_number(&mut self) -> Result<u16, LayoutParseError> {
        let start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.err("a number"));
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| self.err("a u16"))
    }

    fn expect(&mut self, byte: u8) -> Result<(), LayoutParseError> {
        if self.pos < self.bytes.len() && self.bytes[self.pos] == byte {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err("a separator"))
        }
    }

    /// One node: a leaf or a group. Leaves append to `out` in encounter
    /// order — left-to-right for `{...}` groups, top-to-bottom for `[...]`,
    /// depth-first through nesting — the order the daemon's `render` emits
    /// and `list-panes -t` indexes.
    fn parse_node(&mut self, out: &mut Vec<PaneRect>) -> Result<(), LayoutParseError> {
        let width = self.parse_number()?;
        self.expect(b'x')?;
        let height = self.parse_number()?;
        self.expect(b',')?;
        let x = self.parse_number()?;
        self.expect(b',')?;
        let y = self.parse_number()?;

        match self.bytes.get(self.pos) {
            Some(b'{') => {
                self.pos += 1;
                self.parse_children(b'}', out)?;
            }
            Some(b'[') => {
                self.pos += 1;
                self.parse_children(b']', out)?;
            }
            Some(b',') => {
                self.pos += 1;
                let pane = self.parse_number()? as u32;
                out.push(PaneRect {
                    pane,
                    x,
                    y,
                    width,
                    height,
                });
            }
            _ => return Err(self.err("a pane id, '{', or '['")),
        }
        Ok(())
    }

    fn parse_children(
        &mut self,
        close: u8,
        out: &mut Vec<PaneRect>,
    ) -> Result<(), LayoutParseError> {
        loop {
            self.parse_node(out)?;
            match self.bytes.get(self.pos) {
                Some(&b) if b == close => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(b',') => self.pos += 1,
                _ => return Err(self.err("',' or the group's closing bracket")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(s: &str) -> Vec<PaneRect> {
        parse_layout(s).expect("parses")
    }

    /// A single pane, checksum-prefixed — real tmux's untouched-window
    /// shape and the daemon's zoomed visible layout.
    #[test]
    fn single_pane_with_checksum() {
        assert_eq!(
            rects("b7c8,238x58,0,0,0"),
            vec![PaneRect {
                pane: 0,
                x: 0,
                y: 0,
                width: 238,
                height: 58
            }]
        );
    }

    /// The daemon's own render form: `0000` checksum, two side-by-side
    /// panes tiling the window with no divider gap.
    #[test]
    fn two_pane_vertical_split_daemon_form() {
        assert_eq!(
            rects("0000,80x24,0,0{40x24,0,0,1,40x24,40,0,2}"),
            vec![
                PaneRect {
                    pane: 1,
                    x: 0,
                    y: 0,
                    width: 40,
                    height: 24
                },
                PaneRect {
                    pane: 2,
                    x: 40,
                    y: 0,
                    width: 40,
                    height: 24
                },
            ]
        );
    }

    /// Real tmux golden: a stacked split — the divider row is part of the
    /// window extent there (29 + 1 + 28 = 58), and the parser carries the
    /// strings' own coordinates through untouched.
    #[test]
    fn two_pane_horizontal_split_real_tmux_golden() {
        assert_eq!(
            rects("4b7c,238x58,0,0[238x29,0,0,0,238x28,0,30,1]"),
            vec![
                PaneRect {
                    pane: 0,
                    x: 0,
                    y: 0,
                    width: 238,
                    height: 29
                },
                PaneRect {
                    pane: 1,
                    x: 0,
                    y: 30,
                    width: 238,
                    height: 28
                },
            ]
        );
    }

    /// Real tmux golden: three side-by-side panes in one N-ary group with
    /// unequal widths and non-zero pane ids.
    #[test]
    fn three_pane_nary_group_real_tmux_golden() {
        assert_eq!(
            rects("d0e1,238x58,0,0{79x58,0,0,3,79x58,80,0,7,79x58,160,0,11}"),
            vec![
                PaneRect {
                    pane: 3,
                    x: 0,
                    y: 0,
                    width: 79,
                    height: 58
                },
                PaneRect {
                    pane: 7,
                    x: 80,
                    y: 0,
                    width: 79,
                    height: 58
                },
                PaneRect {
                    pane: 11,
                    x: 160,
                    y: 0,
                    width: 79,
                    height: 58
                },
            ]
        );
    }

    /// Real tmux golden: a nested mixed tree — vertical group whose middle
    /// child is itself a horizontal (stacked) split.
    #[test]
    fn nested_mixed_tree_real_tmux_golden() {
        assert_eq!(
            rects("e2f3,178x46,0,0{88x46,0,0,0,89x46,90,0[89x23,90,0,1,89x22,90,24,2]}"),
            vec![
                PaneRect {
                    pane: 0,
                    x: 0,
                    y: 0,
                    width: 88,
                    height: 46
                },
                PaneRect {
                    pane: 1,
                    x: 90,
                    y: 0,
                    width: 89,
                    height: 23
                },
                PaneRect {
                    pane: 2,
                    x: 90,
                    y: 24,
                    width: 89,
                    height: 22
                },
            ]
        );
    }

    /// Without the checksum the same string parses identically (the
    /// checksum is optional decoration as far as geometry goes).
    #[test]
    fn bare_form_without_checksum() {
        assert_eq!(
            rects("80x24,0,0{40x24,0,0,1,40x24,40,0,2}"),
            rects("0000,80x24,0,0{40x24,0,0,1,40x24,40,0,2}")
        );
    }

    /// Round-trip against the daemon's own renderer: for every tree the
    /// daemon can build, `LayoutTree::render` output must parse back to
    /// exactly the rects `LayoutTree::geometry` computes, in the same leaf
    /// order. This is the "same leaf order" contract the attach client
    /// depends on.
    #[test]
    fn daemon_render_round_trips_through_the_client_parser() {
        use crate::mux::ids::PaneId;
        use crate::mux::layout::{LayoutTree, SplitDirection};

        // Vertical split (side by side), 80x24.
        let mut tree = LayoutTree::leaf(PaneId(1));
        tree.split_pane(PaneId(1), PaneId(2), SplitDirection::Vertical, 0.5)
            .unwrap();
        let rendered = tree.render(0, 0, 80, 24);
        assert_eq!(
            parse_layout(&rendered).expect("parses"),
            tree.geometry(0, 0, 80, 24)
                .into_iter()
                .map(|g| PaneRect {
                    pane: g.pane.0,
                    x: g.x as u16,
                    y: g.y as u16,
                    width: g.width as u16,
                    height: g.height as u16,
                })
                .collect::<Vec<_>>()
        );

        // Horizontal split (stacked), quarter/three-quarters.
        let mut tree = LayoutTree::leaf(PaneId(5));
        tree.split_pane(PaneId(5), PaneId(6), SplitDirection::Horizontal, 0.25)
            .unwrap();
        let rendered = tree.render(0, 0, 80, 24);
        assert_eq!(
            parse_layout(&rendered).expect("parses"),
            tree.geometry(0, 0, 80, 24)
                .into_iter()
                .map(|g| PaneRect {
                    pane: g.pane.0,
                    x: g.x as u16,
                    y: g.y as u16,
                    width: g.width as u16,
                    height: g.height as u16,
                })
                .collect::<Vec<_>>()
        );

        // Nested mixed tree: three panes, two directions.
        let mut tree = LayoutTree::leaf(PaneId(1));
        tree.split_pane(PaneId(1), PaneId(2), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.split_pane(PaneId(2), PaneId(3), SplitDirection::Horizontal, 0.5)
            .unwrap();
        let rendered = tree.render(0, 0, 100, 40);
        assert_eq!(
            parse_layout(&rendered).expect("parses"),
            tree.geometry(0, 0, 100, 40)
                .into_iter()
                .map(|g| PaneRect {
                    pane: g.pane.0,
                    x: g.x as u16,
                    y: g.y as u16,
                    width: g.width as u16,
                    height: g.height as u16,
                })
                .collect::<Vec<_>>()
        );

        // A collapsed N-ary run: three same-direction siblings.
        let mut tree = LayoutTree::leaf(PaneId(1));
        tree.split_pane(PaneId(1), PaneId(2), SplitDirection::Vertical, 0.5)
            .unwrap();
        tree.split_pane(PaneId(2), PaneId(3), SplitDirection::Vertical, 0.5)
            .unwrap();
        let rendered = tree.render(0, 0, 90, 30);
        assert_eq!(
            parse_layout(&rendered).expect("parses"),
            tree.geometry(0, 0, 90, 30)
                .into_iter()
                .map(|g| PaneRect {
                    pane: g.pane.0,
                    x: g.x as u16,
                    y: g.y as u16,
                    width: g.width as u16,
                    height: g.height as u16,
                })
                .collect::<Vec<_>>()
        );
    }

    /// The zoom rule: flags `Z` selects the visible layout (the zoomed
    /// pane alone) over the underlying split; without the flag the layout
    /// string wins.
    #[test]
    fn zoomed_triple_uses_the_visible_layout() {
        let layout = "0000,80x24,0,0{40x24,0,0,1,40x24,40,0,2}";
        let visible = "0000,80x24,0,0,2";

        assert_eq!(
            parse_layout_triple(layout, visible, "Z").expect("parses"),
            vec![PaneRect {
                pane: 2,
                x: 0,
                y: 0,
                width: 80,
                height: 24
            }]
        );
        assert_eq!(
            parse_layout_triple(layout, visible, "")
                .expect("parses")
                .len(),
            2,
            "unzoomed renders the underlying split"
        );
    }

    /// A bare `%window-add` line's empty triple parses as no panes rather
    /// than an error (docs/MUX.md: the triple is optional there).
    #[test]
    fn empty_triple_parses_as_no_panes() {
        assert!(parse_layout_triple("", "", "").expect("parses").is_empty());
    }

    /// Malformed strings error with an offset instead of panicking — the
    /// string is wire-controlled state, and a bad line must degrade to
    /// "keep the previous layout".
    #[test]
    fn malformed_strings_error_with_offsets() {
        for (bad, expected_at) in [
            ("", 0usize),
            ("80x24,0,0", 9),
            ("80x24,0,0,", 10),
            ("80x24,0,0{40x24,0,0,1", 21),
            ("x24,0,0,1", 0),
            ("80q24,0,0,1", 2),
            ("80x24,0,0,1junk", 11),
        ] {
            let err = parse_layout(bad).expect_err(bad);
            assert_eq!(err.offset, expected_at, "input {bad:?}");
        }
    }

    /// Overflowing dimension numbers error rather than saturate — a
    /// silently-clamped rect would render a wrong grid.
    #[test]
    fn overflowing_dimensions_error() {
        assert!(parse_layout("99999x24,0,0,1").is_err());
    }
}
