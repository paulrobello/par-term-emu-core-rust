//! Terminal snapshot types for the Instant Replay feature.
//!
//! These structs capture a complete, clonable snapshot of terminal state
//! at a point in time, enabling efficient restore for replay navigation.

use crate::cell::{Cell, CellFlags};
use crate::color::Color;
use crate::cursor::Cursor;
use crate::mouse::{MouseEncoding, MouseMode};
use crate::zone::Zone;

/// Snapshot of a single Grid's state (primary or alternate screen).
#[derive(Debug, Clone)]
pub struct GridSnapshot {
    /// Visible screen cells (row-major, cols * rows)
    pub cells: Vec<Cell>,
    /// Scrollback buffer cells (flat, circular buffer linearized)
    pub scrollback_cells: Vec<Cell>,
    /// Start index of the circular scrollback buffer
    pub scrollback_start: usize,
    /// Number of lines currently in scrollback
    pub scrollback_lines: usize,
    /// Maximum scrollback capacity
    pub max_scrollback: usize,
    /// Number of columns
    pub cols: usize,
    /// Number of rows
    pub rows: usize,
    /// Line-wrap flags for visible rows
    pub wrapped: Vec<bool>,
    /// Line-wrap flags for scrollback rows
    pub scrollback_wrapped: Vec<bool>,
    /// Semantic zones
    pub zones: Vec<Zone>,
    /// Total number of lines ever scrolled into scrollback
    pub total_lines_scrolled: usize,
}

/// Complete snapshot of terminal state at a point in time.
#[derive(Debug, Clone)]
pub struct TerminalSnapshot {
    /// Timestamp in Unix milliseconds when this snapshot was captured
    pub timestamp: u64,
    /// Terminal width in columns
    pub cols: usize,
    /// Terminal height in rows
    pub rows: usize,

    // --- Grids ---
    /// Primary screen grid snapshot
    pub grid: GridSnapshot,
    /// Alternate screen grid snapshot
    pub alt_grid: GridSnapshot,
    /// Whether the alternate screen is currently active
    pub alt_screen_active: bool,

    // --- Cursors ---
    /// Primary cursor state
    pub cursor: Cursor,
    /// Alternate screen cursor state
    pub alt_cursor: Cursor,
    /// Saved cursor (DECSC/DECRC)
    pub saved_cursor: Option<Cursor>,

    // --- Current colors and attributes ---
    /// Current foreground color
    pub fg: Color,
    /// Current background color
    pub bg: Color,
    /// Current underline color (None = use foreground)
    pub underline_color: Option<Color>,
    /// Current cell attribute flags
    pub flags: CellFlags,

    // --- Saved colors and attributes ---
    /// Saved foreground color
    pub saved_fg: Color,
    /// Saved background color
    pub saved_bg: Color,
    /// Saved underline color
    pub saved_underline_color: Option<Color>,
    /// Saved cell attribute flags
    pub saved_flags: CellFlags,

    // --- Terminal modes and state ---
    /// Terminal title
    pub title: String,
    /// Auto-wrap mode (DECAWM)
    pub auto_wrap: bool,
    /// Origin mode (DECOM)
    pub origin_mode: bool,
    /// Insert mode (IRM)
    pub insert_mode: bool,
    /// Reverse video mode (DECSCNM)
    pub reverse_video: bool,
    /// Line feed / new line mode (LNM)
    pub line_feed_new_line_mode: bool,
    /// Application cursor keys mode
    pub application_cursor: bool,
    /// Bracketed paste mode
    pub bracketed_paste: bool,
    /// Focus tracking mode
    pub focus_tracking: bool,
    /// Mouse tracking mode
    pub mouse_mode: MouseMode,
    /// Mouse encoding format
    pub mouse_encoding: MouseEncoding,
    /// Whether left/right margins are enabled (DECLRMM)
    pub use_lr_margins: bool,
    /// Left margin column (0-indexed)
    pub left_margin: usize,
    /// Right margin column (0-indexed)
    pub right_margin: usize,
    /// Keyboard flags (kitty keyboard protocol)
    pub keyboard_flags: u16,
    /// modifyOtherKeys mode level
    pub modify_other_keys_mode: u8,
    /// Character protection attribute (DECSCA)
    pub char_protected: bool,
    /// Whether bold text uses bright colors
    pub bold_brightening: bool,

    // --- Scroll region ---
    /// Scroll region top row (0-indexed)
    pub scroll_region_top: usize,
    /// Scroll region bottom row (0-indexed)
    pub scroll_region_bottom: usize,

    // --- Misc ---
    /// Tab stop positions (one bool per column)
    pub tab_stops: Vec<bool>,
    /// Pending wrap flag (DECAWM delayed wrap)
    pub pending_wrap: bool,
    /// Estimated memory footprint of this snapshot in bytes
    pub estimated_size_bytes: usize,
}

impl TerminalSnapshot {
    /// Estimate the memory footprint of this snapshot in bytes.
    ///
    /// This is a rough estimate covering the dominant cost centres
    /// (cell Vecs, scrollback, wrapped flags, tab stops, and zones).
    /// Small fixed-size fields are approximated by `size_of::<Self>()`.
    pub fn estimate_size(&self) -> usize {
        let base = std::mem::size_of::<Self>();

        // Grid cells: each Cell owns a Vec<char> for combining chars.
        // Approximate per-cell overhead as size_of::<Cell>() + 24 bytes for the
        // Vec header (pointer + len + cap) even when empty.
        let cell_size = std::mem::size_of::<Cell>();
        let grid_cells = (self.grid.cells.len() + self.grid.scrollback_cells.len()) * cell_size;
        let alt_grid_cells =
            (self.alt_grid.cells.len() + self.alt_grid.scrollback_cells.len()) * cell_size;

        let wrapped_size = self.grid.wrapped.len()
            + self.grid.scrollback_wrapped.len()
            + self.alt_grid.wrapped.len()
            + self.alt_grid.scrollback_wrapped.len();

        let zone_size =
            (self.grid.zones.len() + self.alt_grid.zones.len()) * std::mem::size_of::<Zone>();

        let tab_stops_size = self.tab_stops.len();
        let title_size = self.title.len();

        base + grid_cells + alt_grid_cells + wrapped_size + zone_size + tab_stops_size + title_size
    }
}

use crate::terminal::Terminal;

impl Terminal {
    /// Capture a complete snapshot of the current terminal state.
    pub fn capture_snapshot(&self) -> TerminalSnapshot {
        let (cols, rows) = self.size();
        let grid = self.grid.capture_snapshot();
        let alt_grid = self.alt_grid.capture_snapshot();

        let mut snap = TerminalSnapshot {
            timestamp: crate::terminal::unix_millis(),
            cols,
            rows,
            grid,
            alt_grid,
            alt_screen_active: self.alt_screen_active,
            cursor: self.cursor,
            alt_cursor: self.alt_cursor,
            saved_cursor: self.saved_cursor,
            fg: self.fg,
            bg: self.bg,
            underline_color: self.underline_color,
            flags: self.flags,
            saved_fg: self.saved_fg,
            saved_bg: self.saved_bg,
            saved_underline_color: self.saved_underline_color,
            saved_flags: self.saved_flags,
            title: self.title.clone(),
            auto_wrap: self.auto_wrap,
            origin_mode: self.origin_mode,
            insert_mode: self.insert_mode,
            reverse_video: self.reverse_video,
            line_feed_new_line_mode: self.line_feed_new_line_mode,
            application_cursor: self.application_cursor,
            bracketed_paste: self.bracketed_paste,
            focus_tracking: self.focus_tracking,
            mouse_mode: self.mouse_mode,
            mouse_encoding: self.mouse_encoding,
            use_lr_margins: self.use_lr_margins,
            left_margin: self.left_margin,
            right_margin: self.right_margin,
            keyboard_flags: self.keyboard_flags,
            modify_other_keys_mode: self.modify_other_keys_mode,
            char_protected: self.char_protected,
            bold_brightening: self.bold_brightening,
            scroll_region_top: self.scroll_region_top,
            scroll_region_bottom: self.scroll_region_bottom,
            tab_stops: self.tab_stops.clone(),
            pending_wrap: self.pending_wrap,
            estimated_size_bytes: 0,
        };
        snap.estimated_size_bytes = snap.estimate_size();
        snap
    }

    /// Restore the terminal state from a previously captured snapshot.
    pub fn restore_from_snapshot(&mut self, snap: TerminalSnapshot) {
        self.grid.restore_from_snapshot(&snap.grid);
        self.alt_grid.restore_from_snapshot(&snap.alt_grid);
        self.alt_screen_active = snap.alt_screen_active;
        self.cursor = snap.cursor;
        self.alt_cursor = snap.alt_cursor;
        self.saved_cursor = snap.saved_cursor;
        self.fg = snap.fg;
        self.bg = snap.bg;
        self.underline_color = snap.underline_color;
        self.flags = snap.flags;
        self.saved_fg = snap.saved_fg;
        self.saved_bg = snap.saved_bg;
        self.saved_underline_color = snap.saved_underline_color;
        self.saved_flags = snap.saved_flags;
        self.title = snap.title;
        self.auto_wrap = snap.auto_wrap;
        self.origin_mode = snap.origin_mode;
        self.insert_mode = snap.insert_mode;
        self.reverse_video = snap.reverse_video;
        self.line_feed_new_line_mode = snap.line_feed_new_line_mode;
        self.application_cursor = snap.application_cursor;
        self.bracketed_paste = snap.bracketed_paste;
        self.focus_tracking = snap.focus_tracking;
        self.mouse_mode = snap.mouse_mode;
        self.mouse_encoding = snap.mouse_encoding;
        self.use_lr_margins = snap.use_lr_margins;
        self.left_margin = snap.left_margin;
        self.right_margin = snap.right_margin;
        self.keyboard_flags = snap.keyboard_flags;
        self.modify_other_keys_mode = snap.modify_other_keys_mode;
        self.char_protected = snap.char_protected;
        self.bold_brightening = snap.bold_brightening;
        self.scroll_region_top = snap.scroll_region_top;
        self.scroll_region_bottom = snap.scroll_region_bottom;
        self.tab_stops = snap.tab_stops;
        self.pending_wrap = snap.pending_wrap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellFlags;
    use crate::color::{Color, NamedColor};
    use crate::cursor::Cursor;

    fn make_grid_snapshot(cols: usize, rows: usize) -> GridSnapshot {
        GridSnapshot {
            cells: vec![Cell::default(); cols * rows],
            scrollback_cells: Vec::new(),
            scrollback_start: 0,
            scrollback_lines: 0,
            max_scrollback: 1000,
            cols,
            rows,
            wrapped: vec![false; rows],
            scrollback_wrapped: Vec::new(),
            zones: Vec::new(),
            total_lines_scrolled: 0,
        }
    }

    fn make_terminal_snapshot(cols: usize, rows: usize) -> TerminalSnapshot {
        let grid = make_grid_snapshot(cols, rows);
        let alt_grid = make_grid_snapshot(cols, rows);

        let mut snap = TerminalSnapshot {
            timestamp: 1_700_000_000_000,
            cols,
            rows,
            grid,
            alt_grid,
            alt_screen_active: false,
            cursor: Cursor::default(),
            alt_cursor: Cursor::default(),
            saved_cursor: None,
            fg: Color::Named(NamedColor::White),
            bg: Color::Named(NamedColor::Black),
            underline_color: None,
            flags: CellFlags::default(),
            saved_fg: Color::Named(NamedColor::White),
            saved_bg: Color::Named(NamedColor::Black),
            saved_underline_color: None,
            saved_flags: CellFlags::default(),
            title: String::new(),
            auto_wrap: true,
            origin_mode: false,
            insert_mode: false,
            reverse_video: false,
            line_feed_new_line_mode: false,
            application_cursor: false,
            bracketed_paste: false,
            focus_tracking: false,
            mouse_mode: MouseMode::Off,
            mouse_encoding: MouseEncoding::Default,
            use_lr_margins: false,
            left_margin: 0,
            right_margin: cols.saturating_sub(1),
            keyboard_flags: 0,
            modify_other_keys_mode: 0,
            char_protected: false,
            bold_brightening: true,
            scroll_region_top: 0,
            scroll_region_bottom: rows.saturating_sub(1),
            tab_stops: vec![false; cols],
            pending_wrap: false,
            estimated_size_bytes: 0,
        };
        snap.estimated_size_bytes = snap.estimate_size();
        snap
    }

    #[test]
    fn test_terminal_snapshot_creation() {
        let snap = make_terminal_snapshot(80, 24);
        assert_eq!(snap.cols, 80);
        assert_eq!(snap.rows, 24);
        assert_eq!(snap.grid.cells.len(), 80 * 24);
        assert_eq!(snap.alt_grid.cells.len(), 80 * 24);
        assert!(!snap.alt_screen_active);
        assert_eq!(snap.cursor, Cursor::default());
        assert_eq!(snap.fg, Color::Named(NamedColor::White));
        assert_eq!(snap.bg, Color::Named(NamedColor::Black));
    }

    #[test]
    fn test_terminal_snapshot_clone() {
        let snap = make_terminal_snapshot(80, 24);
        let cloned = snap.clone();
        assert_eq!(cloned.cols, snap.cols);
        assert_eq!(cloned.rows, snap.rows);
        assert_eq!(cloned.timestamp, snap.timestamp);
        assert_eq!(cloned.grid.cells.len(), snap.grid.cells.len());
        assert_eq!(cloned.alt_grid.cells.len(), snap.alt_grid.cells.len());
        assert_eq!(cloned.cursor, snap.cursor);
        assert_eq!(cloned.fg, snap.fg);
        assert_eq!(cloned.bg, snap.bg);
    }

    #[test]
    fn test_terminal_snapshot_size_estimation() {
        let snap = make_terminal_snapshot(80, 24);
        let size = snap.estimate_size();
        // Should be at least the size of the cell data
        let min_cells = 80 * 24 * 2 * std::mem::size_of::<Cell>();
        assert!(
            size >= min_cells,
            "estimated size {size} should be >= cell data size {min_cells}"
        );
        assert_eq!(snap.estimated_size_bytes, size);
    }

    #[test]
    fn test_terminal_snapshot_with_scrollback() {
        let mut grid = make_grid_snapshot(80, 24);
        grid.scrollback_cells = vec![Cell::default(); 80 * 100];
        grid.scrollback_lines = 100;

        let mut snap = make_terminal_snapshot(80, 24);
        snap.grid = grid;
        snap.estimated_size_bytes = snap.estimate_size();

        // Scrollback should increase the estimated size
        let no_scrollback_snap = make_terminal_snapshot(80, 24);
        assert!(
            snap.estimated_size_bytes > no_scrollback_snap.estimated_size_bytes,
            "snapshot with scrollback should be larger"
        );
    }

    #[test]
    fn test_terminal_snapshot_with_colored_cells() {
        let mut snap = make_terminal_snapshot(10, 5);
        // Set some cells to have non-default colors
        snap.grid.cells[0] = Cell::with_colors('A', Color::Rgb(255, 0, 0), Color::Rgb(0, 0, 255));
        snap.grid.cells[1] =
            Cell::with_colors('B', Color::Indexed(196), Color::Named(NamedColor::Green));

        let cloned = snap.clone();
        assert_eq!(cloned.grid.cells[0].c, 'A');
        assert_eq!(cloned.grid.cells[0].fg, Color::Rgb(255, 0, 0));
        assert_eq!(cloned.grid.cells[0].bg, Color::Rgb(0, 0, 255));
        assert_eq!(cloned.grid.cells[1].c, 'B');
        assert_eq!(cloned.grid.cells[1].fg, Color::Indexed(196));
        assert_eq!(cloned.grid.cells[1].bg, Color::Named(NamedColor::Green));
    }

    #[test]
    fn test_grid_snapshot_creation() {
        let gs = make_grid_snapshot(120, 40);
        assert_eq!(gs.cols, 120);
        assert_eq!(gs.rows, 40);
        assert_eq!(gs.cells.len(), 120 * 40);
        assert_eq!(gs.wrapped.len(), 40);
        assert_eq!(gs.scrollback_cells.len(), 0);
        assert_eq!(gs.scrollback_lines, 0);
        assert_eq!(gs.max_scrollback, 1000);
        assert_eq!(gs.total_lines_scrolled, 0);
    }

    #[test]
    fn test_grid_snapshot_with_zones() {
        let mut gs = make_grid_snapshot(80, 24);
        gs.zones.push(Zone::new(
            1,
            crate::zone::ZoneType::Prompt,
            0,
            Some(1_700_000_000_000),
        ));
        gs.zones.push(Zone::new(
            2,
            crate::zone::ZoneType::Command,
            1,
            Some(1_700_000_000_001),
        ));
        assert_eq!(gs.zones.len(), 2);

        let cloned = gs.clone();
        assert_eq!(cloned.zones.len(), 2);
        assert_eq!(cloned.zones[0].id, 1);
        assert_eq!(cloned.zones[1].id, 2);
    }
}
