/// Cursor shape/style (DECSCUSR)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    /// Blinking block (default)
    #[default]
    BlinkingBlock,
    /// Steady block
    SteadyBlock,
    /// Blinking underline
    BlinkingUnderline,
    /// Steady underline
    SteadyUnderline,
    /// Blinking bar (I-beam)
    BlinkingBar,
    /// Steady bar (I-beam)
    SteadyBar,
}

/// Cursor state and position
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Current column (x position)
    pub col: usize,
    /// Current row (y position)
    pub row: usize,
    /// Whether the cursor is visible
    pub visible: bool,
    /// Cursor style (DECSCUSR)
    pub style: CursorStyle,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            col: 0,
            row: 0,
            visible: true,
            style: CursorStyle::default(),
        }
    }
}

impl Cursor {
    /// Create a new cursor at position (0, 0)
    pub fn new() -> Self {
        Self::default()
    }

    /// Move cursor to a specific position
    pub fn goto(&mut self, col: usize, row: usize) {
        self.col = col;
        self.row = row;
    }

    /// Move cursor up by n rows
    pub fn move_up(&mut self, n: usize) {
        self.row = self.row.saturating_sub(n);
    }

    /// Move cursor down by n rows
    pub fn move_down(&mut self, n: usize, max_row: usize) {
        self.row = (self.row + n).min(max_row);
    }

    /// Move cursor left by n columns
    pub fn move_left(&mut self, n: usize) {
        self.col = self.col.saturating_sub(n);
    }

    /// Move cursor right by n columns
    pub fn move_right(&mut self, n: usize, max_col: usize) {
        self.col = (self.col + n).min(max_col);
    }

    /// Move to beginning of line
    pub fn move_to_line_start(&mut self) {
        self.col = 0;
    }

    /// Move to next line
    pub fn move_to_next_line(&mut self, max_row: usize) {
        self.col = 0;
        self.move_down(1, max_row);
    }

    /// Show the cursor
    pub fn show(&mut self) {
        self.visible = true;
    }

    /// Hide the cursor
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Set cursor style (DECSCUSR)
    pub fn set_style(&mut self, style: CursorStyle) {
        self.style = style;
    }

    /// Get cursor style
    pub fn style(&self) -> CursorStyle {
        self.style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_default() {
        let cursor = Cursor::default();
        assert_eq!(cursor.col, 0);
        assert_eq!(cursor.row, 0);
        assert!(cursor.visible);
    }

    #[test]
    fn test_cursor_movement() {
        let mut cursor = Cursor::new();

        cursor.move_right(5, 80);
        assert_eq!(cursor.col, 5);

        cursor.move_down(3, 24);
        assert_eq!(cursor.row, 3);

        cursor.move_left(2);
        assert_eq!(cursor.col, 3);

        cursor.move_up(1);
        assert_eq!(cursor.row, 2);
    }

    #[test]
    fn test_cursor_goto() {
        let mut cursor = Cursor::new();
        cursor.goto(10, 5);
        assert_eq!(cursor.col, 10);
        assert_eq!(cursor.row, 5);
    }

    #[test]
    fn test_cursor_visibility() {
        let mut cursor = Cursor::new();
        assert!(cursor.visible);

        cursor.hide();
        assert!(!cursor.visible);

        cursor.show();
        assert!(cursor.visible);
    }
}
