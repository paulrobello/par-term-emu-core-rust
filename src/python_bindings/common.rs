//! Shared Terminal access + method macros (ARC-003 / QA-001).
//!
//! `PyTerminal` owns a `Terminal` directly; `PyPtyTerminal` holds one behind an
//! `Arc<Mutex<Terminal>>` (via `PtySession`). Historically every shared method
//! was hand-duplicated between the two — ~155 copies that drift on every PR.
//!
//! [`TerminalAccess`] abstracts the "give me the terminal" step (returning a
//! `Deref<Target=Terminal>` guard for each type's native form), and the macros
//! below emit the shared `#[pymethods]` definitions ONCE, invoked per type.
//! Behavior is preserved (the macros use the clean `Terminal`-method form; the
//! duplicated `Ok::<_, ()>(lock())` dead-fallback copies in `pty.rs` are
//! dropped, which is safe because `parking_lot` locks never fail).

use crate::terminal::Terminal;

/// Unified read/write access to the underlying [`Terminal`] for both Python
/// wrapper types.
///
/// `term_ref` / `term_mut` return opaque `Deref`/`DerefMut` guards so each impl
/// can return its native borrow form (`&Terminal` for `PyTerminal`,
/// `MutexGuard<Terminal>` for `PyPtyTerminal`) while callers see a uniform
/// `Deref<Target = Terminal>`.
pub(crate) trait TerminalAccess {
    /// Shared (immutable) access to the terminal.
    fn term_ref(&self) -> impl std::ops::Deref<Target = Terminal>;
    /// Exclusive (mutable) access to the terminal.
    #[allow(dead_code)] // used once setters/mutating methods are migrated (ARC-003/QA-001 scaling)
    fn term_mut(&mut self) -> impl std::ops::DerefMut<Target = Terminal>;
}

/// Emit a small set of simple read-only getters for `$ty`, using
/// [`TerminalAccess::term_ref`]. Validates the shared-method macro pattern
/// (ARC-003/QA-001); the same shape scales to the full duplicated set.
#[macro_export]
macro_rules! impl_terminal_simple_getters {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Get the cursor color (OSC 12)
            fn cursor_color(&self) -> pyo3::PyResult<(u8, u8, u8)> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.cursor_color().to_rgb())
            }

            /// Whether the cursor is visible (DECTCE)
            fn cursor_visible(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.cursor().visible)
            }

            /// Whether bracketed-paste mode is active
            fn bracketed_paste(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.bracketed_paste())
            }

            /// Whether OSC 52 clipboard reads are allowed
            fn allow_clipboard_read(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.allow_clipboard_read())
            }

            /// Whether OSC 7 directory-tracking sequences are accepted
            fn accept_osc7(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.accept_osc7())
            }
        }
    };
}

/// Emit a batch of read-only query/state getters for `$ty`, using
/// [`TerminalAccess::term_ref`]. (ARC-003/QA-001 scaling batch 1.)
#[macro_export]
macro_rules! impl_terminal_query_getters {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Get the current terminal dimensions
            ///
            /// Returns:
            ///     Tuple of (cols, rows)
            fn size(&self) -> pyo3::PyResult<(usize, usize)> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.size())
            }

            /// Get the terminal title
            ///
            /// Returns:
            ///     Current terminal title string
            fn title(&self) -> pyo3::PyResult<String> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.title().to_string())
            }

            /// Get the cursor position
            ///
            /// Returns:
            ///     Tuple of (col, row)
            fn cursor_position(&self) -> pyo3::PyResult<(usize, usize)> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let cursor = t.cursor();
                Ok((cursor.col, cursor.row))
            }

            /// Get current Kitty Keyboard Protocol flags
            ///
            /// Returns:
            ///     Current keyboard protocol flags (u16)
            ///     Flags: 1=disambiguate, 2=report events, 4=alternate keys, 8=report all, 16=associated text
            fn keyboard_flags(&self) -> pyo3::PyResult<u16> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.keyboard_flags())
            }

            /// Get insert mode (IRM - Mode 4) state
            ///
            /// Returns:
            ///     True if insert mode is enabled (characters are inserted), False if replace mode (default)
            fn insert_mode(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.insert_mode())
            }

            /// Get line feed/new line mode (LNM - Mode 20) state
            ///
            /// Returns:
            ///     True if LNM is enabled (LF does CR+LF), False if LF only (default)
            fn line_feed_new_line_mode(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.line_feed_new_line_mode())
            }

            /// Get default foreground color (OSC 10)
            ///
            /// Returns RGB tuple (r, g, b) where each component is 0-255.
            ///
            /// Returns:
            ///     Tuple of (r, g, b) integers
            fn default_fg(&self) -> pyo3::PyResult<(u8, u8, u8)> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.default_fg().to_rgb())
            }

            /// Get default background color (OSC 11)
            ///
            /// Returns RGB tuple (r, g, b) where each component is 0-255.
            ///
            /// Returns:
            ///     Tuple of (r, g, b) integers
            fn default_bg(&self) -> pyo3::PyResult<(u8, u8, u8)> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.default_bg().to_rgb())
            }

            /// Get faint/dim text alpha multiplier
            ///
            /// This value is applied to SGR 2 (dim/faint) text during rendering.
            /// A value of 0.5 means 50% opacity (the default).
            ///
            /// Returns:
            ///     Alpha multiplier between 0.0 and 1.0
            fn faint_text_alpha(&self) -> pyo3::PyResult<f32> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.faint_text_alpha())
            }

            /// Get scrollback content as a list of strings
            ///
            /// Returns:
            ///     List of scrollback lines
            fn scrollback(&self) -> pyo3::PyResult<Vec<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.scrollback())
            }

            /// Check if alternate screen is active
            ///
            /// Returns:
            ///     True if alternate screen is active
            fn is_alt_screen_active(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.is_alt_screen_active())
            }

            /// Get mouse tracking mode
            ///
            /// Returns:
            ///     String representing the mouse mode: "off", "normal", "button", "any"
            fn mouse_mode(&self) -> pyo3::PyResult<String> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let mode = match t.mouse_mode() {
                    $crate::mouse::MouseMode::Off => "off",
                    $crate::mouse::MouseMode::X10 => "x10",
                    $crate::mouse::MouseMode::Normal => "normal",
                    $crate::mouse::MouseMode::ButtonEvent => "button",
                    $crate::mouse::MouseMode::AnyEvent => "any",
                };
                Ok(mode.to_string())
            }

            /// Check if focus tracking is enabled
            ///
            /// Returns:
            ///     True if focus tracking is enabled
            fn focus_tracking(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.focus_tracking())
            }

            /// Check if synchronized updates mode is enabled (DEC 2026)
            ///
            /// Returns:
            ///     True if synchronized updates mode is enabled
            fn synchronized_updates(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.synchronized_updates())
            }

            /// Get the Unicode width configuration
            ///
            /// Returns:
            ///     WidthConfig: The current width configuration
            fn width_config(
                &self,
            ) -> pyo3::PyResult<$crate::python_bindings::enums::PyWidthConfig> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok((*t.width_config()).into())
            }

            /// Get the configured answerback string (ENQ response)
            ///
            /// Returns:
            ///     The current answerback string or None if disabled (default)
            fn answerback_string(&self) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.answerback_string().map(std::string::ToString::to_string))
            }

            /// Get the current working directory reported via OSC 7,
            /// or None if no directory has been reported yet.
            ///
            /// Returns:
            ///     Optional string with current directory path
            fn current_directory(&self) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.current_directory().map(|s| s.to_string()))
            }

            /// Get the display width of a single character
            ///
            /// Args:
            ///     c: A single character to measure
            ///
            /// Returns:
            ///     int: The display width in cells (0, 1, or 2)
            fn char_width(&self, c: &str) -> pyo3::PyResult<usize> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                if let Some(ch) = c.chars().next() {
                    Ok(t.char_width(ch))
                } else {
                    Ok(0)
                }
            }

            /// Count non-whitespace lines in visible screen
            ///
            /// Returns:
            ///     Number of lines containing non-whitespace characters
            fn count_non_whitespace_lines(&self) -> pyo3::PyResult<usize> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.count_non_whitespace_lines())
            }
        }
    };
}

/// Emit the custom rendering-hint color setters for `$ty`, using
/// [`TerminalAccess::term_mut`]. (ARC-003/QA-001 scaling batch 2.)
#[macro_export]
macro_rules! impl_terminal_color_setters {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Set link/hyperlink color
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_link_color(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_link_color($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Set bold text color (when use_bold_color is enabled)
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_bold_color(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_bold_color($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Set cursor guide color (vertical line following cursor)
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_cursor_guide_color(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_cursor_guide_color($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Set badge color
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_badge_color(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_badge_color($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Set match/search highlight color
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_match_color(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_match_color($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Set selection background color
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_selection_bg_color(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_selection_bg_color($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Set selection foreground/text color
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_selection_fg_color(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_selection_fg_color($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Enable/disable custom bold color
            ///
            /// When enabled, bold text uses set_bold_color() instead of bright ANSI variant.
            ///
            /// Args:
            ///     use_bold: Whether to use custom bold color
            fn set_use_bold_color(&mut self, use_bold: bool) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_use_bold_color(use_bold);
                Ok(())
            }

            /// Enable/disable custom underline color
            ///
            /// When enabled, underlined text uses a custom underline color.
            ///
            /// Args:
            ///     use_underline: Whether to use custom underline color
            fn set_use_underline_color(&mut self, use_underline: bool) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_use_underline_color(use_underline);
                Ok(())
            }

            /// Set bold brightening mode
            ///
            /// When enabled, bold text with ANSI colors 0-7 is brightened to 8-15.
            /// This is a legacy terminal behavior that some applications rely on.
            ///
            /// Args:
            ///     enabled: True to enable bold brightening, False to disable
            fn set_bold_brightening(&mut self, enabled: bool) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_bold_brightening(enabled);
                Ok(())
            }
        }
    };
}

/// Emit core state/config setters for `$ty`, using [`TerminalAccess::term_mut`].
/// (ARC-003/QA-001 scaling batch 3.)
#[macro_export]
macro_rules! impl_terminal_state_setters {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Set clipboard content programmatically
            ///
            /// This bypasses OSC 52 sequences and directly sets the clipboard.
            /// Useful for integration with system clipboard or testing.
            ///
            /// Args:
            ///     content: Content to set (None to clear)
            fn set_clipboard(&mut self, content: Option<String>) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_clipboard(content);
                Ok(())
            }

            /// Set whether clipboard read operations are allowed
            ///
            /// When disabled (default), OSC 52 queries are silently ignored for security.
            /// When enabled, terminal applications can query clipboard contents.
            ///
            /// Args:
            ///     allow: True to allow clipboard read, False to block (default)
            fn set_allow_clipboard_read(&mut self, allow: bool) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_allow_clipboard_read(allow);
                Ok(())
            }

            /// Set default foreground color (OSC 10)
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_default_fg(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_default_fg($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Set default background color (OSC 11)
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_default_bg(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_default_bg($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Set cursor color (OSC 12)
            ///
            /// Args:
            ///     r: Red component (0-255)
            ///     g: Green component (0-255)
            ///     b: Blue component (0-255)
            fn set_cursor_color(&mut self, r: u8, g: u8, b: u8) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_cursor_color($crate::color::Color::Rgb(r, g, b));
                Ok(())
            }

            /// Set faint/dim text alpha multiplier
            ///
            /// This value is applied to SGR 2 (dim/faint) text during rendering.
            /// Values are clamped to the range 0.0-1.0.
            ///
            /// Args:
            ///     alpha: Alpha multiplier (0.0 = fully transparent, 1.0 = fully opaque)
            ///
            /// Example:
            ///     >>> term.set_faint_text_alpha(0.3)  # 30% opacity for dim text
            fn set_faint_text_alpha(&mut self, alpha: f32) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_faint_text_alpha(alpha);
                Ok(())
            }

            /// Set cursor style (DECSCUSR)
            ///
            /// This is equivalent to sending CSI <n> SP q escape sequence.
            ///
            /// Args:
            ///     style: CursorStyle enum value (e.g., CursorStyle.BlinkingBlock)
            fn set_cursor_style(
                &mut self,
                style: $crate::python_bindings::enums::PyCursorStyle,
            ) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                // Send DECSCUSR escape sequence (CSI <n> SP q)
                let sequence = format!(
                    "\x1b[{} q",
                    match style {
                        $crate::python_bindings::enums::PyCursorStyle::BlinkingBlock => 1,
                        $crate::python_bindings::enums::PyCursorStyle::SteadyBlock => 2,
                        $crate::python_bindings::enums::PyCursorStyle::BlinkingUnderline => 3,
                        $crate::python_bindings::enums::PyCursorStyle::SteadyUnderline => 4,
                        $crate::python_bindings::enums::PyCursorStyle::BlinkingBar => 5,
                        $crate::python_bindings::enums::PyCursorStyle::SteadyBar => 6,
                    }
                );
                t.process(sequence.as_bytes());
                Ok(())
            }

            /// Manually flush the synchronized update buffer
            ///
            /// This is useful for flushing buffered updates without disabling synchronized mode.
            /// Note: The buffer is automatically flushed when synchronized mode is disabled via CSI ? 2026 l
            fn flush_synchronized_updates(&mut self) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.flush_synchronized_updates();
                Ok(())
            }

            /// Set the answerback string sent in response to ENQ (0x05)
            ///
            /// The answerback payload is sent whenever the terminal receives the ENQ
            /// control character. Default is None (disabled) for security. Use with
            /// caution in untrusted sessions.
            ///
            /// Args:
            ///     answerback: Custom string to return, or None to disable
            fn set_answerback_string(&mut self, answerback: Option<String>) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_answerback_string(answerback);
                Ok(())
            }

            /// Set the Unicode width configuration
            ///
            /// This controls how character widths are calculated, particularly for:
            /// - East Asian Ambiguous characters (Greek, Cyrillic, symbols)
            /// - Unicode version-specific width tables
            ///
            /// Args:
            ///     config: WidthConfig with unicode_version and ambiguous_width settings
            fn set_width_config(
                &mut self,
                config: $crate::python_bindings::enums::PyWidthConfig,
            ) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_width_config(config.into());
                Ok(())
            }

            /// Set the treatment of East Asian Ambiguous width characters
            ///
            /// This is a convenience method to just change the ambiguous width setting
            /// without modifying the Unicode version.
            ///
            /// Args:
            ///     width: AmbiguousWidth.Narrow (1 cell) or AmbiguousWidth.Wide (2 cells)
            fn set_ambiguous_width(
                &mut self,
                width: $crate::python_bindings::enums::PyAmbiguousWidth,
            ) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_ambiguous_width(width.into());
                Ok(())
            }

            /// Set the Unicode version for width calculation tables
            ///
            /// This is a convenience method to just change the Unicode version setting
            /// without modifying the ambiguous width treatment.
            ///
            /// Args:
            ///     version: UnicodeVersion enum value (e.g., UnicodeVersion.Auto)
            fn set_unicode_version(
                &mut self,
                version: $crate::python_bindings::enums::PyUnicodeVersion,
            ) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_unicode_version(version.into());
                Ok(())
            }

            /// Set whether OSC 7 directory tracking sequences are accepted
            ///
            /// When disabled, OSC 7 sequences are silently ignored.
            /// When enabled (default), allows shell to report current working directory.
            ///
            /// Args:
            ///     accept: True to accept OSC 7 (default), False to ignore
            fn set_accept_osc7(&mut self, accept: bool) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_accept_osc7(accept);
                Ok(())
            }
        }
    };
}

/// Emit pure-function static utility helpers for `$ty`. These do not access
/// the terminal at all (they wrap `crate::ansi_utils`); they are duplicated
/// only for API symmetry. (ARC-003/QA-001 batch: static helpers.)
#[macro_export]
macro_rules! impl_terminal_static_helpers {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Strip ANSI escape sequences from text
            ///
            /// Args:
            ///     text: Text containing ANSI codes
            ///
            /// Returns:
            ///     Text with all ANSI sequences removed
            #[staticmethod]
            fn strip_ansi(text: &str) -> pyo3::PyResult<String> {
                Ok($crate::ansi_utils::strip_ansi(text))
            }

            /// Measure text width without ANSI codes
            ///
            /// Accounts for wide characters (CJK, emoji) and strips ANSI sequences.
            ///
            /// Args:
            ///     text: Text to measure
            ///
            /// Returns:
            ///     Display width in columns
            #[staticmethod]
            fn measure_text_width(text: &str) -> pyo3::PyResult<usize> {
                Ok($crate::ansi_utils::measure_text_width(text))
            }

            /// Parse color from string (hex, rgb, or name)
            ///
            /// Supported formats:
            /// - Hex: "#RRGGBB" or "#RGB"
            /// - RGB: "rgb(r, g, b)"
            /// - Names: "red", "blue", "green", etc.
            ///
            /// Args:
            ///     color_string: Color specification
            ///
            /// Returns:
            ///     RGB tuple (r, g, b) or None if invalid
            #[staticmethod]
            fn parse_color(color_string: &str) -> pyo3::PyResult<Option<(u8, u8, u8)>> {
                if let Some(color) = $crate::ansi_utils::parse_color(color_string) {
                    Ok(Some(color.to_rgb()))
                } else {
                    Ok(None)
                }
            }
        }
    };
}

/// Emit the Sixel resource-limit / graphics-query methods for `$ty`.
/// (ARC-003/QA-001 batch: sixel + graphics.)
#[macro_export]
macro_rules! impl_terminal_sixel_graphics {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Get graphics that overlap the specified row
            ///
            /// Args:
            ///     row: Row index (0-based)
            ///
            /// Returns:
            ///     List of graphics that overlap the given row
            fn graphics_at_row(
                &self,
                row: usize,
            ) -> pyo3::PyResult<Vec<$crate::python_bindings::types::PyGraphic>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let graphics = t.graphics_at_row(row);
                Ok(graphics
                    .iter()
                    .map(|g| $crate::python_bindings::types::PyGraphic::from(*g))
                    .collect())
            }

            /// Get total number of graphics
            ///
            /// Returns:
            ///     Total count of Sixel graphics
            fn graphics_count(&self) -> pyo3::PyResult<usize> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.graphics_count())
            }

            /// Get all graphics
            ///
            /// Returns:
            ///     List of all Sixel graphics
            fn graphics(&self) -> pyo3::PyResult<Vec<$crate::python_bindings::types::PyGraphic>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let graphics = t.all_graphics();
                Ok(graphics
                    .iter()
                    .map($crate::python_bindings::types::PyGraphic::from)
                    .collect())
            }

            /// Clear all graphics
            fn clear_graphics(&mut self) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.clear_graphics();
                Ok(())
            }

            /// Get Sixel resource limits (max width, height, repeat)
            ///
            /// Returns:
            ///     Tuple of (max_width_px, max_height_px, max_repeat)
            fn get_sixel_limits(&self) -> pyo3::PyResult<(usize, usize, usize)> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let limits = t.sixel_limits();
                Ok((limits.max_width, limits.max_height, limits.max_repeat))
            }

            /// Set Sixel resource limits (max width, height, repeat)
            ///
            /// Args:
            ///     max_width: Maximum Sixel bitmap width in pixels
            ///     max_height: Maximum Sixel bitmap height in pixels
            ///     max_repeat: Maximum repeat count for !Pn sequences
            ///
            /// Limits are clamped to safe hard maxima at the Rust layer.
            fn set_sixel_limits(
                &mut self,
                max_width: usize,
                max_height: usize,
                max_repeat: usize,
            ) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_sixel_limits(max_width, max_height, max_repeat);
                Ok(())
            }

            /// Get maximum number of Sixel graphics retained
            ///
            /// Returns:
            ///     Maximum number of in-memory Sixel graphics for this terminal
            fn get_sixel_graphics_limit(&self) -> pyo3::PyResult<usize> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.max_sixel_graphics())
            }

            /// Set maximum number of Sixel graphics retained
            ///
            /// Args:
            ///     max_graphics: Maximum number of in-memory Sixel graphics
            ///
            /// Oldest graphics are dropped if the new limit is lower than the
            /// current number of graphics. The value is clamped to a safe range.
            fn set_sixel_graphics_limit(&mut self, max_graphics: usize) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_max_sixel_graphics(max_graphics);
                Ok(())
            }

            /// Get count of Sixel graphics dropped due to limits
            ///
            /// Returns:
            ///     Number of Sixel graphics that have been dropped because of size or count limits
            fn get_dropped_sixel_graphics(&self) -> pyo3::PyResult<usize> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.dropped_sixel_graphics())
            }

            /// Get Sixel statistics as a dictionary
            ///
            /// Returns:
            ///     {
            ///       "max_width_px": int,
            ///       "max_height_px": int,
            ///       "max_repeat": int,
            ///       "max_graphics": int,
            ///       "current_graphics": int,
            ///       "dropped_graphics": int,
            ///     }
            fn get_sixel_stats(&self) -> pyo3::PyResult<std::collections::HashMap<String, usize>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let (limits, max_graphics, current_graphics, dropped_graphics) = t.sixel_stats();
                let mut stats = std::collections::HashMap::new();
                stats.insert("max_width_px".to_string(), limits.max_width);
                stats.insert("max_height_px".to_string(), limits.max_height);
                stats.insert("max_repeat".to_string(), limits.max_repeat);
                stats.insert("max_graphics".to_string(), max_graphics);
                stats.insert("current_graphics".to_string(), current_graphics);
                stats.insert("dropped_graphics".to_string(), dropped_graphics);
                Ok(stats)
            }
        }
    };
}

/// Emit badge / session-variable methods for `$ty`. (ARC-003/QA-001 batch:
/// badge API.) `set_badge_color` already lives in `impl_terminal_color_setters!`.
#[macro_export]
macro_rules! impl_terminal_badge_session {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Get the current badge format template
            ///
            /// Returns the badge format string if one has been set via OSC 1337 SetBadgeFormat.
            /// The format may contain `\(variable)` placeholders for session variables.
            ///
            /// Returns:
            ///     Optional string containing the badge format template, or None if not set
            fn badge_format(&self) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.badge_format().map(|s| s.to_string()))
            }

            /// Set the badge format template
            ///
            /// This method is typically called when processing OSC 1337 SetBadgeFormat sequences.
            /// The format string should contain `\(variable)` placeholders.
            ///
            /// Args:
            ///     format: The badge format template string, or None to clear
            fn set_badge_format(&mut self, format: Option<String>) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_badge_format(format);
                Ok(())
            }

            /// Clear the badge format
            ///
            /// Removes any previously set badge format template.
            fn clear_badge_format(&mut self) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.clear_badge_format();
                Ok(())
            }

            /// Evaluate the current badge format with session variables
            ///
            /// Returns the evaluated badge string with all variables substituted,
            /// or None if no badge format is set.
            ///
            /// Returns:
            ///     Evaluated badge string with variables replaced, or None
            fn evaluate_badge(&self) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.evaluate_badge())
            }

            /// Get a session variable value by name
            ///
            /// Session variables are used for badge format evaluation.
            /// Supports both `session.variable` and just `variable` syntax.
            ///
            /// Args:
            ///     name: Variable name (e.g., "username", "hostname", "session.path")
            ///
            /// Returns:
            ///     Variable value as string, or None if not set
            fn get_badge_session_variable(&self, name: &str) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.session_variables().get(name))
            }

            /// Set a session variable for badge format evaluation
            ///
            /// Sets a custom session variable that can be referenced in badge formats.
            ///
            /// Args:
            ///     name: Variable name
            ///     value: Variable value
            fn set_badge_session_variable(
                &mut self,
                name: &str,
                value: &str,
            ) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.session_variables_mut().set_custom(name, value);
                Ok(())
            }

            /// Get all session variables as a dictionary
            ///
            /// Returns all session variables that can be used in badge evaluation,
            /// including built-in variables like columns, rows, bell_count, etc.
            ///
            /// Returns:
            ///     Dictionary mapping variable names to their string values
            fn get_badge_session_variables(
                &self,
            ) -> pyo3::PyResult<std::collections::HashMap<String, String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let vars = t.session_variables();
                let mut result = std::collections::HashMap::new();

                // Add built-in variables
                if let Some(hostname) = &vars.hostname {
                    result.insert("hostname".to_string(), hostname.clone());
                }
                if let Some(username) = &vars.username {
                    result.insert("username".to_string(), username.clone());
                }
                if let Some(path) = &vars.path {
                    result.insert("path".to_string(), path.clone());
                }
                if let Some(job) = &vars.job {
                    result.insert("job".to_string(), job.clone());
                }
                if let Some(last_command) = &vars.last_command {
                    result.insert("last_command".to_string(), last_command.clone());
                }
                if let Some(profile_name) = &vars.profile_name {
                    result.insert("profile_name".to_string(), profile_name.clone());
                }
                if let Some(tty) = &vars.tty {
                    result.insert("tty".to_string(), tty.clone());
                }
                if let Some(selection) = &vars.selection {
                    result.insert("selection".to_string(), selection.clone());
                }
                if let Some(tmux_pane_title) = &vars.tmux_pane_title {
                    result.insert("tmux_pane_title".to_string(), tmux_pane_title.clone());
                }
                if let Some(session_name) = &vars.session_name {
                    result.insert("session_name".to_string(), session_name.clone());
                }
                if let Some(title) = &vars.title {
                    result.insert("title".to_string(), title.clone());
                }

                // Always include dimension and bell count
                result.insert("columns".to_string(), vars.columns.to_string());
                result.insert("rows".to_string(), vars.rows.to_string());
                result.insert("bell_count".to_string(), vars.bell_count.to_string());

                // Add custom variables
                for (k, v) in &vars.custom {
                    result.insert(k.clone(), v.clone());
                }

                Ok(result)
            }
        }
    };
}

/// Emit progress-bar (OSC 9;4) and notification (OSC 9 / OSC 777) methods
/// plus device-query response drainers for `$ty`. (ARC-003/QA-001 batch.)
#[macro_export]
macro_rules! impl_terminal_progress_notifications {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Drain and return pending device query responses
            ///
            /// Device queries like DA (Device Attributes) and DSR (Device Status Report)
            /// generate responses that are buffered. This method retrieves and clears them.
            ///
            /// Returns:
            ///     Bytes containing all pending responses
            fn drain_responses(&mut self) -> pyo3::PyResult<Vec<u8>> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                Ok(t.drain_responses())
            }

            /// Check if there are pending device query responses
            ///
            /// Returns:
            ///     True if there are responses waiting to be retrieved
            fn has_pending_responses(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.has_pending_responses())
            }

            /// Check if there are pending notifications
            ///
            /// Returns:
            ///     True if there are notifications waiting to be retrieved
            fn has_notifications(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.has_notifications())
            }

            /// Get all pending notifications
            ///
            /// Returns a list of tuples: [(title, message), ...]
            /// For OSC 9 notifications, title will be empty string.
            /// Clears the notification queue after retrieval.
            ///
            /// Returns:
            ///     List of (title, message) tuples
            fn take_notifications(&mut self) -> pyo3::PyResult<Vec<(String, String)>> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                let notifications = t.take_notifications();
                Ok(notifications
                    .into_iter()
                    .map(|n| (n.title, n.message))
                    .collect())
            }

            /// Get all pending notifications (alias for take_notifications)
            ///
            /// Returns a list of tuples: [(title, message), ...]
            /// Clears the notification queue after retrieval.
            ///
            /// Returns:
            ///     List of (title, message) tuples
            fn drain_notifications(&mut self) -> pyo3::PyResult<Vec<(String, String)>> {
                self.take_notifications()
            }

            /// Get the current progress bar state
            ///
            /// Returns the progress bar state set via OSC 9;4 sequences.
            /// The progress bar has a state (hidden, normal, indeterminate, warning, error)
            /// and a percentage (0-100) for states that support it.
            ///
            /// Returns:
            ///     ProgressBar object with state and progress fields
            fn progress_bar(
                &self,
            ) -> pyo3::PyResult<$crate::python_bindings::types::PyProgressBar> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.progress_bar().into())
            }

            /// Check if the progress bar is currently active (visible)
            ///
            /// Returns:
            ///     True if the progress bar is in any state other than Hidden
            fn has_progress(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.has_progress())
            }

            /// Get the current progress percentage (0-100)
            ///
            /// Returns the progress percentage. Only meaningful when the progress bar
            /// state is Normal, Warning, or Error.
            ///
            /// Returns:
            ///     Progress percentage (0-100)
            fn progress_value(&self) -> pyo3::PyResult<u8> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.progress_value())
            }

            /// Get the current progress bar state enum
            ///
            /// Returns:
            ///     ProgressState enum value (Hidden, Normal, Indeterminate, Warning, Error)
            fn progress_state(
                &self,
            ) -> pyo3::PyResult<$crate::python_bindings::enums::PyProgressState> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.progress_state().into())
            }

            /// Manually set the progress bar state
            ///
            /// This can be used to programmatically control the progress bar
            /// without receiving OSC 9;4 sequences.
            ///
            /// Args:
            ///     state: ProgressState enum value
            ///     progress: Progress percentage (0-100, clamped if out of range)
            fn set_progress(
                &mut self,
                state: $crate::python_bindings::enums::PyProgressState,
                progress: u8,
            ) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_progress(state.into(), progress);
                Ok(())
            }

            /// Clear/hide the progress bar
            ///
            /// Equivalent to receiving OSC 9;4;0 (hidden state).
            fn clear_progress(&mut self) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.clear_progress();
                Ok(())
            }
        }
    };
}

/// Emit session-recording methods for `$ty`. (ARC-003/QA-001 batch: recording.)
/// NOTE: the pty.rs copies were declared `&self` despite mutating recording
/// state (an artifact of the lock-from-shared-ref pattern); the canonical
/// `&mut self` form from `mod.rs`/`recording_api.rs` is used here, which is
/// the correct receiver since these mutate terminal state. Python-visible
/// behavior is unchanged (PyO3 does not enforce `&mut self` at the Python
/// level — the GIL already provides exclusive access).
#[macro_export]
macro_rules! impl_terminal_recording {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Start recording a terminal session
            ///
            /// Args:
            ///     title: Optional session title
            fn start_recording(&mut self, title: Option<String>) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.start_recording(title);
                Ok(())
            }

            /// Stop recording and return the session
            ///
            /// Returns:
            ///     RecordingSession object if recording was active, None otherwise
            fn stop_recording(
                &mut self,
            ) -> pyo3::PyResult<Option<$crate::python_bindings::types::PyRecordingSession>> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                Ok(t.stop_recording()
                    .map($crate::python_bindings::types::PyRecordingSession::from))
            }

            /// Record output data
            ///
            /// Args:
            ///     data: Output data bytes
            fn record_output(&mut self, data: &[u8]) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.record_output(data);
                Ok(())
            }

            /// Record input data
            ///
            /// Args:
            ///     data: Input data bytes
            fn record_input(&mut self, data: &[u8]) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.record_input(data);
                Ok(())
            }

            /// Record terminal resize
            ///
            /// Args:
            ///     cols: Number of columns
            ///     rows: Number of rows
            fn record_resize(&mut self, cols: usize, rows: usize) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.record_resize(cols, rows);
                Ok(())
            }

            /// Add a marker/bookmark to the recording
            ///
            /// Args:
            ///     label: Marker label
            fn record_marker(&mut self, label: String) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.record_marker(label);
                Ok(())
            }

            /// Get current recording session
            ///
            /// Returns:
            ///     RecordingSession object if recording is active, None otherwise
            fn get_recording_session(
                &self,
            ) -> pyo3::PyResult<Option<$crate::python_bindings::types::PyRecordingSession>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.get_recording_session()
                    .map($crate::python_bindings::types::PyRecordingSession::from))
            }

            /// Check if currently recording
            ///
            /// Returns:
            ///     True if recording is active
            fn is_recording(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.is_recording())
            }
        }
    };
}

/// Emit cell / color / line query methods for `$ty`. (ARC-003/QA-001 batch:
/// cell & line queries.) All read-only, using [`TerminalAccess::term_ref`].
#[macro_export]
macro_rules! impl_terminal_cell_line_queries {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Get a cell's foreground color at the specified position
            ///
            /// Args:
            ///     col: Column index (0-based)
            ///     row: Row index (0-based)
            ///
            /// Returns:
            ///     Tuple of (r, g, b) values, or None if out of bounds
            fn get_fg_color(&self, col: usize, row: usize) -> pyo3::PyResult<Option<(u8, u8, u8)>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                if let Some(cell) = t.active_grid().get(col, row) {
                    Ok(Some(cell.fg.to_rgb()))
                } else {
                    Ok(None)
                }
            }

            /// Get a cell's background color at the specified position
            ///
            /// Args:
            ///     col: Column index (0-based)
            ///     row: Row index (0-based)
            ///
            /// Returns:
            ///     Tuple of (r, g, b) values, or None if out of bounds
            fn get_bg_color(&self, col: usize, row: usize) -> pyo3::PyResult<Option<(u8, u8, u8)>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                if let Some(cell) = t.active_grid().get(col, row) {
                    Ok(Some(cell.bg.to_rgb()))
                } else {
                    Ok(None)
                }
            }

            /// Get a cell's underline color at the specified position (SGR 58)
            ///
            /// Args:
            ///     col: Column index (0-based)
            ///     row: Row index (0-based)
            ///
            /// Returns:
            ///     Tuple of (r, g, b) values, or None if no underline color set or out of bounds
            fn get_underline_color(
                &self,
                col: usize,
                row: usize,
            ) -> pyo3::PyResult<Option<(u8, u8, u8)>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                if let Some(cell) = t.active_grid().get(col, row) {
                    Ok(cell.underline_color.map(|c| c.to_rgb()))
                } else {
                    Ok(None)
                }
            }

            /// Get cell attributes at the specified position
            ///
            /// Args:
            ///     col: Column index (0-based)
            ///     row: Row index (0-based)
            ///
            /// Returns:
            ///     Dictionary with boolean flags: bold, italic, underline, etc., or None if out of bounds
            fn get_attributes(
                &self,
                col: usize,
                row: usize,
            ) -> pyo3::PyResult<Option<$crate::python_bindings::types::PyAttributes>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                if let Some(cell) = t.active_grid().get(col, row) {
                    Ok(Some($crate::python_bindings::types::PyAttributes::from(
                        cell,
                    )))
                } else {
                    Ok(None)
                }
            }

            /// Get hyperlink URL at the specified position
            ///
            /// Args:
            ///     col: Column index (0-based)
            ///     row: Row index (0-based)
            ///
            /// Returns:
            ///     URL string if the cell has a hyperlink, or None if no hyperlink or out of bounds
            fn get_hyperlink(&self, col: usize, row: usize) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                if let Some(cell) = t.active_grid().get(col, row) {
                    if let Some(id) = cell.flags.hyperlink_id {
                        return Ok(t.get_hyperlink_url(id));
                    }
                }
                Ok(None)
            }

            /// Check if a line wraps to the next row
            ///
            /// Args:
            ///     row: Row index (0-based)
            ///
            /// Returns:
            ///     True if the line wraps to the next row, False otherwise
            fn is_line_wrapped(&self, row: usize) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.active_grid().is_line_wrapped(row))
            }

            /// Get all cell data for a row in a single atomic operation
            ///
            /// This method retrieves all cell information for an entire row atomically,
            /// preventing race conditions in multi-threaded scenarios.
            ///
            /// Args:
            ///     row: Row index (0-based)
            ///
            /// Returns:
            ///     List of tuples (char, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes) for each column,
            ///     or empty list if row is out of bounds
            fn get_line_cells(
                &self,
                row: usize,
            ) -> pyo3::PyResult<$crate::python_bindings::types::LineCellData> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let grid = t.active_grid();
                let rows = grid.rows();

                if row >= rows {
                    return Ok(Vec::new());
                }

                let cols = grid.cols();
                let result = (0..cols)
                    .filter_map(|col| {
                        grid.get(col, row).map(|cell| {
                            (
                                cell.get_grapheme(),
                                cell.fg.to_rgb(),
                                cell.bg.to_rgb(),
                                $crate::python_bindings::types::PyAttributes::from(cell),
                            )
                        })
                    })
                    .collect();

                Ok(result)
            }

            /// Get word at cursor position
            ///
            /// Args:
            ///     col: Column position (0-indexed)
            ///     row: Row position (0-indexed)
            ///     word_chars: Optional custom word characters (default: "/-+\\~_." iTerm2-compatible)
            ///
            /// Returns:
            ///     Word at position or None if not on a word
            fn get_word_at(
                &self,
                col: usize,
                row: usize,
                word_chars: Option<&str>,
            ) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.get_word_at(col, row, word_chars))
            }

            /// Get URL at cursor position
            ///
            /// Detects URLs with schemes: http://, https://, ftp://, file://, mailto:, ssh://
            ///
            /// Args:
            ///     col: Column position (0-indexed)
            ///     row: Row position (0-indexed)
            ///
            /// Returns:
            ///     URL at position or None if not on a URL
            fn get_url_at(&self, col: usize, row: usize) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.get_url_at(col, row))
            }

            /// Get full logical line following wrapping
            ///
            /// Args:
            ///     row: Row position (0-indexed)
            ///
            /// Returns:
            ///     Complete unwrapped line or None if row is invalid
            fn get_line_unwrapped(&self, row: usize) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.get_line_unwrapped(row))
            }
        }
    };
}

/// Emit content, clipboard, cursor-style, shell-integration, insecure-sequence,
/// and focus/paste event-sequence methods for `$ty`. (ARC-003/QA-001 batch.)
#[macro_export]
macro_rules! impl_terminal_content_misc {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Get the terminal content as a string
            ///
            /// Returns:
            ///     String representation of the terminal buffer
            fn content(&self) -> pyo3::PyResult<String> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.content())
            }

            fn __str__(&self) -> pyo3::PyResult<String> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.content())
            }

            /// Get the current clipboard content
            ///
            /// Returns:
            ///     Clipboard content as string, or None if empty
            fn clipboard(&self) -> pyo3::PyResult<Option<String>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.clipboard().map(|s| s.to_string()))
            }

            /// Get the current cursor style
            ///
            /// Returns:
            ///     CursorStyle enum value
            fn cursor_style(
                &self,
            ) -> pyo3::PyResult<$crate::python_bindings::enums::PyCursorStyle> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.cursor().style().into())
            }

            /// Get focus in event sequence
            ///
            /// Returns:
            ///     Bytes for focus in event (if focus tracking is enabled)
            fn get_focus_in_event(&self) -> pyo3::PyResult<Vec<u8>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.report_focus_in())
            }

            /// Get focus out event sequence
            ///
            /// Returns:
            ///     Bytes for focus out event (if focus tracking is enabled)
            fn get_focus_out_event(&self) -> pyo3::PyResult<Vec<u8>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.report_focus_out())
            }

            /// Get bracketed paste start sequence
            ///
            /// Returns:
            ///     Bytes for paste start (if bracketed paste is enabled)
            fn get_paste_start(&self) -> pyo3::PyResult<Vec<u8>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.bracketed_paste_start().to_vec())
            }

            /// Get bracketed paste end sequence
            ///
            /// Returns:
            ///     Bytes for paste end (if bracketed paste is enabled)
            fn get_paste_end(&self) -> pyo3::PyResult<Vec<u8>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.bracketed_paste_end().to_vec())
            }

            /// Get shell integration state
            ///
            /// Returns:
            ///     Dictionary with shell integration info
            fn shell_integration_state(
                &self,
            ) -> pyo3::PyResult<$crate::python_bindings::types::PyShellIntegration> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let si = t.shell_integration();
                Ok($crate::python_bindings::types::PyShellIntegration {
                    in_prompt: si.in_prompt(),
                    in_command_input: si.in_command_input(),
                    in_command_output: si.in_command_output(),
                    current_command: si.command().map(|s| s.to_string()),
                    last_exit_code: si.exit_code(),
                    cwd: si.cwd().map(|s| s.to_string()),
                    hostname: si.hostname().map(|s| s.to_string()),
                    username: si.username().map(|s| s.to_string()),
                })
            }

            /// Check if insecure sequences are disabled
            ///
            /// Returns:
            ///     True if insecure sequences are blocked, False otherwise
            fn disable_insecure_sequences(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.disable_insecure_sequences())
            }

            /// Set whether to filter potentially insecure escape sequences
            ///
            /// When enabled, certain sequences that could pose security risks are blocked:
            /// - OSC 52 (clipboard operations - can leak data)
            /// - OSC 8 (hyperlinks - can be used for phishing)
            /// - OSC 9/777 (notifications - can be annoying/misleading)
            /// - Sixel graphics (can consume excessive memory)
            ///
            /// When disabled (default), all standard sequences are processed normally.
            ///
            /// Args:
            ///     disable: True to block insecure sequences, False to allow (default)
            fn set_disable_insecure_sequences(&mut self, disable: bool) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_disable_insecure_sequences(disable);
                Ok(())
            }
        }
    };
}

/// Emit search / selection / scrollback-line-query methods for `$ty`.
/// (ARC-003/QA-001 batch: search & selection.)
#[macro_export]
macro_rules! impl_terminal_search_select {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Find all occurrences of text in the visible screen
            ///
            /// Args:
            ///     pattern: Text to search for
            ///     case_sensitive: Whether search is case-sensitive (default: True)
            ///
            /// Returns:
            ///     List of (col, row) positions where pattern was found
            #[pyo3(signature = (pattern, case_sensitive = true))]
            fn find_text(
                &self,
                pattern: &str,
                case_sensitive: bool,
            ) -> pyo3::PyResult<Vec<(usize, usize)>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.find_text(pattern, case_sensitive)
                    .into_iter()
                    .map(|m| (m.col, m.row as usize))
                    .collect())
            }

            /// Find next occurrence of text from given position
            ///
            /// Args:
            ///     pattern: Text to search for
            ///     from_col: Starting column position
            ///     from_row: Starting row position
            ///     case_sensitive: Whether search is case-sensitive (default: True)
            ///
            /// Returns:
            ///     (col, row) of next match, or None if not found
            #[pyo3(signature = (pattern, from_col, from_row, case_sensitive = true))]
            fn find_next(
                &self,
                pattern: &str,
                from_col: usize,
                from_row: usize,
                case_sensitive: bool,
            ) -> pyo3::PyResult<Option<(usize, usize)>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.find_next(pattern, from_col, from_row, case_sensitive)
                    .map(|m| (m.col, m.row as usize)))
            }

            /// Find matching bracket/parenthesis at cursor position
            ///
            /// Supports: (), [], {}, <>
            ///
            /// Args:
            ///     col: Column position (0-indexed)
            ///     row: Row position (0-indexed)
            ///
            /// Returns:
            ///     (col, row) position of matching bracket, or None
            fn find_matching_bracket(
                &self,
                col: usize,
                row: usize,
            ) -> pyo3::PyResult<Option<(usize, usize)>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.find_matching_bracket(col, row))
            }

            /// Get word boundaries at cursor position for smart selection
            ///
            /// Args:
            ///     col: Column position (0-indexed)
            ///     row: Row position (0-indexed)
            ///     word_chars: Optional custom word characters
            ///
            /// Returns:
            ///     ((start_col, start_row), (end_col, end_row)) or None if not on a word
            #[allow(clippy::type_complexity)]
            fn select_word(
                &mut self,
                col: usize,
                row: usize,
                word_chars: Option<&str>,
            ) -> pyo3::PyResult<Option<((usize, usize), (usize, usize))>> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                Ok(t.select_word(col, row, word_chars))
            }

            /// Select text within semantic delimiters
            ///
            /// Extracts content between matching delimiters around cursor.
            /// Supports: (), [], {}, <>, "", '', ``
            ///
            /// Args:
            ///     col: Column position (0-indexed)
            ///     row: Row position (0-indexed)
            ///     delimiters: String of delimiters to check (e.g., "()[]{}\"'")
            ///
            /// Returns:
            ///     Content between delimiters, or None if not inside delimiters
            fn select_semantic_region(
                &mut self,
                col: usize,
                row: usize,
                delimiters: &str,
            ) -> pyo3::PyResult<Option<String>> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                Ok(t.select_semantic_region(col, row, Some(delimiters)))
            }

            /// Get a specific line from the scrollback buffer with full cell data
            ///
            /// Args:
            ///     index: Scrollback line index (0 = oldest, scrollback_len()-1 = most recent)
            ///
            /// Returns:
            ///     List of tuples (char, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes),
            ///     or None if index is out of bounds
            #[allow(clippy::type_complexity)]
            fn scrollback_line(
                &self,
                index: usize,
            ) -> pyo3::PyResult<
                Option<
                    Vec<(
                        String,
                        (u8, u8, u8),
                        (u8, u8, u8),
                        $crate::python_bindings::types::PyAttributes,
                    )>,
                >,
            > {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let grid = t.grid();
                if let Some(line) = grid.scrollback_line(index) {
                    let cells: Vec<_> = line
                        .iter()
                        .map(|cell| {
                            (
                                cell.get_grapheme(),
                                cell.fg.to_rgb(),
                                cell.bg.to_rgb(),
                                $crate::python_bindings::types::PyAttributes::from(cell),
                            )
                        })
                        .collect();
                    Ok(Some(cells))
                } else {
                    Ok(None)
                }
            }
        }
    };
}

/// Emit debug-snapshot methods for `$ty`. (ARC-003/QA-001 batch: debug.)
#[macro_export]
macro_rules! impl_terminal_debug_snapshots {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Get a debug snapshot of the current buffer state
            ///
            /// Returns:
            ///     String containing a formatted view of the buffer
            fn debug_snapshot_buffer(&self) -> pyo3::PyResult<String> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let grid = t.active_grid();
                Ok(grid.debug_snapshot())
            }

            /// Get a debug snapshot of the grid
            ///
            /// Returns:
            ///     String containing a formatted view of the grid
            fn debug_snapshot_grid(&self) -> pyo3::PyResult<String> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.grid().debug_snapshot())
            }

            /// Get a debug snapshot of the primary screen buffer
            ///
            /// Returns:
            ///     String containing a formatted view of the primary buffer
            fn debug_snapshot_primary(&self) -> pyo3::PyResult<String> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.grid().debug_snapshot())
            }

            /// Get a debug snapshot of the alternate screen buffer
            ///
            /// Returns:
            ///     String containing a formatted view of the alternate buffer
            fn debug_snapshot_alt(&self) -> pyo3::PyResult<String> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.alt_grid().debug_snapshot())
            }

            /// Log a debug snapshot with a label
            ///
            /// Args:
            ///     label: Description of this snapshot
            fn debug_log_snapshot(&self, label: &str) -> pyo3::PyResult<()> {
                use $crate::debug;
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let grid = t.active_grid();
                let snapshot = grid.debug_snapshot();
                debug::log_buffer_snapshot(label, grid.rows(), grid.cols(), &snapshot);
                Ok(())
            }
        }
    };
}

/// Emit file-transfer API methods for `$ty`. (ARC-003/QA-001 batch: file transfer.)
/// Uses `transfer_to_py_dict` defined in `python_bindings::terminal::mod` (pub(super),
/// visible to both the `terminal::*` submodules and the sibling `pty` module).
#[macro_export]
macro_rules! impl_terminal_file_transfer {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Get all active (in-progress) file transfers
            ///
            /// Returns a list of dictionaries, each describing an active transfer.
            ///
            /// Returns:
            ///     List of transfer dictionaries
            #[pyo3(text_signature = "($self)")]
            fn get_active_transfers(&self) -> pyo3::PyResult<Vec<pyo3::Py<pyo3::types::PyDict>>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let transfers = t.get_active_transfers();
                pyo3::Python::attach(|py| {
                    let mut result = Vec::with_capacity(transfers.len());
                    for transfer in &transfers {
                        result.push($crate::python_bindings::terminal::transfer_to_py_dict(
                            py, transfer, false,
                        )?);
                    }
                    Ok(result)
                })
            }

            /// Get all completed file transfers (includes failed and cancelled)
            ///
            /// Returns:
            ///     List of transfer dictionaries
            #[pyo3(text_signature = "($self)")]
            fn get_completed_transfers(
                &self,
            ) -> pyo3::PyResult<Vec<pyo3::Py<pyo3::types::PyDict>>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                let transfers = t.get_completed_transfers();
                pyo3::Python::attach(|py| {
                    let mut result = Vec::with_capacity(transfers.len());
                    for transfer in &transfers {
                        result.push($crate::python_bindings::terminal::transfer_to_py_dict(
                            py, transfer, false,
                        )?);
                    }
                    Ok(result)
                })
            }

            /// Get a specific active transfer by ID
            ///
            /// Args:
            ///     transfer_id: The unique transfer identifier
            ///
            /// Returns:
            ///     Transfer dictionary if found, None otherwise
            #[pyo3(text_signature = "($self, transfer_id)")]
            fn get_transfer(
                &self,
                transfer_id: u64,
            ) -> pyo3::PyResult<Option<pyo3::Py<pyo3::types::PyDict>>> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                match t.get_transfer(transfer_id) {
                    Some(transfer) => pyo3::Python::attach(|py| {
                        Ok(Some(
                            $crate::python_bindings::terminal::transfer_to_py_dict(
                                py, &transfer, false,
                            )?,
                        ))
                    }),
                    None => Ok(None),
                }
            }

            /// Take a completed transfer by ID, removing it from the completed buffer
            ///
            /// Args:
            ///     transfer_id: The unique transfer identifier
            ///
            /// Returns:
            ///     Transfer dictionary with "data" key (bytes) if found, None otherwise
            #[pyo3(text_signature = "($self, transfer_id)")]
            fn take_completed_transfer(
                &mut self,
                transfer_id: u64,
            ) -> pyo3::PyResult<Option<pyo3::Py<pyo3::types::PyDict>>> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                match t.take_completed_transfer(transfer_id) {
                    Some(transfer) => pyo3::Python::attach(|py| {
                        Ok(Some(
                            $crate::python_bindings::terminal::transfer_to_py_dict(
                                py, &transfer, true,
                            )?,
                        ))
                    }),
                    None => Ok(None),
                }
            }

            /// Cancel an active file transfer
            ///
            /// Args:
            ///     transfer_id: The unique transfer identifier
            ///
            /// Returns:
            ///     True if the transfer was found and cancelled, False otherwise
            #[pyo3(text_signature = "($self, transfer_id)")]
            fn cancel_file_transfer(&mut self, transfer_id: u64) -> pyo3::PyResult<bool> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                Ok(t.cancel_file_transfer(transfer_id))
            }

            /// Send upload data in response to an UploadRequested event
            ///
            /// Args:
            ///     data: Raw file data bytes to upload
            #[pyo3(text_signature = "($self, data)")]
            fn send_upload_data(&mut self, data: &[u8]) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.send_upload_data(data);
                Ok(())
            }

            /// Cancel an upload request
            #[pyo3(text_signature = "($self)")]
            fn cancel_upload(&mut self) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.cancel_upload();
                Ok(())
            }

            /// Set the maximum allowed file transfer size in bytes
            ///
            /// Args:
            ///     max_bytes: Maximum transfer size in bytes
            #[pyo3(text_signature = "($self, max_bytes)")]
            fn set_max_transfer_size(&mut self, max_bytes: usize) -> pyo3::PyResult<()> {
                let mut t = $crate::python_bindings::common::TerminalAccess::term_mut(self);
                t.set_max_transfer_size(max_bytes);
                Ok(())
            }

            /// Get the current maximum allowed file transfer size in bytes
            ///
            /// Returns:
            ///     Maximum transfer size in bytes (default: 50 MB)
            #[pyo3(text_signature = "($self)")]
            fn get_max_transfer_size(&self) -> pyo3::PyResult<usize> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.get_max_transfer_size())
            }
        }
    };
}
