//! The session/window/pane tree: the server's single source of truth.

use crate::mux::ids::{IdAllocator, PaneId, SessionId, Target, WindowId};
use crate::mux::layout::{LayoutTree, ResizeDirection, SplitDirection};
use crate::mux::pane::{MuxError, MuxPane, PaneFactory, SpawnContext};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Kill a pane the tree has already removed, off the tree lock: killing is
/// signal-then-reap with a bounded wait (portable-pty polls its SIGHUP
/// grace for ~200 ms before SIGKILL, and the reap adds a short wait after),
/// and running that under the tree mutex would stall every command for the
/// duration. The thread always exits — SIGKILL cannot be trapped — and the
/// pane it owns is dropped reaped.
fn kill_detached(mut pane: MuxPane) {
    std::thread::spawn(move || {
        let _ = pane.kill();
    });
}

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
    /// The session environment (`set-environment`, `new-session -e`),
    /// applied on top of the daemon's environment for every pane spawned
    /// into this session after it is set.
    pub env: BTreeMap<String, String>,
}

/// The server's whole state: every session, window, and pane.
///
/// Flat maps keyed by id rather than a nested ownership tree, because panes are
/// addressed directly by the protocol (`%3`) far more often than they are
/// walked to from a session.
pub struct MuxTree {
    // pub(crate): the persistence conversions (mux::persist) read and rebuild
    // the tree wholesale; the field set IS the save format's source.
    pub(crate) sessions: HashMap<SessionId, MuxSession>,
    pub(crate) windows: HashMap<WindowId, MuxWindow>,
    pub(crate) panes: HashMap<PaneId, MuxPane>,
    pub(crate) ids: IdAllocator,
    factory: Box<dyn PaneFactory>,
    /// Named paste buffers (`set-buffer`/`show-buffer`). A single value per
    /// name, not tmux's numbered stack — the Phase 2 non-goal in par-mux.md D3.
    pub(crate) buffers: HashMap<String, String>,
    /// The client's per-cell pixel size (`refresh-client -p`), the one
    /// renderer metric every pane shares. Latest report wins, the same
    /// policy the grid-size report (`-C`) uses — par-mux has no other
    /// client-metrics input. `None` until a client reports, so panes keep
    /// the 10×20 construction default. Never persisted: a reconnecting
    /// client re-reports on attach.
    pub(crate) client_cell_pixels: Option<(u16, u16)>,
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
            client_cell_pixels: None,
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

    /// Resolve a pane target: typed `%N` ids pass through untouched (the
    /// caller's pane lookup reports unknown ids as before), any other
    /// value matches the pane's sticky user title exactly — the OSC 0/2
    /// program title never matches, it changes with the running program
    /// and would make name targets flaky.
    ///
    /// A title held by more than one pane is an error listing the
    /// candidate ids, never a silent pick.
    pub fn resolve_pane_target(&self, target: Target<PaneId>) -> Result<PaneId, MuxError> {
        let name = match target {
            Target::Id(id) => return Ok(id),
            Target::Name(name) => name,
        };
        match match_name(
            self.panes
                .iter()
                .filter_map(|(id, pane)| (pane.user_title() == Some(name.as_str())).then_some(*id)),
        ) {
            Match::None => Err(MuxError::NoSuchPaneNamed(name)),
            Match::One(id) => Ok(id),
            Match::Many(ids) => Err(MuxError::AmbiguousPaneTarget(name, ids)),
        }
    }

    /// Resolve a window target: typed `@N` ids pass through; a name
    /// matches window names exactly, across every session (no command
    /// pairs a window target with a session scope today). Ambiguous names
    /// error with the candidates.
    pub fn resolve_window_target(&self, target: Target<WindowId>) -> Result<WindowId, MuxError> {
        let name = match target {
            Target::Id(id) => return Ok(id),
            Target::Name(name) => name,
        };
        match match_name(
            self.windows
                .iter()
                .filter_map(|(id, window)| (window.name == name).then_some(*id)),
        ) {
            Match::None => Err(MuxError::NoSuchWindowNamed(name)),
            Match::One(id) => Ok(id),
            Match::Many(ids) => Err(MuxError::AmbiguousWindowTarget(name, ids)),
        }
    }

    /// Resolve a session target: typed `$N` ids pass through; a name
    /// matches session names exactly. Ambiguous names error with the
    /// candidates.
    pub fn resolve_session_target(&self, target: Target<SessionId>) -> Result<SessionId, MuxError> {
        let name = match target {
            Target::Id(id) => return Ok(id),
            Target::Name(name) => name,
        };
        match match_name(
            self.sessions
                .iter()
                .filter_map(|(id, session)| (session.name == name).then_some(*id)),
        ) {
            Match::None => Err(MuxError::NoSuchSessionNamed(name)),
            Match::One(id) => Ok(id),
            Match::Many(ids) => Err(MuxError::AmbiguousSessionTarget(name, ids)),
        }
    }

    /// Create a session, with one window holding one pane — tmux's shape.
    pub fn new_session(&mut self, name: &str, cols: u16, rows: u16) -> Result<SessionId, MuxError> {
        self.new_session_with_env(name, cols, rows, BTreeMap::new())
    }

    /// [`Self::new_session`] with an initial session environment
    /// (`new-session -e`), applied to the first pane as well.
    pub fn new_session_with_env(
        &mut self,
        name: &str,
        cols: u16,
        rows: u16,
        env: BTreeMap<String, String>,
    ) -> Result<SessionId, MuxError> {
        let session_id = self.ids.next_session();
        let window_id = self.ids.next_window();
        let pane_id = self.ids.next_pane();

        let context = SpawnContext {
            session: Some((session_id, name)),
            window: Some(window_id),
            env: Some(&env),
            cwd: None,
        };
        let pane = self
            .factory
            .create_pane(pane_id, cols, rows, None, &context)?;
        self.panes.insert(pane_id, pane);
        self.apply_cell_pixels(pane_id, cols, rows);

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
                env,
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
        self.new_window_with_cwd(session_id, name, cols, rows, None)
    }

    /// [`Self::new_window`] with a start directory for the new pane — the
    /// `new-window -c` path. `None` keeps the factory-wide default; the
    /// caller (dispatch) owns the gone-directory degrade-to-home rule.
    pub fn new_window_with_cwd(
        &mut self,
        session_id: SessionId,
        name: &str,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
    ) -> Result<WindowId, MuxError> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(MuxError::NoSuchSession(session_id))?;
        let window_id = self.ids.next_window();
        let pane_id = self.ids.next_pane();

        let context = SpawnContext {
            session: Some((session_id, &session.name)),
            window: Some(window_id),
            env: Some(&session.env),
            cwd,
        };
        let pane = self
            .factory
            .create_pane(pane_id, cols, rows, None, &context)?;
        self.panes.insert(pane_id, pane);
        self.apply_cell_pixels(pane_id, cols, rows);
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
    /// becomes the window's active pane. Afterwards every pane terminal in
    /// the window is resized to the new geometry — the layout is the source
    /// of truth for pane extents, and the terminals follow it.
    pub fn split_pane(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
        command: Option<&str>,
    ) -> Result<PaneId, MuxError> {
        self.split_pane_in_window(target, direction, new_share, command, None)
            .map(|(pane_id, _)| pane_id)
    }

    /// [`Self::split_pane`] also reporting the window the new pane landed
    /// in — the dispatcher's form, so its `%layout-change` broadcast names
    /// the window without re-deriving it after the fact. `cwd` is the
    /// `split-window -c` start directory; `None` keeps the factory-wide
    /// default and the caller owns the gone-directory degrade.
    pub fn split_pane_in_window(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
        command: Option<&str>,
        cwd: Option<&Path>,
    ) -> Result<(PaneId, WindowId), MuxError> {
        let window_id = self
            .window_of_pane(target)
            .ok_or(MuxError::NoSuchPane(target))?;
        let (cols, rows) = {
            let window = self.windows.get(&window_id).expect("just found");
            (window.cols, window.rows)
        };
        let pane_id = self.ids.next_pane();
        let session = self
            .session_of_window(window_id)
            .and_then(|id| self.sessions.get(&id));
        let context = SpawnContext {
            session: session.map(|s| (s.id, s.name.as_str())),
            window: Some(window_id),
            env: session.map(|s| &s.env),
            cwd,
        };
        let pane = self
            .factory
            .create_pane(pane_id, cols, rows, command, &context)?;
        self.panes.insert(pane_id, pane);
        {
            let window = self.windows.get_mut(&window_id).expect("just found");
            // `LayoutTree::split_pane`'s ratio is the fraction kept by `first`
            // (the target), while the command speaks in the NEW pane's share.
            window
                .layout
                .split_pane(target, pane_id, direction, 1.0 - new_share)
                .expect("window_of_pane only returns windows holding the pane as a leaf");
            window.active = pane_id;
        }
        self.sync_pane_sizes(window_id);
        Ok((pane_id, window_id))
    }

    /// Set (`Some`) or remove (`None`) one variable in a session's
    /// environment. Running panes are untouched.
    pub fn set_session_env(
        &mut self,
        session_id: SessionId,
        name: &str,
        value: Option<&str>,
    ) -> Result<(), MuxError> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(MuxError::NoSuchSession(session_id))?;
        match value {
            Some(value) => {
                session.env.insert(name.to_string(), value.to_string());
            }
            None => {
                session.env.remove(name);
            }
        }
        Ok(())
    }

    /// The session whose window list holds `window`, if any.
    pub fn session_of_window(&self, window: WindowId) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|session| session.windows.contains(&window))
            .map(|session| session.id)
    }

    /// The window whose layout holds `pane`, if any.
    pub fn window_of_pane(&self, pane: PaneId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, window)| window.layout.pane_ids().contains(&pane))
            .map(|(id, _)| *id)
    }

    /// Make `pane` its window's active pane, returning the window — the
    /// dispatcher's `%layout-change` target.
    pub fn select_pane(&mut self, pane: PaneId) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        self.windows
            .get_mut(&window_id)
            .expect("window_of_pane only returns live windows")
            .active = pane;
        Ok(window_id)
    }

    /// Swap two panes' positions within their window, returning it — the
    /// dispatcher's `%layout-change` target.
    ///
    /// tmux's `swap-pane` exchanges panes inside one window; panes in
    /// different windows have no shared split structure to trade places in.
    /// Both terminals are resized to their traded geometry.
    pub fn swap_panes(&mut self, target: PaneId, source: PaneId) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(target)
            .ok_or(MuxError::NoSuchPane(target))?;
        {
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
                .map_err(|_| MuxError::PanesInDifferentWindows(target, source))?;
        }
        self.sync_pane_sizes(window_id);
        Ok(window_id)
    }

    /// Grow or shrink `pane` by `cells` toward `direction` (tmux's
    /// `-L`/`-R`/`-U`/`-D`), adjusting the ratio of the split it borders —
    /// from either side of it.
    ///
    /// Only a split of the matching orientation can absorb the adjustment:
    /// `-L`/`-R` move a side-by-side divider, `-U`/`-D` a stacked one. A
    /// pane with no such bordering split — a lone pane, or one whose only
    /// bordering split is the other orientation — is an error, not a no-op.
    /// Pane terminals are resized to the new geometry. The Ok payload is the
    /// pane's window — the dispatcher's `%layout-change` target.
    pub fn resize_pane(
        &mut self,
        pane: PaneId,
        direction: ResizeDirection,
        cells: u32,
    ) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        {
            let window = self
                .windows
                .get_mut(&window_id)
                .expect("window_of_pane only returns live windows");
            let Some((split_direction, ratio, target_is_first)) =
                window.layout.bordering_split(pane)
            else {
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
            // The pane's own share of the split grows by the adjustment,
            // whichever side of the divider it sits on; the setter converts
            // back to the split's first-perspective ratio.
            let current_share = if target_is_first { ratio } else { 1.0 - ratio };
            let new_share = current_share + sign * (cells as f32) / extent;
            window
                .layout
                .set_bordering_share(pane, new_share)
                .expect("bordering_split found the split set_bordering_share adjusts");
        }
        self.sync_pane_sizes(window_id);
        Ok(window_id)
    }

    /// Set `pane`'s absolute width and/or height (tmux's `resize-pane -x`/
    /// `-y`), moving the pane's innermost enclosing split of the matching
    /// orientation — the renderer-driven form par-term sends.
    ///
    /// Either bound may be `None` (only the given axis is set). A pane with
    /// no enclosing split along a requested axis — one that already spans
    /// the window there — is an error, not a no-op: there is no divider to
    /// move. Pane terminals are resized to the new geometry. The Ok payload
    /// is the pane's window — the dispatcher's `%layout-change` target.
    pub fn resize_pane_absolute(
        &mut self,
        pane: PaneId,
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        {
            let window = self
                .windows
                .get_mut(&window_id)
                .expect("window_of_pane only returns live windows");
            let bounds = [
                (cols, SplitDirection::Vertical, window.cols as usize),
                (rows, SplitDirection::Horizontal, window.rows as usize),
            ];
            for (bound, axis, extent) in bounds {
                let Some(cells) = bound else { continue };
                match window
                    .layout
                    .set_leaf_extent(pane, axis, cells, extent)
                    .map_err(|_| MuxError::NoSuchPane(pane))?
                {
                    crate::mux::layout::ExtentOutcome::Adjusted => {}
                    crate::mux::layout::ExtentOutcome::SpansAxis => {
                        return Err(MuxError::PaneNotResizable(pane));
                    }
                }
            }
        }
        self.sync_pane_sizes(window_id);
        Ok(window_id)
    }

    /// Set a window's extent and re-fit every pane terminal to the
    /// re-divided geometry.
    ///
    /// The landing point of the window-size policy (par-mux.md Phase 4
    /// T4.C): `refresh-client -C WxH` carries a client's renderer size, and
    /// the latest such report wins — par-mux has no other client-size
    /// input, so "latest-attached-client" and "latest `-C`" are the same
    /// rule here.
    pub fn resize_window(
        &mut self,
        window_id: WindowId,
        cols: u16,
        rows: u16,
    ) -> Result<(), MuxError> {
        let window = self
            .windows
            .get_mut(&window_id)
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        window.cols = cols;
        window.rows = rows;
        self.sync_pane_sizes(window_id);
        Ok(())
    }

    /// Record the client's per-cell pixel size and re-fit every pane to it.
    ///
    /// `refresh-client -p WxH`'s landing point: the cell size is the one
    /// renderer metric every pane shares (grid extents differ per pane, the
    /// font does not), so it is held daemon-wide and every pane's terminal,
    /// PTY `TIOCGWINSZ`, and image cell-span math re-derive from it — see
    /// [`MuxPane::resize_with_cell_pixels`]. Latest report wins, matching
    /// the `-C` grid-size policy.
    pub fn set_client_cell_pixels(&mut self, cell_w: u16, cell_h: u16) {
        self.client_cell_pixels = Some((cell_w, cell_h));
        let window_ids: Vec<WindowId> = self.windows.keys().copied().collect();
        for window_id in window_ids {
            self.sync_pane_sizes(window_id);
        }
    }

    /// Apply the recorded cell pixel size to one just-inserted pane — the
    /// creation paths that build the window around the pane and never
    /// re-fit through [`Self::sync_pane_sizes`]. A no-op until a client
    /// has reported, so fresh daemons spawn with the construction default.
    fn apply_cell_pixels(&mut self, pane_id: PaneId, cols: u16, rows: u16) {
        if let Some((cell_w, cell_h)) = self.client_cell_pixels {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                let _ = pane.resize_with_cell_pixels(cols, rows, cell_w, cell_h);
            }
        }
    }

    /// Resize every pane terminal (and PTY) in `window_id` to the window's
    /// current layout geometry — the step every extent-affecting mutation
    /// ends with, so the terminals `capture-pane` reads and clients render
    /// agree with the layout string the server broadcasts.
    ///
    /// A pane whose PTY cannot be resized (its child is gone) does not fail
    /// the structural mutation around it — the same best-effort treatment
    /// [`Self::kill_pane`] gives pane teardown. The terminal itself, which
    /// is what capture and rendering read, always resizes.
    pub(crate) fn sync_pane_sizes(&mut self, window_id: WindowId) {
        let Some(window) = self.windows.get(&window_id) else {
            return;
        };
        let geometry = window
            .layout
            .geometry(0, 0, window.cols as usize, window.rows as usize);
        for pane_geometry in geometry {
            if let Some(pane) = self.panes.get_mut(&pane_geometry.pane) {
                // A reported cell pixel size rides every re-fit, so grid
                // changes keep XTWINOPS/TIOCGWINSZ/image-span math correct
                // instead of reverting to the construction default.
                let resized = match self.client_cell_pixels {
                    Some((cell_w, cell_h)) => pane.resize_with_cell_pixels(
                        pane_geometry.width as u16,
                        pane_geometry.height as u16,
                        cell_w,
                        cell_h,
                    ),
                    None => pane.resize(pane_geometry.width as u16, pane_geometry.height as u16),
                };
                let _ = resized;
            }
        }
    }

    /// Kill a pane, closing its window when it was the last one, and return
    /// the window that held it — resolved BEFORE the kill, because the pane's
    /// window membership is gone afterwards. The dispatcher's
    /// `%layout-change` target; a window closing entirely reports its own
    /// `%window-close` instead. The second element names the session the
    /// cascade removed, when the window's closure emptied it — the caller's
    /// cue to broadcast `%sessions-changed`.
    ///
    /// Cascading matches tmux: a window with no panes and a session with no
    /// windows do not linger. A surviving pane is resized to the extent the
    /// killed pane freed.
    pub fn kill_pane(
        &mut self,
        pane_id: PaneId,
    ) -> Result<(WindowId, Option<SessionId>), MuxError> {
        let affected_window = self
            .window_of_pane(pane_id)
            .ok_or(MuxError::NoSuchPane(pane_id))?;
        let pane = self
            .panes
            .remove(&pane_id)
            .ok_or(MuxError::NoSuchPane(pane_id))?;
        kill_detached(pane);
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

        let mut removed_session = None;
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
                removed_session = Some(session_id);
            }
        }

        // The killed pane left its window's layout; a surviving pane takes
        // the freed extent and its terminal must grow into it.
        self.sync_pane_sizes(affected_window);

        Ok((affected_window, removed_session))
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
    /// was the last window — the same cascade [`Self::kill_pane`] uses. The
    /// Ok value names that removed session, when the cascade reached it, so
    /// the caller can broadcast `%sessions-changed`.
    pub fn kill_window(&mut self, window_id: WindowId) -> Result<Option<SessionId>, MuxError> {
        let window = self
            .windows
            .remove(&window_id)
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        for pane_id in window.panes() {
            if let Some(pane) = self.panes.remove(&pane_id) {
                kill_detached(pane);
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
        let removed = empty_session.inspect(|&session_id| {
            self.sessions.remove(&session_id);
        });

        Ok(removed)
    }
}

/// The outcome of matching a name against the tree: the resolvers above
/// turn each case into its id-pass-through, not-found, or ambiguity error.
enum Match<I> {
    None,
    One(I),
    Many(Vec<I>),
}

/// Reduce a name's candidate ids to zero/one/many, sorted so an ambiguity
/// error lists candidates in id order regardless of map iteration order.
fn match_name<I: Ord + Copy>(candidates: impl Iterator<Item = I>) -> Match<I> {
    let mut ids: Vec<I> = candidates.collect();
    match ids.len() {
        0 => Match::None,
        1 => Match::One(ids.remove(0)),
        _ => {
            ids.sort_unstable();
            Match::Many(ids)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::pane::test_support::ContextRecordingFactory;
    use crate::mux::pane::ShellPaneFactory;

    fn tree() -> MuxTree {
        MuxTree::new(Box::new(ShellPaneFactory::default()))
    }

    fn recording_tree() -> (MuxTree, ContextRecordingFactory) {
        let factory = ContextRecordingFactory::default();
        (MuxTree::new(Box::new(factory.clone())), factory)
    }

    #[test]
    fn session_env_reaches_panes_spawned_after_it_is_set_only() {
        let (mut tree, factory) = recording_tree();
        let initial = BTreeMap::from([("A".to_string(), "1".to_string())]);
        let session = tree.new_session_with_env("work", 80, 24, initial).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];
        assert_eq!(
            factory.spawn_of(first).env.get("A").map(String::as_str),
            Some("1")
        );

        tree.set_session_env(session, "B", Some("2")).unwrap();
        tree.set_session_env(session, "A", None).unwrap();
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        let later = factory.spawn_of(second).env;
        assert_eq!(later.get("B").map(String::as_str), Some("2"));
        assert!(
            !later.contains_key("A"),
            "an unset var is gone for new panes"
        );
        assert_eq!(
            factory.spawn_of(first).env.get("B"),
            None,
            "the earlier pane's spawn is not rewritten"
        );
        let third_window = tree.new_window(session, "w2", 80, 24).unwrap();
        let third = tree.window(third_window).unwrap().panes()[0];
        assert_eq!(factory.spawn_of(third).env, later);

        assert!(tree.set_session_env(SessionId(99), "X", Some("y")).is_err());
    }

    #[test]
    fn new_session_spawns_with_its_session_and_window_identity() {
        let (mut tree, factory) = recording_tree();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let pane = tree.window(window).unwrap().panes()[0];
        let spawn = factory.spawn_of(pane);
        assert_eq!(spawn.session, Some((session, "work".to_string())));
        assert_eq!(spawn.window, Some(window));
    }

    #[test]
    fn new_window_spawns_with_its_session_and_new_window_identity() {
        let (mut tree, factory) = recording_tree();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.new_window(session, "second", 80, 24).unwrap();
        let pane = tree.window(window).unwrap().panes()[0];
        let spawn = factory.spawn_of(pane);
        assert_eq!(spawn.session, Some((session, "work".to_string())));
        assert_eq!(spawn.window, Some(window));
    }

    /// Card 01a0d9e6f012: the client's cell pixel size is daemon-wide state
    /// that reaches every pane — existing ones re-fit through the sync path,
    /// panes created later inherit it at insert — and the pane terminal's
    /// pixel state (XTWINOPS 14 t's answer) and graphics cell dimensions
    /// (image cell-span math) both derive from it.
    #[test]
    fn client_cell_pixels_reach_existing_and_later_panes() {
        let mut tree = tree();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];

        {
            let term = tree.pane(first).unwrap().terminal();
            let term = term.read();
            assert_eq!(
                (term.pixel_width, term.pixel_height),
                (800, 480),
                "pre-report: the 10x20 construction default (80x24 grid)"
            );
        }

        tree.set_client_cell_pixels(12, 24);
        {
            let term = tree.pane(first).unwrap().terminal();
            let term = term.read();
            assert_eq!(
                (term.pixel_width, term.pixel_height),
                (960, 576),
                "an existing pane re-fits: 80x24 cells at 12x24 px"
            );
            assert_eq!(
                term.graphics.cell_dimensions,
                (12, 24),
                "image cell-span math uses the client's cell size, not the (1,2) default"
            );
        }

        // A pane created after the report inherits it at insert — the
        // creation paths that never run sync_pane_sizes.
        let later_window = tree.new_window(session, "w2", 40, 10).unwrap();
        let later = tree.window(later_window).unwrap().panes()[0];
        {
            let term = tree.pane(later).unwrap().terminal();
            let term = term.read();
            assert_eq!(
                (term.pixel_width, term.pixel_height),
                (480, 240),
                "a later pane derives its totals from its own 40x10 grid"
            );
        }
    }

    #[test]
    fn split_pane_spawns_with_the_target_windows_identity() {
        let (mut tree, factory) = recording_tree();
        tree.new_session("other", 80, 24).unwrap();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.new_window(session, "second", 80, 24).unwrap();
        let target = tree.window(window).unwrap().panes()[0];
        let pane = tree
            .split_pane(target, SplitDirection::Horizontal, 0.5, None)
            .unwrap();
        let spawn = factory.spawn_of(pane);
        assert_eq!(spawn.session, Some((session, "work".to_string())));
        assert_eq!(spawn.window, Some(window));
    }

    /// `split-window -c` / `new-window -c`: the start directory reaches the
    /// factory's spawn context on both dispatcher forms (the plain wrappers
    /// keep the factory-wide default).
    #[test]
    fn a_start_directory_reaches_the_spawn_on_split_and_new_window() {
        let (mut tree, factory) = recording_tree();
        let session = tree.new_session("work", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let target = tree.window(window).unwrap().panes()[0];
        let dir = tempfile::tempdir().unwrap();
        let (split, _) = tree
            .split_pane_in_window(
                target,
                SplitDirection::Horizontal,
                0.5,
                None,
                Some(dir.path()),
            )
            .unwrap();
        assert_eq!(
            factory.spawn_of(split).cwd.as_deref(),
            Some(dir.path()),
            "the split pane spawns in the -c directory"
        );

        let second = tree
            .new_window_with_cwd(session, "second", 80, 24, Some(dir.path()))
            .unwrap();
        let pane = tree.window(second).unwrap().panes()[0];
        assert_eq!(
            factory.spawn_of(pane).cwd.as_deref(),
            Some(dir.path()),
            "the new window's pane spawns in the -c directory"
        );
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

    /// A pane whose child ignores SIGHUP must not survive its kill as a
    /// zombie, and the kill must not hold the tree lock through the ~200 ms
    /// SIGHUP-grace poll portable-pty runs before SIGKILL (card
    /// 01a0d9b4789c79219e7720d61729544c).
    #[cfg(unix)]
    #[test]
    fn killing_a_hup_ignoring_pane_reaps_it_and_returns_promptly() {
        let mut tree = tree();
        let session_id = tree.new_session("zombies", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        // The loop keeps the SHELL itself as the live process: `sh -c` (and
        // zsh) exec-optimizes a trailing simple command, which would turn
        // the pane's process into `sleep` — a fresh binary that never ran
        // the trap and dies to the first SIGHUP. The echo is a readiness
        // marker: kill must not race the shell's own startup (a SIGHUP
        // delivered before the trap line runs kills the shell outright).
        let doomed = tree
            .split_pane(
                first,
                SplitDirection::Vertical,
                0.5,
                Some("trap '' HUP; echo PANEMUX-TRAP-SET; while true; do sleep 57; done"),
            )
            .unwrap();
        let pid = tree
            .pane(doomed)
            .expect("doomed pane exists")
            .child_pid()
            .expect("child pid");
        let ready = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let screen = tree
                .pane(doomed)
                .expect("doomed pane exists")
                .terminal()
                .read()
                .content();
            if screen.contains("PANEMUX-TRAP-SET") {
                break;
            }
            assert!(
                std::time::Instant::now() < ready,
                "trap marker never reached the pane screen: {screen}"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        let started = std::time::Instant::now();
        tree.kill_pane(doomed).expect("kill succeeds");
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(150),
            "kill_pane held the tree lock through the SIGHUP grace poll: {elapsed:?}"
        );

        // A reaped child vanishes from the process table; a zombie keeps
        // answering signal 0 until someone waits for it. The detached kill
        // needs a moment, so poll to a deadline instead of asserting at
        // once — the assertion is that it EVER goes away, promptly.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let gone = unsafe { libc::kill(pid as libc::pid_t, 0) != 0 };
            if gone {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child {pid} still in the process table after kill — unreaped zombie"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
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
    fn kill_results_name_the_session_the_cascade_removed() {
        // The %sessions-changed cue: both entry points report the removed
        // session, and report None when the session survives.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let second = tree.new_window(session_id, "logs", 80, 24).unwrap();
        let pane_id = tree.window(second).unwrap().panes()[0];

        let (_, removed) = tree.kill_pane(pane_id).expect("kill succeeds");
        assert_eq!(removed, None, "the session survives its non-last window");
        let (_, removed) = tree
            .kill_pane(tree.window(window_id).unwrap().panes()[0])
            .expect("kill succeeds");
        assert_eq!(removed, Some(session_id), "the cascade names the session");

        let session_id = tree.new_session("next", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        assert_eq!(
            tree.kill_window(window_id).expect("kill succeeds"),
            Some(session_id),
            "kill-window names the session it emptied"
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
    fn split_pane_resizes_both_terminals_to_the_layout_geometry() {
        // The Phase 4 T4.C fidelity contract: the layout tree is the source
        // of truth for pane extents, and the terminals (with their PTYs)
        // follow it. Before T4.C the new pane spawned at the window's full
        // size and the target kept its old size, so capture-pane and client
        // rendering disagreed with the layout string.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];

        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        let size_of = |pane| {
            tree.pane(pane)
                .expect("pane in tree")
                .terminal()
                .read()
                .size()
        };
        assert_eq!(size_of(first), (40, 24), "the target shrank to its half");
        assert_eq!(
            size_of(second),
            (40, 24),
            "the new pane spawned at its geometry, not the window's full size"
        );
    }

    #[test]
    fn resize_pane_relative_syncs_pane_terminals() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.resize_pane(first, ResizeDirection::Right, 10).unwrap();

        let size_of = |pane| tree.pane(pane).unwrap().terminal().read().size();
        assert_eq!(size_of(first), (50, 24));
        assert_eq!(size_of(second), (30, 24));
    }

    #[test]
    fn resize_pane_relative_works_for_the_second_pane_too() {
        // A pane on either side of its bordering split can grow; before
        // T4.C only the split's `first` child was resizable.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        // -R on the right-hand pane: it grows by taking from its sibling.
        tree.resize_pane(second, ResizeDirection::Right, 10)
            .unwrap();

        let geo = tree
            .window(window_id)
            .unwrap()
            .layout
            .geometry(0, 0, 80, 24);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(width_of(second), 50);
        assert_eq!(width_of(first), 30);
    }

    #[test]
    fn resize_pane_absolute_sets_exact_dimensions() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        tree.split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.resize_pane_absolute(first, Some(25), None).unwrap();

        let geo = tree
            .window(window_id)
            .unwrap()
            .layout
            .geometry(0, 0, 80, 24);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(width_of(first), 25);
        assert_eq!(
            width_of(tree.window(window_id).unwrap().panes()[1]),
            55,
            "the sibling absorbs the difference"
        );
        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (25, 24),
            "the terminal follows the absolute size"
        );
    }

    #[test]
    fn resize_pane_absolute_through_a_cross_orientation_ancestor() {
        // Split(V){0, Split(H){1,2}}: pane 1's width is set by the OUTER
        // vertical divider — its direct parent is horizontal, so a naive
        // direct-parent lookup would wrongly call it unresizable.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();
        let third = tree
            .split_pane(second, SplitDirection::Horizontal, 0.5, None)
            .unwrap();

        tree.resize_pane_absolute(second, Some(20), None).unwrap();

        let geo = tree
            .window(window_id)
            .unwrap()
            .layout
            .geometry(0, 0, 80, 24);
        let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
        assert_eq!(width_of(second), 20);
        assert_eq!(width_of(third), 20, "the stacked sibling shares the width");
        assert_eq!(width_of(first), 60);
    }

    #[test]
    fn resize_pane_absolute_on_a_spanning_pane_is_an_error() {
        // A lone pane spans both axes; there is no divider to move.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];

        let result = tree.resize_pane_absolute(first, Some(40), None);
        assert!(matches!(result, Err(MuxError::PaneNotResizable(_))));
    }

    #[test]
    fn resize_pane_absolute_rejects_an_unknown_pane() {
        let mut tree = tree();
        let result = tree.resize_pane_absolute(PaneId(999), Some(40), None);
        assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
    }

    #[test]
    fn killing_a_pane_resizes_the_survivor_to_the_full_window() {
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.kill_pane(second).unwrap();

        assert_eq!(
            tree.pane(first).unwrap().terminal().read().size(),
            (80, 24),
            "the surviving pane takes the freed extent"
        );
    }

    #[test]
    fn swapping_panes_trades_their_terminal_sizes() {
        // An asymmetric split: swap must resize both terminals to their new
        // geometry, not just exchange tree positions.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.25, None)
            .unwrap();
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (60, 24));
        assert_eq!(
            tree.pane(second).unwrap().terminal().read().size(),
            (20, 24)
        );

        tree.swap_panes(first, second).unwrap();

        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (20, 24));
        assert_eq!(
            tree.pane(second).unwrap().terminal().read().size(),
            (60, 24)
        );
    }

    #[test]
    fn resize_window_refits_every_pane_terminal() {
        // The refresh-client -C landing point: the window's extent changes,
        // the layout re-divides it, the terminals follow.
        let mut tree = tree();
        let session_id = tree.new_session("main", 80, 24).unwrap();
        let window_id = tree.session(session_id).unwrap().windows[0];
        let first = tree.window(window_id).unwrap().panes()[0];
        tree.split_pane(first, SplitDirection::Vertical, 0.5, None)
            .unwrap();

        tree.resize_window(window_id, 120, 40).unwrap();

        let window = tree.window(window_id).unwrap();
        assert_eq!((window.cols, window.rows), (120, 40));
        assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (60, 40));
        assert_eq!(
            tree.pane(window.panes()[1])
                .unwrap()
                .terminal()
                .read()
                .size(),
            (60, 40)
        );
    }

    #[test]
    fn resize_window_rejects_an_unknown_window() {
        let mut tree = tree();
        let result = tree.resize_window(WindowId(999), 120, 40);
        assert!(matches!(result, Err(MuxError::NoSuchWindow(_))));
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

    /// Two sessions of one window of one pane each, with the panes titled
    /// per the test's needs — the shape every name-target test starts from.
    fn named_tree(first_title: Option<&str>, second_title: Option<&str>) -> MuxTree {
        let (mut tree, _factory) = recording_tree();
        let session = tree.new_session("alpha", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];
        let second_session = tree.new_session("beta", 80, 24).unwrap();
        let second_window = tree.session(second_session).unwrap().windows[0];
        let second = tree.window(second_window).unwrap().panes()[0];
        if let Some(title) = first_title {
            tree.pane_mut(first).unwrap().set_user_title(title);
        }
        if let Some(title) = second_title {
            tree.pane_mut(second).unwrap().set_user_title(title);
        }
        tree
    }

    #[test]
    fn pane_target_resolves_by_user_title_per_kind() {
        let tree = named_tree(Some("build"), None);
        let resolved = tree
            .resolve_pane_target(Target::Name("build".to_string()))
            .unwrap();
        // The titled pane is %0 (first session's first pane).
        assert_eq!(resolved, PaneId(0));
        // The other pane stays reachable by its own distinct title.
        let other = named_tree(None, Some("logs"));
        assert_eq!(
            other
                .resolve_pane_target(Target::Name("logs".to_string()))
                .unwrap(),
            PaneId(1)
        );
    }

    #[test]
    fn window_and_session_targets_resolve_by_name() {
        let mut tree = named_tree(None, None);
        // new_session names the window after the session, so rename one
        // window to prove name resolution is not id-shaped luck.
        tree.rename_window(WindowId(1), "logs").unwrap();
        assert_eq!(
            tree.resolve_window_target(Target::Name("logs".to_string()))
                .unwrap(),
            WindowId(1)
        );
        assert_eq!(
            tree.resolve_session_target(Target::Name("beta".to_string()))
                .unwrap(),
            SessionId(1)
        );
    }

    #[test]
    fn unknown_names_error_without_touching_the_tree() {
        let tree = named_tree(Some("build"), None);
        assert!(matches!(
            tree.resolve_pane_target(Target::Name("nope".to_string())),
            Err(MuxError::NoSuchPaneNamed(n)) if n == "nope"
        ));
        assert!(matches!(
            tree.resolve_window_target(Target::Name("nope".to_string())),
            Err(MuxError::NoSuchWindowNamed(n)) if n == "nope"
        ));
        assert!(matches!(
            tree.resolve_session_target(Target::Name("nope".to_string())),
            Err(MuxError::NoSuchSessionNamed(n)) if n == "nope"
        ));
    }

    #[test]
    fn ambiguous_names_error_listing_sorted_candidate_ids() {
        let tree = named_tree(Some("dup"), Some("dup"));
        match tree.resolve_pane_target(Target::Name("dup".to_string())) {
            Err(MuxError::AmbiguousPaneTarget(name, ids)) => {
                assert_eq!(name, "dup");
                assert_eq!(ids, vec![PaneId(0), PaneId(1)], "candidates in id order");
            }
            other => panic!("ambiguity must error, got {other:?}"),
        }
        // Windows: two sessions' initial windows both carry their
        // session's name, so one shared name makes them ambiguous.
        let mut tree = named_tree(None, None);
        tree.rename_window(WindowId(1), "alpha").unwrap();
        match tree.resolve_window_target(Target::Name("alpha".to_string())) {
            Err(MuxError::AmbiguousWindowTarget(_, ids)) => {
                assert_eq!(ids, vec![WindowId(0), WindowId(1)]);
            }
            other => panic!("ambiguity must error, got {other:?}"),
        }
    }

    #[test]
    fn typed_ids_pass_through_resolution_untouched() {
        let tree = named_tree(Some("%1"), None);
        // A pane TITLED "%1" must not capture id targets: %1 still means
        // pane 1, whose existence the caller reports as before.
        assert_eq!(
            tree.resolve_pane_target(Target::Id(PaneId(1))).unwrap(),
            PaneId(1)
        );
        // And the title "%1" is unreachable BY NAME (the parser classifies
        // sigil-prefixed values as ids), so no name can shadow an id.
        assert!(matches!(
            tree.resolve_pane_target(Target::parse("%1").unwrap()),
            Ok(PaneId(1))
        ));
    }

    #[test]
    fn duplicate_session_names_are_ambiguous_not_silently_picked() {
        let (mut tree, _factory) = recording_tree();
        tree.new_session("dup", 80, 24).unwrap();
        tree.new_session("dup", 80, 24).unwrap();
        match tree.resolve_session_target(Target::Name("dup".to_string())) {
            Err(MuxError::AmbiguousSessionTarget(_, ids)) => {
                assert_eq!(ids, vec![SessionId(0), SessionId(1)]);
            }
            other => panic!("ambiguity must error, got {other:?}"),
        }
    }
}
