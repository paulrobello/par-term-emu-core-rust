//! The session/window/pane tree: the server's single source of truth.

use crate::mux::ids::{IdAllocator, PaneId, SessionId, WindowId};
use crate::mux::pane::{MuxError, MuxPane, PaneFactory};
use std::collections::HashMap;

/// One window: an ordered set of panes with one of them active.
#[derive(Debug)]
pub struct MuxWindow {
    /// This window's identifier.
    pub id: WindowId,
    /// Display name.
    pub name: String,
    /// Panes in layout order.
    pub panes: Vec<PaneId>,
    /// Index into `panes` of the active pane.
    pub active: usize,
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
        }
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
                panes: vec![pane_id],
                active: 0,
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
            // MuxError has no session/window variants yet; the server task
            // (Task 6) extends the enum when it starts reporting these to
            // clients. The placeholder pane id is never rendered.
            return Err(MuxError::NoSuchPane(PaneId(0)));
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
                panes: vec![pane_id],
                active: 0,
            },
        );
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.windows.push(window_id);
        }
        Ok(window_id)
    }

    /// Add a pane to an existing window.
    pub fn new_pane(
        &mut self,
        window_id: WindowId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
    ) -> Result<PaneId, MuxError> {
        if !self.windows.contains_key(&window_id) {
            return Err(MuxError::NoSuchPane(PaneId(0)));
        }
        let pane_id = self.ids.next_pane();
        let pane = self.factory.create_pane(pane_id, cols, rows, command)?;
        self.panes.insert(pane_id, pane);
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.panes.push(pane_id);
        }
        Ok(pane_id)
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
            if let Some(pos) = window.panes.iter().position(|p| *p == pane_id) {
                window.panes.remove(pos);
                if window.active >= window.panes.len() && !window.panes.is_empty() {
                    window.active = window.panes.len() - 1;
                }
                if window.panes.is_empty() {
                    return Some(*id);
                }
            }
            None
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
        assert_eq!(window.panes.len(), 1, "a new window has exactly one pane");

        let pane_id = window.panes[0];
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
    fn new_pane_joins_an_existing_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];

        let pane_id = tree
            .new_pane(window_id, 80, 12, None)
            .expect("pane creates");
        let window = tree.window(window_id).unwrap();
        assert_eq!(window.panes.len(), 2);
        assert!(window.panes.contains(&pane_id));
    }

    #[test]
    fn killing_a_pane_removes_it_from_its_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let extra = tree.new_pane(window_id, 80, 12, None).unwrap();

        tree.kill_pane(extra).expect("kill succeeds");
        assert!(tree.pane(extra).is_none(), "pane is gone from the tree");
        assert_eq!(tree.window(window_id).unwrap().panes.len(), 1);
    }

    #[test]
    fn killing_the_last_pane_closes_its_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let pane_id = tree.window(window_id).unwrap().panes[0];

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
        let pane_id = tree.window(window_id).unwrap().panes[0];

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
}
