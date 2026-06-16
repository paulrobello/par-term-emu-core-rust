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
