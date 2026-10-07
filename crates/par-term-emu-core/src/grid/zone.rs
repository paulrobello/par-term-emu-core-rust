//! Zone management for the terminal grid

use crate::grid::Grid;
use crate::zone::Zone;

/// Maximum number of evicted zones a grid retains until the owning terminal
/// drains them into `ZoneScrolledOut` events. Past this, the oldest evicted
/// zones are dropped (FIFO), so a host that never drains cannot grow the
/// buffer without bound. Matches `MAX_TERMINAL_EVENTS`, the cap on the event
/// queue these zones feed.
/// cap: Evicted zones buffered per grid before they become events.
pub(crate) const MAX_EVICTED_ZONES: usize = 10_000;

impl Grid {
    /// Get all semantic zones
    pub fn zones(&self) -> &[Zone] {
        &self.zones
    }

    /// Get all semantic zones mutably
    pub fn zones_mut(&mut self) -> &mut Vec<Zone> {
        &mut self.zones
    }

    /// Push a new zone
    pub fn push_zone(&mut self, zone: Zone) {
        self.zones.push(zone);
    }

    /// Close the current zone at the given row
    pub fn close_current_zone(&mut self, abs_row: usize) {
        if let Some(zone) = self.zones.last_mut() {
            zone.close(abs_row);
        }
    }

    /// Get the zone containing the given absolute row
    pub fn zone_at(&self, abs_row: usize) -> Option<&Zone> {
        self.zones.iter().find(|z| z.contains_row(abs_row))
    }

    /// Evict zones whose entire range is before the given floor
    pub fn evict_zones(&mut self, floor: usize) {
        let (evicted, mut remaining): (Vec<_>, Vec<_>) =
            self.zones.drain(..).partition(|z| z.abs_row_end < floor);
        self.evicted_zones.extend(evicted);
        let excess = self.evicted_zones.len().saturating_sub(MAX_EVICTED_ZONES);
        if excess > 0 {
            self.evicted_zones.drain(..excess);
        }

        // Clamp start of remaining zones
        for zone in &mut remaining {
            if zone.abs_row_start < floor {
                zone.abs_row_start = floor;
            }
        }

        self.zones = remaining;
    }

    /// Clear all zones
    pub fn clear_zones(&mut self) {
        self.zones.clear();
    }

    /// Drain evicted zones
    pub fn drain_evicted_zones(&mut self) -> Vec<Zone> {
        std::mem::take(&mut self.evicted_zones)
    }
}
