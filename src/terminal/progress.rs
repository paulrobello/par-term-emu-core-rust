//! Progress bar support for OSC 9;4 sequences (ConEmu/Windows Terminal style)
//!
//! The OSC 9;4 protocol allows terminal applications to report progress to the terminal
//! emulator, which can display it in the tab bar, window title, or other UI elements.
//!
//! ## Protocol Format
//!
//! `OSC 9 ; 4 ; state ; progress ST`
//!
//! Where:
//! - `state` is one of: 0 (hidden), 1 (normal), 2 (indeterminate), 3 (warning), 4 (error)
//! - `progress` is 0-100 (percentage, only required for states 1, 3, 4)
//!
//! ## Examples
//!
//! ```text
//! \x1b]9;4;1;50\x1b\\   # Set progress to 50%
//! \x1b]9;4;0\x1b\\      # Hide progress bar
//! \x1b]9;4;2\x1b\\      # Show indeterminate progress
//! \x1b]9;4;3;75\x1b\\   # Show warning state at 75%
//! \x1b]9;4;4;100\x1b\\  # Show error state at 100%
//! ```

/// Progress bar state from OSC 9;4 sequences
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressState {
    /// Progress bar is hidden (state 0)
    #[default]
    Hidden,
    /// Normal progress display (state 1)
    Normal,
    /// Indeterminate/busy indicator (state 2)
    Indeterminate,
    /// Warning state - operation may have issues (state 3)
    Warning,
    /// Error state - operation failed (state 4)
    Error,
}

impl ProgressState {
    /// Parse state from OSC 9;4 state parameter
    pub fn from_param(param: u8) -> Self {
        match param {
            0 => Self::Hidden,
            1 => Self::Normal,
            2 => Self::Indeterminate,
            3 => Self::Warning,
            4 => Self::Error,
            _ => Self::Hidden, // Invalid state defaults to hidden
        }
    }

    /// Convert state to OSC 9;4 parameter value
    pub fn to_param(self) -> u8 {
        match self {
            Self::Hidden => 0,
            Self::Normal => 1,
            Self::Indeterminate => 2,
            Self::Warning => 3,
            Self::Error => 4,
        }
    }

    /// Check if the state represents an active progress bar
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Hidden)
    }

    /// Check if the state requires a progress percentage
    pub fn requires_progress(self) -> bool {
        matches!(self, Self::Normal | Self::Warning | Self::Error)
    }

    /// Get a human-readable description of the state
    pub fn description(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Normal => "normal",
            Self::Indeterminate => "indeterminate",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Progress bar data from OSC 9;4 sequences
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressBar {
    /// Current progress state
    pub state: ProgressState,
    /// Progress percentage (0-100), only meaningful for Normal/Warning/Error states
    pub progress: u8,
}

impl Default for ProgressBar {
    fn default() -> Self {
        Self {
            state: ProgressState::Hidden,
            progress: 0,
        }
    }
}

impl ProgressBar {
    /// Create a new progress bar with the given state and progress
    pub fn new(state: ProgressState, progress: u8) -> Self {
        Self {
            state,
            progress: progress.min(100), // Clamp to 100%
        }
    }

    /// Create a hidden (inactive) progress bar
    pub fn hidden() -> Self {
        Self::default()
    }

    /// Create a normal progress bar at the given percentage
    pub fn normal(progress: u8) -> Self {
        Self::new(ProgressState::Normal, progress)
    }

    /// Create an indeterminate progress bar
    pub fn indeterminate() -> Self {
        Self::new(ProgressState::Indeterminate, 0)
    }

    /// Create a warning progress bar at the given percentage
    pub fn warning(progress: u8) -> Self {
        Self::new(ProgressState::Warning, progress)
    }

    /// Create an error progress bar at the given percentage
    pub fn error(progress: u8) -> Self {
        Self::new(ProgressState::Error, progress)
    }

    /// Check if the progress bar is active (visible)
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// Generate the OSC 9;4 escape sequence for this progress bar
    pub fn to_escape_sequence(&self) -> String {
        if self.state.requires_progress() {
            format!("\x1b]9;4;{};{}\x1b\\", self.state.to_param(), self.progress)
        } else {
            format!("\x1b]9;4;{}\x1b\\", self.state.to_param())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_state_from_param() {
        assert_eq!(ProgressState::from_param(0), ProgressState::Hidden);
        assert_eq!(ProgressState::from_param(1), ProgressState::Normal);
        assert_eq!(ProgressState::from_param(2), ProgressState::Indeterminate);
        assert_eq!(ProgressState::from_param(3), ProgressState::Warning);
        assert_eq!(ProgressState::from_param(4), ProgressState::Error);
        // Invalid values default to Hidden
        assert_eq!(ProgressState::from_param(5), ProgressState::Hidden);
        assert_eq!(ProgressState::from_param(255), ProgressState::Hidden);
    }

    #[test]
    fn test_progress_state_to_param() {
        assert_eq!(ProgressState::Hidden.to_param(), 0);
        assert_eq!(ProgressState::Normal.to_param(), 1);
        assert_eq!(ProgressState::Indeterminate.to_param(), 2);
        assert_eq!(ProgressState::Warning.to_param(), 3);
        assert_eq!(ProgressState::Error.to_param(), 4);
    }

    #[test]
    fn test_progress_state_is_active() {
        assert!(!ProgressState::Hidden.is_active());
        assert!(ProgressState::Normal.is_active());
        assert!(ProgressState::Indeterminate.is_active());
        assert!(ProgressState::Warning.is_active());
        assert!(ProgressState::Error.is_active());
    }

    #[test]
    fn test_progress_state_requires_progress() {
        assert!(!ProgressState::Hidden.requires_progress());
        assert!(ProgressState::Normal.requires_progress());
        assert!(!ProgressState::Indeterminate.requires_progress());
        assert!(ProgressState::Warning.requires_progress());
        assert!(ProgressState::Error.requires_progress());
    }

    #[test]
    fn test_progress_bar_new() {
        let pb = ProgressBar::new(ProgressState::Normal, 50);
        assert_eq!(pb.state, ProgressState::Normal);
        assert_eq!(pb.progress, 50);
    }

    #[test]
    fn test_progress_bar_clamps_to_100() {
        let pb = ProgressBar::new(ProgressState::Normal, 150);
        assert_eq!(pb.progress, 100);
    }

    #[test]
    fn test_progress_bar_constructors() {
        let hidden = ProgressBar::hidden();
        assert_eq!(hidden.state, ProgressState::Hidden);
        assert!(!hidden.is_active());

        let normal = ProgressBar::normal(75);
        assert_eq!(normal.state, ProgressState::Normal);
        assert_eq!(normal.progress, 75);
        assert!(normal.is_active());

        let indeterminate = ProgressBar::indeterminate();
        assert_eq!(indeterminate.state, ProgressState::Indeterminate);
        assert!(indeterminate.is_active());

        let warning = ProgressBar::warning(90);
        assert_eq!(warning.state, ProgressState::Warning);
        assert_eq!(warning.progress, 90);

        let error = ProgressBar::error(100);
        assert_eq!(error.state, ProgressState::Error);
        assert_eq!(error.progress, 100);
    }

    #[test]
    fn test_progress_bar_default() {
        let pb = ProgressBar::default();
        assert_eq!(pb.state, ProgressState::Hidden);
        assert_eq!(pb.progress, 0);
        assert!(!pb.is_active());
    }

    #[test]
    fn test_progress_bar_escape_sequence() {
        assert_eq!(
            ProgressBar::hidden().to_escape_sequence(),
            "\x1b]9;4;0\x1b\\"
        );
        assert_eq!(
            ProgressBar::normal(50).to_escape_sequence(),
            "\x1b]9;4;1;50\x1b\\"
        );
        assert_eq!(
            ProgressBar::indeterminate().to_escape_sequence(),
            "\x1b]9;4;2\x1b\\"
        );
        assert_eq!(
            ProgressBar::warning(75).to_escape_sequence(),
            "\x1b]9;4;3;75\x1b\\"
        );
        assert_eq!(
            ProgressBar::error(100).to_escape_sequence(),
            "\x1b]9;4;4;100\x1b\\"
        );
    }

    #[test]
    fn test_progress_state_description() {
        assert_eq!(ProgressState::Hidden.description(), "hidden");
        assert_eq!(ProgressState::Normal.description(), "normal");
        assert_eq!(ProgressState::Indeterminate.description(), "indeterminate");
        assert_eq!(ProgressState::Warning.description(), "warning");
        assert_eq!(ProgressState::Error.description(), "error");
    }

    #[test]
    fn test_progress_bar_clone() {
        let pb1 = ProgressBar::normal(50);
        let pb2 = pb1;
        assert_eq!(pb1, pb2);
    }

    #[test]
    fn test_progress_bar_debug() {
        let pb = ProgressBar::normal(50);
        let debug_str = format!("{:?}", pb);
        assert!(debug_str.contains("Normal"));
        assert!(debug_str.contains("50"));
    }
}
