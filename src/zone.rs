//! Semantic buffer zones for tracking logical blocks in terminal output
//!
//! Zones segment the scrollback buffer into Prompt, Command, and Output
//! blocks using FinalTerm/OSC 133 shell integration markers.

/// Type of semantic zone in the terminal buffer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneType {
    /// Shell prompt text (between OSC 133;A and OSC 133;B)
    Prompt,
    /// Command input text (between OSC 133;B and OSC 133;C)
    Command,
    /// Command output text (between OSC 133;C and OSC 133;D)
    Output,
}

impl std::fmt::Display for ZoneType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZoneType::Prompt => write!(f, "prompt"),
            ZoneType::Command => write!(f, "command"),
            ZoneType::Output => write!(f, "output"),
        }
    }
}

/// A semantic zone in the terminal buffer
///
/// Zones track logical blocks of terminal content using absolute row numbers.
/// They are created by OSC 133 shell integration markers and stored in a
/// Vec on the Grid, sorted by `abs_row_start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// Type of this zone
    pub zone_type: ZoneType,
    /// Absolute row where this zone starts (scrollback_len + cursor.row at creation)
    pub abs_row_start: usize,
    /// Absolute row where this zone ends (inclusive). Updated as zone grows.
    /// Equal to abs_row_start when zone is first created; updated when zone is closed.
    pub abs_row_end: usize,
    /// Command text (from OSC 133;B parameter), set on Command and Output zones
    pub command: Option<String>,
    /// Exit code (from OSC 133;D parameter), set on Output zones when command finishes
    pub exit_code: Option<i32>,
    /// Timestamp in Unix milliseconds when this zone was created
    pub timestamp: Option<u64>,
}

impl Zone {
    /// Create a new zone starting at the given absolute row
    pub fn new(zone_type: ZoneType, abs_row: usize, timestamp: Option<u64>) -> Self {
        Self {
            zone_type,
            abs_row_start: abs_row,
            abs_row_end: abs_row,
            command: None,
            exit_code: None,
            timestamp,
        }
    }

    /// Close this zone at the given absolute row
    pub fn close(&mut self, abs_row: usize) {
        self.abs_row_end = abs_row.max(self.abs_row_start);
    }

    /// Check if a given absolute row falls within this zone
    pub fn contains_row(&self, abs_row: usize) -> bool {
        abs_row >= self.abs_row_start && abs_row <= self.abs_row_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zone_new() {
        let zone = Zone::new(ZoneType::Prompt, 10, Some(1000));
        assert_eq!(zone.zone_type, ZoneType::Prompt);
        assert_eq!(zone.abs_row_start, 10);
        assert_eq!(zone.abs_row_end, 10);
        assert!(zone.command.is_none());
        assert!(zone.exit_code.is_none());
        assert_eq!(zone.timestamp, Some(1000));
    }

    #[test]
    fn test_zone_close() {
        let mut zone = Zone::new(ZoneType::Output, 5, None);
        zone.close(15);
        assert_eq!(zone.abs_row_end, 15);
    }

    #[test]
    fn test_zone_close_same_row() {
        let mut zone = Zone::new(ZoneType::Prompt, 5, None);
        zone.close(5);
        assert_eq!(zone.abs_row_end, 5);
    }

    #[test]
    fn test_zone_close_clamps_to_start() {
        let mut zone = Zone::new(ZoneType::Command, 10, None);
        zone.close(3);
        assert_eq!(zone.abs_row_end, 10);
    }

    #[test]
    fn test_zone_contains_row() {
        let mut zone = Zone::new(ZoneType::Output, 5, None);
        zone.close(15);
        assert!(!zone.contains_row(4));
        assert!(zone.contains_row(5));
        assert!(zone.contains_row(10));
        assert!(zone.contains_row(15));
        assert!(!zone.contains_row(16));
    }

    #[test]
    fn test_zone_type_display() {
        assert_eq!(ZoneType::Prompt.to_string(), "prompt");
        assert_eq!(ZoneType::Command.to_string(), "command");
        assert_eq!(ZoneType::Output.to_string(), "output");
    }
}
