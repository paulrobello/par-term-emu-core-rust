//! The window's chrome rows: the status row, the tab strip, the status-bar
//! toggle, and the transient flash cue drawn in the status row's head.

use super::FLASH_LIFETIME;
use crate::mux::attach::status::StatusRow;
use crate::mux::attach::tabs::TabStrip;
use std::time::Instant;

pub(super) struct ChromeRows {
    /// The bottom row: paint/diff pair.
    pub(super) status_row: StatusRow,
    /// The top row: the shown session's windows, clickable.
    pub(super) tab_strip: TabStrip,
    /// The status bar's visibility (the `status_bar` chord toggles it;
    /// shown by default).
    pub(super) status_bar_on: bool,
    /// A transient confirmation/error cue drawn in place of the status
    /// line's head and cleared after about a second.
    flash: Option<String>,
    /// When the current flash clears: armed by the first tick that sees
    /// it, so every flash site stays a plain call. Wall-clock, not a tick
    /// count: a pump woken early by a burst of events would burn a tick
    /// budget in milliseconds (QA-235).
    pub(super) flash_until: Option<Instant>,
}

impl ChromeRows {
    pub(super) fn new(cols: u16) -> Self {
        Self {
            status_row: StatusRow::new(cols),
            tab_strip: TabStrip::new(cols),
            status_bar_on: true,
            flash: None,
            flash_until: None,
        }
    }

    /// Raise the flash cue; the caller repaints the status row.
    pub(super) fn flash(&mut self, text: impl Into<String>) {
        self.flash = Some(text.into());
    }

    pub(super) fn flash_text(&self) -> Option<&str> {
        self.flash.as_deref()
    }

    /// Age the flash: true when it cleared (the caller repaints the
    /// status row).
    pub(super) fn tick(&mut self, now: Instant) -> bool {
        if self.flash.is_none() {
            self.flash_until = None;
            return false;
        }
        let until = *self.flash_until.get_or_insert(now + FLASH_LIFETIME);
        if now >= until {
            self.flash = None;
            self.flash_until = None;
            return true;
        }
        false
    }
}
