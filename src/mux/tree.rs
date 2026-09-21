//! The session/window/pane tree: the server's single source of truth.

use crate::mux::ids::{IdAllocator, PaneId, SessionId, WindowId};
use crate::mux::layout::{LayoutTree, ResizeDirection, SplitDirection};
use crate::mux::pane::{MuxError, MuxPane, PaneFactory};
use std::collections::HashMap;

/// One window: an interior split structure (Task 2.1) with one pane active.
///
/// `active` is a pane id, not an index — [`LayoutTree`] has no stable linear
/// order for a mutation (`split_pane`/`swap_pane`) to preserve, unlike the
/// flat `Vec<PaneId>` this replaced.
#[derive(Debug)]
pub struct MuxWindow {
    /// This window's identifier.
    pub id: WindowId,
    /// Display name.
    pub name: String,
    /// The window's interior pane structure.
    pub layout: LayoutTree,
    /// The currently active pane.
    pub active: PaneId,
    /// The window's width in columns — the extent the layout renders into
    /// and the base a resize delta converts against.
    pub cols: u16,
    /// The window's height in rows.
    pub rows: u16,
}

impl MuxWindow {
    /// Every pane id in this window's layout, in tree order.
    pub fn panes(&self) -> Vec<PaneId> {
        self.layout.pane_ids()
    }
}

/// One session: an ordered set of windows with one of them active.
#[derive(Debug)]
pub struct MuxSession {
    /// This session's identifier.
    pub id: SessionId,
    /// Display name.
    pub name: String,
    /// Windows in order.
    pub windows: Vec<WindowId>,
    /// Index into `windows` of the active window.
    pub active: usize,
}

/// The server's whole state: every session, window, and pane.
///
/// Flat maps keyed by id rather than a nested ownership tree, because panes are
/// addressed directly by the protocol (`%3`) far more often than they are
/// walked to from a session.
pub struct MuxTree {
    sessions: HashMap<SessionId, MuxSession>,
    windows: HashMap<WindowId, MuxWindow>,
    panes: HashMap<PaneId, MuxPane>,
    ids: IdAllocator,
    factory: Box<dyn PaneFactory>,
    /// Named paste buffers (`set-buffer`/`show-buffer`). A single value per
    /// name, not tmux's numbered stack — the Phase 2 non-goal in par-mux.md D3.
    buffers: HashMap<String, String>,
}

impl MuxTree {
    /// Create an empty tree that builds panes with `factory` (seam S1).
    pub fn new(factory: Box<dyn PaneFactory>) -> Self {
        Self {
            sessions: HashMap::new(),
            windows: HashMap::new(),
            panes: HashMap::new(),
            ids: IdAllocator::new(),
            factory,
            buffers: HashMap::new(),
        }
    }

    /// Store `content` under `name`, overwriting any existing value.
    pub fn set_buffer(&mut self, name: &str, content: String) {
        self.buffers.insert(name.to_string(), content);
    }

    /// Look up a named buffer's content.
    pub fn get_buffer(&self, name: &str) -> Option<&str> {
        self.buffers.get(name).map(String::as_str)
    }

    /// Every session id currently live.
    pub fn sessions(&self) -> Vec<SessionId> {
        self.sessions.keys().copied().collect()
    }

    /// Look up a session.
    pub fn session(&self, id: SessionId) -> Option<&MuxSession> {
        self.sessions.get(&id)
    }

    /// Look up a window.
    pub fn window(&self, id: WindowId) -> Option<&MuxWindow> {
        self.windows.get(&id)
    }

    /// Look up a pane.
    pub fn pane(&self, id: PaneId) -> Option<&MuxPane> {
        self.panes.get(&id)
    }

    /// Look up a pane mutably.
    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut MuxPane> {
        self.panes.get_mut(&id)
    }

    /// Create a session, with one window holding one pane — tmux's shape.
    pub fn new_session(&mut self, name: &str, cols: u16, rows: u16) -> Result<SessionId, MuxError> {
        let session_id = self.ids.next_session();
        let window_id = self.ids.next_window();
        let pane_id = self.ids.next_pane();

        let pane = self.factory.create_pane(pane_id, cols, rows, None)?;
        self.panes.insert(pane_id, pane);

        self.windows.insert(
            window_id,
            MuxWindow {
                id: window_id,
                name: name.to_string(),
                layout: LayoutTree::leaf(pane_id),
                active: pane_id,
                cols,
                rows,
            },
        );

        self.sessions.insert(
            session_id,
            MuxSession {
                id: session_id,
                name: name.to_string(),
                windows: vec![window_id],
                active: 0,
            },
        );

        Ok(session_id)
    }

    /// Add a window to a session.
    pub fn new_window(
        &mut self,
        session_id: SessionId,
        name: &str,
        cols: u16,
        rows: u16,
    ) -> Result<WindowId, MuxError> {
        if !self.sessions.contains_key(&session_id) {
            return Err(MuxError::NoSuchSession(session_id));
        }
        let window_id = self.ids.next_window();
        let pane_id = self.ids.next_pane();

        let pane = self.factory.create_pane(pane_id, cols, rows, None)?;
        self.panes.insert(pane_id, pane);
        self.windows.insert(
            window_id,
            MuxWindow {
                id: window_id,
                name: name.to_string(),
                layout: LayoutTree::leaf(pane_id),
                active: pane_id,
                cols,
                rows,
            },
        );
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.windows.push(window_id);
        }
        Ok(window_id)
    }

    /// Split `target`'s area in two and spawn a new pane in the other half.
    ///
    /// tmux's `split-window`: the new pane divides the target's extent along
    /// `direction`, receiving `new_share` of it (the `-p` percentage), and
    /// becomes the window's active pane. The new pane's terminal is created
    /// at the window's full size — per-pane geometry is a render-time
    /// concern (the layout string), not a terminal-size concern.
    pub fn split_pane(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
        command: Option<&str>,
    ) -> Result<PaneId, MuxError> {
        let window_id = self
            .window_of_pane(target)
            .ok_or(MuxError::NoSuchPane(target))?;
        let (cols, rows) = {
            let window = self.windows.get(&window_id).expect("just found");
            (window.cols, window.rows)
        };
        let pane_id = self.ids.next_pane();
        let pane = self.factory.create_pane(pane_id, cols, rows, command)?;
        self.panes.insert(pane_id, pane);
        let window = self.windows.get_mut(&window_id).expect("just found");
        // `LayoutTree::split_pane`'s ratio is the fraction kept by `first`
        // (the target), while the command speaks in the NEW pane's share.
        window
            .layout
            .split_pane(target, pane_id, direction, 1.0 - new_share)
            .expect("window_of_pane only returns windows holding the pane as a leaf");
        window.active = pane_id;
        Ok(pane_id)
    }

    /// The window whose layout holds `pane`, if any.
    pub fn window_of_pane(&self, pane: PaneId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, window)| window.layout.pane_ids().contains(&pane))
            .map(|(id, _)| *id)
    }

    /// Make `pane` its window's active pane.
    pub fn select_pane(&mut self, pane: PaneId) -> Result<(), MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        self.windows
            .get_mut(&window_id)
            .expect("window_of_pane only returns live windows")
            .active = pane;
        Ok(())
    }

    /// Swap two panes' positions within their window.
    ///
    /// tmux's `swap-pane` exchanges panes inside one window; panes in
    /// different windows have no shared split structure to trade places in.
    pub fn swap_panes(&mut self, target: PaneId, source: PaneId) -> Result<(), MuxError> {
        let window_id = self
            .window_of_pane(target)
            .ok_or(MuxError::NoSuchPane(target))?;
        let window = self
            .windows
            .get_mut(&window_id)
            .expect("window_of_pane only returns live windows");
        if !window.layout.pane_ids().contains(&source) {
            return Err(MuxError::PanesInDifferentWindows(target, source));
        }
        window
            .layout
            .swap_pane(target, source)
            .map_err(|_| MuxError::PanesInDifferentWindows(target, source))
    }

    /// Grow or shrink `pane` by `cells` toward `direction` (tmux's
    /// `-L`/`-R`/`-U`/`-D`), adjusting the ratio of the split it borders.
    ///
    /// Only a split of the matching orientation can absorb the adjustment:
    /// `-L`/`-R` move a side-by-side divider, `-U`/`-D` a stacked one. A
    /// pane with no such bordering split — a lone pane, or one whose only
    /// bordering split is the other orientation — is an error, not a no-op.
    pub fn resize_pane(
        &mut self,
        pane: PaneId,
        direction: ResizeDirection,
        cells: u32,
    ) -> Result<(), MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        let window = self
            .windows
            .get_mut(&window_id)
            .expect("window_of_pane only returns live windows");
        let Some((split_direction, ratio)) = window.layout.bordering_split(pane) else {
            return Err(MuxError::PaneNotResizable(pane));
        };
        let axis_matches = matches!(
            (direction, split_direction),
            (
                ResizeDirection::Left | ResizeDirection::Right,
                SplitDirection::Vertical
            ) | (
                ResizeDirection::Up | ResizeDirection::Down,
                SplitDirection::Horizontal
            )
        );
        if !axis_matches {
            return Err(MuxError::PaneNotResizable(pane));
        }
        let extent = match split_direction {
            SplitDirection::Vertical => window.cols as f32,
            SplitDirection::Horizontal => window.rows as f32,
        };
        let sign = match direction {
            ResizeDirection::Right | ResizeDirection::Down => 1.0,
            ResizeDirection::Left | ResizeDirection::Up => -1.0,
        };
        let new_ratio = ratio + sign * (cells as f32) / extent;
        window
            .layout
            .resize_pane(pane, new_ratio)
            .expect("bordering_split found the split resize_pane adjusts");
        Ok(())
    }

    /// Kill a pane, closing its window when it was the last one.
    ///
    /// Cascading matches tmux: a window with no panes and a session with no
    /// windows do not linger.
    pub fn kill_pane(&mut self, pane_id: PaneId) -> Result<(), MuxError> {
        let mut pane = self
            .panes
            .remove(&pane_id)
            .ok_or(MuxError::NoSuchPane(pane_id))?;
        let _ = pane.kill();

        let empty_window = self.windows.iter_mut().find_map(|(id, window)| {
            match window.layout.remove_pane(pane_id) {
                Ok(()) => {
                    if window.active == pane_id {
                        // Killed pane was active; the tree always has at
                        // least one pane left here (remove_pane only errors
                        // on the last pane, handled by the Err arm below),
                        // so the first surviving leaf is a reasonable new
                        // active pane. tmux's own choice of successor is
                        // more elaborate (last-focused history); matching
                        // that is Task 2.4's concern, not this constructor's.
                        window.active = window.layout.pane_ids()[0];
                    }
                    None
                }
                Err(_)
                    if window.active == pane_id && window.layout == LayoutTree::leaf(pane_id) =>
                {
                    // The window's only pane — the window itself closes.
                    Some(*id)
                }
                Err(_) => None,
            }
        });

        if let Some(window_id) = empty_window {
            self.windows.remove(&window_id);
            let empty_session = self.sessions.iter_mut().find_map(|(id, session)| {
                if let Some(pos) = session.windows.iter().position(|w| *w == window_id) {
                    session.windows.remove(pos);
                    if session.active >= session.windows.len() && !session.windows.is_empty() {
                        session.active = session.windows.len() - 1;
                    }
                    if session.windows.is_empty() {
                        return Some(*id);
                    }
                }
                None
            });
            if let Some(session_id) = empty_session {
                self.sessions.remove(&session_id);
            }
        }

        Ok(())
    }

    /// Make `window_id` its session's active window.
    ///
    /// The session is derived from the window itself — a client targets
    /// `@window_id` directly, the way `select-pane -t %pane_id` already does.
    pub fn select_window(&mut self, window_id: WindowId) -> Result<(), MuxError> {
        if !self.windows.contains_key(&window_id) {
            return Err(MuxError::NoSuchWindow(window_id));
        }
        let session = self
            .sessions
            .values_mut()
            .find(|s| s.windows.contains(&window_id))
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        let index = session
            .windows
            .iter()
            .position(|w| *w == window_id)
            .expect("just found by contains");
        session.active = index;
        Ok(())
    }

    /// Rename a window.
    pub fn rename_window(&mut self, window_id: WindowId, name: &str) -> Result<(), MuxError> {
        let window = self
            .windows
            .get_mut(&window_id)
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        window.name = name.to_string();
        Ok(())
    }

    /// Kill a window and every pane it holds, closing its session when it
    /// was the last window — the same cascade [`Self::kill_pane`] uses.
    pub fn kill_window(&mut self, window_id: WindowId) -> Result<(), MuxError> {
        let window = self
            .windows
            .remove(&window_id)
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        for pane_id in window.panes() {
            if let Some(mut pane) = self.panes.remove(&pane_id) {
                let _ = pane.kill();
            }
        }

        let empty_session = self.sessions.iter_mut().find_map(|(id, session)| {
            let pos = session.windows.iter().position(|w| *w == window_id)?;
            session.windows.remove(pos);
            if session.active >= session.windows.len() && !session.windows.is_empty() {
                session.active = session.windows.len() - 1;
            }
            session.windows.is_empty().then_some(*id)
        });
        if let Some(session_id) = empty_session {
            self.sessions.remove(&session_id);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::pane::ShellPaneFactory;

    fn tree() -> MuxTree {
        MuxTree::new(Box::new(ShellPaneFactory::default()))
    }

    #[test]
    fn new_session_creates_a_window_and_a_pane() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).expect("session creates");

        let session = tree.session(session_id).expect("session exists");
        assert_eq!(session.name, "main");
        assert_eq!(
            session.windows.len(),
            1,
            "a new session has exactly one window"
        );

        let window_id = session.windows[0];
        let window = tree.window(window_id).expect("window exists");
        assert_eq!(window.panes().len(), 1, "a new window has exactly one pane");

        let pane_id = window.panes()[0];
        assert!(tree.pane(pane_id).is_some(), "the pane is in the tree");
    }

    #[test]
    fn ids_are_unique_across_sessions() {
        let mut tree = tree();
        let a = tree.new_session("a", 80, 24).unwrap();
        let b = tree.new_session("b", 80, 24).unwrap();
        assert_ne!(a, b);
        assert_eq!(tree.sessions().len(), 2);
    }

    #[test]
    fn splitting_a_pane_joins_the_window_and_takes_its_requested_share() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];

        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.25, None)
            .expect("split creates");
        let window = tree.window(window_id).unwrap();
        assert_eq!(window.panes().len(), 2);
        assert!(window.panes().contains(&second));

        // -p 25 semantics: the NEW pane gets a quarter of the 80-column
        // extent, the target keeps the rest.
        let geo = window
            .layout
            .geometry(0, 0, window.cols as usize, window.rows as usize);
        let width_of = |pane| {
            geo.iter()
                .find(|g| g.pane == pane)
                .unwrap_or_else(|| panic!("pane {pane} in geometry"))
                .width
        };
        assert_eq!(width_of(first), 60);
        assert_eq!(width_of(second), 20);
        assert_eq!(
            window.active, second,
            "tmux's split-window makes the new pane active"
        );
    }

    #[test]
    fn killing_a_pane_removes_it_from_its_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let extra = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.kill_pane(extra).expect("kill succeeds");
        assert!(tree.pane(extra).is_none(), "pane is gone from the tree");
        assert_eq!(tree.window(window_id).unwrap().panes().len(), 1);
    }

    #[test]
    fn killing_the_last_pane_closes_its_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let pane_id = tree.window(window_id).unwrap().panes()[0];

        tree.kill_pane(pane_id).expect("kill succeeds");
        assert!(
            tree.window(window_id).is_none(),
            "a window with no panes does not survive — matches tmux"
        );
    }

    #[test]
    fn killing_the_last_pane_of_the_only_window_removes_the_session() {
        // The second half of the tmux cascade: window closes, and a session
        // with no windows does not linger either.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let pane_id = tree.window(window_id).unwrap().panes()[0];

        tree.kill_pane(pane_id).expect("kill succeeds");
        assert!(tree.window(window_id).is_none());
        assert!(
            tree.session(session_id).is_none(),
            "a session with no windows does not survive — matches tmux"
        );
    }

    #[test]
    fn killing_an_unknown_pane_is_an_error_not_a_panic() {
        let mut tree = tree();
        let result = tree.kill_pane(PaneId(999));
        assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
    }

    #[test]
    fn new_window_adds_a_window_to_the_session() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();

        let window_id = tree
            .new_window(session_id, "logs", 80, 24)
            .expect("window creates");
        let session = tree.session(session_id).unwrap();
        assert_eq!(session.windows.len(), 2);
        assert!(session.windows.contains(&window_id));
        assert_eq!(tree.window(window_id).unwrap().name, "logs");
    }

    #[test]
    fn new_window_rejects_an_unknown_session() {
        let mut tree = tree();
        let result = tree.new_window(SessionId(999), "logs", 80, 24);
        assert!(matches!(result, Err(MuxError::NoSuchSession(_))));
    }

    #[test]
    fn split_pane_rejects_an_unknown_pane() {
        let mut tree = tree();
        let result = tree.split_pane(PaneId(999), SplitDirection::Vertical, 0.5, None);
        assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
    }

    #[test]
    fn select_pane_changes_the_active_pane() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        assert_eq!(tree.window(window_id).unwrap().active, second);

        tree.select_pane(first).expect("select succeeds");
        assert_eq!(tree.window(window_id).unwrap().active, first);
    }

    #[test]
    fn select_pane_rejects_an_unknown_pane() {
        let mut tree = tree();
        let result = tree.select_pane(PaneId(999));
        assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
    }

    #[test]
    fn swap_panes_exchanges_positions_within_a_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.swap_panes(first, second).expect("swap succeeds");
        assert_eq!(
            tree.window(window_id).unwrap().panes(),
            vec![second, first],
            "the panes traded tree positions"
        );
    }

    #[test]
    fn swap_panes_across_windows_is_an_error() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let first_window = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(first_window).unwrap().panes()[0];
        let second_window = tree.new_window(session_id, "logs", 80, 24).unwrap();
        let outsider = tree.window(second_window).unwrap().panes()[0];

        let result = tree.swap_panes(first, outsider);
        assert!(matches!(
            result,
            Err(MuxError::PanesInDifferentWindows(a, b)) if a == first && b == outsider
        ));
    }

    #[test]
    fn resize_pane_grows_the_bordering_split_by_cells() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        tree.split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        // -R 10 on a 0.5 ratio over 80 columns: 0.5 + 10/80 = 0.625.
        tree.resize_pane(first, ResizeDirection::Right, 10)
            .expect("resize succeeds");
        let window = tree.window(window_id).unwrap();
        let geo = window
            .layout
            .geometry(0, 0, window.cols as usize, window.rows as usize);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(width_of(first), 50);
        assert_eq!(geo.iter().find(|g| g.pane != first).unwrap().width, 30);
    }

    #[test]
    fn resize_pane_on_the_wrong_axis_is_an_error() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        // A stacked (Horizontal) split: -R moves a side-by-side divider.
        tree.split_pane(first, SplitDirection::Horizontal, 0.5, None)
            .unwrap();

        let result = tree.resize_pane(first, ResizeDirection::Right, 5);
        assert!(matches!(result, Err(MuxError::PaneNotResizable(_))));
    }

    #[test]
    fn resize_pane_on_a_lone_pane_is_an_error() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];

        let result = tree.resize_pane(first, ResizeDirection::Right, 5);
        assert!(matches!(result, Err(MuxError::PaneNotResizable(_))));
    }

    #[test]
    fn select_window_changes_the_session_active_index() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let second = tree.new_window(session_id, "logs", 80, 24).unwrap();
        assert_eq!(tree.session(session_id).unwrap().active, 0);

        tree.select_window(second).expect("select succeeds");
        assert_eq!(tree.session(session_id).unwrap().active, 1);
    }

    #[test]
    fn select_window_rejects_an_unknown_window() {
        let mut tree = tree();
        let result = tree.select_window(WindowId(999));
        assert!(matches!(result, Err(MuxError::NoSuchWindow(_))));
    }

    #[test]
    fn rename_window_updates_the_name() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];

        tree.rename_window(window_id, "scratch")
            .expect("rename succeeds");
        assert_eq!(tree.window(window_id).unwrap().name, "scratch");
    }

    #[test]
    fn rename_window_rejects_an_unknown_window() {
        let mut tree = tree();
        let result = tree.rename_window(WindowId(999), "x");
        assert!(matches!(result, Err(MuxError::NoSuchWindow(_))));
    }

    #[test]
    fn kill_window_removes_it_and_its_panes_but_keeps_the_session() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let second = tree.new_window(session_id, "logs", 80, 24).unwrap();
        let pane_id = tree.window(second).unwrap().panes()[0];

        tree.kill_window(second).expect("kill succeeds");
        assert!(tree.window(second).is_none(), "window is gone");
        assert!(tree.pane(pane_id).is_none(), "its pane is gone too");
        assert_eq!(
            tree.session(session_id).unwrap().windows,
            vec![window_id],
            "the surviving window remains"
        );
    }

    #[test]
    fn kill_window_of_the_only_window_cascades_to_the_session() {
        // The same cascade kill_pane already has, entered from the window
        // side: a session with no windows does not survive.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];

        tree.kill_window(window_id).expect("kill succeeds");
        assert!(tree.window(window_id).is_none());
        assert!(
            tree.session(session_id).is_none(),
            "a session with no windows does not survive — matches tmux"
        );
    }

    #[test]
    fn kill_window_rejects_an_unknown_window() {
        let mut tree = tree();
        let result = tree.kill_window(WindowId(999));
        assert!(matches!(result, Err(MuxError::NoSuchWindow(_))));
    }

    #[test]
    fn buffer_starts_empty_and_round_trips_through_set_and_get() {
        let mut tree = tree();
        assert_eq!(tree.get_buffer("default"), None, "no buffer yet");

        tree.set_buffer("default", "hello".to_string());
        assert_eq!(tree.get_buffer("default"), Some("hello"));
    }

    #[test]
    fn set_buffer_overwrites_the_previous_value() {
        let mut tree = tree();
        tree.set_buffer("default", "first".to_string());
        tree.set_buffer("default", "second".to_string());
        assert_eq!(tree.get_buffer("default"), Some("second"));
    }
}
