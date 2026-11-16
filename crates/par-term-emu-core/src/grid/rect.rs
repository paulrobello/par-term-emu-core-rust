//! Rectangular operations for the terminal grid

use crate::cell::{Cell, PackedColor};
use crate::color::{Color, NamedColor};
use crate::grid::Grid;

impl Grid {
    /// Fill a rectangular area with a character
    pub fn fill_rectangle(
        &mut self,
        fill_cell: Cell,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) {
        if top >= self.rows || left >= self.cols {
            return;
        }
        let bottom = bottom.min(self.rows - 1);
        let right = right.min(self.cols - 1);
        if top > bottom || left > right {
            return;
        }

        for row in top..=bottom {
            // The rectangle's edges orphan the pair halves just outside it
            // when the inside half is overwritten; blank them first.
            self.split_wide_chars_at(row, left, right + 1, fill_cell.bg);
            for col in left..=right {
                if let Some(cell) = self.get_mut(col, row) {
                    *cell = fill_cell.clone();
                }
            }
        }
    }

    /// Copy a rectangular area to another location
    pub fn copy_rectangle(
        &mut self,
        src_top: usize,
        src_left: usize,
        src_bottom: usize,
        src_right: usize,
        dst_top: usize,
        dst_left: usize,
    ) {
        if src_top >= self.rows || src_left >= self.cols {
            return;
        }
        let src_bottom = src_bottom.min(self.rows - 1);
        let src_right = src_right.min(self.cols - 1);
        if src_top > src_bottom || src_left > src_right {
            return;
        }

        let height = src_bottom - src_top + 1;
        let width = src_right - src_left + 1;

        if dst_top >= self.rows || dst_left >= self.cols {
            return;
        }
        let dst_bottom = (dst_top + height - 1).min(self.rows - 1);
        let dst_right = (dst_left + width - 1).min(self.cols - 1);

        let mut buffer = Vec::with_capacity(height * width);
        for row in src_top..=src_bottom {
            for col in src_left..=src_right {
                if let Some(cell) = self.get(col, row) {
                    buffer.push(cell.clone());
                }
            }
        }

        let mut buffer_idx = 0;
        for row in dst_top..=dst_bottom {
            for col in dst_left..=dst_right {
                if buffer_idx < buffer.len() {
                    if let Some(cell) = self.get_mut(col, row) {
                        *cell = buffer[buffer_idx].clone();
                    }
                    buffer_idx += 1;
                }
            }
            // A source edge that cut a wide pair copies one lone half; blank
            // it, and blank the outside halves the copy orphaned.
            self.fix_copy_pair_edges(row, dst_left, dst_right);
        }
    }

    /// After a rectangle copy into `[left, right]` on `row`, blank the pair
    /// halves the copy left without their partners: a lone half at either
    /// edge of the copied span, and the outside halves whose partner the
    /// copy overwrote.
    fn fix_copy_pair_edges(&mut self, row: usize, left: usize, right: usize) {
        if row >= self.rows || left > right || right >= self.cols {
            return;
        }
        let cells = &mut self.screen[row];
        // Left edge: a copied spacer with no base beside it is lone; a copied
        // non-spacer orphans the base just outside the span.
        if cells[left].flags.wide_char_spacer() && (left == 0 || !cells[left - 1].flags.wide_char())
        {
            cells[left].reset();
        } else if !cells[left].flags.wide_char_spacer()
            && left > 0
            && cells[left - 1].flags.wide_char()
        {
            cells[left - 1].reset();
        }
        // Right edge: a copied base with no spacer beside it is lone; a
        // copied non-spacer orphans the spacer just outside the span.
        if cells[right].flags.wide_char()
            && (right + 1 >= self.cols || !cells[right + 1].flags.wide_char_spacer())
        {
            cells[right].reset();
        } else if !cells[right].flags.wide_char()
            && right + 1 < self.cols
            && cells[right + 1].flags.wide_char_spacer()
        {
            cells[right + 1].reset();
        }
    }

    /// Erase a rectangular area unconditionally
    pub fn erase_rectangle_unconditional(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) {
        if top >= self.rows || left >= self.cols {
            return;
        }
        let bottom = bottom.min(self.rows - 1);
        let right = right.min(self.cols - 1);
        if top > bottom || left > right {
            return;
        }

        for row in top..=bottom {
            // Same edge rule as the fill: halves just outside the rectangle
            // lose their overwritten partners.
            self.split_wide_chars_at(
                row,
                left,
                right + 1,
                PackedColor::pack(Color::Named(NamedColor::Black)),
            );
            for col in left..=right {
                if let Some(cell) = self.get_mut(col, row) {
                    cell.reset();
                }
            }
        }
    }

    /// Change attributes in rectangular area
    pub fn change_attributes_in_rectangle(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        attributes: &[u16],
    ) {
        if top >= self.rows || left >= self.cols {
            return;
        }
        let bottom = bottom.min(self.rows - 1);
        let right = right.min(self.cols - 1);
        if top > bottom || left > right {
            return;
        }

        for row in top..=bottom {
            for col in left..=right {
                if let Some(cell) = self.get_mut(col, row) {
                    for &attr in attributes {
                        match attr {
                            0 => {
                                cell.flags.set_bold(false);
                                cell.flags.set_dim(false);
                                cell.flags.set_italic(false);
                                cell.flags.set_underline(false);
                                cell.flags.set_blink(false);
                                cell.flags.set_reverse(false);
                                cell.flags.set_hidden(false);
                                cell.flags.set_strikethrough(false);
                            }
                            1 => cell.flags.set_bold(true),
                            2 => cell.flags.set_dim(true),
                            3 => cell.flags.set_italic(true),
                            4 => cell.flags.set_underline(true),
                            5 => cell.flags.set_blink(true),
                            7 => cell.flags.set_reverse(true),
                            8 => cell.flags.set_hidden(true),
                            9 => cell.flags.set_strikethrough(true),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// Reverse attributes in rectangular area
    pub fn reverse_attributes_in_rectangle(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        attributes: &[u16],
    ) {
        if top >= self.rows || left >= self.cols {
            return;
        }
        let bottom = bottom.min(self.rows - 1);
        let right = right.min(self.cols - 1);
        if top > bottom || left > right {
            return;
        }

        for row in top..=bottom {
            for col in left..=right {
                if let Some(cell) = self.get_mut(col, row) {
                    for &attr in attributes {
                        match attr {
                            0 => {
                                cell.flags.set_bold(!cell.flags.bold());
                                cell.flags.set_underline(!cell.flags.underline());
                                cell.flags.set_blink(!cell.flags.blink());
                                cell.flags.set_reverse(!cell.flags.reverse());
                            }
                            1 => cell.flags.set_bold(!cell.flags.bold()),
                            4 => cell.flags.set_underline(!cell.flags.underline()),
                            5 => cell.flags.set_blink(!cell.flags.blink()),
                            7 => cell.flags.set_reverse(!cell.flags.reverse()),
                            8 => cell.flags.set_hidden(!cell.flags.hidden()),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}
