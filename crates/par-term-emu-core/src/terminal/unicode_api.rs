//! Unicode width and normalization configuration (ARC-003).

use super::Terminal;

impl Terminal {
    /// Get the current Unicode width configuration
    ///
    /// Returns the configuration used for character width calculations,
    /// including Unicode version and ambiguous width handling.
    pub fn width_config(&self) -> &crate::unicode_width_config::WidthConfig {
        &self.unicode_state.width_config
    }

    /// Set the Unicode width configuration
    ///
    /// This affects how character widths are calculated for terminal display,
    /// particularly for:
    /// - East Asian Ambiguous width characters (narrow vs wide)
    /// - Emoji and other Unicode characters
    ///
    /// # Arguments
    /// * `config` - The new width configuration to use
    pub fn set_width_config(&mut self, config: crate::unicode_width_config::WidthConfig) {
        self.unicode_state.width_config = config;
    }

    /// Set the ambiguous width setting
    ///
    /// Convenience method to change only the ambiguous width treatment
    /// without modifying other width configuration settings.
    ///
    /// # Arguments
    /// * `width` - The ambiguous width setting (Narrow or Wide)
    pub fn set_ambiguous_width(&mut self, width: crate::unicode_width_config::AmbiguousWidth) {
        self.unicode_state.width_config.ambiguous_width = width;
    }

    /// Set the Unicode version for width calculations
    ///
    /// Convenience method to change only the Unicode version
    /// without modifying other width configuration settings.
    ///
    /// # Arguments
    /// * `version` - The Unicode version to use for width tables
    pub fn set_unicode_version(&mut self, version: crate::unicode_width_config::UnicodeVersion) {
        self.unicode_state.width_config.unicode_version = version;
    }

    /// Calculate the display width of a character using current config
    ///
    /// This uses the terminal's width configuration to determine
    /// how many cells a character occupies.
    ///
    /// # Arguments
    /// * `c` - The character to measure
    ///
    /// # Returns
    /// The display width in cells (0, 1, or 2)
    #[inline]
    pub fn char_width(&self, c: char) -> usize {
        crate::unicode_width_config::char_width(c, &self.unicode_state.width_config)
    }

    /// Get the current Unicode normalization form
    ///
    /// Returns the normalization form used for text stored in terminal cells.
    pub fn normalization_form(&self) -> crate::unicode_normalization_config::NormalizationForm {
        self.unicode_state.normalization_form
    }

    /// Set the Unicode normalization form
    ///
    /// Controls how Unicode text is normalized before being stored in cells.
    /// Default is NFC (Canonical Decomposition, followed by Canonical Composition).
    ///
    /// # Arguments
    /// * `form` - The normalization form to use
    pub fn set_normalization_form(
        &mut self,
        form: crate::unicode_normalization_config::NormalizationForm,
    ) {
        self.unicode_state.normalization_form = form;
    }
}
