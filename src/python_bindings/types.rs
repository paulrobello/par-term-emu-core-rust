//! Python data types and structures for the terminal API
//!
//! This module contains the main Python-facing data structures:
//! - PyAttributes: Cell text attributes (bold, italic, etc.)
//! - PyScreenSnapshot: Atomic snapshot of terminal screen state
//! - PyShellIntegration: Shell integration (OSC 133) state
//! - PyGraphic: Sixel graphics representation
//! - LineCellData: Type alias for row cell data

use pyo3::prelude::*;

use super::enums::{PyCursorStyle, PyUnderlineStyle};

/// Type alias for a row of cell data returned by get_line_cells
/// Tuple contains: (character, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes)
pub type LineCellData = Vec<(String, (u8, u8, u8), (u8, u8, u8), PyAttributes)>;

/// Type alias for half-block rendering colors
/// Tuple contains: ((top_r, top_g, top_b, top_a), (bottom_r, bottom_g, bottom_b, bottom_a))
type HalfBlockColors = ((u8, u8, u8, u8), (u8, u8, u8, u8));

/// Cell attributes
#[pyclass(name = "Attributes")]
#[derive(Clone)]
pub struct PyAttributes {
    #[pyo3(get)]
    pub bold: bool,
    #[pyo3(get)]
    pub dim: bool,
    #[pyo3(get)]
    pub italic: bool,
    #[pyo3(get)]
    pub underline: bool,
    #[pyo3(get)]
    pub blink: bool,
    #[pyo3(get)]
    pub reverse: bool,
    #[pyo3(get)]
    pub hidden: bool,
    #[pyo3(get)]
    pub strikethrough: bool,
    #[pyo3(get)]
    pub underline_style: PyUnderlineStyle,
    #[pyo3(get)]
    pub wide_char: bool,
    #[pyo3(get)]
    pub wide_char_spacer: bool,
    #[pyo3(get)]
    pub hyperlink_id: Option<u32>,
}

impl Default for PyAttributes {
    fn default() -> Self {
        Self {
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            blink: false,
            reverse: false,
            hidden: false,
            strikethrough: false,
            underline_style: PyUnderlineStyle::None,
            wide_char: false,
            wide_char_spacer: false,
            hyperlink_id: None,
        }
    }
}

#[pymethods]
impl PyAttributes {
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Attributes(bold={}, italic={}, underline={}, underline_style={:?})",
            self.bold, self.italic, self.underline, self.underline_style
        ))
    }
}

/// Atomic snapshot of terminal screen state for race-free rendering
///
/// Captures all lines, cursor state, and screen identity at a single point in time.
/// This immutable snapshot prevents race conditions where alternate screen switches
/// happen between individual line render calls.
#[pyclass(name = "ScreenSnapshot")]
#[allow(clippy::type_complexity)]
pub struct PyScreenSnapshot {
    /// All screen lines captured atomically
    /// Format: Vec<Vec<(String, fg_rgb, bg_rgb, attributes)>>
    #[pyo3(get)]
    pub lines: Vec<Vec<(String, (u8, u8, u8), (u8, u8, u8), PyAttributes)>>,

    /// Wrapped state for each line (true = line continues to next row)
    #[pyo3(get)]
    pub wrapped_lines: Vec<bool>,

    /// Cursor position at snapshot time (col, row)
    #[pyo3(get)]
    pub cursor_pos: (usize, usize),

    /// Cursor visibility at snapshot time
    #[pyo3(get)]
    pub cursor_visible: bool,

    /// Cursor style at snapshot time
    #[pyo3(get)]
    pub cursor_style: PyCursorStyle,

    /// Which screen buffer was active (true = alternate)
    #[pyo3(get)]
    pub is_alt_screen: bool,

    /// Generation counter at snapshot time
    #[pyo3(get)]
    pub generation: u64,

    /// Terminal dimensions at snapshot time (cols, rows)
    #[pyo3(get)]
    pub size: (usize, usize),
}

#[pymethods]
impl PyScreenSnapshot {
    /// Get line cells for a specific row from snapshot
    ///
    /// Filters control characters (< 32, except space and tab) and replaces them with space.
    /// This optimization moves control character filtering from Python to compiled Rust code.
    ///
    /// Args:
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     List of tuples (char, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes),
    ///     or empty list if row is out of bounds
    #[allow(clippy::type_complexity)]
    fn get_line(&self, row: usize) -> Vec<(String, (u8, u8, u8), (u8, u8, u8), PyAttributes)> {
        if row < self.lines.len() {
            // Clone and filter control characters in one pass
            self.lines[row]
                .iter()
                .map(|(c, fg, bg, attrs)| {
                    // Filter out control characters (< 32) except space and tab
                    // Check the first character of the grapheme string
                    let first_char = c.chars().next().unwrap_or(' ');
                    let filtered_char =
                        if (first_char as u32) < 32 && first_char != ' ' && first_char != '\t' {
                            " ".to_string() // Replace control chars with space
                        } else {
                            c.clone()
                        };
                    (filtered_char, *fg, *bg, attrs.clone())
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "ScreenSnapshot(size={}x{}, gen={}, alt={})",
            self.size.0, self.size.1, self.generation, self.is_alt_screen
        ))
    }
}

/// Shell integration state
#[pyclass(name = "ShellIntegration")]
#[derive(Clone)]
pub struct PyShellIntegration {
    #[pyo3(get)]
    pub in_prompt: bool,
    #[pyo3(get)]
    pub in_command_input: bool,
    #[pyo3(get)]
    pub in_command_output: bool,
    #[pyo3(get)]
    pub current_command: Option<String>,
    #[pyo3(get)]
    pub last_exit_code: Option<i32>,
    #[pyo3(get)]
    pub cwd: Option<String>,
}

#[pymethods]
impl PyShellIntegration {
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "ShellIntegration(in_prompt={}, in_command_input={}, in_command_output={})",
            self.in_prompt, self.in_command_input, self.in_command_output
        ))
    }
}

/// Progress bar state from OSC 9;4 sequences (ConEmu/Windows Terminal style)
///
/// This struct represents the current progress bar state as set via OSC 9;4 sequences.
/// Terminal emulators like ConEmu and Windows Terminal use this to display progress
/// in the tab bar, taskbar, or window title.
///
/// ## States
/// - Hidden: No progress bar displayed
/// - Normal: Standard progress (0-100%)
/// - Indeterminate: Busy/loading indicator
/// - Warning: Progress with warning (yellow)
/// - Error: Progress with error (red)
///
/// ## Examples
/// ```python
/// term = Terminal(80, 24)
/// term.process(b"\\x1b]9;4;1;50\\x1b\\\\")  # Set progress to 50%
/// pb = term.progress_bar()
/// print(f"Progress: {pb.progress}%")  # Output: Progress: 50%
/// print(f"State: {pb.state}")  # Output: State: ProgressState.NORMAL
/// ```
#[pyclass(name = "ProgressBar")]
#[derive(Clone)]
pub struct PyProgressBar {
    /// Current progress state
    #[pyo3(get)]
    pub state: super::enums::PyProgressState,
    /// Progress percentage (0-100)
    #[pyo3(get)]
    pub progress: u8,
}

#[pymethods]
impl PyProgressBar {
    /// Create a new progress bar with given state and progress
    #[new]
    #[pyo3(signature = (state=super::enums::PyProgressState::Hidden, progress=0))]
    fn new(state: super::enums::PyProgressState, progress: u8) -> Self {
        Self {
            state,
            progress: progress.min(100),
        }
    }

    /// Check if the progress bar is currently active (visible)
    fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// Generate the OSC 9;4 escape sequence for this progress bar
    fn to_escape_sequence(&self) -> String {
        if self.state.requires_progress() {
            format!("\x1b]9;4;{};{}\x1b\\", self.state as u8, self.progress)
        } else {
            format!("\x1b]9;4;{}\x1b\\", self.state as u8)
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ProgressBar(state={:?}, progress={})",
            self.state, self.progress
        )
    }
}

impl From<&crate::terminal::ProgressBar> for PyProgressBar {
    fn from(pb: &crate::terminal::ProgressBar) -> Self {
        Self {
            state: pb.state.into(),
            progress: pb.progress,
        }
    }
}

/// Graphics representation (Sixel, iTerm2, or Kitty)
#[pyclass(name = "Graphic")]
#[derive(Clone)]
pub struct PyGraphic {
    #[pyo3(get)]
    pub id: u64,
    #[pyo3(get)]
    pub protocol: String,
    #[pyo3(get)]
    pub position: (usize, usize),
    #[pyo3(get)]
    pub width: usize,
    #[pyo3(get)]
    pub height: usize,
    #[pyo3(get)]
    pub scroll_offset_rows: usize,
    #[pyo3(get)]
    pub cell_dimensions: Option<(u32, u32)>,
    pixels: Vec<u8>,
}

#[pymethods]
impl PyGraphic {
    /// Get pixel color at (x, y) coordinates
    ///
    /// Args:
    ///     x: X coordinate (0-based)
    ///     y: Y coordinate (0-based)
    ///
    /// Returns:
    ///     Tuple of (r, g, b, a) values, or None if out of bounds
    fn get_pixel(&self, x: usize, y: usize) -> Option<(u8, u8, u8, u8)> {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) * 4;
            Some((
                self.pixels[idx],
                self.pixels[idx + 1],
                self.pixels[idx + 2],
                self.pixels[idx + 3],
            ))
        } else {
            None
        }
    }

    /// Get raw pixel data as bytes (RGBA format)
    ///
    /// Returns:
    ///     Bytes containing RGBA pixel data in row-major order
    fn pixels(&self) -> Vec<u8> {
        self.pixels.clone()
    }

    /// Get size in terminal cells
    fn cell_size(&self, cell_width: u32, cell_height: u32) -> (usize, usize) {
        let cols = self.width.div_ceil(cell_width as usize);
        let rows = self.height.div_ceil(cell_height as usize);
        (cols, rows)
    }

    /// Sample for half-block rendering at cell (col, row)
    /// Returns ((top_r, top_g, top_b, top_a), (bottom_r, bottom_g, bottom_b, bottom_a))
    fn sample_half_block(
        &self,
        cell_col: usize,
        cell_row: usize,
        cell_width: u32,
        cell_height: u32,
    ) -> Option<HalfBlockColors> {
        // Calculate pixel coordinates relative to graphic position
        let rel_col = cell_col.checked_sub(self.position.0)?;
        let rel_row = cell_row.checked_sub(self.position.1)?;

        let px_x = rel_col * cell_width as usize;
        let px_y = rel_row * cell_height as usize;

        // Sample center of top and bottom halves
        let top_y = px_y + cell_height as usize / 4;
        let bottom_y = px_y + (cell_height as usize * 3) / 4;
        let center_x = px_x + cell_width as usize / 2;

        let top = self.get_pixel(center_x, top_y)?;
        let bottom = self.get_pixel(center_x, bottom_y)?;

        Some((top, bottom))
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Graphic(id={}, protocol='{}', position=({},{}), size={}x{})",
            self.id, self.protocol, self.position.0, self.position.1, self.width, self.height
        ))
    }
}

impl From<&crate::sixel::SixelGraphic> for PyGraphic {
    fn from(graphic: &crate::sixel::SixelGraphic) -> Self {
        Self {
            id: graphic.id,
            protocol: "sixel".to_string(),
            position: graphic.position,
            width: graphic.width,
            height: graphic.height,
            scroll_offset_rows: graphic.scroll_offset_rows,
            cell_dimensions: graphic.cell_dimensions,
            pixels: graphic.pixels.clone(),
        }
    }
}

impl From<&crate::graphics::TerminalGraphic> for PyGraphic {
    fn from(graphic: &crate::graphics::TerminalGraphic) -> Self {
        Self {
            id: graphic.id,
            protocol: graphic.protocol.as_str().to_string(),
            position: graphic.position,
            width: graphic.width,
            height: graphic.height,
            scroll_offset_rows: graphic.scroll_offset_rows,
            cell_dimensions: graphic.cell_dimensions,
            pixels: (*graphic.pixels).clone(),
        }
    }
}

/// Tmux control protocol notification
#[pyclass(name = "TmuxNotification")]
#[derive(Clone)]
pub struct PyTmuxNotification {
    /// Notification type (e.g., "output", "window-add", "session-changed")
    #[pyo3(get)]
    pub notification_type: String,

    /// Pane ID (for notifications that involve a pane)
    #[pyo3(get)]
    pub pane_id: Option<String>,

    /// Window ID (for notifications that involve a window)
    #[pyo3(get)]
    pub window_id: Option<String>,

    /// Session ID (for notifications that involve a session)
    #[pyo3(get)]
    pub session_id: Option<String>,

    /// Name (for window/session rename notifications)
    #[pyo3(get)]
    pub name: Option<String>,

    /// Client name (for client-related notifications)
    #[pyo3(get)]
    pub client: Option<String>,

    /// Output data (for output notifications, as bytes)
    #[pyo3(get)]
    pub data: Option<Vec<u8>>,

    /// Timestamp (for begin/end/error notifications)
    #[pyo3(get)]
    pub timestamp: Option<u64>,

    /// Command number (for begin/end/error notifications)
    #[pyo3(get)]
    pub command_number: Option<u32>,

    /// Flags (for begin/end/error notifications)
    #[pyo3(get)]
    pub flags: Option<String>,

    /// Delay in milliseconds (for extended-output notifications)
    #[pyo3(get)]
    pub delay_ms: Option<u64>,

    /// Subscription name (for subscription-changed notifications)
    #[pyo3(get)]
    pub subscription_name: Option<String>,

    /// Subscription value (for subscription-changed notifications)
    #[pyo3(get)]
    pub value: Option<String>,

    /// Window layout (for layout-change notifications)
    #[pyo3(get)]
    pub window_layout: Option<String>,

    /// Window visible layout (for layout-change notifications)
    #[pyo3(get)]
    pub window_visible_layout: Option<String>,

    /// Window raw flags (for layout-change notifications)
    #[pyo3(get)]
    pub window_raw_flags: Option<String>,

    /// Raw line (for unknown notifications)
    #[pyo3(get)]
    pub raw_line: Option<String>,
}

#[pymethods]
impl PyTmuxNotification {
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!("TmuxNotification(type={})", self.notification_type))
    }
}

impl From<&crate::tmux_control::TmuxNotification> for PyTmuxNotification {
    fn from(notif: &crate::tmux_control::TmuxNotification) -> Self {
        use crate::tmux_control::TmuxNotification;

        match notif {
            TmuxNotification::Begin {
                timestamp,
                command_number,
                flags,
            } => PyTmuxNotification {
                notification_type: "begin".to_string(),
                timestamp: Some(*timestamp),
                command_number: Some(*command_number),
                flags: Some(flags.clone()),
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::End {
                timestamp,
                command_number,
                flags,
            } => PyTmuxNotification {
                notification_type: "end".to_string(),
                timestamp: Some(*timestamp),
                command_number: Some(*command_number),
                flags: Some(flags.clone()),
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Error {
                timestamp,
                command_number,
                flags,
            } => PyTmuxNotification {
                notification_type: "error".to_string(),
                timestamp: Some(*timestamp),
                command_number: Some(*command_number),
                flags: Some(flags.clone()),
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Output { pane_id, data } => PyTmuxNotification {
                notification_type: "output".to_string(),
                pane_id: Some(pane_id.clone()),
                data: Some(data.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::PaneModeChanged { pane_id } => PyTmuxNotification {
                notification_type: "pane-mode-changed".to_string(),
                pane_id: Some(pane_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::WindowPaneChanged { window_id, pane_id } => PyTmuxNotification {
                notification_type: "window-pane-changed".to_string(),
                window_id: Some(window_id.clone()),
                pane_id: Some(pane_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::WindowClose { window_id } => PyTmuxNotification {
                notification_type: "window-close".to_string(),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::UnlinkedWindowClose { window_id } => PyTmuxNotification {
                notification_type: "unlinked-window-close".to_string(),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::WindowAdd { window_id } => PyTmuxNotification {
                notification_type: "window-add".to_string(),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::UnlinkedWindowAdd { window_id } => PyTmuxNotification {
                notification_type: "unlinked-window-add".to_string(),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::WindowRenamed { window_id, name } => PyTmuxNotification {
                notification_type: "window-renamed".to_string(),
                window_id: Some(window_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::UnlinkedWindowRenamed { window_id, name } => PyTmuxNotification {
                notification_type: "unlinked-window-renamed".to_string(),
                window_id: Some(window_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SessionChanged { session_id, name } => PyTmuxNotification {
                notification_type: "session-changed".to_string(),
                session_id: Some(session_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::ClientSessionChanged {
                client,
                session_id,
                name,
            } => PyTmuxNotification {
                notification_type: "client-session-changed".to_string(),
                client: Some(client.clone()),
                session_id: Some(session_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SessionRenamed { session_id, name } => PyTmuxNotification {
                notification_type: "session-renamed".to_string(),
                session_id: Some(session_id.clone()),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SessionsChanged => PyTmuxNotification {
                notification_type: "sessions-changed".to_string(),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SessionWindowChanged {
                session_id,
                window_id,
            } => PyTmuxNotification {
                notification_type: "session-window-changed".to_string(),
                session_id: Some(session_id.clone()),
                window_id: Some(window_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::ClientDetached { client } => PyTmuxNotification {
                notification_type: "client-detached".to_string(),
                client: Some(client.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Exit => PyTmuxNotification {
                notification_type: "exit".to_string(),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Pause { pane_id } => PyTmuxNotification {
                notification_type: "pause".to_string(),
                pane_id: Some(pane_id.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::ExtendedOutput {
                pane_id,
                delay_ms,
                data,
            } => PyTmuxNotification {
                notification_type: "extended-output".to_string(),
                pane_id: Some(pane_id.clone()),
                delay_ms: Some(*delay_ms),
                data: Some(data.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Continue => PyTmuxNotification {
                notification_type: "continue".to_string(),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::SubscriptionChanged { name, value } => PyTmuxNotification {
                notification_type: "subscription-changed".to_string(),
                subscription_name: Some(name.clone()),
                value: Some(value.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::LayoutChange {
                window_id,
                window_layout,
                window_visible_layout,
                window_raw_flags,
            } => PyTmuxNotification {
                notification_type: "layout-change".to_string(),
                window_id: Some(window_id.clone()),
                window_layout: Some(window_layout.clone()),
                window_visible_layout: Some(window_visible_layout.clone()),
                window_raw_flags: Some(window_raw_flags.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                raw_line: None,
            },
            TmuxNotification::PasteBufferChanged { name } => PyTmuxNotification {
                notification_type: "paste-buffer-changed".to_string(),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::PasteBufferDeleted { name } => PyTmuxNotification {
                notification_type: "paste-buffer-deleted".to_string(),
                name: Some(name.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
            TmuxNotification::Unknown { line } => PyTmuxNotification {
                notification_type: "unknown".to_string(),
                raw_line: Some(line.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                data: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
            },
            TmuxNotification::TerminalOutput { data } => PyTmuxNotification {
                notification_type: "terminal-output".to_string(),
                data: Some(data.clone()),
                timestamp: None,
                command_number: None,
                flags: None,
                pane_id: None,
                window_id: None,
                session_id: None,
                name: None,
                client: None,
                delay_ms: None,
                subscription_name: None,
                value: None,
                window_layout: None,
                window_visible_layout: None,
                window_raw_flags: None,
                raw_line: None,
            },
        }
    }
}

/// Search match result
#[pyclass(name = "SearchMatch")]
#[derive(Clone)]
pub struct PySearchMatch {
    /// Row index (negative for scrollback, 0+ for visible screen)
    #[pyo3(get)]
    pub row: isize,
    /// Column index
    #[pyo3(get)]
    pub col: usize,
    /// Length of the match
    #[pyo3(get)]
    pub length: usize,
    /// Matched text
    #[pyo3(get)]
    pub text: String,
}

#[pymethods]
impl PySearchMatch {
    fn __repr__(&self) -> String {
        format!(
            "SearchMatch(row={}, col={}, length={}, text={:?})",
            self.row, self.col, self.length, self.text
        )
    }
}

/// Detected semantic item
#[pyclass(name = "DetectedItem")]
#[derive(Clone)]
pub struct PyDetectedItem {
    /// Item type: "url", "filepath", "git_hash", "ip", or "email"
    #[pyo3(get)]
    pub item_type: String,
    /// The detected text
    #[pyo3(get)]
    pub text: String,
    /// Row index
    #[pyo3(get)]
    pub row: usize,
    /// Column index
    #[pyo3(get)]
    pub col: usize,
    /// Optional line number (for file paths like "file.txt:123")
    #[pyo3(get)]
    pub line_number: Option<usize>,
}

#[pymethods]
impl PyDetectedItem {
    fn __repr__(&self) -> String {
        format!(
            "DetectedItem(type={}, text={:?}, row={}, col={})",
            self.item_type, self.text, self.row, self.col
        )
    }
}

/// Selection mode
#[pyclass(name = "SelectionMode")]
#[derive(Clone)]
pub enum PySelectionMode {
    Character,
    Line,
    Block,
}

/// Selection state
#[pyclass(name = "Selection")]
#[derive(Clone)]
pub struct PySelection {
    /// Start position (col, row)
    #[pyo3(get)]
    pub start: (usize, usize),
    /// End position (col, row)
    #[pyo3(get)]
    pub end: (usize, usize),
    /// Selection mode
    #[pyo3(get)]
    pub mode: String,
}

#[pymethods]
impl PySelection {
    fn __repr__(&self) -> String {
        format!(
            "Selection(start={:?}, end={:?}, mode={})",
            self.start, self.end, self.mode
        )
    }
}

/// Scrollback statistics
#[pyclass(name = "ScrollbackStats")]
#[derive(Clone)]
pub struct PyScrollbackStats {
    /// Total number of scrollback lines
    #[pyo3(get)]
    pub total_lines: usize,
    /// Estimated memory usage in bytes
    #[pyo3(get)]
    pub memory_bytes: usize,
    /// Whether the scrollback buffer has wrapped (cycled)
    #[pyo3(get)]
    pub has_wrapped: bool,
}

#[pymethods]
impl PyScrollbackStats {
    fn __repr__(&self) -> String {
        format!(
            "ScrollbackStats(total_lines={}, memory_bytes={}, has_wrapped={})",
            self.total_lines, self.memory_bytes, self.has_wrapped
        )
    }
}

/// Bookmark
#[pyclass(name = "Bookmark")]
#[derive(Clone)]
pub struct PyBookmark {
    /// Bookmark ID
    #[pyo3(get)]
    pub id: usize,
    /// Row index (negative for scrollback, 0+ for visible screen)
    #[pyo3(get)]
    pub row: isize,
    /// Bookmark label
    #[pyo3(get)]
    pub label: String,
}

#[pymethods]
impl PyBookmark {
    fn __repr__(&self) -> String {
        format!(
            "Bookmark(id={}, row={}, label={:?})",
            self.id, self.row, self.label
        )
    }
}

// === Feature 7: Performance Metrics ===

/// Performance metrics
#[pyclass(name = "PerformanceMetrics")]
#[derive(Clone)]
pub struct PyPerformanceMetrics {
    #[pyo3(get)]
    pub frames_rendered: u64,
    #[pyo3(get)]
    pub cells_updated: u64,
    #[pyo3(get)]
    pub bytes_processed: u64,
    #[pyo3(get)]
    pub total_processing_us: u64,
    #[pyo3(get)]
    pub peak_frame_us: u64,
    #[pyo3(get)]
    pub scroll_count: u64,
    #[pyo3(get)]
    pub wrap_count: u64,
    #[pyo3(get)]
    pub escape_sequences: u64,
}

#[pymethods]
impl PyPerformanceMetrics {
    fn __repr__(&self) -> String {
        format!(
            "PerformanceMetrics(frames={}, cells={}, fps={:.1})",
            self.frames_rendered,
            self.cells_updated,
            if self.total_processing_us > 0 {
                1_000_000.0 * self.frames_rendered as f64 / self.total_processing_us as f64
            } else {
                0.0
            }
        )
    }
}

/// Frame timing
#[pyclass(name = "FrameTiming")]
#[derive(Clone)]
pub struct PyFrameTiming {
    #[pyo3(get)]
    pub frame_number: u64,
    #[pyo3(get)]
    pub processing_us: u64,
    #[pyo3(get)]
    pub cells_updated: usize,
    #[pyo3(get)]
    pub bytes_processed: usize,
}

#[pymethods]
impl PyFrameTiming {
    fn __repr__(&self) -> String {
        format!(
            "FrameTiming(frame={}, time={}us, cells={})",
            self.frame_number, self.processing_us, self.cells_updated
        )
    }
}

// === Feature 8: Advanced Color Operations ===

/// HSV color
#[pyclass(name = "ColorHSV")]
#[derive(Clone)]
pub struct PyColorHSV {
    #[pyo3(get)]
    pub h: f32,
    #[pyo3(get)]
    pub s: f32,
    #[pyo3(get)]
    pub v: f32,
}

#[pymethods]
impl PyColorHSV {
    #[new]
    fn new(h: f32, s: f32, v: f32) -> Self {
        Self { h, s, v }
    }

    fn __repr__(&self) -> String {
        format!(
            "ColorHSV(h={:.1}, s={:.2}, v={:.2})",
            self.h, self.s, self.v
        )
    }
}

/// HSL color
#[pyclass(name = "ColorHSL")]
#[derive(Clone)]
pub struct PyColorHSL {
    #[pyo3(get)]
    pub h: f32,
    #[pyo3(get)]
    pub s: f32,
    #[pyo3(get)]
    pub l: f32,
}

#[pymethods]
impl PyColorHSL {
    #[new]
    fn new(h: f32, s: f32, l: f32) -> Self {
        Self { h, s, l }
    }

    fn __repr__(&self) -> String {
        format!(
            "ColorHSL(h={:.1}, s={:.2}, l={:.2})",
            self.h, self.s, self.l
        )
    }
}

/// Color palette
#[pyclass(name = "ColorPalette")]
#[derive(Clone)]
pub struct PyColorPalette {
    #[pyo3(get)]
    pub base: (u8, u8, u8),
    #[pyo3(get)]
    pub colors: Vec<(u8, u8, u8)>,
    #[pyo3(get)]
    pub mode: String,
}

#[pymethods]
impl PyColorPalette {
    fn __repr__(&self) -> String {
        format!(
            "ColorPalette(mode={}, colors={})",
            self.mode,
            self.colors.len()
        )
    }
}

// === Feature 9: Line Wrapping Utilities ===

/// Joined lines result
#[pyclass(name = "JoinedLines")]
#[derive(Clone)]
pub struct PyJoinedLines {
    #[pyo3(get)]
    pub text: String,
    #[pyo3(get)]
    pub start_row: usize,
    #[pyo3(get)]
    pub end_row: usize,
    #[pyo3(get)]
    pub lines_joined: usize,
}

#[pymethods]
impl PyJoinedLines {
    fn __repr__(&self) -> String {
        format!(
            "JoinedLines(rows={}-{}, lines={}, len={})",
            self.start_row,
            self.end_row,
            self.lines_joined,
            self.text.len()
        )
    }
}

// === Feature 10: Clipboard Integration ===

/// Clipboard entry
#[pyclass(name = "ClipboardEntry")]
#[derive(Clone)]
pub struct PyClipboardEntry {
    #[pyo3(get)]
    pub content: String,
    #[pyo3(get)]
    pub timestamp: u64,
    #[pyo3(get)]
    pub label: Option<String>,
}

#[pymethods]
impl PyClipboardEntry {
    fn __repr__(&self) -> String {
        format!(
            "ClipboardEntry(len={}, timestamp={})",
            self.content.len(),
            self.timestamp
        )
    }
}

// === Feature 17: Advanced Mouse Support ===

/// Mouse event
#[pyclass(name = "MouseEvent")]
#[derive(Clone)]
pub struct PyMouseEvent {
    #[pyo3(get)]
    pub event_type: String,
    #[pyo3(get)]
    pub button: String,
    #[pyo3(get)]
    pub col: usize,
    #[pyo3(get)]
    pub row: usize,
    #[pyo3(get)]
    pub pixel_x: Option<u16>,
    #[pyo3(get)]
    pub pixel_y: Option<u16>,
    #[pyo3(get)]
    pub modifiers: u8,
    #[pyo3(get)]
    pub timestamp: u64,
}

#[pymethods]
impl PyMouseEvent {
    fn __repr__(&self) -> String {
        format!(
            "MouseEvent(type={}, button={}, pos=({}, {}), timestamp={})",
            self.event_type, self.button, self.col, self.row, self.timestamp
        )
    }
}

impl From<&crate::terminal::MouseEventRecord> for PyMouseEvent {
    fn from(event: &crate::terminal::MouseEventRecord) -> Self {
        use crate::terminal::{MouseButton, MouseEventType};

        let event_type = match event.event_type {
            MouseEventType::Press => "press",
            MouseEventType::Release => "release",
            MouseEventType::Move => "move",
            MouseEventType::Drag => "drag",
            MouseEventType::ScrollUp => "scrollup",
            MouseEventType::ScrollDown => "scrolldown",
        }
        .to_string();

        let button = match event.button {
            MouseButton::Left => "left",
            MouseButton::Middle => "middle",
            MouseButton::Right => "right",
            MouseButton::None => "none",
        }
        .to_string();

        PyMouseEvent {
            event_type,
            button,
            col: event.col,
            row: event.row,
            pixel_x: event.pixel_x,
            pixel_y: event.pixel_y,
            modifiers: event.modifiers,
            timestamp: event.timestamp,
        }
    }
}

/// Mouse position
#[pyclass(name = "MousePosition")]
#[derive(Clone)]
pub struct PyMousePosition {
    #[pyo3(get)]
    pub col: usize,
    #[pyo3(get)]
    pub row: usize,
    #[pyo3(get)]
    pub timestamp: u64,
}

#[pymethods]
impl PyMousePosition {
    fn __repr__(&self) -> String {
        format!(
            "MousePosition(col={}, row={}, timestamp={})",
            self.col, self.row, self.timestamp
        )
    }
}

impl From<&crate::terminal::MousePosition> for PyMousePosition {
    fn from(pos: &crate::terminal::MousePosition) -> Self {
        PyMousePosition {
            col: pos.col,
            row: pos.row,
            timestamp: pos.timestamp,
        }
    }
}

// === Feature 19: Custom Rendering Hints ===

/// Damage region
#[pyclass(name = "DamageRegion")]
#[derive(Clone)]
pub struct PyDamageRegion {
    #[pyo3(get)]
    pub left: usize,
    #[pyo3(get)]
    pub top: usize,
    #[pyo3(get)]
    pub right: usize,
    #[pyo3(get)]
    pub bottom: usize,
}

#[pymethods]
impl PyDamageRegion {
    fn __repr__(&self) -> String {
        format!(
            "DamageRegion(left={}, top={}, right={}, bottom={})",
            self.left, self.top, self.right, self.bottom
        )
    }
}

impl From<&crate::terminal::DamageRegion> for PyDamageRegion {
    fn from(region: &crate::terminal::DamageRegion) -> Self {
        PyDamageRegion {
            left: region.left,
            top: region.top,
            right: region.right,
            bottom: region.bottom,
        }
    }
}

/// Rendering hint
#[pyclass(name = "RenderingHint")]
#[derive(Clone)]
pub struct PyRenderingHint {
    #[pyo3(get)]
    pub damage: PyDamageRegion,
    #[pyo3(get)]
    pub layer: String,
    #[pyo3(get)]
    pub animation: String,
    #[pyo3(get)]
    pub priority: u8,
}

#[pymethods]
impl PyRenderingHint {
    fn __repr__(&self) -> String {
        format!(
            "RenderingHint(layer={}, animation={}, priority={})",
            self.layer, self.animation, self.priority
        )
    }
}

impl From<&crate::terminal::RenderingHint> for PyRenderingHint {
    fn from(hint: &crate::terminal::RenderingHint) -> Self {
        use crate::terminal::{AnimationHint, ZLayer};

        let layer = match hint.layer {
            ZLayer::Background => "background",
            ZLayer::Normal => "normal",
            ZLayer::Overlay => "overlay",
            ZLayer::Cursor => "cursor",
        }
        .to_string();

        let animation = match hint.animation {
            AnimationHint::None => "none",
            AnimationHint::SmoothScroll => "smoothscroll",
            AnimationHint::Fade => "fade",
            AnimationHint::CursorBlink => "cursorblink",
        }
        .to_string();

        PyRenderingHint {
            damage: PyDamageRegion::from(&hint.damage),
            layer,
            animation,
            priority: hint.priority as u8,
        }
    }
}

// === Feature 16: Performance Profiling ===

/// Escape sequence profile
#[pyclass(name = "EscapeSequenceProfile")]
#[derive(Clone)]
pub struct PyEscapeSequenceProfile {
    #[pyo3(get)]
    pub count: u64,
    #[pyo3(get)]
    pub total_time_us: u64,
    #[pyo3(get)]
    pub peak_time_us: u64,
    #[pyo3(get)]
    pub avg_time_us: u64,
}

#[pymethods]
impl PyEscapeSequenceProfile {
    fn __repr__(&self) -> String {
        format!(
            "EscapeSequenceProfile(count={}, avg_us={}, peak_us={})",
            self.count, self.avg_time_us, self.peak_time_us
        )
    }
}

impl From<&crate::terminal::EscapeSequenceProfile> for PyEscapeSequenceProfile {
    fn from(profile: &crate::terminal::EscapeSequenceProfile) -> Self {
        PyEscapeSequenceProfile {
            count: profile.count,
            total_time_us: profile.total_time_us,
            peak_time_us: profile.peak_time_us,
            avg_time_us: profile.avg_time_us,
        }
    }
}

/// Profiling data
#[pyclass(name = "ProfilingData")]
#[derive(Clone)]
pub struct PyProfilingData {
    #[pyo3(get)]
    pub categories: std::collections::HashMap<String, PyEscapeSequenceProfile>,
    #[pyo3(get)]
    pub allocations: u64,
    #[pyo3(get)]
    pub bytes_allocated: u64,
    #[pyo3(get)]
    pub peak_memory: usize,
}

#[pymethods]
impl PyProfilingData {
    fn __repr__(&self) -> String {
        format!(
            "ProfilingData(categories={}, allocations={}, peak_memory={})",
            self.categories.len(),
            self.allocations,
            self.peak_memory
        )
    }
}

impl From<&crate::terminal::ProfilingData> for PyProfilingData {
    fn from(data: &crate::terminal::ProfilingData) -> Self {
        use crate::terminal::ProfileCategory;

        let mut categories = std::collections::HashMap::new();
        for (cat, profile) in &data.categories {
            let key = match cat {
                ProfileCategory::CSI => "csi",
                ProfileCategory::OSC => "osc",
                ProfileCategory::ESC => "esc",
                ProfileCategory::DCS => "dcs",
                ProfileCategory::Print => "print",
                ProfileCategory::Control => "control",
            }
            .to_string();
            categories.insert(key, PyEscapeSequenceProfile::from(profile));
        }

        PyProfilingData {
            categories,
            allocations: data.allocations,
            bytes_allocated: data.bytes_allocated,
            peak_memory: data.peak_memory,
        }
    }
}

// === Feature 14: Snapshot Diffing ===

/// Line diff
#[pyclass(name = "LineDiff")]
#[derive(Clone)]
pub struct PyLineDiff {
    #[pyo3(get)]
    pub change_type: String,
    #[pyo3(get)]
    pub old_row: Option<usize>,
    #[pyo3(get)]
    pub new_row: Option<usize>,
    #[pyo3(get)]
    pub old_content: Option<String>,
    #[pyo3(get)]
    pub new_content: Option<String>,
}

#[pymethods]
impl PyLineDiff {
    fn __repr__(&self) -> String {
        format!(
            "LineDiff(type={}, old_row={:?}, new_row={:?})",
            self.change_type, self.old_row, self.new_row
        )
    }
}

impl From<&crate::terminal::LineDiff> for PyLineDiff {
    fn from(diff: &crate::terminal::LineDiff) -> Self {
        use crate::terminal::DiffChangeType;

        let change_type = match diff.change_type {
            DiffChangeType::Added => "added",
            DiffChangeType::Removed => "removed",
            DiffChangeType::Modified => "modified",
            DiffChangeType::Unchanged => "unchanged",
        }
        .to_string();

        PyLineDiff {
            change_type,
            old_row: diff.old_row,
            new_row: diff.new_row,
            old_content: diff.old_content.clone(),
            new_content: diff.new_content.clone(),
        }
    }
}

/// Snapshot diff
#[pyclass(name = "SnapshotDiff")]
#[derive(Clone)]
pub struct PySnapshotDiff {
    #[pyo3(get)]
    pub diffs: Vec<PyLineDiff>,
    #[pyo3(get)]
    pub added: usize,
    #[pyo3(get)]
    pub removed: usize,
    #[pyo3(get)]
    pub modified: usize,
    #[pyo3(get)]
    pub unchanged: usize,
}

#[pymethods]
impl PySnapshotDiff {
    fn __repr__(&self) -> String {
        format!(
            "SnapshotDiff(added={}, removed={}, modified={}, unchanged={})",
            self.added, self.removed, self.modified, self.unchanged
        )
    }
}

impl From<&crate::terminal::SnapshotDiff> for PySnapshotDiff {
    fn from(diff: &crate::terminal::SnapshotDiff) -> Self {
        PySnapshotDiff {
            diffs: diff.diffs.iter().map(PyLineDiff::from).collect(),
            added: diff.added,
            removed: diff.removed,
            modified: diff.modified,
            unchanged: diff.unchanged,
        }
    }
}

// === Feature 15: Regex Search ===

/// Regex match
#[pyclass(name = "RegexMatch")]
#[derive(Clone)]
pub struct PyRegexMatch {
    #[pyo3(get)]
    pub row: usize,
    #[pyo3(get)]
    pub col: usize,
    #[pyo3(get)]
    pub end_row: usize,
    #[pyo3(get)]
    pub end_col: usize,
    #[pyo3(get)]
    pub text: String,
    #[pyo3(get)]
    pub captures: Vec<String>,
}

#[pymethods]
impl PyRegexMatch {
    fn __repr__(&self) -> String {
        format!(
            "RegexMatch(row={}, col={}, text={:?})",
            self.row, self.col, self.text
        )
    }
}

impl From<&crate::terminal::RegexMatch> for PyRegexMatch {
    fn from(m: &crate::terminal::RegexMatch) -> Self {
        PyRegexMatch {
            row: m.row,
            col: m.col,
            end_row: m.end_row,
            end_col: m.end_col,
            text: m.text.clone(),
            captures: m.captures.clone(),
        }
    }
}

// === Feature 13: Multiplexing ===

/// Pane state
#[pyclass(name = "PaneState")]
#[derive(Clone)]
pub struct PyPaneState {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub title: String,
    #[pyo3(get)]
    pub size: (usize, usize),
    #[pyo3(get)]
    pub position: (usize, usize),
    #[pyo3(get)]
    pub cwd: Option<String>,
    #[pyo3(get)]
    pub content: Vec<String>,
    #[pyo3(get)]
    pub cursor: (usize, usize),
    #[pyo3(get)]
    pub alt_screen: bool,
    #[pyo3(get)]
    pub scroll_offset: usize,
    #[pyo3(get)]
    pub created_at: u64,
    #[pyo3(get)]
    pub last_activity: u64,
}

#[pymethods]
impl PyPaneState {
    fn __repr__(&self) -> String {
        format!(
            "PaneState(id={}, title={}, size={}x{})",
            self.id, self.title, self.size.0, self.size.1
        )
    }
}

impl From<&crate::terminal::PaneState> for PyPaneState {
    fn from(state: &crate::terminal::PaneState) -> Self {
        PyPaneState {
            id: state.id.clone(),
            title: state.title.clone(),
            size: state.size,
            position: state.position,
            cwd: state.cwd.clone(),
            content: state.content.clone(),
            cursor: state.cursor,
            alt_screen: state.alt_screen,
            scroll_offset: state.scroll_offset,
            created_at: state.created_at,
            last_activity: state.last_activity,
        }
    }
}

/// Window layout
#[pyclass(name = "WindowLayout")]
#[derive(Clone)]
pub struct PyWindowLayout {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub direction: String,
    #[pyo3(get)]
    pub panes: Vec<String>,
    #[pyo3(get)]
    pub sizes: Vec<u8>,
    #[pyo3(get)]
    pub active_pane: usize,
}

#[pymethods]
impl PyWindowLayout {
    fn __repr__(&self) -> String {
        format!(
            "WindowLayout(id={}, name={}, panes={})",
            self.id,
            self.name,
            self.panes.len()
        )
    }
}

impl From<&crate::terminal::WindowLayout> for PyWindowLayout {
    fn from(layout: &crate::terminal::WindowLayout) -> Self {
        use crate::terminal::LayoutDirection;

        let direction = match layout.direction {
            LayoutDirection::Horizontal => "horizontal",
            LayoutDirection::Vertical => "vertical",
        }
        .to_string();

        PyWindowLayout {
            id: layout.id.clone(),
            name: layout.name.clone(),
            direction,
            panes: layout.panes.clone(),
            sizes: layout.sizes.clone(),
            active_pane: layout.active_pane,
        }
    }
}

/// Session state
#[pyclass(name = "SessionState")]
#[derive(Clone)]
pub struct PySessionState {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub panes: Vec<PyPaneState>,
    #[pyo3(get)]
    pub layouts: Vec<PyWindowLayout>,
    #[pyo3(get)]
    pub active_layout: usize,
    #[pyo3(get)]
    pub created_at: u64,
    #[pyo3(get)]
    pub last_saved: u64,
}

#[pymethods]
impl PySessionState {
    fn __repr__(&self) -> String {
        format!(
            "SessionState(id={}, name={}, panes={}, layouts={})",
            self.id,
            self.name,
            self.panes.len(),
            self.layouts.len()
        )
    }
}

impl From<&crate::terminal::SessionState> for PySessionState {
    fn from(session: &crate::terminal::SessionState) -> Self {
        PySessionState {
            id: session.id.clone(),
            name: session.name.clone(),
            panes: session.panes.iter().map(PyPaneState::from).collect(),
            layouts: session.layouts.iter().map(PyWindowLayout::from).collect(),
            active_layout: session.active_layout,
            created_at: session.created_at,
            last_saved: session.last_saved,
        }
    }
}

// === Feature 21: Image Protocol Support ===

/// Image protocol
#[pyclass(name = "ImageProtocol")]
#[derive(Clone)]
pub enum PyImageProtocol {
    Sixel,
    ITerm2,
    Kitty,
}

/// Image format
#[pyclass(name = "ImageFormat")]
#[derive(Clone)]
pub enum PyImageFormat {
    PNG,
    JPEG,
    GIF,
    BMP,
    RGBA,
    RGB,
}

/// Inline image
#[pyclass(name = "InlineImage")]
#[derive(Clone)]
pub struct PyInlineImage {
    #[pyo3(get)]
    pub id: Option<String>,
    #[pyo3(get)]
    pub protocol: String,
    #[pyo3(get)]
    pub format: String,
    #[pyo3(get)]
    pub data: Vec<u8>,
    #[pyo3(get)]
    pub width: u32,
    #[pyo3(get)]
    pub height: u32,
    #[pyo3(get)]
    pub position: (usize, usize),
    #[pyo3(get)]
    pub display_cols: usize,
    #[pyo3(get)]
    pub display_rows: usize,
}

#[pymethods]
impl PyInlineImage {
    fn __repr__(&self) -> String {
        format!(
            "InlineImage(protocol={}, format={}, size={}x{}, pos={:?})",
            self.protocol, self.format, self.width, self.height, self.position
        )
    }
}

impl From<&crate::terminal::InlineImage> for PyInlineImage {
    fn from(img: &crate::terminal::InlineImage) -> Self {
        use crate::terminal::{ImageFormat, ImageProtocol};

        let protocol = match img.protocol {
            ImageProtocol::Sixel => "sixel",
            ImageProtocol::ITerm2 => "iterm2",
            ImageProtocol::Kitty => "kitty",
        }
        .to_string();

        let format = match img.format {
            ImageFormat::PNG => "png",
            ImageFormat::JPEG => "jpeg",
            ImageFormat::GIF => "gif",
            ImageFormat::BMP => "bmp",
            ImageFormat::RGBA => "rgba",
            ImageFormat::RGB => "rgb",
        }
        .to_string();

        PyInlineImage {
            id: img.id.clone(),
            protocol,
            format,
            data: img.data.clone(),
            width: img.width,
            height: img.height,
            position: img.position,
            display_cols: img.display_cols,
            display_rows: img.display_rows,
        }
    }
}

// === Feature 28: Benchmarking Suite ===

/// Benchmark result
#[pyclass(name = "BenchmarkResult")]
#[derive(Clone)]
pub struct PyBenchmarkResult {
    #[pyo3(get)]
    pub category: String,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub iterations: u64,
    #[pyo3(get)]
    pub total_time_us: u64,
    #[pyo3(get)]
    pub avg_time_us: u64,
    #[pyo3(get)]
    pub min_time_us: u64,
    #[pyo3(get)]
    pub max_time_us: u64,
    #[pyo3(get)]
    pub ops_per_sec: f64,
    #[pyo3(get)]
    pub memory_bytes: Option<usize>,
}

#[pymethods]
impl PyBenchmarkResult {
    fn __repr__(&self) -> String {
        format!(
            "BenchmarkResult(category={}, name={}, iterations={}, avg_us={}, ops/sec={:.0})",
            self.category, self.name, self.iterations, self.avg_time_us, self.ops_per_sec
        )
    }
}

impl From<&crate::terminal::BenchmarkResult> for PyBenchmarkResult {
    fn from(result: &crate::terminal::BenchmarkResult) -> Self {
        use crate::terminal::BenchmarkCategory;

        let category = match result.category {
            BenchmarkCategory::Rendering => "rendering",
            BenchmarkCategory::Parsing => "parsing",
            BenchmarkCategory::GridOps => "gridops",
            BenchmarkCategory::Scrollback => "scrollback",
            BenchmarkCategory::Memory => "memory",
            BenchmarkCategory::Throughput => "throughput",
        }
        .to_string();

        PyBenchmarkResult {
            category,
            name: result.name.clone(),
            iterations: result.iterations,
            total_time_us: result.total_time_us,
            avg_time_us: result.avg_time_us,
            min_time_us: result.min_time_us,
            max_time_us: result.max_time_us,
            ops_per_sec: result.ops_per_sec,
            memory_bytes: result.memory_bytes,
        }
    }
}

/// Benchmark suite
#[pyclass(name = "BenchmarkSuite")]
#[derive(Clone)]
pub struct PyBenchmarkSuite {
    #[pyo3(get)]
    pub results: Vec<PyBenchmarkResult>,
    #[pyo3(get)]
    pub total_time_ms: u64,
    #[pyo3(get)]
    pub suite_name: String,
}

#[pymethods]
impl PyBenchmarkSuite {
    fn __repr__(&self) -> String {
        format!(
            "BenchmarkSuite(name={}, tests={}, time={}ms)",
            self.suite_name,
            self.results.len(),
            self.total_time_ms
        )
    }
}

impl From<&crate::terminal::BenchmarkSuite> for PyBenchmarkSuite {
    fn from(suite: &crate::terminal::BenchmarkSuite) -> Self {
        PyBenchmarkSuite {
            results: suite.results.iter().map(PyBenchmarkResult::from).collect(),
            total_time_ms: suite.total_time_ms,
            suite_name: suite.suite_name.clone(),
        }
    }
}

// === Feature 29: Terminal Compliance Testing ===

/// Compliance test
#[pyclass(name = "ComplianceTest")]
#[derive(Clone)]
pub struct PyComplianceTest {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub category: String,
    #[pyo3(get)]
    pub passed: bool,
    #[pyo3(get)]
    pub expected: String,
    #[pyo3(get)]
    pub actual: String,
    #[pyo3(get)]
    pub notes: Option<String>,
}

#[pymethods]
impl PyComplianceTest {
    fn __repr__(&self) -> String {
        format!(
            "ComplianceTest(name={}, category={}, passed={})",
            self.name, self.category, self.passed
        )
    }
}

impl From<&crate::terminal::ComplianceTest> for PyComplianceTest {
    fn from(test: &crate::terminal::ComplianceTest) -> Self {
        PyComplianceTest {
            name: test.name.clone(),
            category: test.category.clone(),
            passed: test.passed,
            expected: test.expected.clone(),
            actual: test.actual.clone(),
            notes: test.notes.clone(),
        }
    }
}

/// Compliance report
#[pyclass(name = "ComplianceReport")]
#[derive(Clone)]
pub struct PyComplianceReport {
    #[pyo3(get)]
    pub terminal_info: String,
    #[pyo3(get)]
    pub level: String,
    #[pyo3(get)]
    pub tests: Vec<PyComplianceTest>,
    #[pyo3(get)]
    pub passed: usize,
    #[pyo3(get)]
    pub failed: usize,
    #[pyo3(get)]
    pub compliance_percent: f64,
}

#[pymethods]
impl PyComplianceReport {
    fn __repr__(&self) -> String {
        format!(
            "ComplianceReport(level={}, passed={}/{}, compliance={:.1}%)",
            self.level,
            self.passed,
            self.passed + self.failed,
            self.compliance_percent
        )
    }
}

impl From<&crate::terminal::ComplianceReport> for PyComplianceReport {
    fn from(report: &crate::terminal::ComplianceReport) -> Self {
        use crate::terminal::ComplianceLevel;

        let level = match report.level {
            ComplianceLevel::VT52 => "vt52",
            ComplianceLevel::VT100 => "vt100",
            ComplianceLevel::VT220 => "vt220",
            ComplianceLevel::VT320 => "vt320",
            ComplianceLevel::VT420 => "vt420",
            ComplianceLevel::VT520 => "vt520",
            ComplianceLevel::XTerm => "xterm",
        }
        .to_string();

        PyComplianceReport {
            terminal_info: report.terminal_info.clone(),
            level,
            tests: report.tests.iter().map(PyComplianceTest::from).collect(),
            passed: report.passed,
            failed: report.failed,
            compliance_percent: report.compliance_percent,
        }
    }
}

// === Feature 30: OSC 52 Clipboard Sync ===

/// Clipboard sync event
#[pyclass(name = "ClipboardSyncEvent")]
#[derive(Clone)]
pub struct PyClipboardSyncEvent {
    #[pyo3(get)]
    pub target: String,
    #[pyo3(get)]
    pub operation: String,
    #[pyo3(get)]
    pub content: Option<String>,
    #[pyo3(get)]
    pub timestamp: u64,
    #[pyo3(get)]
    pub is_remote: bool,
}

#[pymethods]
impl PyClipboardSyncEvent {
    fn __repr__(&self) -> String {
        format!(
            "ClipboardSyncEvent(target={}, operation={}, is_remote={})",
            self.target, self.operation, self.is_remote
        )
    }
}

impl From<&crate::terminal::ClipboardSyncEvent> for PyClipboardSyncEvent {
    fn from(event: &crate::terminal::ClipboardSyncEvent) -> Self {
        use crate::terminal::{ClipboardOperation, ClipboardTarget};

        let target = match event.target {
            ClipboardTarget::Clipboard => "clipboard",
            ClipboardTarget::Primary => "primary",
            ClipboardTarget::Secondary => "secondary",
            ClipboardTarget::CutBuffer0 => "cutbuffer0",
        }
        .to_string();

        let operation = match event.operation {
            ClipboardOperation::Set => "set",
            ClipboardOperation::Query => "query",
            ClipboardOperation::Clear => "clear",
        }
        .to_string();

        PyClipboardSyncEvent {
            target,
            operation,
            content: event.content.clone(),
            timestamp: event.timestamp,
            is_remote: event.is_remote,
        }
    }
}

/// Clipboard history entry
#[pyclass(name = "ClipboardHistoryEntry")]
#[derive(Clone)]
pub struct PyClipboardHistoryEntry {
    #[pyo3(get)]
    pub target: String,
    #[pyo3(get)]
    pub content: String,
    #[pyo3(get)]
    pub timestamp: u64,
    #[pyo3(get)]
    pub source: Option<String>,
}

#[pymethods]
impl PyClipboardHistoryEntry {
    fn __repr__(&self) -> String {
        format!(
            "ClipboardHistoryEntry(target={}, content_len={}, timestamp={})",
            self.target,
            self.content.len(),
            self.timestamp
        )
    }
}

impl From<&crate::terminal::ClipboardHistoryEntry> for PyClipboardHistoryEntry {
    fn from(entry: &crate::terminal::ClipboardHistoryEntry) -> Self {
        use crate::terminal::ClipboardTarget;

        let target = match entry.target {
            ClipboardTarget::Clipboard => "clipboard",
            ClipboardTarget::Primary => "primary",
            ClipboardTarget::Secondary => "secondary",
            ClipboardTarget::CutBuffer0 => "cutbuffer0",
        }
        .to_string();

        PyClipboardHistoryEntry {
            target,
            content: entry.content.clone(),
            timestamp: entry.timestamp,
            source: entry.source.clone(),
        }
    }
}

// === Feature 31: Shell Integration++ ===

/// Command execution record
#[pyclass(name = "CommandExecution")]
#[derive(Clone)]
pub struct PyCommandExecution {
    #[pyo3(get)]
    pub command: String,
    #[pyo3(get)]
    pub cwd: Option<String>,
    #[pyo3(get)]
    pub start_time: u64,
    #[pyo3(get)]
    pub end_time: Option<u64>,
    #[pyo3(get)]
    pub exit_code: Option<i32>,
    #[pyo3(get)]
    pub duration_ms: Option<u64>,
    #[pyo3(get)]
    pub success: Option<bool>,
}

#[pymethods]
impl PyCommandExecution {
    fn __repr__(&self) -> String {
        format!(
            "CommandExecution(command={:?}, exit_code={:?}, duration={:?}ms)",
            self.command, self.exit_code, self.duration_ms
        )
    }
}

impl From<&crate::terminal::CommandExecution> for PyCommandExecution {
    fn from(cmd: &crate::terminal::CommandExecution) -> Self {
        PyCommandExecution {
            command: cmd.command.clone(),
            cwd: cmd.cwd.clone(),
            start_time: cmd.start_time,
            end_time: cmd.end_time,
            exit_code: cmd.exit_code,
            duration_ms: cmd.duration_ms,
            success: cmd.success,
        }
    }
}

/// Shell integration statistics
#[pyclass(name = "ShellIntegrationStats")]
#[derive(Clone)]
pub struct PyShellIntegrationStats {
    #[pyo3(get)]
    pub total_commands: usize,
    #[pyo3(get)]
    pub successful_commands: usize,
    #[pyo3(get)]
    pub failed_commands: usize,
    #[pyo3(get)]
    pub avg_duration_ms: f64,
    #[pyo3(get)]
    pub total_duration_ms: u64,
}

#[pymethods]
impl PyShellIntegrationStats {
    fn __repr__(&self) -> String {
        format!(
            "ShellIntegrationStats(total={}, success={}, failed={}, avg_ms={:.1})",
            self.total_commands,
            self.successful_commands,
            self.failed_commands,
            self.avg_duration_ms
        )
    }
}

impl From<&crate::terminal::ShellIntegrationStats> for PyShellIntegrationStats {
    fn from(stats: &crate::terminal::ShellIntegrationStats) -> Self {
        PyShellIntegrationStats {
            total_commands: stats.total_commands,
            successful_commands: stats.successful_commands,
            failed_commands: stats.failed_commands,
            avg_duration_ms: stats.avg_duration_ms,
            total_duration_ms: stats.total_duration_ms,
        }
    }
}

/// CWD change notification
#[pyclass(name = "CwdChange")]
#[derive(Clone)]
pub struct PyCwdChange {
    #[pyo3(get)]
    pub old_cwd: Option<String>,
    #[pyo3(get)]
    pub new_cwd: String,
    #[pyo3(get)]
    pub timestamp: u64,
}

#[pymethods]
impl PyCwdChange {
    fn __repr__(&self) -> String {
        format!("CwdChange(old={:?}, new={:?})", self.old_cwd, self.new_cwd)
    }
}

impl From<&crate::terminal::CwdChange> for PyCwdChange {
    fn from(change: &crate::terminal::CwdChange) -> Self {
        PyCwdChange {
            old_cwd: change.old_cwd.clone(),
            new_cwd: change.new_cwd.clone(),
            timestamp: change.timestamp,
        }
    }
}

// === Feature 37: Terminal Notifications ===

/// Notification event
#[pyclass(name = "NotificationEvent")]
#[derive(Clone)]
pub struct PyNotificationEvent {
    #[pyo3(get)]
    pub trigger: String,
    #[pyo3(get)]
    pub alert: String,
    #[pyo3(get)]
    pub message: Option<String>,
    #[pyo3(get)]
    pub timestamp: u64,
    #[pyo3(get)]
    pub delivered: bool,
}

#[pymethods]
impl PyNotificationEvent {
    fn __repr__(&self) -> String {
        format!(
            "NotificationEvent(trigger={}, alert={}, delivered={})",
            self.trigger, self.alert, self.delivered
        )
    }
}

impl From<&crate::terminal::NotificationEvent> for PyNotificationEvent {
    fn from(event: &crate::terminal::NotificationEvent) -> Self {
        let trigger = match event.trigger {
            crate::terminal::NotificationTrigger::Bell => "Bell".to_string(),
            crate::terminal::NotificationTrigger::Activity => "Activity".to_string(),
            crate::terminal::NotificationTrigger::Silence => "Silence".to_string(),
            crate::terminal::NotificationTrigger::Custom(id) => format!("Custom({})", id),
        };

        let alert = match event.alert {
            crate::terminal::NotificationAlert::Desktop => "Desktop".to_string(),
            crate::terminal::NotificationAlert::Sound(vol) => format!("Sound({})", vol),
            crate::terminal::NotificationAlert::Visual => "Visual".to_string(),
        };

        PyNotificationEvent {
            trigger,
            alert,
            message: event.message.clone(),
            timestamp: event.timestamp,
            delivered: event.delivered,
        }
    }
}

/// Notification configuration
#[pyclass(name = "NotificationConfig")]
#[derive(Clone)]
pub struct PyNotificationConfig {
    #[pyo3(get, set)]
    pub bell_desktop: bool,
    #[pyo3(get, set)]
    pub bell_sound: u8,
    #[pyo3(get, set)]
    pub bell_visual: bool,
    #[pyo3(get, set)]
    pub activity_enabled: bool,
    #[pyo3(get, set)]
    pub activity_threshold: u64,
    #[pyo3(get, set)]
    pub silence_enabled: bool,
    #[pyo3(get, set)]
    pub silence_threshold: u64,
}

#[pymethods]
impl PyNotificationConfig {
    #[new]
    fn new() -> Self {
        PyNotificationConfig::default()
    }

    fn __repr__(&self) -> String {
        format!(
            "NotificationConfig(bell_desktop={}, bell_visual={}, activity={}, silence={})",
            self.bell_desktop, self.bell_visual, self.activity_enabled, self.silence_enabled
        )
    }
}

impl Default for PyNotificationConfig {
    fn default() -> Self {
        let config = crate::terminal::NotificationConfig::default();
        PyNotificationConfig {
            bell_desktop: config.bell_desktop,
            bell_sound: config.bell_sound,
            bell_visual: config.bell_visual,
            activity_enabled: config.activity_enabled,
            activity_threshold: config.activity_threshold,
            silence_enabled: config.silence_enabled,
            silence_threshold: config.silence_threshold,
        }
    }
}

impl From<&crate::terminal::NotificationConfig> for PyNotificationConfig {
    fn from(config: &crate::terminal::NotificationConfig) -> Self {
        PyNotificationConfig {
            bell_desktop: config.bell_desktop,
            bell_sound: config.bell_sound,
            bell_visual: config.bell_visual,
            activity_enabled: config.activity_enabled,
            activity_threshold: config.activity_threshold,
            silence_enabled: config.silence_enabled,
            silence_threshold: config.silence_threshold,
        }
    }
}

impl From<&PyNotificationConfig> for crate::terminal::NotificationConfig {
    fn from(config: &PyNotificationConfig) -> Self {
        crate::terminal::NotificationConfig {
            bell_desktop: config.bell_desktop,
            bell_sound: config.bell_sound,
            bell_visual: config.bell_visual,
            activity_enabled: config.activity_enabled,
            activity_threshold: config.activity_threshold,
            silence_enabled: config.silence_enabled,
            silence_threshold: config.silence_threshold,
        }
    }
}

// === Feature 24: Terminal Replay/Recording ===

/// Recording event
#[pyclass(name = "RecordingEvent")]
#[derive(Clone)]
pub struct PyRecordingEvent {
    #[pyo3(get)]
    pub timestamp: u64,
    #[pyo3(get)]
    pub event_type: String,
    #[pyo3(get)]
    pub data: Vec<u8>,
    #[pyo3(get)]
    pub metadata: Option<(usize, usize)>,
}

#[pymethods]
impl PyRecordingEvent {
    fn __repr__(&self) -> String {
        format!(
            "RecordingEvent(type={}, timestamp={}ms, data_len={})",
            self.event_type,
            self.timestamp,
            self.data.len()
        )
    }

    /// Get event data as string
    fn get_data_str(&self) -> String {
        String::from_utf8_lossy(&self.data).to_string()
    }
}

impl From<&crate::terminal::RecordingEvent> for PyRecordingEvent {
    fn from(event: &crate::terminal::RecordingEvent) -> Self {
        let event_type = match event.event_type {
            crate::terminal::RecordingEventType::Input => "Input".to_string(),
            crate::terminal::RecordingEventType::Output => "Output".to_string(),
            crate::terminal::RecordingEventType::Resize => "Resize".to_string(),
            crate::terminal::RecordingEventType::Marker => "Marker".to_string(),
        };

        PyRecordingEvent {
            timestamp: event.timestamp,
            event_type,
            data: event.data.clone(),
            metadata: event.metadata,
        }
    }
}

/// Recording session
#[pyclass(name = "RecordingSession")]
#[derive(Clone)]
pub struct PyRecordingSession {
    pub(crate) inner: crate::terminal::RecordingSession,
}

#[pymethods]
impl PyRecordingSession {
    fn __repr__(&self) -> String {
        format!(
            "RecordingSession(duration={}ms, size={:?}, events={})",
            self.inner.duration,
            self.inner.initial_size,
            self.inner.events.len()
        )
    }

    /// Get recording size (cols, rows)
    fn get_size(&self) -> (usize, usize) {
        self.inner.initial_size
    }

    /// Get duration in seconds
    fn get_duration_seconds(&self) -> f64 {
        self.inner.duration as f64 / 1000.0
    }

    #[getter]
    fn start_time(&self) -> u64 {
        self.inner.start_time
    }

    #[getter]
    fn initial_size(&self) -> (usize, usize) {
        self.inner.initial_size
    }

    #[getter]
    fn duration(&self) -> u64 {
        self.inner.duration
    }

    #[getter]
    fn title(&self) -> Option<String> {
        self.inner.title.clone()
    }

    #[getter]
    fn event_count(&self) -> usize {
        self.inner.events.len()
    }
}

impl From<&crate::terminal::RecordingSession> for PyRecordingSession {
    fn from(session: &crate::terminal::RecordingSession) -> Self {
        PyRecordingSession {
            inner: session.clone(),
        }
    }
}

impl From<crate::terminal::RecordingSession> for PyRecordingSession {
    fn from(session: crate::terminal::RecordingSession) -> Self {
        PyRecordingSession { inner: session }
    }
}

/// Macro event
#[pyclass(name = "MacroEvent")]
#[derive(Clone)]
pub struct PyMacroEvent {
    #[pyo3(get)]
    pub event_type: String,
    #[pyo3(get)]
    pub timestamp: u64,
    #[pyo3(get)]
    pub key: Option<String>,
    #[pyo3(get)]
    pub duration: Option<u64>,
    #[pyo3(get)]
    pub label: Option<String>,
}

#[pymethods]
impl PyMacroEvent {
    fn __repr__(&self) -> String {
        match self.event_type.as_str() {
            "key" => format!(
                "MacroEvent(key={}, timestamp={}ms)",
                self.key.as_ref().unwrap(),
                self.timestamp
            ),
            "delay" => format!(
                "MacroEvent(delay={}ms, timestamp={}ms)",
                self.duration.unwrap(),
                self.timestamp
            ),
            "screenshot" => format!(
                "MacroEvent(screenshot, label={:?}, timestamp={}ms)",
                self.label, self.timestamp
            ),
            _ => "MacroEvent(unknown)".to_string(),
        }
    }
}

impl From<&crate::macros::MacroEvent> for PyMacroEvent {
    fn from(event: &crate::macros::MacroEvent) -> Self {
        match event {
            crate::macros::MacroEvent::KeyPress { key, timestamp } => PyMacroEvent {
                event_type: "key".to_string(),
                timestamp: *timestamp,
                key: Some(key.clone()),
                duration: None,
                label: None,
            },
            crate::macros::MacroEvent::Delay {
                duration,
                timestamp,
            } => PyMacroEvent {
                event_type: "delay".to_string(),
                timestamp: *timestamp,
                key: None,
                duration: Some(*duration),
                label: None,
            },
            crate::macros::MacroEvent::Screenshot { label, timestamp } => PyMacroEvent {
                event_type: "screenshot".to_string(),
                timestamp: *timestamp,
                key: None,
                duration: None,
                label: label.clone(),
            },
        }
    }
}

/// Macro recording
#[pyclass(name = "Macro")]
#[derive(Clone)]
pub struct PyMacro {
    pub(crate) inner: crate::macros::Macro,
}

#[pymethods]
impl PyMacro {
    /// Create a new macro
    #[new]
    fn new(name: String) -> Self {
        PyMacro {
            inner: crate::macros::Macro::new(name),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Macro(name={}, duration={}ms, events={})",
            self.inner.name,
            self.inner.duration,
            self.inner.events.len()
        )
    }

    /// Add a key press event
    fn add_key(&mut self, key: String) {
        self.inner.add_key(key);
    }

    /// Add a delay event
    fn add_delay(&mut self, duration_ms: u64) {
        self.inner.add_delay(duration_ms);
    }

    /// Add a screenshot trigger
    fn add_screenshot(&mut self, label: Option<String>) {
        self.inner.add_screenshot_labeled(label);
    }

    /// Set description
    fn set_description(&mut self, description: String) {
        self.inner.description = Some(description);
    }

    /// Save to YAML file
    fn save_yaml(&self, path: String) -> PyResult<()> {
        self.inner
            .save_yaml(path)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Load from YAML file
    #[staticmethod]
    fn load_yaml(path: String) -> PyResult<Self> {
        crate::macros::Macro::load_yaml(path)
            .map(|inner| PyMacro { inner })
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Convert to YAML string
    fn to_yaml(&self) -> PyResult<String> {
        self.inner
            .to_yaml()
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    }

    /// Parse from YAML string
    #[staticmethod]
    fn from_yaml(yaml: String) -> PyResult<Self> {
        crate::macros::Macro::from_yaml(&yaml)
            .map(|inner| PyMacro { inner })
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    }

    /// Get macro name
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// Get description
    #[getter]
    fn description(&self) -> Option<String> {
        self.inner.description.clone()
    }

    /// Get duration in milliseconds
    #[getter]
    fn duration(&self) -> u64 {
        self.inner.duration
    }

    /// Get terminal size (cols, rows)
    #[getter]
    fn terminal_size(&self) -> Option<(usize, usize)> {
        self.inner.terminal_size
    }

    /// Get number of events
    #[getter]
    fn event_count(&self) -> usize {
        self.inner.events.len()
    }

    /// Get all events
    #[getter]
    fn events(&self) -> Vec<PyMacroEvent> {
        self.inner.events.iter().map(PyMacroEvent::from).collect()
    }
}

impl From<crate::macros::Macro> for PyMacro {
    fn from(macro_data: crate::macros::Macro) -> Self {
        PyMacro { inner: macro_data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pyattributes_default() {
        let attrs = PyAttributes::default();

        assert!(!attrs.bold);
        assert!(!attrs.dim);
        assert!(!attrs.italic);
        assert!(!attrs.underline);
        assert!(!attrs.blink);
        assert!(!attrs.reverse);
        assert!(!attrs.hidden);
        assert!(!attrs.strikethrough);
        assert!(matches!(attrs.underline_style, PyUnderlineStyle::None));
        assert!(!attrs.wide_char);
        assert!(!attrs.wide_char_spacer);
        assert_eq!(attrs.hyperlink_id, None);
    }

    #[test]
    fn test_pyattributes_repr() {
        let attrs = PyAttributes {
            bold: true,
            italic: true,
            underline: true,
            underline_style: PyUnderlineStyle::Straight,
            ..Default::default()
        };

        let repr = attrs.__repr__().unwrap();
        assert!(repr.contains("bold=true"));
        assert!(repr.contains("italic=true"));
        assert!(repr.contains("underline=true"));
        assert!(repr.contains("Straight"));
    }

    #[test]
    fn test_pyattributes_repr_all_false() {
        let attrs = PyAttributes::default();
        let repr = attrs.__repr__().unwrap();

        assert!(repr.contains("bold=false"));
        assert!(repr.contains("italic=false"));
        assert!(repr.contains("underline=false"));
    }

    #[test]
    fn test_pyattributes_clone() {
        let attrs1 = PyAttributes {
            bold: true,
            italic: true,
            hyperlink_id: Some(42),
            ..Default::default()
        };

        let attrs2 = attrs1.clone();

        assert_eq!(attrs1.bold, attrs2.bold);
        assert_eq!(attrs1.italic, attrs2.italic);
        assert_eq!(attrs1.hyperlink_id, attrs2.hyperlink_id);
    }

    #[test]
    fn test_pyattributes_with_hyperlink() {
        let attrs = PyAttributes {
            hyperlink_id: Some(123),
            ..Default::default()
        };

        assert_eq!(attrs.hyperlink_id, Some(123));
    }

    #[test]
    fn test_pyattributes_all_flags() {
        let attrs = PyAttributes {
            bold: true,
            dim: true,
            italic: true,
            underline: true,
            blink: true,
            reverse: true,
            hidden: true,
            strikethrough: true,
            wide_char: true,
            wide_char_spacer: true,
            underline_style: PyUnderlineStyle::Curly,
            hyperlink_id: Some(99),
        };

        assert!(attrs.bold);
        assert!(attrs.dim);
        assert!(attrs.italic);
        assert!(attrs.underline);
        assert!(attrs.blink);
        assert!(attrs.reverse);
        assert!(attrs.hidden);
        assert!(attrs.strikethrough);
        assert!(attrs.wide_char);
        assert!(attrs.wide_char_spacer);
        assert!(matches!(attrs.underline_style, PyUnderlineStyle::Curly));
        assert_eq!(attrs.hyperlink_id, Some(99));
    }

    #[test]
    fn test_pyscreensnapshot_get_line_valid_row() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![
                (
                    "H".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ),
                (
                    "i".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ),
            ]],
            wrapped_lines: vec![false],
            cursor_pos: (0, 0),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: false,
            generation: 1,
            size: (80, 24),
        };

        let line = snapshot.get_line(0);
        assert_eq!(line.len(), 2);
        assert_eq!(line[0].0, "H");
        assert_eq!(line[1].0, "i");
    }

    #[test]
    fn test_pyscreensnapshot_get_line_out_of_bounds() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![(
                "A".to_string(),
                (255, 255, 255),
                (0, 0, 0),
                PyAttributes::default(),
            )]],
            wrapped_lines: vec![false],
            cursor_pos: (0, 0),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: false,
            generation: 1,
            size: (80, 24),
        };

        let line = snapshot.get_line(5); // Row 5 doesn't exist
        assert_eq!(line.len(), 0);
    }

    #[test]
    fn test_pyscreensnapshot_get_line_filters_control_chars() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![
                (
                    "\x00".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Control char
                (
                    "A".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Regular char
                (
                    "\x00".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // ESC
                (
                    " ".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Space (allowed)
                (
                    "\t".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Tab (allowed)
            ]],
            wrapped_lines: vec![false],
            cursor_pos: (0, 0),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: false,
            generation: 1,
            size: (80, 24),
        };

        let line = snapshot.get_line(0);
        assert_eq!(line.len(), 5);
        assert_eq!(line[0].0, " "); // Control char replaced with space
        assert_eq!(line[1].0, "A"); // Regular char unchanged
        assert_eq!(line[2].0, " "); // ESC replaced with space
        assert_eq!(line[3].0, " "); // Space unchanged
        assert_eq!(line[4].0, "\t"); // Tab unchanged
    }

    #[test]
    fn test_pyscreensnapshot_repr() {
        let snapshot = PyScreenSnapshot {
            lines: vec![],
            wrapped_lines: vec![],
            cursor_pos: (10, 5),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: true,
            generation: 42,
            size: (80, 24),
        };

        let repr = snapshot.__repr__().unwrap();
        assert!(repr.contains("80x24"));
        assert!(repr.contains("gen=42"));
        assert!(repr.contains("alt=true"));
    }

    #[test]
    fn test_pyscreensnapshot_repr_not_alt_screen() {
        let snapshot = PyScreenSnapshot {
            lines: vec![],
            wrapped_lines: vec![],
            cursor_pos: (0, 0),
            cursor_visible: false,
            cursor_style: PyCursorStyle::BlinkingBlock,
            is_alt_screen: false,
            generation: 100,
            size: (120, 30),
        };

        let repr = snapshot.__repr__().unwrap();
        assert!(repr.contains("120x30"));
        assert!(repr.contains("gen=100"));
        assert!(repr.contains("alt=false"));
    }

    #[test]
    fn test_pyshellintegration_repr() {
        let shell_int = PyShellIntegration {
            in_prompt: true,
            in_command_input: false,
            in_command_output: false,
            current_command: Some("ls -la".to_string()),
            last_exit_code: Some(0),
            cwd: Some("/home/user".to_string()),
        };

        let repr = shell_int.__repr__().unwrap();
        assert!(repr.contains("in_prompt=true"));
        assert!(repr.contains("in_command_input=false"));
        assert!(repr.contains("in_command_output=false"));
    }

    #[test]
    fn test_pyshellintegration_all_states() {
        let shell_int = PyShellIntegration {
            in_prompt: false,
            in_command_input: true,
            in_command_output: false,
            current_command: None,
            last_exit_code: None,
            cwd: None,
        };

        assert!(!shell_int.in_prompt);
        assert!(shell_int.in_command_input);
        assert!(!shell_int.in_command_output);
        assert_eq!(shell_int.current_command, None);
        assert_eq!(shell_int.last_exit_code, None);
        assert_eq!(shell_int.cwd, None);
    }

    #[test]
    fn test_pyshellintegration_clone() {
        let shell_int1 = PyShellIntegration {
            in_prompt: true,
            in_command_input: true,
            in_command_output: true,
            current_command: Some("echo test".to_string()),
            last_exit_code: Some(1),
            cwd: Some("/tmp".to_string()),
        };

        let shell_int2 = shell_int1.clone();

        assert_eq!(shell_int1.in_prompt, shell_int2.in_prompt);
        assert_eq!(shell_int1.current_command, shell_int2.current_command);
        assert_eq!(shell_int1.last_exit_code, shell_int2.last_exit_code);
        assert_eq!(shell_int1.cwd, shell_int2.cwd);
    }

    #[test]
    fn test_pygraphic_get_pixel_valid() {
        // Create a 2x2 pixel graphic with RGBA data
        let pixels = vec![
            255, 0, 0, 255, // Red pixel at (0, 0)
            0, 255, 0, 255, // Green pixel at (1, 0)
            0, 0, 255, 255, // Blue pixel at (0, 1)
            255, 255, 0, 255, // Yellow pixel at (1, 1)
        ];

        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 2,
            height: 2,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            pixels,
        };

        assert_eq!(graphic.get_pixel(0, 0), Some((255, 0, 0, 255))); // Red
        assert_eq!(graphic.get_pixel(1, 0), Some((0, 255, 0, 255))); // Green
        assert_eq!(graphic.get_pixel(0, 1), Some((0, 0, 255, 255))); // Blue
        assert_eq!(graphic.get_pixel(1, 1), Some((255, 255, 0, 255))); // Yellow
    }

    #[test]
    fn test_pygraphic_get_pixel_out_of_bounds() {
        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 2,
            height: 2,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            pixels: vec![0; 16], // 2x2 RGBA
        };

        assert_eq!(graphic.get_pixel(2, 0), None); // X out of bounds
        assert_eq!(graphic.get_pixel(0, 2), None); // Y out of bounds
        assert_eq!(graphic.get_pixel(2, 2), None); // Both out of bounds
    }

    #[test]
    fn test_pygraphic_get_pixel_edge_cases() {
        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (5, 10),
            width: 3,
            height: 3,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            pixels: vec![128; 36], // 3x3 RGBA with all values at 128
        };

        // Test valid edge pixels
        assert_eq!(graphic.get_pixel(0, 0), Some((128, 128, 128, 128)));
        assert_eq!(graphic.get_pixel(2, 0), Some((128, 128, 128, 128)));
        assert_eq!(graphic.get_pixel(0, 2), Some((128, 128, 128, 128)));
        assert_eq!(graphic.get_pixel(2, 2), Some((128, 128, 128, 128)));

        // Test just outside bounds
        assert_eq!(graphic.get_pixel(3, 0), None);
        assert_eq!(graphic.get_pixel(0, 3), None);
    }

    #[test]
    fn test_pygraphic_pixels_returns_copy() {
        let original_pixels = vec![10, 20, 30, 40, 50, 60, 70, 80];
        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 2,
            height: 1,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            pixels: original_pixels.clone(),
        };

        let retrieved = graphic.pixels();
        assert_eq!(retrieved, original_pixels);
        assert_eq!(retrieved.len(), 8); // 2 pixels * 4 channels
    }

    #[test]
    fn test_pygraphic_repr() {
        let graphic = PyGraphic {
            id: 42,
            protocol: "sixel".to_string(),
            position: (10, 20),
            width: 100,
            height: 50,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            pixels: vec![],
        };

        let repr = graphic.__repr__().unwrap();
        assert!(repr.contains("id=42"));
        assert!(repr.contains("protocol='sixel'"));
        assert!(repr.contains("position=(10,20)"));
        assert!(repr.contains("size=100x50"));
    }

    #[test]
    fn test_pygraphic_clone() {
        let graphic1 = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (5, 10),
            width: 20,
            height: 30,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            pixels: vec![1, 2, 3, 4],
        };

        let graphic2 = graphic1.clone();

        assert_eq!(graphic1.id, graphic2.id);
        assert_eq!(graphic1.protocol, graphic2.protocol);
        assert_eq!(graphic1.position, graphic2.position);
        assert_eq!(graphic1.width, graphic2.width);
        assert_eq!(graphic1.height, graphic2.height);
        assert_eq!(graphic1.pixels(), graphic2.pixels());
    }

    #[test]
    fn test_pygraphic_pixel_index_calculation() {
        // Test that pixel indexing is calculated correctly
        let mut pixels = vec![0u8; 16]; // 2x2 grid, RGBA

        // Manually set pixel at (1, 1) to red
        let x = 1usize;
        let y = 1usize;
        let width = 2usize;
        let idx = (y * width + x) * 4;

        pixels[idx] = 255; // R
        pixels[idx + 1] = 0; // G
        pixels[idx + 2] = 0; // B
        pixels[idx + 3] = 255; // A

        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 2,
            height: 2,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            pixels,
        };

        assert_eq!(graphic.get_pixel(1, 1), Some((255, 0, 0, 255)));
    }

    #[test]
    fn test_line_cell_data_type_alias() {
        // Test that the LineCellData type alias works correctly
        let cell_data: LineCellData = vec![
            (
                "A".to_string(),
                (255, 0, 0),
                (0, 0, 0),
                PyAttributes::default(),
            ),
            (
                "B".to_string(),
                (0, 255, 0),
                (0, 0, 0),
                PyAttributes::default(),
            ),
        ];

        assert_eq!(cell_data.len(), 2);
        assert_eq!(cell_data[0].0, "A");
        assert_eq!(cell_data[0].1, (255, 0, 0)); // Red
        assert_eq!(cell_data[1].0, "B");
        assert_eq!(cell_data[1].1, (0, 255, 0)); // Green
    }

    #[test]
    fn test_pyscreensnapshot_fields() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![]],
            wrapped_lines: vec![true, false],
            cursor_pos: (15, 10),
            cursor_visible: false,
            cursor_style: PyCursorStyle::BlinkingUnderline,
            is_alt_screen: true,
            generation: 999,
            size: (100, 50),
        };

        assert_eq!(snapshot.cursor_pos, (15, 10));
        assert!(!snapshot.cursor_visible);
        assert!(matches!(
            snapshot.cursor_style,
            PyCursorStyle::BlinkingUnderline
        ));
        assert!(snapshot.is_alt_screen);
        assert_eq!(snapshot.generation, 999);
        assert_eq!(snapshot.size, (100, 50));
        assert_eq!(snapshot.wrapped_lines.len(), 2);
        assert!(snapshot.wrapped_lines[0]);
        assert!(!snapshot.wrapped_lines[1]);
    }

    #[test]
    fn test_control_character_filtering_edge_cases() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![
                (
                    "\x00".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // NULL
                (
                    "\x1F".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Unit separator
                (
                    " ".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Space (32)
                (
                    "!".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // "!" (33)
            ]],
            wrapped_lines: vec![false],
            cursor_pos: (0, 0),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: false,
            generation: 1,
            size: (80, 24),
        };

        let line = snapshot.get_line(0);

        // Control chars (< 32) should be replaced with space
        assert_eq!(line[0].0, " "); // NULL -> space
        assert_eq!(line[1].0, " "); // Unit separator -> space

        // Space and above should be unchanged
        assert_eq!(line[2].0, " "); // Space unchanged
        assert_eq!(line[3].0, "!"); // "!" unchanged
    }

    #[test]
    fn test_pygraphic_alpha_channel() {
        // Test graphics with various alpha values
        let pixels = vec![
            255, 0, 0, 0, // Red, fully transparent
            0, 255, 0, 128, // Green, semi-transparent
            0, 0, 255, 255, // Blue, fully opaque
            128, 128, 128, 64, // Gray, mostly transparent
        ];

        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 4,
            height: 1,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            pixels,
        };

        assert_eq!(graphic.get_pixel(0, 0), Some((255, 0, 0, 0)));
        assert_eq!(graphic.get_pixel(1, 0), Some((0, 255, 0, 128)));
        assert_eq!(graphic.get_pixel(2, 0), Some((0, 0, 255, 255)));
        assert_eq!(graphic.get_pixel(3, 0), Some((128, 128, 128, 64)));
    }
}
