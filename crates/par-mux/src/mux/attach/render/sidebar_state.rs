//! Session-level side-panel visibility.

/// Whether the side panel is shown (toggled by the `sidebar` chord); its
/// width is `render_opts.sidebar_width`, its sections live in the renderer.
#[derive(Default)]
pub(super) struct SidebarState {
    on: bool,
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
}
