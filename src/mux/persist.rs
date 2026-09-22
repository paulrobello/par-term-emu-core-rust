//! On-disk persistence envelope for the mux tree (par-mux.md Phase 3, D3.2).
//!
//! The artifact is a versioned envelope, not a bare tree: `format_version`
//! lets a future format change migrate old files rather than refuse them,
//! and each window's pane list rides beside its [`LayoutTree`] so restore
//! can spawn every pane's replacement process before re-hanging content
//! (D3.5: spawn first, restore second — startup bytes must not overwrite a
//! restored screen).

use crate::mux::ids::{IdAllocator, PaneId, SessionId, WindowId};
use crate::mux::layout::LayoutTree;
use crate::mux::pane::{MuxError, PaneFactory};
use crate::mux::tree::{MuxSession, MuxTree, MuxWindow};
use crate::terminal::replay_snapshot::TerminalSnapshot;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// The envelope version this build writes, and the only one it accepts.
/// A file carrying any other version is quarantined at the load site and the
/// server starts fresh (D3.2 — Task 3.3 wires that path).
pub const FORMAT_VERSION: u32 = 1;

/// Errors raised while rebuilding a [`MuxTree`] from persisted state.
#[derive(Debug)]
pub enum PersistError {
    /// The state was written under a `format_version` this build does not
    /// know — migrate or refuse, never guess.
    UnsupportedVersion { found: u32, supported: u32 },
    /// Spawning a restored pane's replacement process failed.
    Mux(MuxError),
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistError::UnsupportedVersion { found, supported } => {
                write!(
                    f,
                    "unsupported state format_version {found} (supported: {supported})"
                )
            }
            PersistError::Mux(err) => write!(f, "restore failed: {err}"),
        }
    }
}

impl std::error::Error for PersistError {}

impl From<MuxError> for PersistError {
    fn from(err: MuxError) -> Self {
        PersistError::Mux(err)
    }
}

/// The whole server's persisted state — the file's top level.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistState {
    /// Envelope version; see [`FORMAT_VERSION`].
    pub format_version: u32,
    /// When this state was captured, Unix milliseconds.
    pub saved_at_unix_ms: u64,
    /// The id allocator's counters at capture — `(session, window, pane)`,
    /// the next id each kind hands out.
    pub next_ids: (u32, u32, u32),
    /// Every live session, in id order.
    pub sessions: Vec<PersistSession>,
    /// Named paste buffers (`set-buffer`/`show-buffer`).
    pub buffers: HashMap<String, String>,
}

/// One persisted session.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistSession {
    /// The session's `$N` number.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// Index into `windows` of the active window.
    pub active_window_index: usize,
    /// The session's windows, in order.
    pub windows: Vec<PersistWindow>,
}

/// One persisted window: its layout tree plus the panes that tree references.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistWindow {
    /// The window's `@N` number.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// Window width in columns.
    pub cols: u16,
    /// Window height in rows.
    pub rows: u16,
    /// The `@N`-style number of the active pane — a plain number, like `id`,
    /// so the envelope carries no newtype coupling.
    pub active_pane: u32,
    /// The interior pane structure; its leaf ids reference `panes` below.
    pub layout: LayoutTree,
    /// The window's panes. Carried beside the layout (D3.2) so restore can
    /// spawn each pane's replacement process before re-hanging content.
    pub panes: Vec<PersistPane>,
}

/// One persisted pane: its captured terminal and how to respawn its process.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PersistPane {
    /// The pane's `%N` number.
    pub id: u32,
    /// The pane's full terminal state — screen, scrollback, modes.
    pub terminal: TerminalSnapshot,
    /// The command the pane ran (`None` = the default shell).
    pub spawn_command: Option<String>,
}

impl MuxTree {
    /// Capture the whole tree into its persisted form (D3.2).
    ///
    /// Sessions serialize in id order so two captures of an unchanged tree
    /// differ only in their timestamps.
    pub fn to_persist_state(&self) -> PersistState {
        let mut sessions: Vec<PersistSession> = self
            .sessions
            .values()
            .map(|session| PersistSession {
                id: session.id.0,
                name: session.name.clone(),
                active_window_index: session.active,
                windows: session
                    .windows
                    .iter()
                    .map(|window_id| self.to_persist_window(*window_id))
                    .collect(),
            })
            .collect();
        sessions.sort_by_key(|session| session.id);

        PersistState {
            format_version: FORMAT_VERSION,
            saved_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            next_ids: self.ids.next_ids(),
            sessions,
            buffers: self.buffers.clone(),
        }
    }

    /// Capture one window and its panes; panes serialize in layout order.
    fn to_persist_window(&self, window_id: WindowId) -> PersistWindow {
        let window = self
            .windows
            .get(&window_id)
            .expect("session window lists only hold live windows");
        let panes = window
            .panes()
            .into_iter()
            .map(|pane_id| {
                let pane = self
                    .panes
                    .get(&pane_id)
                    .expect("layout leaf ids are always live panes");
                PersistPane {
                    id: pane_id.0,
                    terminal: pane.terminal().read().capture_snapshot(),
                    spawn_command: pane.spawn_command().map(str::to_string),
                }
            })
            .collect();
        PersistWindow {
            id: window.id.0,
            name: window.name.clone(),
            cols: window.cols,
            rows: window.rows,
            active_pane: window.active.0,
            layout: window.layout.clone(),
            panes,
        }
    }

    /// Rebuild a tree from persisted state (D3.5): spawn each pane's
    /// replacement process first, then restore its terminal from the
    /// snapshot, so process startup bytes never overwrite restored content.
    ///
    /// The id allocator resumes from the persisted counters, so restored
    /// panes keep their `%N` identities and new panes do not collide.
    pub fn from_persist_state(
        state: &PersistState,
        factory: Box<dyn PaneFactory>,
    ) -> Result<MuxTree, PersistError> {
        if state.format_version != FORMAT_VERSION {
            return Err(PersistError::UnsupportedVersion {
                found: state.format_version,
                supported: FORMAT_VERSION,
            });
        }

        let mut panes = HashMap::new();
        let mut windows = HashMap::new();
        let mut sessions = HashMap::new();

        for session in &state.sessions {
            let mut window_ids = Vec::with_capacity(session.windows.len());
            for window in &session.windows {
                for pane in &window.panes {
                    let created = factory.create_pane(
                        PaneId(pane.id),
                        window.cols,
                        window.rows,
                        pane.spawn_command.as_deref(),
                    )?;
                    panes.insert(PaneId(pane.id), created);
                }
                for pane in &window.panes {
                    panes
                        .get_mut(&PaneId(pane.id))
                        .expect("just inserted above")
                        .terminal()
                        .write()
                        .restore_from_snapshot(pane.terminal.clone());
                }
                window_ids.push(WindowId(window.id));
                windows.insert(
                    WindowId(window.id),
                    MuxWindow {
                        id: WindowId(window.id),
                        name: window.name.clone(),
                        layout: window.layout.clone(),
                        active: PaneId(window.active_pane),
                        cols: window.cols,
                        rows: window.rows,
                    },
                );
            }
            sessions.insert(
                SessionId(session.id),
                MuxSession {
                    id: SessionId(session.id),
                    name: session.name.clone(),
                    windows: window_ids,
                    active: session.active_window_index,
                },
            );
        }

        let mut tree = MuxTree::new(factory);
        tree.ids = IdAllocator::resume(state.next_ids);
        tree.sessions = sessions;
        tree.windows = windows;
        tree.panes = panes;
        tree.buffers = state.buffers.clone();
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::layout::SplitDirection;
    use crate::mux::pane::ShellPaneFactory;

    fn tree() -> MuxTree {
        MuxTree::new(Box::new(ShellPaneFactory::default()))
    }

    /// Two sessions: the first with a split window plus a second window, the
    /// second plain — enough shape that a round trip has something to lose.
    pub(super) fn populated_tree() -> MuxTree {
        let mut tree = tree();
        let main = tree.new_session("main", 80, 24).unwrap();
        let main_window = tree.session(main).unwrap().windows[0];
        let first = tree.window(main_window).unwrap().panes()[0];
        let second = tree
            .split_pane(first, SplitDirection::Vertical, 0.25, None)
            .unwrap();
        tree.select_pane(second).unwrap();
        let logs = tree.new_window(main, "logs", 100, 30).unwrap();
        tree.select_window(logs).unwrap();
        tree.set_buffer("default", "hello".to_string());
        tree.new_session("other", 120, 40).unwrap();
        tree
    }

    /// Assert every structural fact a round trip must preserve.
    pub(super) fn assert_same_shape(restored: &MuxTree, original: &MuxTree) {
        let mut original_sessions = original.sessions();
        original_sessions.sort();
        let mut restored_sessions = restored.sessions();
        restored_sessions.sort();
        assert_eq!(restored_sessions, original_sessions, "same session ids");

        for id in &original_sessions {
            let a = original.session(*id).unwrap();
            let b = restored.session(*id).unwrap();
            assert_eq!(b.name, a.name, "session {id} name");
            assert_eq!(b.windows, a.windows, "session {id} window ids, in order");
            assert_eq!(b.active, a.active, "session {id} active window index");

            for window_id in &a.windows {
                let wa = original.window(*window_id).unwrap();
                let wb = restored.window(*window_id).unwrap();
                assert_eq!(wb.name, wa.name, "window {window_id} name");
                assert_eq!((wb.cols, wb.rows), (wa.cols, wa.rows));
                assert_eq!(wb.active, wa.active, "window {window_id} active pane");
                assert_eq!(
                    wb.layout, wa.layout,
                    "window {window_id} layout shape (splits, ratios, leaf order)"
                );
                for pane_id in wa.panes() {
                    assert!(restored.pane(pane_id).is_some(), "pane {pane_id} restored");
                    assert_eq!(
                        restored.pane(pane_id).unwrap().spawn_command(),
                        original.pane(pane_id).unwrap().spawn_command(),
                        "pane {pane_id} spawn command"
                    );
                }
            }
        }
    }

    #[test]
    fn round_trip_preserves_sessions_windows_layout_and_active_panes() {
        let original = populated_tree();
        let state = original.to_persist_state();

        assert_eq!(state.format_version, FORMAT_VERSION);
        assert_eq!(state.sessions.len(), 2);
        assert_eq!(
            state.sessions.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![0, 1],
            "sessions serialize in id order"
        );

        let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
            .expect("state this build wrote must restore");
        assert_same_shape(&restored, &original);
        assert_eq!(
            restored.get_buffer("default").map(str::to_string),
            Some("hello".to_string()),
            "named buffers survive the round trip"
        );
    }

    #[test]
    fn restored_allocator_resumes_without_colliding() {
        let original = populated_tree();
        let state = original.to_persist_state();

        let mut restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        assert_eq!(
            restored.to_persist_state().next_ids,
            state.next_ids,
            "the allocator's counters travel with the state"
        );

        let fresh = restored.new_session("fresh", 80, 24).unwrap();
        assert_eq!(
            fresh,
            SessionId(state.next_ids.0),
            "a restored server's first new session must take the persisted next id"
        );
    }

    #[test]
    fn unknown_format_version_is_refused() {
        let original = populated_tree();
        let mut state = original.to_persist_state();
        state.format_version = FORMAT_VERSION + 1;

        let result = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()));
        assert!(
            matches!(
                result,
                Err(PersistError::UnsupportedVersion { found, supported })
                    if found == FORMAT_VERSION + 1 && supported == FORMAT_VERSION
            ),
            "an unknown envelope version must refuse, not guess"
        );
    }

    /// Screen content and scrollback survive the round trip. The content
    /// pane runs `sleep` so its process emits nothing — the only bytes in
    /// its terminal are the ones the test fed directly, making the
    /// before/after comparison deterministic.
    #[cfg(unix)]
    #[test]
    fn round_trip_preserves_screen_content_and_scrollback() {
        let mut tree = tree();
        let session = tree.new_session("main", 80, 24).unwrap();
        let window = tree.session(session).unwrap().windows[0];
        let first = tree.window(window).unwrap().panes()[0];
        let quiet = tree
            .split_pane(first, SplitDirection::Vertical, 0.5, Some("sleep 60"))
            .unwrap();

        let terminal = tree.pane(quiet).unwrap().terminal();
        {
            let mut guard = terminal.write();
            for i in 0..40 {
                guard.process(format!("scroll line {i:02}\r\n").as_bytes());
            }
            guard.process(b"ZQX-VISIBLE-TAIL");
        }
        let before = terminal.read().capture_snapshot();
        assert!(
            before.grid.scrollback_lines > 0,
            "test setup must push real content into scrollback"
        );

        let state = tree.to_persist_state();
        let restored =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();

        let after = restored
            .pane(quiet)
            .unwrap()
            .terminal()
            .read()
            .capture_snapshot();
        assert_eq!(
            after.grid.scrollback_lines, before.grid.scrollback_lines,
            "scrollback line count survives"
        );
        assert_eq!(
            after.grid.total_lines_scrolled, before.grid.total_lines_scrolled,
            "scroll history survives"
        );
        let scrollback_text: String = after.grid.scrollback_cells.iter().map(|c| c.c).collect();
        assert!(
            scrollback_text.contains("scroll line 05"),
            "scrollback content survives the round trip"
        );
        let screen_text: String = after.grid.cells.iter().map(|c| c.c).collect();
        assert!(
            screen_text.contains("ZQX-VISIBLE-TAIL"),
            "visible content survives the round trip"
        );

        let pane = restored.pane(quiet).unwrap();
        assert!(
            pane.is_running(),
            "the restored pane's process is new, but running"
        );
        assert_eq!(pane.spawn_command(), Some("sleep 60"));
    }
}

/// The envelope must survive serialize → deserialize whole — LayoutTree and
/// the id newtypes serialize inside it (D3.2), which no other test exercises.
#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::tests::{assert_same_shape, populated_tree};
    use super::*;
    use crate::mux::pane::ShellPaneFactory;

    #[test]
    fn envelope_survives_serde_round_trip() {
        let original = populated_tree();
        let state = original.to_persist_state();

        let json = serde_json::to_string(&state).expect("envelope serializes");
        let revived: PersistState = serde_json::from_str(&json).expect("envelope deserializes");
        assert_eq!(revived.format_version, FORMAT_VERSION);
        assert_eq!(revived.next_ids, state.next_ids);

        // Restoring through the revived envelope must land in the same
        // state as restoring through the original, byte for byte in shape.
        let direct =
            MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
        let via_json = MuxTree::from_persist_state(&revived, Box::new(ShellPaneFactory::default()))
            .expect("revived envelope restores");
        assert_same_shape(&via_json, &direct);
        assert_eq!(
            via_json.get_buffer("default").map(str::to_string),
            Some("hello".to_string())
        );
    }
}
