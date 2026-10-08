//! Layout and geometry operations: splits, selection, zoom, pane and window
//! moves, resizes, client metrics, and the `mutate_layout` choke point
//! every layout-shape edit goes through (ARC-090).

use super::{kill_detached, ClientView, MuxTree, MuxWindow, SplitSpawn};
use crate::color::Color;
use crate::mux::ids::{PaneId, SessionId, WindowId, WorkspaceId};
use crate::mux::layout::{LayoutTree, ResizeDirection, SplitDirection};
use crate::mux::pane::{MuxError, MuxPane};
use std::path::Path;

impl MuxTree {
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
    ///
    /// One-shot form of the two-phase flow for callers that already hold
    /// the tree exclusively.
    pub fn split_pane_in_window(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
        command: Option<&str>,
        cwd: Option<&Path>,
    ) -> Result<(PaneId, WindowId), MuxError> {
        let plan = self.begin_split(target, direction, new_share, cwd, false)?;
        let pane = self.spawn_from(
            &plan.pane_id,
            plan.cols,
            plan.rows,
            command,
            &plan.context(),
        );
        self.complete_split(plan, pane?)
    }

    /// Phase 1 of `split-window`: resolve the target's window, snapshot its
    /// geometry and session environment, reserve the pane id.
    pub fn begin_split(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
        cwd: Option<&Path>,
        before: bool,
    ) -> Result<SplitSpawn, MuxError> {
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
        Ok(SplitSpawn {
            pane_id,
            window_id,
            target,
            direction,
            new_share,
            before,
            cols,
            rows,
            session: session.map(|s| (s.id, s.name.clone())),
            env: session.map(|s| s.env.clone()),
            cwd: cwd.map(Path::to_owned),
        })
    }

    /// Phase 3 of `split-window`: insert the pane and re-shape the layout.
    /// The target pane (or its window) may be gone after the unlocked
    /// spawn; the pane is killed and the error reported rather than leaking
    /// a live PTY.
    pub fn complete_split(
        &mut self,
        plan: SplitSpawn,
        pane: MuxPane,
    ) -> Result<(PaneId, WindowId), MuxError> {
        let SplitSpawn {
            pane_id,
            window_id,
            target,
            direction,
            new_share,
            before,
            cols: _,
            rows: _,
            session: _,
            env: _,
            cwd: _,
        } = plan;
        // `LayoutTree::split_pane`'s ratio is the fraction kept by `first`
        // (the target), while the command speaks in the NEW pane's share.
        // `-b` instead puts the NEW pane in `first`: split at the new
        // pane's share and swap the two leaves, which lands the new pane
        // left/above with exactly its `-p` share.
        // The target (or its window) may be gone: validate before the
        // insert so a failure leaves the tree untouched.
        if self.window_of_pane(target) != Some(window_id) {
            kill_detached(pane);
            return Err(MuxError::NoSuchPane(target));
        }
        self.panes.insert(pane_id, pane);
        // Through the choke point: it ends the zoom that would hide the
        // new pane, indexes the new pane (ARC-096), and re-fits every
        // terminal to the new geometry.
        self.mutate_layout(window_id, |window| {
            let placed = if before {
                window
                    .layout
                    .split_pane(target, pane_id, direction, new_share)
                    .and_then(|_| window.layout.swap_pane(target, pane_id))
            } else {
                window
                    .layout
                    .split_pane(target, pane_id, direction, 1.0 - new_share)
            };
            placed.expect("the target is a leaf of this window, checked above");
            window.active = pane_id;
            Ok(())
        })?;
        Ok((pane_id, window_id))
    }

    /// Make `pane` its window's active pane, returning the window — the
    /// dispatcher's `%layout-change` target.
    pub fn select_pane(&mut self, pane: PaneId) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        let mut unzoomed = false;
        {
            let window = self
                .windows
                .get_mut(&window_id)
                .expect("window_of_pane only returns live windows");
            window.active = pane;
            // Selecting another pane reveals the layout — tmux unzooms on
            // the switch; selecting the zoomed pane itself keeps the zoom.
            if window.zoomed.is_some_and(|z| z != pane) {
                window.zoomed = None;
                unzoomed = true;
            }
        }
        if unzoomed {
            self.sync_pane_sizes(window_id);
        }
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
        if self.window_of_pane(source).is_none() {
            return Err(MuxError::NoSuchPane(source));
        }
        self.mutate_layout(window_id, |window| {
            if !window.layout.pane_ids().contains(&source) {
                return Err(MuxError::PanesInDifferentWindows(target, source));
            }
            window
                .layout
                .swap_pane(target, source)
                .map_err(|_| MuxError::PanesInDifferentWindows(target, source))?;
            Ok(window_id)
        })
    }

    /// Toggle `pane`'s zoom (`resize-pane -Z`): zooming resizes its
    /// terminal and PTY to the full window grid; unzooming re-fits every
    /// pane to the layout, which the zoom never edited — the exact prior
    /// geometry. Zooming a different pane moves the zoom to it. The Ok
    /// payload is the pane's window — the dispatcher's `%layout-change`
    /// target.
    pub fn zoom_pane(&mut self, pane: PaneId) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        {
            let window = self
                .windows
                .get_mut(&window_id)
                .expect("window_of_pane only returns live windows");
            window.zoomed = if window.zoomed == Some(pane) {
                None
            } else {
                Some(pane)
            };
        }
        self.sync_pane_sizes(window_id);
        Ok(window_id)
    }

    /// `break-pane -s %N`: move `pane` out of its window into a new
    /// window appended to the same session, which becomes the session's
    /// active window (tmux's behavior). The pane's process, terminal,
    /// and title all move with it — the move is pure layout surgery, so
    /// nothing is re-spawned. The new window inherits the source
    /// window's grid size, and the pane's terminal is re-fitted to that
    /// full extent. The source window closes when the pane was its last
    /// one; the session cannot die that way — the new window is
    /// inserted before the source is dropped, so the cascade in
    /// [`Self::drop_empty_window`] never finds an empty session. The Ok
    /// payload is `(new window, source window, whether the source
    /// window closed)`.
    pub fn break_pane(
        &mut self,
        pane: PaneId,
        name: &str,
    ) -> Result<(WindowId, WindowId, bool), MuxError> {
        let source_window = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        let session_id = self
            .session_of_window(source_window)
            .ok_or(MuxError::NoSuchWindow(source_window))?;
        let (cols, rows) = {
            let window = self
                .windows
                .get(&source_window)
                .expect("window_of_pane only returns live windows");
            (window.cols, window.rows)
        };
        let only_pane = self
            .windows
            .get(&source_window)
            .expect("window_of_pane only returns live windows")
            .layout
            == LayoutTree::leaf(pane);

        // The new window exists before the pane leaves the source, so an
        // only-pane break never leaves the session windowless. Inserting
        // it re-homes the pane's index entry first; the source's removal
        // below then leaves that entry alone.
        let window_id = self.ids.next_window();
        self.insert_window(
            session_id,
            MuxWindow {
                id: window_id,
                name: name.to_string(),
                layout: LayoutTree::leaf(pane),
                active: pane,
                cols,
                rows,
                zoomed: None,
            },
        );
        {
            let session = self
                .sessions
                .get_mut(&session_id)
                .expect("checked directly above");
            session.active = session.windows.len() - 1;
        }

        let source_closed = if only_pane {
            self.drop_empty_window(source_window);
            true
        } else {
            self.mutate_layout(source_window, |window| {
                window.layout.remove_pane(pane).expect("not the only pane");
                if window.active == pane {
                    window.active = window.layout.pane_ids()[0];
                }
                Ok(())
            })?;
            false
        };
        // A fresh, unzoomed window around the pane — a fit, not a mutation.
        self.sync_pane_sizes(window_id);
        Ok((window_id, source_window, source_closed))
    }

    /// `join-pane -s %N -t %M`: move `source` next to `target` — into
    /// `target`'s window when the panes live in different ones, the
    /// `split-window` arrangement rule (`-h` beside, `-v`/default
    /// below) and `-p` share applied to the moved pane. The moved pane
    /// becomes the destination window's active pane, like a split's new
    /// pane does. The source window closes when the pane was its last
    /// one — and that cascade CAN reach the session (the destination
    /// belongs to whichever window `target` lives in, so nothing
    /// backstops the source's session). The Ok payload is
    /// `(destination window, source window, whether the source window
    /// closed, the session removed by the cascade)`.
    pub fn join_pane(
        &mut self,
        source: PaneId,
        target: PaneId,
        direction: SplitDirection,
        new_share: f32,
    ) -> Result<(WindowId, WindowId, bool, Option<SessionId>), MuxError> {
        let source_window = self
            .window_of_pane(source)
            .ok_or(MuxError::NoSuchPane(source))?;
        if source == target {
            return Err(MuxError::SamePane(source));
        }
        let target_window = self
            .window_of_pane(target)
            .ok_or(MuxError::NoSuchPane(target))?;
        let only_pane = self
            .windows
            .get(&source_window)
            .expect("window_of_pane only returns live windows")
            .layout
            == LayoutTree::leaf(source);
        let remove_source = |window: &mut MuxWindow| {
            window
                .layout
                .remove_pane(source)
                .expect("not the only pane");
            if window.active == source {
                window.active = window.layout.pane_ids()[0];
            }
        };
        let place_source = |window: &mut MuxWindow| {
            window
                .layout
                .split_pane(target, source, direction, new_share)
                .expect("target pane exists in its own window");
            window.active = source;
        };

        if source_window == target_window {
            if only_pane {
                // The target lives in the same single-pane window, so it
                // is the source itself — rejected above. Unreachable.
                return Err(MuxError::SamePane(source));
            }
            // One mutation, so no pane re-fits to the transient layout
            // between the removal and the re-split.
            self.mutate_layout(target_window, |window| {
                remove_source(window);
                place_source(window);
                Ok(())
            })?;
            return Ok((target_window, source_window, false, None));
        }

        let mut removed_session = None;
        if only_pane {
            removed_session = self.drop_empty_window(source_window);
        } else {
            self.mutate_layout(source_window, |window| {
                remove_source(window);
                Ok(())
            })?;
        }
        self.mutate_layout(target_window, |window| {
            place_source(window);
            Ok(())
        })?;
        Ok((target_window, source_window, only_pane, removed_session))
    }

    /// `move-window -s @N -t <index>`: move `window_id` to `index` in
    /// its session's window list, clamping out-of-range positions to
    /// the ends. The active window is tracked by identity, not
    /// position — the window that was active stays active after the
    /// list moves.
    pub fn move_window(&mut self, window_id: WindowId, index: usize) -> Result<(), MuxError> {
        let session = self
            .session_of_window(window_id)
            .and_then(|id| self.sessions.get_mut(&id))
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        let active_window = session.windows.get(session.active).copied();
        let pos = session
            .windows
            .iter()
            .position(|w| *w == window_id)
            .expect("the index names this window's session");
        session.windows.remove(pos);
        let insert_at = index.min(session.windows.len());
        session.windows.insert(insert_at, window_id);
        if let Some(active) = active_window {
            session.active = session
                .windows
                .iter()
                .position(|w| *w == active)
                .expect("the active window is in the list");
        }
        Ok(())
    }

    /// `swap-window -s @A -t @B`: exchange the two windows' positions in
    /// their session's window list. Same session only — a swap across
    /// sessions changes window ownership, a different operation than a
    /// reorder. The active window is tracked by identity, not position.
    pub fn swap_windows(&mut self, a: WindowId, b: WindowId) -> Result<(), MuxError> {
        for id in [a, b] {
            if !self.windows.contains_key(&id) {
                return Err(MuxError::NoSuchWindow(id));
            }
        }
        if a == b {
            return Ok(());
        }
        let session_id = match (self.session_of_window(a), self.session_of_window(b)) {
            (Some(sa), Some(sb)) if sa == sb => sa,
            _ => return Err(MuxError::WindowsInDifferentSessions(a, b)),
        };
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(MuxError::WindowsInDifferentSessions(a, b))?;
        let active_window = session.windows.get(session.active).copied();
        let pos_a = session
            .windows
            .iter()
            .position(|w| *w == a)
            .expect("the index names this window's session");
        let pos_b = session
            .windows
            .iter()
            .position(|w| *w == b)
            .expect("the index names this window's session");
        session.windows.swap(pos_a, pos_b);
        if let Some(active) = active_window {
            session.active = session
                .windows
                .iter()
                .position(|w| *w == active)
                .expect("the active window is in the list");
        }
        Ok(())
    }

    /// Grow or shrink `pane` by `cells` (tmux's `-L`/`-R`/`-U`/`-D`): the
    /// pane's extent along the pressed direction's axis changes by `cells`
    /// (`-R`/`-D` grow, `-L`/`-U` shrink), absorbed by the innermost
    /// enclosing split of that orientation — an ancestor at any depth, so
    /// a pane nested under cross-orientation splits resizes on both axes
    /// (the manual-pass report: with neighbors on both axes, arrows could
    /// only resize along one).
    ///
    /// A pane already spanning the pressed axis — a lone pane, or a window
    /// whose only splits run the other way — is an error, not a no-op.
    /// Pane terminals are resized to the new geometry. The Ok payload is
    /// the pane's window — the dispatcher's `%layout-change` target.
    pub fn resize_pane(
        &mut self,
        pane: PaneId,
        direction: ResizeDirection,
        cells: u32,
    ) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        self.mutate_layout(window_id, |window| {
            let rects = window
                .layout
                .geometry(0, 0, window.cols as usize, window.rows as usize);
            let Some(rect) = rects.iter().find(|g| g.pane == pane) else {
                return Err(MuxError::NoSuchPane(pane));
            };
            let (cols, rows) = match direction {
                ResizeDirection::Left => {
                    (Some(rect.width.saturating_sub(cells as usize).max(1)), None)
                }
                ResizeDirection::Right => (
                    Some((rect.width + cells as usize).min(window.cols as usize)),
                    None,
                ),
                ResizeDirection::Up => (
                    None,
                    Some(rect.height.saturating_sub(cells as usize).max(1)),
                ),
                ResizeDirection::Down => (
                    None,
                    Some((rect.height + cells as usize).min(window.rows as usize)),
                ),
            };
            let mut next = window.layout.clone();
            Self::set_leaf_extents(
                &mut next,
                pane,
                cols.map(|value| value as u16),
                rows.map(|value| value as u16),
                window.cols as usize,
                window.rows as usize,
            )?;
            window.layout = next;
            Ok(window_id)
        })
    }

    /// Set `pane`'s absolute width and/or height (tmux's `resize-pane -x`/
    /// `-y`), moving the pane's innermost enclosing split of the matching
    /// orientation — the renderer-driven form par-term sends.
    ///
    /// Either bound may be `None` (only the given axis is set). A pane with
    /// no enclosing split along a requested axis — one that already spans
    /// the window there — is an error, not a no-op: there is no divider to
    /// move. All-or-nothing: when either requested axis fails, neither is
    /// applied. Pane terminals are resized to the new geometry. The Ok
    /// payload is the pane's window — the dispatcher's `%layout-change`
    /// target.
    pub fn resize_pane_absolute(
        &mut self,
        pane: PaneId,
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> Result<WindowId, MuxError> {
        let window_id = self
            .window_of_pane(pane)
            .ok_or(MuxError::NoSuchPane(pane))?;
        self.mutate_layout(window_id, |window| {
            let mut next = window.layout.clone();
            Self::set_leaf_extents(
                &mut next,
                pane,
                cols,
                rows,
                window.cols as usize,
                window.rows as usize,
            )?;
            window.layout = next;
            Ok(window_id)
        })
    }

    /// Set `pane`'s extent along one or both axes by walking its
    /// enclosing splits — the shared body of `resize-pane`'s relative and
    /// absolute forms. `None` skips that axis. Targets arrive pre-clamped
    /// to the window extent; the walk maps a pane that spans the axis (no
    /// divider to move) to `PaneNotResizable` and an absent pane to
    /// `NoSuchPane`.
    fn set_leaf_extents(
        layout: &mut crate::mux::layout::LayoutTree,
        pane: PaneId,
        cols: Option<u16>,
        rows: Option<u16>,
        window_cols: usize,
        window_rows: usize,
    ) -> Result<(), MuxError> {
        let bounds = [
            (cols, SplitDirection::Vertical, window_cols),
            (rows, SplitDirection::Horizontal, window_rows),
        ];
        for (bound, axis, extent) in bounds {
            let Some(cells) = bound else { continue };
            match layout.set_leaf_extent(pane, axis, cells, extent) {
                Ok(crate::mux::layout::ExtentOutcome::Adjusted) => {}
                Ok(crate::mux::layout::ExtentOutcome::SpansAxis) => {
                    return Err(MuxError::PaneNotResizable(pane));
                }
                Err(_) => return Err(MuxError::NoSuchPane(pane)),
            }
        }
        Ok(())
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

    /// Record `client_id`'s sizing contribution — the grid it reported via
    /// `refresh-client -C` and the window it is displaying — and re-fit the
    /// windows the smallest-attached-client rule governs.
    ///
    /// Returns the windows whose extent actually changed (a grow or shrink
    /// under the new minimum), so the caller broadcasts `%layout-change`
    /// only for those. Extents are clamped positive: a sub-minimum report
    /// that would make the window 0 wide/tall cannot land.
    pub fn set_client_view(
        &mut self,
        client_id: u64,
        window: WindowId,
        cols: u16,
        rows: u16,
    ) -> Vec<WindowId> {
        let view = ClientView {
            window,
            cols: cols.max(1),
            rows: rows.max(1),
        };
        let (previous_window, updated) = match self.client_views.get(&client_id) {
            Some(previous) => (Some(previous.window), *previous == view),
            None => (None, false),
        };
        self.client_views.insert(client_id, view);
        if updated {
            return Vec::new();
        }
        // Only the windows the change can move: the displayed one and, when
        // the view switched windows, the one it left.
        let mut candidates = vec![window];
        if let Some(previous_window) = previous_window {
            if previous_window != window {
                candidates.push(previous_window);
            }
        }
        self.refit_reported_windows(&candidates)
    }

    /// Drop `client_id`'s sizing contribution and re-fit the window it was
    /// displaying — the disconnect path: with the constraining report gone
    /// the window may grow to the remaining viewers' minimum.
    pub fn clear_client_view(&mut self, client_id: u64) -> Vec<WindowId> {
        let Some(removed) = self.client_views.remove(&client_id) else {
            return Vec::new();
        };
        self.refit_reported_windows(&[removed.window])
    }

    /// Point every tracked client view displaying a window of `session_id`
    /// at `window_id` — the shared-selection follow for a tab switch: all
    /// render clients attached to the session display its (new) active
    /// window. Returns the windows whose extent changed under the moved
    /// contributions.
    pub fn follow_session_window(
        &mut self,
        session_id: SessionId,
        window: WindowId,
    ) -> Vec<WindowId> {
        // The windows the moved viewers leave (each re-fits to the minimum
        // of whoever is still attached, if anyone) plus the target.
        let mut candidates = vec![window];
        let ids: Vec<u64> = self
            .client_views
            .iter()
            .filter(|(_, view)| {
                self.window(view.window)
                    .is_some_and(|w| self.session_of_window(w.id) == Some(session_id))
                    && view.window != window
            })
            .map(|(id, view)| {
                candidates.push(view.window);
                *id
            })
            .collect();
        for id in ids {
            if let Some(view) = self.client_views.get_mut(&id) {
                view.window = window;
            }
        }
        self.refit_reported_windows(&candidates)
    }

    /// Point every tracked client view displaying a window of
    /// `workspace_id`'s sessions at `window` — the shared-selection follow
    /// for a workspace switch. Returns the windows whose extent changed.
    pub fn follow_workspace_views(
        &mut self,
        workspace_id: WorkspaceId,
        window: WindowId,
    ) -> Vec<WindowId> {
        let session_ids: Vec<SessionId> = self
            .workspace(workspace_id)
            .map(|ws| ws.sessions.clone())
            .unwrap_or_default();
        let mut resized = Vec::new();
        let ids: Vec<u64> = self
            .client_views
            .iter()
            .filter(|(_, view)| {
                view.window != window
                    && self
                        .session_of_window(view.window)
                        .is_some_and(|s| session_ids.contains(&s))
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            resized.extend(self.set_client_view_keep_size(id, window));
        }
        resized.extend(self.refit_reported_windows(&[window]));
        resized
    }

    /// Move one client's view pointer without re-deriving extents from its
    /// stored report (the follow paths size the target window from ALL
    /// moved contributions at once, not client by client).
    fn set_client_view_keep_size(&mut self, client_id: u64, window: WindowId) -> Vec<WindowId> {
        match self.client_views.get_mut(&client_id) {
            Some(view) => {
                view.window = window;
            }
            None => return Vec::new(),
        }
        Vec::new()
    }

    /// Re-fit every window in `candidates` to the componentwise minimum of
    /// its contributors' reported sizes. A window with no contributors is
    /// left at its current extent (it re-fits when next displayed); a
    /// window whose minimum equals its current extent is left alone — the
    /// idempotence that keeps repeated identical reports and the follow
    /// moves from looping.
    fn refit_reported_windows(&mut self, candidates: &[WindowId]) -> Vec<WindowId> {
        let mut resized = Vec::new();
        for window_id in candidates {
            let mut cols: Option<u16> = None;
            let mut rows: Option<u16> = None;
            for view in self.client_views.values() {
                if view.window == *window_id {
                    cols = Some(cols.map_or(view.cols, |c: u16| c.min(view.cols)));
                    rows = Some(rows.map_or(view.rows, |r: u16| r.min(view.rows)));
                }
            }
            if let (Some(cols), Some(rows)) = (cols, rows) {
                let changed = self
                    .window(*window_id)
                    .map(|win| win.cols != cols || win.rows != rows)
                    .unwrap_or(false);
                if changed {
                    let _ = self.resize_window(*window_id, cols, rows);
                    resized.push(*window_id);
                }
            }
        }
        resized
    }

    /// Record the client's theme colors and apply them to every pane
    /// terminal — the OSC 10/11 answer path. Each half is set only when
    /// reported; panes created later inherit at insert. Never persisted.
    pub fn set_client_colors(&mut self, fg: Option<Color>, bg: Option<Color>) {
        if fg.is_some() {
            self.client_fg = fg;
        }
        if bg.is_some() {
            self.client_bg = bg;
        }
        let fg = self.client_fg;
        let bg = self.client_bg;
        for pane in self.panes.values_mut() {
            pane.with_terminal_mut(|term| {
                if let Some(fg) = fg {
                    term.set_default_fg(fg);
                }
                if let Some(bg) = bg {
                    term.set_default_bg(bg);
                }
            });
        }
    }

    /// Apply the recorded cell pixel size to one just-inserted pane — the
    /// creation paths that build the window around the pane and never
    /// re-fit through [`Self::sync_pane_sizes`]. A no-op until a client
    /// has reported, so fresh daemons spawn with the construction default.
    pub(super) fn apply_cell_pixels(&mut self, pane_id: PaneId, cols: u16, rows: u16) {
        if let Some((cell_w, cell_h)) = self.client_cell_pixels {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                let (resized, batch) =
                    pane.resize_with_cell_pixels_deferred(cols, rows, cell_w, cell_h);
                if let Err(err) = resized {
                    if !pane.dead() {
                        log::warn!(
                            "par-mux: resize of pane {pane_id} to {cols}x{rows} failed: {err}"
                        );
                    }
                }
                self.defer_observer_batch(batch);
            }
        }
        let (fg, bg) = (self.client_fg, self.client_bg);
        if fg.is_some() || bg.is_some() {
            if let Some(pane) = self.panes.get(&pane_id) {
                pane.with_terminal_mut(|term| {
                    if let Some(fg) = fg {
                        term.set_default_fg(fg);
                    }
                    if let Some(bg) = bg {
                        term.set_default_bg(bg);
                    }
                });
            }
        }
    }

    /// The single entry for layout-shape mutations (ARC-090): run `f` on
    /// the window, then end any zoom and re-fit every pane terminal. On
    /// `Err` the window must be untouched, so the zoom and sizes stay as
    /// they were — `f` validates before it edits.
    ///
    /// `f` holds `&mut MuxWindow` borrowed from the tree, so it cannot call
    /// other tree methods; resolve lookups such as `window_of_pane` first.
    ///
    /// The pane → window index is maintained here (ARC-096): panes the
    /// edit added map to this window, and panes it removed are dropped
    /// unless they already map elsewhere (a move re-homes the pane in its
    /// destination first).
    pub(super) fn mutate_layout<R>(
        &mut self,
        window_id: WindowId,
        f: impl FnOnce(&mut MuxWindow) -> Result<R, MuxError>,
    ) -> Result<R, MuxError> {
        let window = self
            .windows
            .get_mut(&window_id)
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        let before = window.panes();
        let out = f(window)?;
        let after = window.panes();
        self.reindex_window_panes(window_id, &before, &after);
        self.end_layout_mutation(window_id);
        #[cfg(test)]
        self.assert_indexes_consistent();
        Ok(out)
    }

    /// Bring the pane → window index in line with one window's pane set
    /// changing from `before` to `after`.
    fn reindex_window_panes(&mut self, window_id: WindowId, before: &[PaneId], after: &[PaneId]) {
        for pane in before {
            if !after.contains(pane) && self.pane_window.get(pane) == Some(&window_id) {
                self.pane_window.remove(pane);
            }
        }
        for pane in after {
            self.pane_window.insert(*pane, window_id);
        }
    }

    /// The post-step of [`Self::mutate_layout`] alone, for a mutation
    /// already applied outside a closure (the kill cascade's removal).
    fn end_layout_mutation(&mut self, window_id: WindowId) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.zoomed = None;
        }
        self.sync_pane_sizes(window_id);
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
        let zoomed = window.zoomed;
        let (window_cols, window_rows) = (window.cols as usize, window.rows as usize);
        let geometry = window
            .layout
            .geometry(0, 0, window.cols as usize, window.rows as usize);
        for pane_geometry in geometry {
            if let Some(pane) = self.panes.get_mut(&pane_geometry.pane) {
                // A reported cell pixel size rides every re-fit, so grid
                // changes keep XTWINOPS/TIOCGWINSZ/image-span math correct
                // instead of reverting to the construction default. A
                // zoomed pane takes the full window grid instead of its
                // layout cell.
                let (width, height) = if zoomed == Some(pane_geometry.pane) {
                    (window_cols, window_rows)
                } else {
                    (pane_geometry.width, pane_geometry.height)
                };
                let (resized, batch) = match self.client_cell_pixels {
                    Some((cell_w, cell_h)) => pane.resize_with_cell_pixels_deferred(
                        width as u16,
                        height as u16,
                        cell_w,
                        cell_h,
                    ),
                    None => pane.resize_deferred(width as u16, height as u16),
                };
                // Best-effort (see above), but visible. A held-dead pane's
                // PTY resize outcome is irrelevant, so it stays quiet.
                if let Err(err) = resized {
                    if !pane.dead() {
                        log::warn!(
                            "par-mux: resize of pane {} to {width}x{height} failed: {err}",
                            pane_geometry.pane
                        );
                    }
                }
                // Delivered after the tree lock drops (see
                // `pending_observer_batches`).
                self.defer_observer_batch(batch);
            }
        }
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
            .session_of_window(window_id)
            .and_then(|id| self.sessions.get_mut(&id))
            .ok_or(MuxError::NoSuchWindow(window_id))?;
        let index = session
            .windows
            .iter()
            .position(|w| *w == window_id)
            .expect("the index names this window's session");
        session.active = index;
        Ok(())
    }
}
