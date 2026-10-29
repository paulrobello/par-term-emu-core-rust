//! Host/embedder configuration accessors (ARC-003): conformance level, bell
//! volumes, pixel size and window geometry, Kitty file-media policy, and the
//! clipboard/OSC 7/insecure-sequence/OSC-length security knobs.

use super::Terminal;

impl Terminal {
    /// Get the current conformance level
    pub fn conformance_level(&self) -> crate::conformance_level::ConformanceLevel {
        self.conformance_level
    }

    /// Get the warning bell volume (0=off, 1-8=volume levels)
    pub fn warning_bell_volume(&self) -> u8 {
        self.bell_volumes.warning
    }

    /// Get the margin bell volume (0=off, 1-8=volume levels)
    pub fn margin_bell_volume(&self) -> u8 {
        self.bell_volumes.margin
    }

    /// Set the host-configured conformance level.
    ///
    /// Sets both the live level and the baseline RIS (`ESC c`) restores. A
    /// program's DECSCL changes only the live level, until the next RIS
    /// (ARC-100).
    pub fn set_conformance_level(&mut self, level: crate::conformance_level::ConformanceLevel) {
        self.host.configured_conformance_level = level;
        self.conformance_level = level;
    }

    /// Set the host-configured warning bell volume (0=off, 1-8; larger
    /// values clamp to 8).
    ///
    /// Sets both the live volume and the baseline RIS (`ESC c`) restores. A
    /// program's DECSWBV changes only the live volume, until the next RIS
    /// (ARC-100).
    pub fn set_warning_bell_volume(&mut self, volume: u8) {
        let volume = volume.min(8);
        self.host.configured_warning_bell_volume = volume;
        self.bell_volumes.warning = volume;
    }

    /// Set the host-configured margin bell volume (0=off, 1-8; larger
    /// values clamp to 8).
    ///
    /// Sets both the live volume and the baseline RIS (`ESC c`) restores. A
    /// program's DECSMBV changes only the live volume, until the next RIS
    /// (ARC-100).
    pub fn set_margin_bell_volume(&mut self, volume: u8) {
        let volume = volume.min(8);
        self.host.configured_margin_bell_volume = volume;
        self.bell_volumes.margin = volume;
    }

    /// Set pixel dimensions for XTWINOPS reporting
    ///
    /// Also refreshes the graphics cell dimensions (the per-cell pixel size
    /// image placement converts image extents to cell spans with): a host
    /// that reports real text-area pixels knows its real cell size, and the
    /// (1, 2) no-information default would make a daemon-side terminal with
    /// client pixels advance the cursor a different number of rows after an
    /// image than the client-side mirror rendering it does.
    pub fn set_pixel_size(&mut self, width_px: usize, height_px: usize) {
        self.pixel_width = width_px;
        self.pixel_height = height_px;
        let (cols, rows) = self.size();
        if cols > 0 && rows > 0 && width_px > 0 && height_px > 0 {
            self.graphics.cell_dimensions = ((width_px / cols) as u32, (height_px / rows) as u32);
        }
    }

    /// Keep kitty `t=t` temp files on disk after reading instead of
    /// deleting them (crate-internal: mux daemon wiring).
    ///
    /// A multiplexer's daemon-side terminal processes PTY bytes before the
    /// client mirrors that rebuild their grid from the same raw bytes
    /// forwarded over the socket — the daemon's read of a `t=t` file must
    /// not delete it, or no client can ever load the graphic. The client
    /// that renders it deletes the file with its own (default) read.
    #[cfg(feature = "mux")]
    #[doc(hidden)]
    pub fn set_retain_kitty_temp_files(&mut self, retain: bool) {
        self.kitty_parser.retain_temp_files = retain;
    }

    /// Control whether Kitty graphics may load image payloads from
    /// filesystem paths — the `t=f` (file) and `t=t` (temp-file) media
    /// (SEC-101).
    ///
    /// Terminal output is untrusted input: a `t=t` APC used to delete (or
    /// `t=f` to display) any file the process could reach. The default
    /// (`TempOnly`) allows only the spec's gated form — a
    /// `*tty-graphics-protocol*` file inside an allowed temp root, deleted
    /// only after it decodes as an image. `All` re-enables unrestricted
    /// `t=f` reads (with `..` still rejected); `Off` refuses both media.
    pub fn set_allow_file_media(&mut self, mode: crate::graphics::kitty::FileMediaMode) {
        self.kitty_parser.allow_file_media = mode;
    }

    /// Get the current Kitty file-media mode — see
    /// [`Terminal::set_allow_file_media`].
    pub fn allow_file_media(&self) -> crate::graphics::kitty::FileMediaMode {
        self.kitty_parser.allow_file_media
    }

    /// Set the host-supplied window position for XTWINOPS reporting
    /// (`CSI 13 t` / `CSI 13 ; 2 t`).
    ///
    /// The terminal core is headless and has no window of its own; GUI
    /// hosts (e.g. par-term) should call this whenever the real OS window
    /// moves so that `CSI 13 t` queries reflect the actual on-screen
    /// position instead of defaulting to the origin. Coordinates may be
    /// negative (e.g. a window positioned on a monitor left of/above the
    /// primary display in a multi-monitor setup).
    pub fn set_window_position(&mut self, x: i32, y: i32) {
        self.host.window_position = (x, y);
    }

    /// Get the host-supplied window position in pixels as `(x, y)`,
    /// defaulting to `(0, 0)` if never set via [`Terminal::set_window_position`].
    pub fn window_position(&self) -> (i32, i32) {
        self.host.window_position
    }

    /// Set the host-supplied iconified/minimized state for XTWINOPS
    /// reporting (`CSI 11 t`).
    ///
    /// GUI hosts should call this whenever the real OS window is
    /// minimized/restored so that `CSI 11 t` queries report the correct
    /// state instead of always reporting non-iconified.
    pub fn set_window_iconified(&mut self, iconified: bool) {
        self.host.window_iconified = iconified;
    }

    /// Get the host-supplied iconified/minimized state, defaulting to
    /// `false` if never set via [`Terminal::set_window_iconified`].
    pub fn window_iconified(&self) -> bool {
        self.host.window_iconified
    }

    /// Get clipboard content (OSC 52)
    pub fn clipboard(&self) -> Option<&str> {
        self.clipboard_state.clipboard_content.as_deref()
    }

    /// Check if clipboard read operations are allowed (security flag for OSC 52 queries)
    pub fn allow_clipboard_read(&self) -> bool {
        self.clipboard_state.allow_clipboard_read
    }

    /// Set whether clipboard read operations are allowed (security flag for OSC 52 queries)
    ///
    /// When disabled (default), OSC 52 queries (ESC ] 52 ; c ; ? ST) are silently ignored.
    /// When enabled, terminals can query clipboard contents, which has security implications.
    pub fn set_allow_clipboard_read(&mut self, allow: bool) {
        self.clipboard_state.allow_clipboard_read = allow;
    }

    /// Check if clipboard write operations are allowed (security flag for OSC 52 writes)
    pub fn allow_clipboard_write(&self) -> bool {
        self.clipboard_state.allow_clipboard_write
    }

    /// Set whether clipboard write operations are allowed (security flag for OSC 52 writes)
    ///
    /// When enabled (default), OSC 52 writes (`ESC ] 52 ; c ; <base64> ST`) store the
    /// decoded payload as the terminal clipboard content. When disabled, they are
    /// silently ignored, so untrusted output cannot place text on the clipboard.
    pub fn set_allow_clipboard_write(&mut self, allow: bool) {
        self.clipboard_state.allow_clipboard_write = allow;
    }

    /// Get default foreground color (OSC 10)
    /// Get current working directory from shell integration (OSC 7)
    ///
    /// Returns the directory path reported by the shell via OSC 7 sequences,
    /// or None if no directory has been reported yet.
    pub fn current_directory(&self) -> Option<&str> {
        self.shell_state.shell_integration.cwd()
    }

    /// Check if OSC 7 directory tracking is enabled
    pub fn accept_osc7(&self) -> bool {
        self.security_state.accept_osc7
    }

    /// Set whether OSC 7 directory tracking sequences are accepted
    ///
    /// When disabled, OSC 7 sequences are silently ignored.
    /// When enabled (default), allows shell to report current working directory.
    pub fn set_accept_osc7(&mut self, accept: bool) {
        self.security_state.accept_osc7 = accept;
    }

    /// Check if insecure sequence filtering is enabled
    pub fn disable_insecure_sequences(&self) -> bool {
        self.security_state.disable_insecure_sequences
    }

    /// Set whether to filter potentially insecure escape sequences
    ///
    /// When enabled, certain sequences that could pose security risks are blocked.
    /// When disabled (default), all standard sequences are processed normally.
    pub fn set_disable_insecure_sequences(&mut self, disable: bool) {
        self.security_state.disable_insecure_sequences = disable;
    }

    /// Maximum total OSC data length in bytes before a sequence is rejected
    /// (QA-012/SEC-003). Defaults to [`DEFAULT_MAX_OSC_DATA_LENGTH`](super::DEFAULT_MAX_OSC_DATA_LENGTH) (1 MiB);
    /// raise it for deployments pushing larger inline images, lower it for
    /// tighter memory/security bounds.
    pub fn max_osc_data_length(&self) -> usize {
        self.security_state.max_osc_data_length
    }

    /// Set the maximum total OSC data length in bytes (QA-012).
    ///
    /// Sequences exceeding this are rejected as a memory-exhaustion guard.
    /// Must be large enough for inline images (iTerm2/Kitty base64) if used.
    pub fn set_max_osc_data_length(&mut self, max: usize) {
        self.security_state.max_osc_data_length = max;
    }

    /// Get the answerback string sent in response to ENQ (0x05)
    pub fn answerback_string(&self) -> Option<&str> {
        self.title_state.answerback_string.as_deref()
    }

    /// Set the answerback string sent in response to ENQ (0x05) control character
    ///
    /// The answerback string is sent back to the PTY when the terminal receives
    /// an ENQ (enquiry, ASCII 0x05) character. This was historically used for
    /// terminal identification in multi-terminal environments.
    ///
    /// # Security Note
    /// Default is None (disabled) for security. Setting this may expose
    /// terminal identification information to applications.
    ///
    /// # Arguments
    /// * `answerback` - The string to send, or None to disable
    pub fn set_answerback_string(&mut self, answerback: Option<String>) {
        self.title_state.answerback_string = answerback;
    }
}
