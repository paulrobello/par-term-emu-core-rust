//! Session-level side-panel visibility and its persisted state.

/// Whether the side panel is shown (toggled by the `sidebar` chord) and
/// collapsed to the status rail (the chevron); its expanded width is
/// `render_opts.sidebar_width`, its sections and split live in the
/// renderer. `prefs` is where the dragged/toggled state persists — set
/// only by a real attach, so a unit-test session never writes.
#[derive(Default)]
pub(super) struct SidebarState {
    on: bool,
    collapsed: bool,
    prefs: Option<std::path::PathBuf>,
}

impl SidebarState {
    pub(super) fn is_on(&self) -> bool {
        self.on
    }

    /// Flip visibility; returns the new value.
    pub(super) fn toggle(&mut self) -> bool {
        self.on = !self.on;
        self.on
    }

    pub(super) fn set(&mut self, on: bool) {
        self.on = on;
    }

    pub(super) fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    pub(super) fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    pub(super) fn prefs_path(&self) -> Option<&std::path::Path> {
        self.prefs.as_deref()
    }

    pub(super) fn set_prefs_path(&mut self, path: std::path::PathBuf) {
        self.prefs = Some(path);
    }
}
