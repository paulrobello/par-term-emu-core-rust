use super::format::{
    clamp_restored_grid_dims, MAX_RESTORED_COLS, MAX_RESTORED_ROWS, MIN_RESTORED_COLS,
    MIN_RESTORED_ROWS,
};
use super::restore::agent_of_invocation;
use super::save::{
    kept_snapshots, lastgood_path, maintain_snapshot_history_in, state_has_panes, SNAPSHOT_LIMIT,
};
use super::*;
use crate::mux::agent_resume::SURVIVING_TAIL;
use crate::mux::ids::{PaneId, SessionId, WindowId};
use crate::mux::layout::SplitDirection;
use crate::mux::pane::{MuxError, SpawnContext};
use crate::mux::pane::{MuxPane, PaneFactory, ShellPaneFactory};
use crate::mux::tree::MuxTree;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// A per-test target path inside a fresh `TempDir`. The directory name
/// carries OS-provided randomness, so neither parallel tests nor a later
/// run can share it (a `process::id()`-derived name repeats once the OS
/// recycles the pid, and a stale state file would then be loaded). The
/// returned guard removes the directory — target, `.tmp` sibling, and any
/// quarantined copy — on drop, even when the test panics; bind it to a
/// name, since `let _` would drop it at once.
fn temp_target(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("par-mux-persist-{name}-"))
        .tempdir()
        .expect("create persist temp dir");
    let target = dir.path().join("state.json");
    (dir, target)
}

/// The tmp sibling `save_to` must never leave behind.
fn tmp_sibling(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

/// Criterion 2 (card 01a17f4438): a kill -9 mid-save leaves a partial
/// `<target>.tmp` beside a complete target. The loader reads only the
/// target, and the next save replaces the tmp wholesale — the state
/// file is never the torn half.
#[test]
fn leftover_tmp_never_defeats_the_state() {
    let (_dir, target) = temp_target("tmp-orphan");
    save_to(&tree(), &target).expect("the first save");
    let tmp = tmp_sibling(&target);
    fs::write(&tmp, b"{\"format_version\":").expect("plant a partial tmp");

    assert!(
        matches!(load_or_quarantine(&target), Loaded::State(_)),
        "the loader reads the complete target, not the tmp"
    );

    save_to(&tree(), &target).expect("the next save succeeds");
    assert!(
        !tmp.exists(),
        "the rename consumed the leftover tmp; the state file was never torn"
    );
}

/// Criterion 1 (card 01a17f4438): a second daemon's claim on an
/// occupied state file refuses, naming the holder's pid from the lock
/// file; dropping the holder releases the claim.
#[test]
#[cfg(unix)]
fn state_lock_refuses_a_second_holder_until_released() {
    let (_dir, target) = temp_target("state-lock");
    let first = acquire_state_lock(&target).expect("the first claim");
    let second = acquire_state_lock(&target);
    let err = second.expect_err("a second claim refuses");
    assert!(
        err.to_string().contains("already served by another daemon"),
        "the refusal names the situation: {err}"
    );
    assert!(
        err.to_string().contains(&std::process::id().to_string()),
        "the refusal names the holder's pid: {err}"
    );
    drop(first);
    acquire_state_lock(&target).expect("the claim releases on drop");
}

pub(super) fn tree() -> MuxTree {
    MuxTree::new(Box::new(ShellPaneFactory::default()))
}

#[test]
fn layout_name_validation() {
    for ok in ["dev", "work-1", "a.b_c"] {
        assert!(valid_layout_name(ok), "{ok}");
    }
    for bad in ["", ".", "..", "a/b", "../x", "a b", "üñî"] {
        assert!(!valid_layout_name(bad), "{bad:?}");
    }
}

#[test]
fn layout_save_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let tree = populated_tree();
    let path = save_layout_in(dir.path(), &tree, "dev").unwrap();
    assert_eq!(path, layouts_dir_in(dir.path()).join("dev.json"));
    let loaded = load_layout_in(dir.path(), "dev").unwrap();
    // Live panes keep producing output, so a second capture of `tree`
    // is not comparable; the file's own JSON is the captured state.
    let on_disk: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(&loaded).unwrap(), on_disk);
    assert_eq!(loaded.format_version, FORMAT_VERSION);
    let names: Vec<&str> = loaded.sessions.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"main") && names.contains(&"other"));
    assert!(matches!(
        load_layout_in(dir.path(), "nope"),
        Err(PersistError::Io(_))
    ));
}

#[test]
fn layout_save_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let tree = populated_tree();
    save_layout_in(dir.path(), &tree, "dev").unwrap();
    save_layout_in(dir.path(), &tree, "dev").unwrap();
    assert_eq!(list_layouts_in(dir.path()), vec!["dev".to_string()]);
}

#[test]
fn layout_list_and_kill() {
    let dir = tempfile::tempdir().unwrap();
    assert!(list_layouts_in(dir.path()).is_empty());
    let tree = populated_tree();
    save_layout_in(dir.path(), &tree, "zed").unwrap();
    save_layout_in(dir.path(), &tree, "alpha").unwrap();
    assert_eq!(list_layouts_in(dir.path()), ["alpha", "zed"]);
    kill_layout_in(dir.path(), "alpha").unwrap();
    assert_eq!(list_layouts_in(dir.path()), ["zed"]);
    assert!(kill_layout_in(dir.path(), "alpha").is_err());
}

#[test]
fn layout_load_rejects_corrupt_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = layout_file_in(dir.path(), "bad").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{ not json").unwrap();
    assert!(matches!(
        load_layout_in(dir.path(), "bad"),
        Err(PersistError::Serialize(_))
    ));
    assert!(path.exists(), "a bad layout file is left in place");
}

#[test]
fn layout_save_state_writes_captured_state() {
    let dir = tempfile::tempdir().unwrap();
    let state = populated_tree().to_persist_state();
    save_layout_state_in(dir.path(), &state, "dev").unwrap();
    let loaded = load_layout_in(dir.path(), "dev").unwrap();
    assert_eq!(loaded.saved_at_unix_ms, state.saved_at_unix_ms);
    assert!(save_layout_state_in(dir.path(), &state, "../x").is_err());
}

#[test]
fn layout_rejects_bad_name_without_io() {
    let dir = tempfile::tempdir().unwrap();
    let err = save_layout_in(dir.path(), &populated_tree(), "../x").unwrap_err();
    assert!(matches!(err, PersistError::InvalidLayoutName { .. }));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// Commands restores handed the factory, in spawn order.
type RecordedCommands = Vec<(PaneId, Option<String>)>;

/// The `PAR_MUX_RESUME_SESSION_ID` values restores handed the factory,
/// in spawn order.
type RecordedResumeIds = Vec<(PaneId, Option<String>)>;

/// Records the command each restore computed, delegating the actual
/// spawn to a bounded sleeper: restoring an agent pane must not launch a
/// real agent CLI from a unit test, and the recorded value is the
/// assertion target — exactly what `from_persist_state` handed the
/// factory.
#[derive(Clone, Default)]
struct RecordingFactory {
    received: std::sync::Arc<std::sync::Mutex<RecordedCommands>>,
    /// The size each pane was created at, in spawn order — the grid
    /// allocation input (ARC-113b).
    sizes: std::sync::Arc<std::sync::Mutex<Vec<(PaneId, u16, u16)>>>,
    /// The `PAR_MUX_RESUME_SESSION_ID` each restore handed the factory,
    /// in spawn order — the start-command wrapper's resume hook.
    resume_ids: std::sync::Arc<std::sync::Mutex<RecordedResumeIds>>,
}

impl RecordingFactory {
    fn command_for(&self, id: PaneId) -> Option<String> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .find(|(pane, _)| *pane == id)
            .and_then(|(_, command)| command.clone())
    }

    /// The `PAR_MUX_RESUME_SESSION_ID` pane `id` was spawned with.
    fn resume_session_id_for(&self, id: PaneId) -> Option<String> {
        self.resume_ids
            .lock()
            .unwrap()
            .iter()
            .find(|(pane, _)| *pane == id)
            .and_then(|(_, value)| value.clone())
    }

    /// The size pane `id` was created at — what its grid was allocated
    /// with.
    fn size_for(&self, id: PaneId) -> (u16, u16) {
        self.sizes
            .lock()
            .unwrap()
            .iter()
            .find(|(pane, _, _)| *pane == id)
            .map(|(_, cols, rows)| (*cols, *rows))
            .expect("recorded pane")
    }
}

impl PaneFactory for RecordingFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        self.received
            .lock()
            .unwrap()
            .push((id, command.map(str::to_string)));
        self.sizes.lock().unwrap().push((id, cols, rows));
        self.resume_ids.lock().unwrap().push((
            id,
            context
                .env
                .and_then(|env| env.get("PAR_MUX_RESUME_SESSION_ID").cloned()),
        ));
        ShellPaneFactory::default().create_pane(id, cols, rows, Some("sleep 60"), context)
    }

    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<MuxPane, MuxError> {
        ShellPaneFactory::default().create_dead_pane(id, cols, rows, command, exit_code)
    }
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

/// ARC-032: the two-phase capture (collect under the lock, capture
/// after) produces the same save as the one-shot form, modulo the
/// timestamp.
///
/// Parity is a claim about a QUIESCENT tree: a pane whose PTY produced
/// output between the two captures legitimately differs, so each attempt
/// requires every pane's generation to hold still across both captures
/// and retries while startup output is still landing.
#[test]
fn two_phase_capture_matches_one_shot() {
    let tree = populated_tree();
    let generations = |tree: &MuxTree| {
        tree.sessions()
            .iter()
            .flat_map(|s| tree.session(*s).unwrap().windows.clone())
            .flat_map(|w| tree.window(w).unwrap().panes())
            .map(|p| tree.pane(p).unwrap().update_generation())
            .collect::<Vec<_>>()
    };
    for _ in 0..50 {
        let before = generations(&tree);
        let one_shot = serde_json::to_value(tree.to_persist_state()).unwrap();
        let two_phase = serde_json::to_value(tree.collect_persist_capture().capture()).unwrap();
        if generations(&tree) == before {
            let strip = |mut value: serde_json::Value| {
                value.as_object_mut().unwrap().remove("saved_at_unix_ms");
                value
            };
            assert_eq!(strip(one_shot), strip(two_phase));
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("panes never quiesced within 5 s; parity not testable this run");
}

/// ARC-032: the capture owns no tree borrow — it completes after the
/// tree is gone, the lifetime-level guarantee that the expensive half
/// (grid walks, cwd syscalls) cannot be running under the tree lock.
#[test]
fn capture_outlives_the_tree() {
    let capture = {
        let tree = populated_tree();
        tree.collect_persist_capture()
    };
    let state = capture.capture();
    assert!(!state.sessions.is_empty(), "the captured tree had sessions");
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
fn session_env_round_trips_and_restored_panes_spawn_with_it() {
    let mut original = populated_tree();
    let main = original
        .sessions()
        .into_iter()
        .find(|id| original.session(*id).unwrap().name == "main")
        .unwrap();
    original
        .set_session_env(main, "TOKEN", Some("s3cret value"))
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("env.state.json");
    save_to(&original, &target).unwrap();
    let Loaded::State(state) = load_or_quarantine(&target) else {
        panic!("the saved file must load back");
    };
    let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    let env = &restored.session(main).unwrap().env;
    assert_eq!(env.get("TOKEN").map(String::as_str), Some("s3cret value"));
    for window in &restored.session(main).unwrap().windows {
        for pane in restored.window(*window).unwrap().panes() {
            assert_eq!(&factory.spawn_of(pane).env, env);
        }
    }
}

#[test]
fn a_state_file_without_session_env_loads_with_an_empty_one() {
    let mut json = serde_json::to_value(populated_tree().to_persist_state()).unwrap();
    for session in json["sessions"].as_array_mut().unwrap() {
        session.as_object_mut().unwrap().remove("env");
    }
    let state: PersistState = serde_json::from_value(json).unwrap();
    let restored =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
    for id in restored.sessions() {
        assert!(restored.session(id).unwrap().env.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn a_state_file_is_owner_only_even_over_a_stale_world_readable_tmp() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("s.state.json");
    let tmp = dir.path().join("s.state.json.tmp");
    fs::write(&tmp, b"stale").unwrap();
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)).unwrap();
    save_to(&populated_tree(), &target).unwrap();
    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn restore_spawns_each_pane_with_its_session_and_window_identity() {
    let original = populated_tree();
    let state = original.to_persist_state();
    let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone()))
        .expect("state this build wrote must restore");
    for session_id in restored.sessions() {
        let session = restored.session(session_id).unwrap();
        for window_id in &session.windows {
            for pane_id in restored.window(*window_id).unwrap().panes() {
                let spawn = factory.spawn_of(pane_id);
                assert_eq!(spawn.session, Some((session_id, session.name.clone())));
                assert_eq!(spawn.window, Some(*window_id));
            }
        }
    }
}

/// Criterion 2 (card 01a17f7511): a restore re-lands each pane in its
/// persisted cwd — the directory Paul's agents lost to $HOME after
/// the 2026-10-27 crash-loop restores. The pane's child lives in the
/// workdir and no OSC 7 exists (the agent-pane shape), so the capture
/// reads the cwd off the child.
#[test]
fn restore_spawns_each_pane_in_its_persisted_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("workdir");
    fs::create_dir_all(&workdir).unwrap();
    let mut original = populated_tree();
    let main = original
        .sessions()
        .into_iter()
        .find(|id| original.session(*id).unwrap().name == "main")
        .unwrap();
    let window = original
        .new_window_with_cwd(main, "cwdtest", 80, 24, Some(&workdir))
        .unwrap();
    let pane = original.window(window).unwrap().panes()[0];
    let persisted = original
        .pane(pane)
        .unwrap()
        .persistence_cwd()
        .expect("the capture reads the cwd off the child");
    // The kernel reports macOS's /private prefix; compare canonically.
    assert_eq!(
        persisted.canonicalize().unwrap(),
        workdir.canonicalize().unwrap(),
        "the save holds the spawn cwd"
    );

    let state = original.to_persist_state();
    let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
    let _restored = MuxTree::from_persist_state(&state, Box::new(factory.clone()))
        .expect("state this build wrote must restore");
    let spawn = factory.spawn_of(pane);
    assert_eq!(
        spawn.cwd.map(|cwd| cwd.canonicalize().unwrap()),
        Some(workdir.canonicalize().unwrap()),
        "the restore hands the factory the persisted cwd"
    );
}

/// A pane saved DEAD keeps its cwd: the restore seeds it into the
/// pane's terminal OSC 7 state, so the agent-append resume and any
/// respawn-pane re-land where the pane lived — the seam the churn's
/// daemon deaths fell through when they saved Paul's agents dead.
#[test]
fn a_dead_panes_cwd_is_seeded_for_its_resume() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().join("workdir");
    fs::create_dir_all(&workdir).unwrap();
    let mut original = populated_tree();
    let main = original
        .sessions()
        .into_iter()
        .find(|id| original.session(*id).unwrap().name == "main")
        .unwrap();
    let window = original
        .new_window_with_cwd(main, "cwdtest", 80, 24, Some(&workdir))
        .unwrap();
    let pane = original.window(window).unwrap().panes()[0];
    let mut json = serde_json::to_value(original.to_persist_state()).unwrap();
    for session in json["sessions"].as_array_mut().unwrap() {
        for window_json in session["windows"].as_array_mut().unwrap() {
            for pane_json in window_json["panes"].as_array_mut().unwrap() {
                if pane_json["id"] == pane.0 {
                    pane_json["dead"] = serde_json::Value::Bool(true);
                }
            }
        }
    }
    let state: PersistState = serde_json::from_value(json).unwrap();
    // The dead pane survives the restore only under remain-on-exit
    // (Paul's config shape — the churn's daemon deaths saved his agent
    // panes dead, and the agent-append resume respawns them).
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(crate::mux::pane::ShellPaneFactory::default()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .expect("the state restores");
    let restored_pane = restored.pane(pane).unwrap();
    let seeded = restored_pane.with_terminal_mut(|term| {
        term.current_directory()
            .map(std::path::PathBuf::from)
            .map(|cwd| cwd.canonicalize().unwrap())
    });
    assert_eq!(
        seeded.as_deref(),
        Some(workdir.canonicalize().unwrap().as_path()),
        "the dead pane's terminal carries the persisted cwd for its resume"
    );
}

/// SEC-209: names in a (hand-edited or older) save file are assigned
/// directly on restore, so restore strips them too.
#[test]
fn restore_strips_control_characters_from_tree_names() {
    let mut tree = tree();
    let workspace = tree.new_workspace("ws");
    let session = tree.new_session("s", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let mut state = tree.to_persist_state();
    for ws in &mut state.workspaces {
        ws.name = "w\x1bs".to_string();
    }
    for s in &mut state.sessions {
        s.name = "s\x07n".to_string();
        for w in &mut s.windows {
            w.name = "w\u{9b}n".to_string();
        }
    }
    let restored =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
    assert_eq!(restored.workspace(workspace).unwrap().name, "ws");
    assert_eq!(restored.session(session).unwrap().name, "sn");
    assert_eq!(restored.window(window).unwrap().name, "wn");
}

#[test]
fn round_trip_preserves_user_titles() {
    let mut tree = tree();
    let session = tree.new_session("titled", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane_id = tree.window(window).unwrap().panes()[0];
    tree.pane_mut(pane_id)
        .expect("the session's pane exists")
        .set_user_title("keep me");

    let state = tree.to_persist_state();
    let restored =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
    assert_eq!(
        restored.pane(pane_id).and_then(|p| p.user_title()),
        Some("keep me"),
        "the user title survives the round trip"
    );
}

#[test]
fn a_save_file_without_the_title_field_still_loads() {
    // serde(default): a save file written before user titles existed
    // carries no `user_title` key and must decode with no title rather
    // than fail — the strip below simulates exactly that older file.
    let mut tree = tree();
    let session = tree.new_session("old", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane_id = tree.window(window).unwrap().panes()[0];
    tree.pane_mut(pane_id)
        .unwrap()
        .set_user_title("dropped by the strip");

    let json = serde_json::to_string(&tree.to_persist_state()).unwrap();
    assert!(
        json.contains("user_title"),
        "positive control: the field is on the wire"
    );
    let old_format = json.replace(",\"user_title\":\"dropped by the strip\"", "");
    assert!(
        !old_format.contains("user_title"),
        "the strip removed the only occurrence"
    );

    let state: PersistState =
        serde_json::from_str(&old_format).expect("a pre-title save file decodes");
    let restored =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
    assert_eq!(
        restored.pane(pane_id).and_then(|p| p.user_title()),
        None,
        "a missing field decodes as no title, not an error"
    );
}

/// A pane's last reported cwd (OSC 7 first, the process's live cwd
/// second) is captured on save and handed back to the factory on
/// restore, so a resumed pane — shell or agent — lands where it left
/// off instead of in the daemon's start directory. Unix only: the
/// OSC 7 URL is built from a `/`-rooted tempdir path, and the fallback
/// probe below needs a pid→cwd source Windows does not offer.
#[test]
#[cfg(unix)]
fn osc7_cwd_is_captured_and_restored_panes_spawn_in_it() {
    let mut tree = tree();
    let session = tree.new_session("cwd", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane_id = tree.window(window).unwrap().panes()[0];
    let dir = tempfile::tempdir().unwrap();
    let reported = dir.path().join("project");
    std::fs::create_dir(&reported).unwrap();
    // The shell-integration report a pane with OSC 7 hooks emits on
    // every prompt — the primary cwd source.
    tree.pane(pane_id)
        .unwrap()
        .terminal()
        .write()
        .process(format!("\x1b]7;file://localhost{}\x1b\\", reported.display()).as_bytes());

    let state = tree.to_persist_state();
    assert_eq!(
        state.sessions[0].windows[0].panes[0].cwd.as_deref(),
        Some(reported.to_str().unwrap()),
        "positive control: the OSC 7 cwd is on the wire"
    );

    let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    assert_eq!(
        factory.spawn_of(pane_id).cwd.as_deref(),
        Some(reported.as_path()),
        "the restored spawn carries the persisted cwd"
    );
    assert!(restored.pane(pane_id).is_some());
}

/// Without OSC 7 the pane's live process cwd is captured instead — a
/// plain shell that never reported anything still restores where the
/// user had `cd`-ed, because the child's own cwd is readable. Unix
/// only: `process_cwd` has no Windows implementation (no documented
/// pid→cwd API), so there is nothing to assert there.
#[test]
#[cfg(unix)]
fn a_pane_without_osc7_persists_its_process_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let factory = ShellPaneFactory {
        cwd: Some(dir.path().to_path_buf()),
        ..ShellPaneFactory::default()
    };
    let pane = factory
        .create_pane(
            PaneId(1),
            80,
            24,
            Some("sleep 60"),
            &SpawnContext::default(),
        )
        .unwrap();
    // The kernel resolves symlinks in the vnode path (`/var` →
    // `/private/var` on macOS), so compare canonical-to-canonical.
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    assert_eq!(
        pane.persistence_cwd().as_deref(),
        Some(canonical.as_path()),
        "no OSC 7 report falls back to the process's live cwd"
    );
}

/// OSC 7 wins when both sources exist: the shell's report is the
/// pane's logical cwd even if the child process has moved. Unix only,
/// for the same reason as the process-cwd test above (the report URL
/// is `/`-rooted; the losing fallback is unix's pid→cwd probe).
#[test]
#[cfg(unix)]
fn osc7_wins_over_the_process_cwd() {
    let spawn_dir = tempfile::tempdir().unwrap();
    let factory = ShellPaneFactory {
        cwd: Some(spawn_dir.path().to_path_buf()),
        ..ShellPaneFactory::default()
    };
    let pane = factory
        .create_pane(
            PaneId(1),
            80,
            24,
            Some("sleep 60"),
            &SpawnContext::default(),
        )
        .unwrap();
    let reported = tempfile::tempdir().unwrap();
    pane.terminal()
        .write()
        .process(format!("\x1b]7;file://localhost{}\x1b\\", reported.path().display()).as_bytes());
    assert_eq!(
        pane.persistence_cwd().as_deref(),
        Some(reported.path()),
        "the OSC 7 report outranks the process cwd"
    );
}

/// A persisted cwd whose directory no longer exists must not fail the
/// restore: the pane spawns in home and the pane itself says so.
#[test]
fn a_gone_persisted_cwd_falls_back_to_home_with_a_visible_message() {
    let mut tree = tree();
    let session = tree.new_session("gone", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane_id = tree.window(window).unwrap().panes()[0];
    let mut state = tree.to_persist_state();
    state.sessions[0].windows[0].panes[0].cwd = Some("/par-mux-test-no-such-dir".to_string());

    let mut restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
        .expect("a gone cwd degrades, never fails the restore");
    assert!(
        restored.pane_mut(pane_id).unwrap().poll_running(),
        "the pane is alive in the fallback cwd"
    );
    let snapshot = restored
        .pane(pane_id)
        .unwrap()
        .terminal()
        .read()
        .capture_snapshot();
    let text: String = snapshot.grid.cells.iter().map(|c| c.c).collect();
    assert!(
        text.contains("par-mux: /par-mux-test-no-such-dir is gone"),
        "the fallback announced itself in the pane; screen held: {text}"
    );
    // QA-195 consistency check: the wait-free cursor matches the
    // terminal's after a restore. Not isolating — the re-fit that ends
    // `from_persist_state` republishes too. Both are read under the
    // terminal's read lock, since every publish holds its write lock.
    let pane = restored.pane(pane_id).unwrap();
    let (published, cursor) = {
        let terminal = pane.terminal();
        let term = terminal.read();
        (
            pane.published_cursor(),
            (term.cursor().col, term.cursor().row),
        )
    };
    assert_eq!(
        published, cursor,
        "the geometry mirror follows the restored screen"
    );
}

/// Records whether a zone eviction reached the observer and whether the
/// pane's terminal lock was free while the callback ran (a delivery
/// inside `with_terminal_mut` fails `try_read` on the delivering thread).
#[cfg(unix)]
struct TerminalLockProbe {
    terminal: std::sync::Weak<parking_lot::RwLock<par_term_emu_core::terminal::Terminal>>,
    scrolled_out: std::sync::atomic::AtomicBool,
    terminal_lock_was_free: std::sync::atomic::AtomicBool,
}

#[cfg(unix)]
impl par_term_emu_core::terminal::observer::TerminalObserver for TerminalLockProbe {
    fn on_zone_event(&self, event: &par_term_emu_core::terminal::TerminalEvent) {
        use std::sync::atomic::Ordering;
        if matches!(
            event,
            par_term_emu_core::terminal::TerminalEvent::ZoneScrolledOut { .. }
        ) {
            let free = self
                .terminal
                .upgrade()
                .is_some_and(|terminal| terminal.try_read().is_some());
            self.terminal_lock_was_free.store(free, Ordering::SeqCst);
            self.scrolled_out.store(true, Ordering::SeqCst);
        }
    }
}

/// Spawns quiet `sleep 60` panes, each carrying a [`TerminalLockProbe`]
/// attached before restore touches its terminal.
#[cfg(unix)]
#[derive(Default)]
struct RestoreProbeFactory {
    probes: std::sync::Arc<std::sync::Mutex<Vec<std::sync::Arc<TerminalLockProbe>>>>,
}

#[cfg(unix)]
impl PaneFactory for RestoreProbeFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        _command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        let pane =
            ShellPaneFactory::default().create_pane(id, cols, rows, Some("sleep 60"), context)?;
        let terminal = pane.terminal();
        let probe = std::sync::Arc::new(TerminalLockProbe {
            terminal: std::sync::Arc::downgrade(&terminal),
            scrolled_out: std::sync::atomic::AtomicBool::new(false),
            terminal_lock_was_free: std::sync::atomic::AtomicBool::new(false),
        });
        terminal.write().add_observer(probe.clone());
        self.probes.lock().unwrap().push(probe);
        Ok(pane)
    }

    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<MuxPane, MuxError> {
        ShellPaneFactory::default().create_dead_pane(id, cols, rows, command, exit_code)
    }
}

/// Card 01a119fc: the gone-cwd note restore writes into a restored pane
/// raises observer events (here a zone the note scrolls out of a
/// scrollback parked at its cap). They must not be lost, and they are
/// delivered after the pane's terminal write lock drops — not inline
/// inside `with_terminal_mut`.
#[cfg(unix)]
#[test]
fn restore_note_observer_events_arrive_after_the_terminal_lock_drops() {
    use std::sync::atomic::Ordering;
    let mut tree = tree();
    let session = tree.new_session("note", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane_id = tree.window(window).unwrap().panes()[0];
    let mut state = tree.to_persist_state();
    drop(tree);

    // A screen holding one completed command zone at the top of a
    // scrollback filled exactly to its cap, cursor on the bottom row:
    // the first line the note scrolls evicts that zone.
    let (cols, rows) = (80usize, 24usize);
    let mut term = par_term_emu_core::terminal::Terminal::with_scrollback(cols, rows, 50);
    let mut bytes =
        b"\x1b]133;A\x07$ \x1b]133;B\x07cmd\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07".to_vec();
    let line = "x".repeat(cols);
    let fill = term.grid().max_scrollback() - term.grid().scrollback_len() + rows - 3;
    for _ in 0..fill {
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(line.as_bytes());
    }
    term.process(&bytes);
    assert_eq!(term.grid().scrollback_len(), term.grid().max_scrollback());
    assert_eq!(term.cursor().row, rows - 1, "cursor on the bottom row");
    assert_eq!(term.get_zones().len(), 3, "no zone evicted yet");
    let pane = &mut state.sessions[0].windows[0].panes[0];
    pane.terminal = term.capture_snapshot();
    pane.cwd = Some("/par-mux-test-no-such-dir".to_string());

    let factory = RestoreProbeFactory::default();
    let probes = factory.probes.clone();
    let mut restored = MuxTree::from_persist_state(&state, Box::new(factory))
        .expect("a gone cwd degrades, never fails the restore");
    let probe = probes.lock().unwrap()[0].clone();
    assert!(
        restored.take_observer_batches().is_empty(),
        "restore drained its own batches"
    );

    assert!(
        probe.scrolled_out.load(Ordering::SeqCst),
        "the note's ZoneScrolledOut reached the observer by the time restore returned"
    );
    assert!(
        probe.terminal_lock_was_free.load(Ordering::SeqCst),
        "the observer ran under the pane's terminal write lock"
    );
    let snapshot = restored
        .pane(pane_id)
        .unwrap()
        .terminal()
        .read()
        .capture_snapshot();
    let text: String = snapshot.grid.cells.iter().map(|c| c.c).collect();
    assert!(text.contains("par-mux: /par-mux-test-no-such-dir is gone"));
}

/// serde(default): a save file written before pane cwds existed carries
/// no `cwd` key and must decode — and spawn — with the factory default.
#[test]
fn a_save_file_without_the_cwd_field_still_loads() {
    let mut tree = tree();
    let session = tree.new_session("old", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane_id = tree.window(window).unwrap().panes()[0];
    tree.pane(pane_id)
        .unwrap()
        .terminal()
        .write()
        .process(b"\x1b]7;file://localhost/tmp\x1b\\");

    let json = serde_json::to_string(&tree.to_persist_state()).unwrap();
    assert!(
        json.contains("\"cwd\""),
        "positive control: the field is on the wire"
    );
    let old_format = json.replace(",\"cwd\":\"/tmp\"", "");
    assert!(
        !old_format.contains("\"cwd\""),
        "the strip removed the only occurrence"
    );

    let state: PersistState =
        serde_json::from_str(&old_format).expect("a pre-cwd save file decodes");
    let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    assert_eq!(
        factory.spawn_of(pane_id).cwd,
        None,
        "a missing field decodes as no cwd, not an error"
    );
    assert!(restored.pane(pane_id).is_some());
}

/// A one-pane tree on [`RecordingFactory`] (a silent `sleep`, so no
/// prompt bytes race the assertions) whose pane carries a frozen screen
/// that sets up an alt screen and mouse mode, then is marked held-dead
/// with `exit_code`. Returns the tree and the pane id.
fn held_dead_tree(exit_code: Option<i32>) -> (MuxTree, PaneId) {
    let mut tree = MuxTree::new(Box::new(RecordingFactory::default()));
    let session = tree.new_session("dead", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane_id = tree.window(window).unwrap().panes()[0];
    let pane = tree.pane_mut(pane_id).unwrap();
    pane.terminal()
        .write()
        .process(b"MAIN-LINE\x1b[?1049h\x1b[?1000hALT-LINE");
    pane.set_metadata("agent", "claude");
    pane.set_metadata("agent_session_id", "sess-114");
    pane.mark_dead_with_code(exit_code);
    (tree, pane_id)
}

/// ARC-114: a held-dead pane survives a save/restore as a held-dead
/// pane — no process, exit code kept, screen byte-for-byte (the alt
/// screen and mouse mode a respawned pane would have reset stay), and
/// the spawn command retained for a later `respawn-pane`.
#[test]
fn a_held_dead_pane_round_trips_as_held_dead() {
    let (tree, pane_id) = held_dead_tree(Some(3));
    let state = tree.to_persist_state();
    let saved = &state.sessions[0].windows[0].panes[0];
    assert!(saved.dead, "positive control: dead is on the wire");
    assert_eq!(saved.exit_code, Some(3));
    assert!(saved.terminal.alt_screen_active, "control: alt screen up");

    let json = serde_json::to_string(&state).unwrap();
    let state: PersistState = serde_json::from_str(&json).unwrap();
    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(factory.clone()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();

    assert!(
        factory.command_for(pane_id).is_none() && factory.sizes.lock().unwrap().is_empty(),
        "a dead pane is never handed to the spawning factory path"
    );
    let pane = restored.pane(pane_id).unwrap();
    assert!(pane.dead());
    assert_eq!(pane.exit_code(), Some(3), "the code is not clobbered");
    assert!(pane.child_pid().is_none(), "no process behind it");
    assert_eq!(pane.spawn_command(), Some("sleep 60"));

    // Identity is re-hung for a later respawn, but no resume invocation
    // ran (the factory above was never asked to spawn anything).
    let resaved = restored.to_persist_state();
    assert!(saved.agent_session.is_some(), "control: identity was saved");
    assert_eq!(
        resaved.sessions[0].windows[0].panes[0].agent_session, saved.agent_session,
        "the agent identity survives a dead restore"
    );

    let want = &state.sessions[0].windows[0].panes[0].terminal;
    let got = pane.terminal().read().capture_snapshot();
    assert_eq!(got.grid.cells, want.grid.cells, "main screen identical");
    assert_eq!(
        got.alt_grid.cells, want.alt_grid.cells,
        "alt screen identical"
    );
    assert!(got.alt_screen_active, "alt screen not reset to primary");
    assert_eq!(got.cursor.col, want.cursor.col);
    assert_eq!(got.cursor.row, want.cursor.row);
    assert_ne!(
        pane.terminal().read().mouse_mode(),
        par_term_emu_core::mouse::MouseMode::Off,
        "mouse mode kept: there is no new process to protect"
    );
    let text: String = got.alt_grid.cells.iter().map(|c| c.c).collect();
    assert!(text.contains("ALT-LINE"), "the frozen content is real");
}

/// remain-on-exit off (the default): a persisted dead entry is DROPPED
/// at restore, honoring the current setting — and the kill-pane cascade
/// takes its window, session, and workspace with it when it was the
/// last pane. A live pane in the same save is untouched.
#[test]
fn auto_remove_restore_drops_dead_entries() {
    let (mut tree, dead_id) = held_dead_tree(Some(3));
    let live_id = tree
        .split_pane(dead_id, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    let state = tree.to_persist_state();

    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        false,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    assert!(restored.pane(dead_id).is_none(), "the dead entry is gone");
    assert!(
        restored.pane(live_id).is_some(),
        "the live pane survives the prune"
    );
    assert!(
        !restored.sessions.is_empty(),
        "the window still has a live pane, so the session stays"
    );
    tree_consistency(&restored);
}

/// remain-on-exit off, dead-only save, the DEFAULT: the workspace's
/// last pane cascades closed with its exit — the restored tree is
/// empty (session and workspace gone with it), matching what the live
/// reaper removes.
#[test]
fn auto_remove_restore_cascades_a_workspaces_last_dead_pane() {
    let (tree, pane_id) = held_dead_tree(Some(3));
    let state = tree.to_persist_state();
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        false,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    assert!(
        restored.pane(pane_id).is_none(),
        "the last dead entry cascades away, it is not held"
    );
    assert!(
        restored.sessions.is_empty(),
        "the emptied session cascaded away"
    );
    assert!(restored.workspaces.is_empty(), "the workspace went too");
    tree_consistency(&restored);
}

/// remain-on-exit off, keep-one-tab opt-in: a respawn-on workspace's
/// last dead pane is kept held dead, exactly what the live reaper
/// holds for the `set-workspace-respawn` path.
#[test]
fn auto_remove_restore_keeps_a_respawn_workspaces_last_dead_pane_held() {
    let (mut tree, pane_id) = held_dead_tree(Some(3));
    let ws = tree.workspace_of_pane(pane_id).unwrap();
    tree.set_workspace_respawn(ws, true).unwrap();
    let state = tree.to_persist_state();
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        false,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    let pane = restored.pane(pane_id).expect("the last pane is kept");
    assert!(pane.dead());
    assert_eq!(pane.exit_code(), Some(3));
    assert_eq!(restored.sessions.len(), 1, "its session stays");
    tree_consistency(&restored);
}

/// remain-on-exit off, two dead panes in one workspace (separate
/// windows): both drop through the cascade — the second is also its
/// workspace's last once the first is gone, and the default no longer
/// keeps it.
#[test]
fn auto_remove_restore_drops_dead_entries_down_to_the_last() {
    let (mut tree, first) = held_dead_tree(Some(3));
    let session = tree.sessions()[0];
    let window = tree.new_window(session, "two", 80, 24).unwrap();
    let second = tree.window(window).unwrap().panes()[0];
    tree.pane_mut(second).unwrap().mark_dead_with_code(Some(4));
    let state = tree.to_persist_state();
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        false,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    assert!(
        restored.pane(first).is_none(),
        "the first dead entry is gone"
    );
    assert!(
        restored.pane(second).is_none(),
        "the last dead entry cascades away too"
    );
    assert!(
        restored.workspaces.is_empty(),
        "the emptied workspace cascaded away"
    );
    tree_consistency(&restored);
}

/// remain-on-exit on: the pre-existing ARC-114 behavior — the dead
/// entry restores held-dead even though auto-remove is the product
/// default (the daemon was configured to hold).
#[test]
fn hold_restore_keeps_a_dead_entry_held_dead() {
    let (tree, pane_id) = held_dead_tree(Some(3));
    let state = tree.to_persist_state();
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    assert!(restored.pane(pane_id).unwrap().dead());
    assert_eq!(restored.pane(pane_id).unwrap().exit_code(), Some(3));
}

/// A dead pane with an unknown exit code stays dead with `None`, and a
/// live pane in the same save is respawned as before.
#[test]
fn dead_state_is_per_pane_and_live_panes_still_respawn() {
    let (mut tree, dead_id) = held_dead_tree(None);
    let live_id = tree
        .split_pane(dead_id, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    let state = tree.to_persist_state();
    let factory = RecordingFactory::default();
    let mut restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(factory.clone()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    assert!(restored.pane(dead_id).unwrap().dead());
    assert_eq!(restored.pane(dead_id).unwrap().exit_code(), None);
    assert!(!restored.pane(live_id).unwrap().dead());
    assert!(restored.pane_mut(live_id).unwrap().poll_running());
    assert!(factory.command_for(live_id).is_some(), "live pane spawned");
    assert!(factory.command_for(dead_id).is_none(), "dead pane did not");
}

/// ARC-114 compat: a save file without the new fields (any pre-ARC-114
/// daemon's) decodes and restores every pane alive, and a live pane
/// serializes without them so older daemons read newer files unchanged.
#[test]
fn a_save_file_without_the_dead_fields_loads_alive() {
    let (tree, pane_id) = held_dead_tree(Some(5));
    let mut value = serde_json::to_value(tree.to_persist_state()).unwrap();
    let pane = &mut value["sessions"][0]["windows"][0]["panes"][0];
    assert_eq!(pane["dead"], true, "positive control: on the wire");
    assert_eq!(pane["exit_code"], 5, "positive control: on the wire");
    let pane = pane.as_object_mut().unwrap();
    pane.remove("dead");
    pane.remove("exit_code");
    let state: PersistState = serde_json::from_value(value).expect("old file decodes");
    let mut restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    assert!(!restored.pane(pane_id).unwrap().dead());
    assert_eq!(restored.pane(pane_id).unwrap().exit_code(), None);
    assert!(
        restored.pane_mut(pane_id).unwrap().poll_running(),
        "respawned"
    );

    let live = serde_json::to_value(populated_tree().to_persist_state()).unwrap();
    for pane in live["sessions"][0]["windows"][0]["panes"]
        .as_array()
        .unwrap()
    {
        assert!(
            pane.get("dead").is_none() && pane.get("exit_code").is_none(),
            "a live pane serializes byte-compatibly with the older format"
        );
    }
}

/// The persisted cwd of a dead pane survives the restore (where
/// `begin_respawn` reads it) without any spawn-time landing logic.
#[cfg(unix)]
#[test]
fn a_dead_panes_cwd_survives_restore_for_respawn() {
    let (tree, pane_id) = held_dead_tree(Some(0));
    tree.pane(pane_id)
        .unwrap()
        .terminal()
        .write()
        .process(b"\x1b]7;file://localhost/tmp\x1b\\");
    let state = tree.to_persist_state();
    assert_eq!(
        state.sessions[0].windows[0].panes[0].cwd.as_deref(),
        Some("/tmp")
    );
    let mut restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    assert_eq!(
        restored.pane(pane_id).unwrap().persistence_cwd().as_deref(),
        Some(Path::new("/tmp")),
        "re-saving a restored dead pane keeps its cwd"
    );
    let plan = restored.begin_respawn(pane_id, false, None, None).unwrap();
    assert_eq!(plan.cwd.as_deref(), Some(Path::new("/tmp")));
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
        SessionId(state.next_ids.1),
        "a restored server's first new session must take the persisted next id"
    );
}

/// Workspaces round-trip: members, order, active pointers, and names
/// all travel, and a restored bare `new-session` lands in the
/// persisted ACTIVE workspace.
#[test]
fn workspaces_round_trip_through_persistence() {
    let mut original = populated_tree();
    let ws_main = original
        .resolve_new_session_workspace(None)
        .expect("populated_tree created its default");
    let ws_dev = original.new_workspace("dev");
    original.rename_workspace(ws_main, "prod").unwrap();
    let expected_main = original.workspace(ws_main).unwrap().sessions.clone();
    original.select_workspace(ws_dev).unwrap();
    let s3 = original.new_session("in-dev", 80, 24).unwrap();

    let state = original.to_persist_state();
    assert_eq!(state.active_workspace, Some(ws_dev.0));
    assert_eq!(
        state
            .workspaces
            .iter()
            .find(|w| w.id == ws_main.0)
            .unwrap()
            .name,
        "prod"
    );

    let mut restored =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
    assert_eq!(restored.workspaces().len(), 2);
    assert_eq!(restored.active_workspace(), Some(ws_dev));
    assert_eq!(
        restored.workspace(ws_main).unwrap().sessions,
        expected_main,
        "workspace membership and order travel"
    );
    assert_eq!(
        restored.workspace(ws_dev).unwrap().sessions,
        vec![s3],
        "the post-rename session landed in the active workspace"
    );
    // A restored bare new-session lands in the persisted active
    // workspace — not a fresh default.
    let fresh = restored.new_session("fresh", 80, 24).unwrap();
    assert_eq!(restored.workspace_of_session(fresh), Some(ws_dev));
    tree_consistency(&restored);
}

/// A v2-shaped state file (pre-workspaces) fails the version gate and
/// is quarantined rather than partially read — the documented
/// no-migration policy.
#[test]
fn a_pre_workspace_state_file_is_quarantined() {
    let (dir, target) = temp_target("v2-quarantine");
    let mut value = serde_json::to_value(populated_tree().to_persist_state()).unwrap();
    value["format_version"] = serde_json::json!(2);
    value.as_object_mut().unwrap().remove("workspaces");
    value.as_object_mut().unwrap().remove("active_workspace");
    let bytes = serde_json::to_vec(&value).unwrap();
    std::fs::write(&target, &bytes).unwrap();

    match load_or_quarantine(&target) {
        Loaded::Quarantined { from, .. } => assert_eq!(from, target),
        other => panic!("a v2 file must quarantine, got {other:?}"),
    }
    drop(dir);
}

/// Assert helper: brute-force the reverse indexes on a restored tree.
fn tree_consistency(tree: &MuxTree) {
    tree.assert_indexes_consistent();
    for session in tree.sessions() {
        assert!(
            tree.workspace_of_session(session).is_some(),
            "every restored session belongs to a workspace"
        );
    }
}

/// SEC-128: a dead pane whose OSC 7 report named a remote host saves and
/// restores with the host beside the cwd, so `respawn_cwd` still sees the
/// report as remote.
#[cfg(unix)]
#[test]
fn a_dead_panes_remote_osc7_host_survives_restore() {
    let (tree, pane_id) = held_dead_tree(Some(0));
    tree.pane(pane_id)
        .unwrap()
        .terminal()
        .write()
        .process(b"\x1b]7;file://remote-host/some/path\x1b\\");
    let state = tree.to_persist_state();
    let saved = &state.sessions[0].windows[0].panes[0];
    assert_eq!(saved.cwd.as_deref(), Some("/some/path"));
    assert_eq!(saved.cwd_host.as_deref(), Some("remote-host"));
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    let pane = restored.pane(pane_id).unwrap();
    let term = pane.terminal();
    let term = term.read();
    assert_eq!(term.current_directory(), Some("/some/path"));
    assert_eq!(term.shell_integration().hostname(), Some("remote-host"));
}

/// A workspace's worktree link rides the state file as a `worktree`
/// object, an ordinary workspace carries no key (so files written
/// before worktree workspaces existed keep their shape), and a file
/// without the key restores an unlinked workspace.
#[test]
fn worktree_link_persists_and_old_files_load_unlinked() {
    let (mut tree, pane_id) = held_dead_tree(Some(0));
    let ws = tree.workspace_of_pane(pane_id).unwrap();
    let plain = serde_json::to_value(tree.to_persist_state()).unwrap();
    assert!(
        plain["workspaces"][0].get("worktree").is_none(),
        "an unlinked workspace serializes without the key"
    );
    let link = crate::mux::tree::WorktreeLink {
        repo: PathBuf::from("/src/app"),
        checkout: PathBuf::from("/src/app-worktrees/x"),
        branch: Some("x".to_string()),
    };
    tree.set_worktree_link(ws, Some(link.clone())).unwrap();
    let mut value = serde_json::to_value(tree.to_persist_state()).unwrap();
    assert_eq!(
        value["workspaces"][0]["worktree"],
        serde_json::json!({"repo": "/src/app", "checkout": "/src/app-worktrees/x", "branch": "x"})
    );
    let state: PersistState = serde_json::from_value(value.clone()).unwrap();
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    assert_eq!(restored.workspace(ws).unwrap().worktree, Some(link));

    value["workspaces"][0]
        .as_object_mut()
        .unwrap()
        .remove("worktree");
    let state: PersistState = serde_json::from_value(value).expect("old file decodes");
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    assert_eq!(restored.workspace(ws).unwrap().worktree, None);
}

/// A save file written before `cwd_host` existed decodes with no host
/// (today's behavior), and a pane without one serializes without the key.
#[cfg(unix)]
#[test]
fn a_save_file_without_cwd_host_loads_as_before() {
    let (tree, pane_id) = held_dead_tree(Some(0));
    tree.pane(pane_id)
        .unwrap()
        .terminal()
        .write()
        .process(b"\x1b]7;file://remote-host/some/path\x1b\\");
    let mut value = serde_json::to_value(tree.to_persist_state()).unwrap();
    let pane = value["sessions"][0]["windows"][0]["panes"][0]
        .as_object_mut()
        .unwrap();
    assert_eq!(
        pane["cwd_host"], "remote-host",
        "positive control: on the wire"
    );
    pane.remove("cwd_host");
    let state: PersistState = serde_json::from_value(value).expect("old file decodes");
    assert_eq!(state.sessions[0].windows[0].panes[0].cwd_host, None);
    let restored = MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(RecordingFactory::default()),
        true,
        &crate::mux::config::AgentAppend::default(),
    )
    .unwrap();
    let pane = restored.pane(pane_id).unwrap();
    let term = pane.terminal();
    let term = term.read();
    assert_eq!(term.current_directory(), Some("/some/path"));
    assert_eq!(term.shell_integration().hostname(), None);
    drop(term);

    let (plain, _) = held_dead_tree(Some(0));
    let json = serde_json::to_string(&plain.to_persist_state()).unwrap();
    assert!(!json.contains("cwd_host"), "absent host is not serialized");
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

/// ARC-113b: a corrupt or hostile state file's window size must not
/// reach the factory's grid allocation unchecked — zero collapses the
/// grid's column math and `u16::MAX` (the persisted field's ceiling)
/// asks the allocator for tens of gigabytes. The restore clamps to the
/// same bounds a live client's size request is held to, and the
/// clamped size is both what the factory spawned the pane at and what
/// the restored window reports.
#[test]
fn restore_clamps_a_hostile_window_size() {
    let mut state = populated_tree().to_persist_state();
    let zero_window = state.sessions[0].windows[0].id;
    let huge_window = state.sessions[0].windows[1].id;
    let zero_pane = state.sessions[0].windows[0].panes[0].id;
    let huge_pane = state.sessions[0].windows[1].panes[0].id;
    state.sessions[0].windows[0].cols = 0;
    state.sessions[0].windows[0].rows = 0;
    state.sessions[0].windows[1].cols = u16::MAX;
    state.sessions[0].windows[1].rows = u16::MAX;

    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();

    assert_eq!(
        restored.window(WindowId(zero_window)).unwrap().cols,
        MIN_RESTORED_COLS,
        "a zero-size window restores at the column floor"
    );
    assert_eq!(
        restored.window(WindowId(zero_window)).unwrap().rows,
        MIN_RESTORED_ROWS,
        "a zero-size window restores at the row floor"
    );
    assert_eq!(
        factory.size_for(PaneId(zero_pane)),
        (MIN_RESTORED_COLS, MIN_RESTORED_ROWS)
    );
    assert_eq!(
        restored.window(WindowId(huge_window)).unwrap().cols,
        MAX_RESTORED_COLS,
        "a u16::MAX window restores at the column ceiling"
    );
    assert_eq!(
        restored.window(WindowId(huge_window)).unwrap().rows,
        MAX_RESTORED_ROWS,
        "a u16::MAX window restores at the row ceiling"
    );
    assert_eq!(
        factory.size_for(PaneId(huge_pane)),
        (MAX_RESTORED_COLS, MAX_RESTORED_ROWS)
    );
}

/// ARC-113c2: a hostile snapshot's grid dims must clamp to the same
/// bounds as the window size, and the cell/wrapped vecs must be
/// reshaped to the clamped shape — `Grid::restore_from_snapshot`
/// adopts all of them wholesale.
#[test]
fn clamp_restored_grid_dims_clamps_hostile_dims_and_reshapes_cells() {
    let tree = populated_tree();
    let session_id = *tree.sessions.keys().next().unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let pane_id = tree.window(window_id).unwrap().panes()[0];
    let mut snap = tree
        .pane(pane_id)
        .unwrap()
        .terminal()
        .read()
        .capture_snapshot();

    snap.grid.cols = 0;
    snap.grid.rows = 0;
    clamp_restored_grid_dims(&mut snap);
    assert_eq!(snap.grid.cols, usize::from(MIN_RESTORED_COLS));
    assert_eq!(snap.grid.rows, usize::from(MIN_RESTORED_ROWS));
    assert_eq!(snap.grid.cells.len(), snap.grid.cols * snap.grid.rows);
    assert_eq!(snap.grid.wrapped.len(), snap.grid.rows);

    snap.grid.cols = usize::from(u16::MAX);
    snap.grid.rows = usize::from(u16::MAX);
    clamp_restored_grid_dims(&mut snap);
    assert_eq!(snap.grid.cols, usize::from(MAX_RESTORED_COLS));
    assert_eq!(snap.grid.rows, usize::from(MAX_RESTORED_ROWS));
    assert_eq!(snap.grid.cells.len(), snap.grid.cols * snap.grid.rows);
    assert_eq!(snap.grid.wrapped.len(), snap.grid.rows);
}

/// ARC-113c2 end to end: a state file carrying hostile grid dims in a
/// pane's snapshot must restore, with the pane's terminal left inside
/// the restored bounds and readable rather than panicking on collapsed
/// or giant row-major math.
#[test]
fn restore_survives_a_hostile_snapshot_grid_size() {
    let mut state = populated_tree().to_persist_state();
    let zero_pane = state.sessions[0].windows[0].panes[0].id;
    let huge_pane = state.sessions[0].windows[1].panes[0].id;
    state.sessions[0].windows[0].panes[0].terminal.grid.cols = 0;
    state.sessions[0].windows[0].panes[0].terminal.grid.rows = 0;
    state.sessions[0].windows[1].panes[0].terminal.grid.cols = usize::from(u16::MAX);
    state.sessions[0].windows[1].panes[0].terminal.grid.rows = usize::from(u16::MAX);

    let restored =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();

    for pane_id in [PaneId(zero_pane), PaneId(huge_pane)] {
        let terminal = restored.pane(pane_id).expect("restored pane").terminal();
        let (cols, rows) = terminal.read().size();
        assert!(
            (usize::from(MIN_RESTORED_COLS)..=usize::from(MAX_RESTORED_COLS)).contains(&cols)
                && (usize::from(MIN_RESTORED_ROWS)..=usize::from(MAX_RESTORED_ROWS))
                    .contains(&rows),
            "restored pane is {cols}x{rows}, outside the restored bounds"
        );
        let _ = terminal.read().content();
    }
}

/// A pane saved while a full-screen app held the alternate screen is
/// respawned with a fresh shell, which must write to the main screen:
/// left on the old app's alternate screen, its output never scrolls
/// into history, so the pane has no scrollback after a restart.
#[cfg(unix)]
#[test]
fn restore_returns_a_full_screen_pane_to_the_main_screen() {
    let mut tree = tree();
    let session = tree.new_session("main", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let first = tree.window(window).unwrap().panes()[0];
    let quiet = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, Some("sleep 60"))
        .unwrap();
    {
        let terminal = tree.pane(quiet).unwrap().terminal();
        let mut guard = terminal.write();
        guard.process(b"shell line\r\n\x1b[?1049h\x1b[?1h\x1b=\x1b[?1000h\x1b[3;10rtui");
        assert!(guard.is_alt_screen_active(), "setup: app on the alt screen");
    }

    let state = tree.to_persist_state();
    let restored =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
    let terminal = restored.pane(quiet).unwrap().terminal();
    let mut guard = terminal.write();
    assert!(
        !guard.is_alt_screen_active(),
        "the new shell writes to the main screen"
    );
    let after = guard.capture_snapshot();
    assert!(
        !after.application_cursor,
        "the app's cursor-key mode is gone"
    );
    assert!(!after.application_keypad, "the app's keypad mode is gone");
    assert_eq!(after.mouse_mode, par_term_emu_core::mouse::MouseMode::Off);
    assert_eq!(
        (after.scroll_region_top, after.scroll_region_bottom),
        (0, after.rows - 1),
        "the app's scroll region is gone"
    );
    let screen_text: String = after.grid.cells.iter().map(|c| c.c).collect();
    assert!(
        screen_text.contains("shell line"),
        "the main screen's content survives"
    );

    for i in 0..40 {
        guard.process(format!("new line {i:02}\r\n").as_bytes());
    }
    assert!(
        guard.capture_snapshot().grid.scrollback_lines > 0,
        "new output scrolls into the main screen's history"
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

#[test]
fn save_lands_at_the_target_with_no_tmp_leftover_and_round_trips() {
    let (_dir, target) = temp_target("save");
    let original = populated_tree();

    save_to(&original, &target).expect("save succeeds");

    // Rename semantics (D3.3): the complete state is AT the target and
    // the tmp sibling is gone — a crash mid-save can only leave the old
    // state or this tmp, never a torn target.
    assert!(target.exists(), "state lands at the target");
    assert!(
        !tmp_sibling(&target).exists(),
        "no .tmp sibling survives a completed save"
    );

    match load_or_quarantine(&target) {
        Loaded::State(state) => {
            assert_eq!(state.format_version, FORMAT_VERSION);
            assert_eq!(state.sessions.len(), 2);
            let restored =
                MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
                    .expect("saved state restores");
            assert_same_shape(&restored, &original);
        }
        other => panic!("expected a readable state file, got {other:?}"),
    }
}

/// Write bytes straight to `target`, creating the parent — the shape a
/// torn or foreign write leaves on disk (no save_to on the path).
fn write_raw(target: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(target.parent().expect("temp targets have a parent")).unwrap();
    std::fs::write(target, bytes).unwrap();
}

#[test]
fn torn_write_is_quarantined_and_starts_fresh() {
    let (_dir, target) = temp_target("torn");
    let full = serde_json::to_string(&populated_tree().to_persist_state()).unwrap();
    // Half the bytes made it to "disk" — exactly what a torn
    // non-atomic write leaves behind.
    write_raw(&target, &full.as_bytes()[..full.len() / 2]);

    match load_or_quarantine(&target) {
        Loaded::Quarantined { from, to } => {
            assert_eq!(from, target);
            assert!(to.exists(), "the torn file is preserved aside, not deleted");
            assert!(!target.exists(), "the load path is clear for the next save");
        }
        other => panic!("expected quarantine, got {other:?}"),
    }
}

#[test]
fn unknown_version_state_is_quarantined() {
    let (_dir, target) = temp_target("version");
    let mut state = populated_tree().to_persist_state();
    state.format_version = FORMAT_VERSION + 7;
    write_raw(&target, serde_json::to_string(&state).unwrap().as_bytes());

    assert!(matches!(
        load_or_quarantine(&target),
        Loaded::Quarantined { .. }
    ));
    assert!(!target.exists());
}

#[test]
fn missing_state_file_is_a_quiet_fresh_start() {
    let (_dir, target) = temp_target("missing");
    assert!(matches!(load_or_quarantine(&target), Loaded::Fresh));
    assert!(!target.exists(), "a fresh start must not create anything");
}

// --- the last-good snapshot: the shutdown race cannot wipe the layout ---

/// The reboot/logout race in three deterministic saves: a pane-bearing
/// structural save, then the reaper persisting a burst of pane deaths
/// (empty), then the daemon's own final save (still empty). The load
/// must resurrect the pre-exit layout, not the raced emptiness.
#[test]
fn snapshot_history_writes_throttles_and_prunes() {
    let (_dir, target) = temp_target("history");
    let populated = populated_tree().to_persist_state();
    write_job(SaveOrigin::Command, &populated, &target).unwrap();

    // The save itself wrote the first snapshot (nothing to throttle).
    let after_save = kept_snapshots(&target);
    assert_eq!(after_save.len(), 1, "one snapshot per interval");

    // A long interval throttles the next copy away.
    maintain_snapshot_history_in(
        &target,
        std::time::Duration::from_secs(60 * 60),
        SNAPSHOT_LIMIT,
    )
    .unwrap();
    assert_eq!(kept_snapshots(&target).len(), 1);

    // A short interval writes again, and the cap prunes the oldest.
    for _ in 0..4 {
        maintain_snapshot_history_in(&target, std::time::Duration::ZERO, 2).unwrap();
    }
    let kept = kept_snapshots(&target);
    assert_eq!(kept.len(), 2, "capped at the configured count");
    assert!(
        kept.windows(2).all(|pair| pair[0] <= pair[1]),
        "oldest first: {kept:?}"
    );
}

/// A DELIBERATE empty save (Command origin) clears the history tier
/// with .lastgood — the next start is fresh. A raced empty save (Reap
/// origin keeps both tiers) restores, and with .lastgood gone the
/// history snapshot is the tier that answers.
#[test]
fn an_empty_state_recovers_through_the_history_tier_only_when_raced() {
    let (_dir, target) = temp_target("empty-history");
    let populated = populated_tree().to_persist_state();
    write_job(SaveOrigin::Command, &populated, &target).unwrap();
    write_job(SaveOrigin::Command, &tree().to_persist_state(), &target).unwrap();
    match load_or_quarantine(&target) {
        Loaded::State(state) => assert!(
            !state_has_panes(&state),
            "the deliberate empty is not resurrected from history"
        ),
        other => panic!("expected a state, got {other:?}"),
    }

    // The raced shape: a pane-bearing save, then the reaper persisting
    // a burst of pane deaths with .lastgood lost.
    write_job(SaveOrigin::Command, &populated, &target).unwrap();
    write_job(SaveOrigin::Reap, &tree().to_persist_state(), &target).unwrap();
    std::fs::remove_file(lastgood_path(&target)).unwrap();
    match load_or_quarantine(&target) {
        Loaded::State(state) => assert!(
            state_has_panes(&state),
            "the history snapshot restored the pane-bearing layout"
        ),
        other => panic!("expected a restored state, got {other:?}"),
    }
}

#[test]
fn a_corrupt_state_recovers_through_lastgood_then_the_history_tier() {
    let (dir, target) = temp_target("corrupt-history");
    let populated = populated_tree().to_persist_state();
    write_job(SaveOrigin::Command, &populated, &target).unwrap();
    std::fs::remove_file(lastgood_path(&target)).unwrap();
    std::fs::write(&target, "{ not json").unwrap();

    match load_or_quarantine(&target) {
        Loaded::State(state) => assert!(state_has_panes(&state)),
        other => panic!("expected the history tier, got {other:?}"),
    }
    assert!(
        dir.path().read_dir().unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".quarantine-")),
        "the corrupt file was still quarantined for inspection"
    );
}

/// A shutdown save whose state dir was deleted mid-run respects the
/// deletion (D3: no resurrection) — the Windows VM measured the
/// user-visible half of this card: saves failing 183 left no dir, and
/// the shutdown save correctly skipped instead of recreating it.
#[test]
fn a_shutdown_save_respects_a_state_dir_deleted_mid_run() {
    let (dir, target) = temp_target("deleted-dir");
    let populated = populated_tree().to_persist_state();
    write_job(SaveOrigin::Command, &populated, &target).unwrap();
    assert!(target.is_file());

    std::fs::remove_dir_all(dir.path()).unwrap();

    write_job(SaveOrigin::Shutdown, &populated, &target)
        .expect("the shutdown save skips, it does not fail");
    assert!(
        !dir.path().exists(),
        "the deleted state dir must not be resurrected"
    );
    // A live-daemon save (Command origin) keeps creating the dir — the
    // daemon owns its state until the exit.
    write_job(SaveOrigin::Command, &populated, &target).unwrap();
    assert!(target.is_file());
}

#[test]
fn reap_saves_cannot_degrade_the_last_good_snapshot() {
    let (_dir, target) = temp_target("race");
    let populated = populated_tree().to_persist_state();
    let empty = tree().to_persist_state();
    assert!(state_has_panes(&populated) && !state_has_panes(&empty));

    write_job(SaveOrigin::Command, &populated, &target).unwrap();
    write_job(SaveOrigin::Reap, &empty, &target).unwrap();
    write_job(SaveOrigin::Shutdown, &empty, &target).unwrap();

    match load_or_quarantine(&target) {
        Loaded::State(state) => assert!(
            state_has_panes(&state),
            "the pre-exit layout is restored, not the raced emptiness"
        ),
        other => panic!("a readable state file loaded as {other:?}"),
    }
}

/// A deliberately emptied tree (kill-pane of the last pane, a Command
/// save) clears the snapshot: the next start is fresh, not a
/// resurrection of panes the user closed on purpose.
#[test]
fn a_command_empty_save_clears_the_last_good_snapshot() {
    let (_dir, target) = temp_target("deliberate");
    write_job(
        SaveOrigin::Command,
        &populated_tree().to_persist_state(),
        &target,
    )
    .unwrap();
    write_job(SaveOrigin::Command, &tree().to_persist_state(), &target).unwrap();
    match load_or_quarantine(&target) {
        Loaded::State(state) => assert!(
            !state_has_panes(&state),
            "a deliberate empty is the honest state — no resurrection"
        ),
        other => panic!("a readable state file loaded as {other:?}"),
    }
}

/// The exit-when-empty daemon's final save is deliberate the same way:
/// everything closed, no signal raced it, so the snapshot goes and the
/// next start is fresh. Contrast with the Shutdown arm above, where the
/// emptiness may be the reboot race and the snapshot must survive.
#[test]
fn an_exit_when_empty_final_save_clears_the_last_good_snapshot() {
    let (_dir, target) = temp_target("empty-exit");
    write_job(
        SaveOrigin::Command,
        &populated_tree().to_persist_state(),
        &target,
    )
    .unwrap();
    write_job(
        SaveOrigin::ShutdownEmpty,
        &tree().to_persist_state(),
        &target,
    )
    .unwrap();
    match load_or_quarantine(&target) {
        Loaded::State(state) => assert!(
            !state_has_panes(&state),
            "the empty exit is deliberate — no resurrection"
        ),
        other => panic!("a readable state file loaded as {other:?}"),
    }
}

/// ARC-097: exit-when-empty also fires when every pane is dead. That
/// tree is pane-bearing, so the ShutdownEmpty save refreshes the
/// snapshot instead of clearing it — the next start respawns those
/// panes (MUX.md, Pane Reaping).
#[test]
fn shutdown_empty_with_dead_panes_refreshes_lastgood() {
    let (_dir, target) = temp_target("all-dead-exit");
    write_job(SaveOrigin::Command, &tree().to_persist_state(), &target).unwrap();
    assert!(
        !lastgood_path(&target).exists(),
        "precondition: no snapshot"
    );

    let dead = populated_tree().to_persist_state();
    assert!(state_has_panes(&dead));
    write_job(SaveOrigin::ShutdownEmpty, &dead, &target).unwrap();
    let snapshot: PersistState =
        serde_json::from_slice(&fs::read(lastgood_path(&target)).unwrap()).unwrap();
    assert!(
        state_has_panes(&snapshot),
        "an all-dead exit keeps its panes for the next start"
    );
}

/// The shutdown-family save respects a state dir deleted while the
/// daemon ran (the `--stop; rm -rf state` script window): no dir is
/// re-created, and the save is a silent Ok. A live daemon's earlier
/// saves keep creating the dir.
#[test]
fn a_shutdown_save_does_not_resurrect_a_deleted_state_dir() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("state").join("par-mux-state.json");
    write_job(SaveOrigin::Command, &tree().to_persist_state(), &target).unwrap();
    assert!(target.exists(), "precondition: the live save created it");
    // The script's deletion, inside the --stop-to-exit window.
    fs::remove_file(&target).unwrap();
    fs::remove_dir(target.parent().unwrap()).unwrap();
    write_job(SaveOrigin::Shutdown, &tree().to_persist_state(), &target).unwrap();
    assert!(
        !target.parent().unwrap().exists(),
        "the shutdown save did not re-create the deleted dir"
    );
    // An earlier save of a still-running daemon keeps creating the dir.
    write_job(SaveOrigin::Command, &tree().to_persist_state(), &target).unwrap();
    assert!(target.exists(), "a live save re-creates it");
}

/// ARC-119: the off-lock shutdown save lands the tree's structure,
/// applies the origin's snapshot rule, and leaves the tree mutex free.
/// Content parity with the one-shot capture is
/// `two_phase_capture_matches_one_shot` (`to_persist_state` is the
/// same two calls); live shells make a byte comparison racy here.
#[test]
fn save_off_lock_writes_the_tree_and_releases_the_lock() {
    let (_dir, target) = temp_target("off-lock");
    let tree = parking_lot::Mutex::new(populated_tree());
    save_off_lock(&tree, &target, SaveOrigin::Shutdown).unwrap();
    assert!(
        tree.try_lock().is_some(),
        "the tree mutex is free once the save returns"
    );
    let written: PersistState = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
    let expected = tree.lock().to_persist_state();
    let shape = |state: &PersistState| {
        state
            .sessions
            .iter()
            .map(|s| {
                let windows: Vec<_> = s
                    .windows
                    .iter()
                    .map(|w| (w.id, w.panes.iter().map(|p| p.id).collect::<Vec<_>>()))
                    .collect();
                (s.id, s.name.clone(), windows)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&written), shape(&expected));
    assert_eq!(written.next_ids, expected.next_ids);
    assert_eq!(written.buffers, expected.buffers);
    assert!(
        lastgood_path(&target).exists(),
        "a pane-bearing Shutdown save refreshes the snapshot"
    );
}

/// Positive control for the fallback: an empty state with NO snapshot
/// behind it loads empty — the fallback cannot invent panes.
#[test]
fn an_empty_state_without_a_snapshot_loads_empty() {
    let (_dir, target) = temp_target("no-snapshot");
    write_job(SaveOrigin::Shutdown, &tree().to_persist_state(), &target).unwrap();
    match load_or_quarantine(&target) {
        Loaded::State(state) => assert!(!state_has_panes(&state)),
        other => panic!("a readable state file loaded as {other:?}"),
    }
}

/// The snapshot is a cache, not evidence: a corrupt one is ignored
/// (not quarantined), and the empty main state loads as itself.
#[test]
fn a_corrupt_lastgood_is_ignored() {
    let (_dir, target) = temp_target("corrupt-good");
    write_job(
        SaveOrigin::Command,
        &populated_tree().to_persist_state(),
        &target,
    )
    .unwrap();
    let lastgood = lastgood_path(&target);
    assert!(lastgood.exists(), "a pane-bearing save wrote the snapshot");
    std::fs::write(&lastgood, b"not json").unwrap();
    write_job(SaveOrigin::Reap, &tree().to_persist_state(), &target).unwrap();
    match load_or_quarantine(&target) {
        // The corrupt lastgood is ignored as a source; the history
        // snapshot the populated save left answers in its place (the
        // same content .lastgood would have carried), and the corrupt
        // file itself is left exactly as found.
        Loaded::State(state) => assert!(state_has_panes(&state)),
        other => panic!("a readable state file loaded as {other:?}"),
    }
    assert_eq!(
        std::fs::read(&lastgood).unwrap(),
        b"not json",
        "the corrupt snapshot is never consumed or rewritten"
    );
}

#[test]
fn state_files_are_keyed_by_socket_stem_under_the_par_mux_dir() {
    // default_socket_path produces `<base>/par-mux-<name>.sock`, so the
    // stem that keys the state file is `par-mux-<name>` in full.
    let base = Path::new("/base");
    let a = state_file_in(base, Path::new("/tmp/par-mux-alpha.sock"));
    let b = state_file_in(base, Path::new("/tmp/par-mux-beta.sock"));
    assert!(a.ends_with("par-mux/par-mux-alpha.state.json"));
    assert!(b.ends_with("par-mux/par-mux-beta.state.json"));
    assert_ne!(a, b, "two servers on different sockets never share state");
}

/// One pane carrying the given metadata keys — the shared base for the
/// agent-identity tests.
fn tree_with_metadata(metadata: &[(&str, &str)]) -> (MuxTree, PaneId) {
    let mut tree = tree();
    let session = tree.new_session("agents", 80, 24).unwrap();
    let pane_id = tree
        .session(session)
        .unwrap()
        .windows
        .iter()
        .filter_map(|window| tree.window(*window))
        .flat_map(|window| window.panes())
        .next()
        .unwrap();
    let pane = tree.pane_mut(pane_id).unwrap();
    for (key, value) in metadata {
        pane.set_metadata(key, value);
    }
    (tree, pane_id)
}

/// One pane whose metadata carries the full hook-reported identity.
fn tree_with_agent_pane() -> (MuxTree, PaneId) {
    let (mut tree, pane_id) = tree_with_metadata(&[
        ("agent", "pi"),
        ("agent_session_id", "s-1"),
        ("agent_session_path", "/tmp/pi-session.jsonl"),
        ("agent_source", "par-mux:pi"),
        (
            "agent_resume_argv",
            r#"["pi","--session","/tmp/pi-session.jsonl"]"#,
        ),
        // State travels now: the roster survives a restart (bounded by
        // the liveness sweep and the agent's next report).
        ("agent_state", "working"),
        ("agent_state_source", "hook"),
        // Keys that must NOT travel: process-local bookkeeping and
        // provenance-of-start.
        ("agent_seq", "1000"),
        ("agent_session_start_source", "startup"),
    ]);
    // Nor the typed, display-only half of the claim (ARC-113).
    let pane = tree.pane_mut(pane_id).unwrap();
    pane.telemetry = Some(crate::mux::hooks::StoredTelemetry {
        sampled_at_unix_ms: 1,
        canonical_b64: "e30=".to_string(),
    });
    pane.seq_by_source.insert("par-mux:pi".to_string(), 1000);
    (tree, pane_id)
}

/// The resume spawn re-lands in the pane's persisted cwd: the agent's
/// invocation (claude resolves transcripts per cwd) must run where the
/// session lived, not in the daemon's start directory. Unix only: the
/// persisted cwd is seeded through a `/`-rooted OSC 7 URL.
#[test]
#[cfg(unix)]
fn a_resume_invocation_spawns_in_the_persisted_cwd() {
    let (tree, pane_id) = tree_with_agent_pane();
    let dir = tempfile::tempdir().unwrap();
    let reported = dir.path().join("repo");
    std::fs::create_dir(&reported).unwrap();
    tree.pane(pane_id)
        .unwrap()
        .terminal()
        .write()
        .process(format!("\x1b]7;file://localhost{}\x1b\\", reported.display()).as_bytes());
    let state = tree.to_persist_state();

    let factory = crate::mux::pane::test_support::ContextRecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    let spawn = factory.spawn_of(pane_id);
    assert_eq!(
        spawn.cwd.as_deref(),
        Some(reported.as_path()),
        "the resume spawn carries the persisted cwd"
    );
    assert!(
        restored.pane(pane_id).is_some()
            && state.sessions[0].windows[0].panes[0]
                .agent_session
                .is_some(),
        "positive control: this pane restored through the resume path"
    );
}

/// A released pane restores through its ORIGINAL command, not the
/// resume invocation: the release cleared the session identity, so
/// persistence has nothing to resume and Phase 3 behavior resumes.
#[test]
fn a_released_pane_restores_through_the_original_command() {
    let (tree, pane_id) = tree_with_agent_pane();
    let tree = std::sync::Arc::new(parking_lot::Mutex::new(tree));
    let release = format!(
        r#"{{"id":"t-9","method":"pane.release_agent","params":{{"pane_id":"{pane_id}","agent":"pi","seq":2000,"source":"par-mux:test"}}}}"#
    );
    let (reply, _) = crate::mux::hooks::handle_report(&release, &tree);
    assert!(reply.contains(r#""result":"ok""#), "released: {reply}");
    let tree = std::sync::Arc::into_inner(tree)
        .expect("the test holds the only reference")
        .into_inner();

    let state = tree.to_persist_state();
    assert!(
        state.sessions[0].windows[0].panes[0]
            .agent_session
            .is_none(),
        "the release cleared what the resume path needs"
    );

    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    assert_eq!(
        factory.command_for(pane_id),
        None,
        "the pane respawns its original command (the default shell), not a resume"
    );
    assert!(restored.pane(pane_id).is_some());
}

#[test]
fn agent_session_identity_round_trips() {
    let (original, pane_id) = tree_with_agent_pane();
    let state = original.to_persist_state();
    let persisted = state.sessions[0].windows[0].panes[0]
        .agent_session
        .as_ref()
        .expect("the agent pane carries its identity");
    assert_eq!(
        persisted,
        &PersistAgentSession {
            agent: "pi".to_string(),
            session_id: Some("s-1".to_string()),
            session_path: Some("/tmp/pi-session.jsonl".to_string()),
            source: Some("par-mux:pi".to_string()),
            resume_argv: Some(r#"["pi","--session","/tmp/pi-session.jsonl"]"#.to_string()),
            state: Some("working".to_string()),
            state_source: Some("hook".to_string()),
            message: None,
        },
        "capture reads identity plus the last-known state"
    );

    // Restore writes the identity back as metadata, so a re-capture of
    // the restored tree holds the same identity — the format round-trip.
    // The recording factory keeps the real pi CLI out of the test while
    // proving what restore actually spawned (task 6.3): the reported
    // invocation, verbatim.
    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    {
        let pane = restored.pane(pane_id).expect("restored pane");
        assert_eq!(
            pane.metadata().get("agent_state").map(String::as_str),
            Some("working"),
            "the restored pane is on the roster before its agent's next report"
        );
        assert_eq!(
            pane.metadata()
                .get("agent_state_source")
                .map(String::as_str),
            Some("hook"),
            "the restored claim keeps its hook authority"
        );
    }
    let expected = if cfg!(windows) {
        // The POSIX surviving tail cannot cross cmd.exe; on Windows the
        // recorded spawn is the bare invocation (the real argv path is
        // covered by a_windows_resume_receives_the_exact_arguments).
        "'pi' '--session' '/tmp/pi-session.jsonl'".to_string()
    } else {
        format!("'pi' '--session' '/tmp/pi-session.jsonl'{SURVIVING_TAIL}")
    };
    assert_eq!(
        factory.command_for(pane_id).as_deref(),
        Some(expected.as_str()),
        "restore spawns the hook-reported invocation, not the original command"
    );
    let recaptured = restored.to_persist_state();
    assert_eq!(
        recaptured.sessions[0].windows[0].panes[0].agent_session,
        state.sessions[0].windows[0].panes[0].agent_session,
        "identity survives tree -> format -> tree -> format"
    );

    // The restored pane holds the identity and the last-known state
    // (the roster survives the restart), but none of the process-local
    // bookkeeping.
    let pane = restored.pane(pane_id).unwrap();
    assert_eq!(pane.metadata().get("agent").map(String::as_str), Some("pi"));
    assert!(
        !pane.metadata().contains_key("agent_seq"),
        "the restarted agent's first report is never stale"
    );
    assert!(
        !pane.metadata().contains_key("agent_session_start_source"),
        "start source describes the PREVIOUS process's start — stale after a restart"
    );
    assert!(
            pane.telemetry.is_none(),
            "telemetry is display-only and ephemeral — a restored pane serves absent, never a stale sample"
        );
    assert!(
        pane.seq_by_source.is_empty(),
        "sequence stamps are volatile — the restarted agent's first report is never stale"
    );
}

/// The classifier behind the scrape-tier restore door (its unit
/// cases): a bare, PATH'd, Windows-wrapped, or case-mismatched agent
/// invocation classifies; a shell, an editor editing `claude.md`, a
/// quoted nested command, and no command at all do not.
#[test]
fn agent_of_invocation_names_the_bundled_agents_by_first_token() {
    let engine = crate::mux::scrape::ScrapeEngine::bundled();
    for kind in [
        "claude", "codex", "grok", "muse", "droid", "copilot", "cursor", "kimi", "qwen",
    ] {
        assert_eq!(
            agent_of_invocation(&engine, Some(kind)),
            Some(kind.to_string()),
            "a bare `{kind}` invocation classifies"
        );
    }
    assert_eq!(
        agent_of_invocation(&engine, Some("/usr/local/bin/claude")),
        Some("claude".to_string()),
        "a PATH'd invocation classifies by basename"
    );
    assert_eq!(
        agent_of_invocation(&engine, Some("claude.exe --resume x")),
        Some("claude".to_string()),
        "the Windows wrapper suffix strips before the lookup"
    );
    assert_eq!(
        agent_of_invocation(&engine, Some("Claude")),
        Some("claude".to_string()),
        "the lookup is case-insensitive"
    );
    for invocation in [
        Some("bash"),
        Some("vim claude.md"),
        Some("sh -c 'claude'"),
        Some(""),
        None,
    ] {
        assert_eq!(
            agent_of_invocation(&engine, invocation),
            None,
            "{invocation:?} must not classify"
        );
    }
}

/// A scrape-tier claim saves no identity, so the round trip must still
/// bring the LABEL back: restore re-derives the agent kind from the
/// pane's respawn command under the adopt-pane gate, and the 1s tick
/// rewrites the scraped state from there.
#[test]
fn an_adopted_pane_relabels_from_its_spawn_command_at_restore() {
    let (tree, pane_id) = tree_with_metadata(&[
        ("agent", "claude"),
        ("agent_state", "blocked"),
        ("agent_state_source", "scrape"),
    ]);
    let mut state = tree.to_persist_state();
    let pane_state = &mut state.sessions[0].windows[0].panes[0];
    pane_state.spawn_command = Some("/opt/homebrew/bin/claude".to_string());
    assert!(
        pane_state.agent_session.is_none(),
        "control: a scrape claim saves no identity, so nothing else can restore the label"
    );

    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    let pane = restored.pane(pane_id).expect("restored pane");
    assert_eq!(
        pane.metadata().get("agent").map(String::as_str),
        Some("claude"),
        "restore re-derives the label from the spawn command"
    );
    assert!(
        !pane.metadata().contains_key("agent_state"),
        "the label is restored WITHOUT state — the 1s tick rewrites it"
    );
}

/// The gate is real: a shell invocation stays unlabeled, so a plain
/// shell pane restored after a restart is exactly as absent from the
/// roster as it was before this door existed.
#[test]
fn a_shell_invocation_stays_unlabeled_at_restore() {
    let (tree, pane_id) = tree_with_metadata(&[
        ("agent", "claude"),
        ("agent_state", "blocked"),
        ("agent_state_source", "scrape"),
    ]);
    let mut state = tree.to_persist_state();
    state.sessions[0].windows[0].panes[0].spawn_command = Some("bash -l".to_string());

    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    assert!(
        restored
            .pane(pane_id)
            .expect("restored pane")
            .metadata()
            .get("agent")
            .is_none(),
        "a shell invocation never re-derives an agent label"
    );
}

/// ARC-113c: a PRE-CHANGE state file still loads. The on-disk identity
/// shape is unchanged by the typed claim — the fixture hand-writes the
/// legacy `agent_session` block (argv as a verbatim JSON string,
/// nothing state-shaped) into an otherwise captured state, and the
/// restored pane's claim reads back through the typed view.
#[test]
fn a_pre_change_agent_identity_json_loads_and_reads_typed() {
    let (original, pane_id) = tree_with_agent_pane();
    let mut value = serde_json::to_value(original.to_persist_state()).unwrap();
    value["sessions"][0]["windows"][0]["panes"][0]["agent_session"] = serde_json::json!({
        "agent": "pi",
        "session_id": "s-1",
        "session_path": "/tmp/pi-session.jsonl",
        "source": "par-mux:pi",
        "resume_argv": r#"["pi","--session","/tmp/pi-session.jsonl"]"#
    });
    let json = serde_json::to_string(&value).unwrap();
    assert!(
        json.contains(r#""resume_argv":"[\"pi\",\"--session\",\"/tmp/pi-session.jsonl\"]""#),
        "the on-disk shape must keep argv a JSON string, not an array: {json}"
    );

    let state: PersistState = serde_json::from_str(&json).unwrap();
    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    let pane = restored.pane(pane_id).unwrap();
    assert_eq!(pane.metadata().get("agent").map(String::as_str), Some("pi"));
    assert_eq!(
        pane.metadata().get("agent_session_id").map(String::as_str),
        Some("s-1")
    );
    assert_eq!(
        pane.metadata()
            .get("agent_session_path")
            .map(String::as_str),
        Some("/tmp/pi-session.jsonl")
    );
    assert_eq!(
        pane.metadata().get("agent_source").map(String::as_str),
        Some("par-mux:pi")
    );
    assert_eq!(
        pane.metadata().get("agent_resume_argv").map(String::as_str),
        Some(r#"["pi","--session","/tmp/pi-session.jsonl"]"#),
        "a valid argv lands byte-identical, not normalized"
    );

    let claim = pane.agent_claim().expect("restored identity is a claim");
    assert_eq!(claim.agent, "pi");
    assert_eq!(claim.session_id.as_deref(), Some("s-1"));
    assert_eq!(claim.session_path.as_deref(), Some("/tmp/pi-session.jsonl"));
    assert_eq!(claim.source.as_deref(), Some("par-mux:pi"));
    assert_eq!(
        claim.resume_argv.as_deref(),
        Some(
            &[
                "pi".to_string(),
                "--session".to_string(),
                "/tmp/pi-session.jsonl".to_string()
            ][..]
        )
    );
    assert_eq!(
        claim.state, None,
        "the old file carries nothing state-shaped"
    );
    assert_eq!(claim.seq, None);
    assert_eq!(claim.session_start_source, None);
}

/// The pi/omp shape on the wire today: path-only identity plus the
/// reported invocation, no session id at all.
#[test]
fn pi_shaped_path_only_identity_round_trips() {
    let mut tree = tree();
    let session = tree.new_session("agents", 80, 24).unwrap();
    let pane_id = tree
        .session(session)
        .unwrap()
        .windows
        .iter()
        .filter_map(|window| tree.window(*window))
        .flat_map(|window| window.panes())
        .next()
        .unwrap();
    let pane = tree.pane_mut(pane_id).unwrap();
    pane.set_metadata("agent", "omp");
    pane.set_metadata("agent_session_path", "/tmp/omp-session.jsonl");
    pane.set_metadata(
        "agent_resume_argv",
        r#"["omp","--resume=/tmp/omp-session.jsonl"]"#,
    );

    let state = tree.to_persist_state();
    assert_eq!(
        state.sessions[0].windows[0].panes[0].agent_session,
        Some(PersistAgentSession {
            agent: "omp".to_string(),
            session_id: None,
            session_path: Some("/tmp/omp-session.jsonl".to_string()),
            source: None,
            resume_argv: Some(r#"["omp","--resume=/tmp/omp-session.jsonl"]"#.to_string()),
            state: None,
            state_source: None,
            message: None,
        }),
        "a path-only ref is identity enough — the id-OR-path wire contract"
    );

    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    assert!(
        restored
            .pane(pane_id)
            .unwrap()
            .metadata()
            .contains_key("agent_resume_argv"),
        "the override 6.2 reads survives the restart"
    );
    let expected = if cfg!(windows) {
        "'omp' '--resume=/tmp/omp-session.jsonl'".to_string()
    } else {
        format!("'omp' '--resume=/tmp/omp-session.jsonl'{SURVIVING_TAIL}")
    };
    assert_eq!(
        factory.command_for(pane_id).as_deref(),
        Some(expected.as_str()),
        "restore spawns the reported invocation for the path-only shape too"
    );
}

/// Task 6.3: an agent with NO reported invocation gets the table's argv
/// through the same restore chain.
#[test]
fn restore_spawns_a_table_resume_for_an_agent_without_a_reported_invocation() {
    let (original, pane_id) =
        tree_with_metadata(&[("agent", "claude"), ("agent_session_id", "abc-123")]);
    let state = original.to_persist_state();
    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    let expected = if cfg!(windows) {
        "'claude' '--resume' 'abc-123'".to_string()
    } else {
        format!("'claude' '--resume' 'abc-123'{SURVIVING_TAIL}")
    };
    assert_eq!(
        factory.command_for(pane_id).as_deref(),
        Some(expected.as_str()),
        "the table builds the invocation when nothing was reported"
    );
    assert_eq!(
        restored
            .pane(pane_id)
            .unwrap()
            .metadata()
            .get("agent")
            .map(String::as_str),
        Some("claude"),
        "identity still lands as metadata for the next cycle"
    );
}

/// The `[daemon.agent-append]` table rides along on a table-built
/// resume: the restart-resumed agent gets the flags an `agent-start`
/// would have typed, `{name}` falls back to the pane id when no cwd
/// was persisted, and the pinned form keeps the host's config out.
#[test]
fn restore_appends_the_agent_append_words_onto_the_resume_argv() {
    let (original, pane_id) =
        tree_with_metadata(&[("agent", "claude"), ("agent_session_id", "abc-123")]);
    let state = original.to_persist_state();
    let factory = RecordingFactory::default();
    let table = crate::mux::config::AgentAppend {
        claude: "--continue --dangerously-skip-permissions".into(),
        ..Default::default()
    };
    MuxTree::from_persist_state_with_policy(&state, Box::new(factory.clone()), false, &table)
        .unwrap();
    let invocation = "'claude' '--resume' 'abc-123' '--continue' '--dangerously-skip-permissions'";
    let expected = if cfg!(windows) {
        invocation.to_string()
    } else {
        format!("{invocation}{}", SURVIVING_TAIL)
    };
    assert_eq!(
        factory.command_for(pane_id).as_deref(),
        Some(expected.as_str()),
        "the resume argv carries the configured words, deduped and cwd-free"
    );
}

/// Task 6.3 degradation 1: an agent with no table entry and no report
/// falls back to the pane's original spawn command.
#[test]
fn an_agent_with_no_table_entry_and_no_report_falls_back_to_the_original_command() {
    let mut tree = tree();
    let session = tree.new_session("agents", 80, 24).unwrap();
    let main_window = tree.session(session).unwrap().windows[0];
    let first = tree.window(main_window).unwrap().panes()[0];
    let pane_id = tree
        .split_pane(first, SplitDirection::Vertical, 0.25, Some("sleep 60"))
        .unwrap();
    let pane = tree.pane_mut(pane_id).unwrap();
    pane.set_metadata("agent", "kimi");
    pane.set_metadata("agent_session_id", "kimi-arc-1");

    let state = tree.to_persist_state();
    // Real factory: the fallback command is the original sleep, safe to
    // actually spawn.
    let restored =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
    assert_eq!(
        restored.pane(pane_id).unwrap().spawn_command(),
        Some("sleep 60"),
        "no table entry means fresh-spawn behavior, not an error"
    );
}

/// Task 6.3 degradation 2: a session ref the table cannot use (a path
/// for an id-only agent) falls back the same way.
#[test]
fn a_ref_the_table_cannot_use_falls_back_to_the_original_command() {
    let (original, pane_id) =
        tree_with_metadata(&[("agent", "claude"), ("agent_session_path", "/tmp/s.jsonl")]);
    let state = original.to_persist_state();
    let factory = RecordingFactory::default();
    MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    assert_eq!(
        factory.command_for(pane_id),
        None,
        "claude cannot resume from a path — the pane restores as it was"
    );
}

/// Task 6.3 degradation 3: an agent binary that no longer exists. No
/// probe detects the absence at restore time — the invocation spawns
/// anyway, carrying the surviving tail, so the runtime failure lands
/// in the pane (shell error message, fallback shell) and not in the
/// restore chain.
#[test]
fn an_absent_agent_binary_degrades_inside_the_pane_not_the_restore() {
    let (original, pane_id) = tree_with_metadata(&[
        ("agent", "pi"),
        ("agent_session_id", "s-1"),
        (
            "agent_resume_argv",
            r#"["par-mux-test-no-such-binary","--resume","s-1"]"#,
        ),
    ]);
    let state = original.to_persist_state();
    let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
        .expect("restore completes — an absent binary is the pane's problem");
    let expected = if cfg!(windows) {
        "'par-mux-test-no-such-binary' '--resume' 's-1'".to_string()
    } else {
        format!("'par-mux-test-no-such-binary' '--resume' 's-1'{SURVIVING_TAIL}")
    };
    assert_eq!(
        restored.pane(pane_id).unwrap().spawn_command(),
        Some(expected.as_str()),
        "no probe swapped in a fallback — the failure is contained in the pane"
    );
}

/// The D6.3 promise behind degradation 3: a restored agent pane whose
/// resume invocation exits non-zero must stay ALIVE — the reaper never
/// gets a dead pane, so the restored screen, scrollback, and agent
/// identity survive for a retry or an explicit `kill-pane`. Runs the
/// real spawn path (`sh -c` with the missing binary), unix only.
#[test]
#[cfg(unix)]
fn a_failed_resume_leaves_a_live_pane_with_its_restored_history() {
    let (tree, pane_id) = tree_with_metadata(&[
        ("agent", "pi"),
        ("agent_session_id", "s-1"),
        (
            "agent_resume_argv",
            r#"["par-mux-test-no-such-binary","--resume","s-1"]"#,
        ),
    ]);
    // Markers written straight into the terminal (not through the
    // child): 40 lines of scrollback plus on-screen content that the
    // restore must still carry after the resume fails.
    {
        let terminal = tree.pane(pane_id).unwrap().terminal();
        let mut guard = terminal.write();
        for i in 0..40u16 {
            guard.process(format!("PRE-MARK-{:02}\r\n", i).as_bytes());
        }
        guard.process(b"ON-SCREEN-MARK\r\n");
    }
    let state = tree.to_persist_state();

    // Restore through the REAL factory: the pane's process is
    // `sh -c '<missing binary> ... || { ...; exec shell }'`. sh fails
    // near-instantly (127); the tail decides whether the pane survives.
    let mut restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
        .expect("restore completes");

    // Give the failed exec far longer than `sh` needs to die, requiring
    // the pane running throughout — pre-fix, poll_running() flips false
    // within the first poll and the daemon-side reaper would have
    // persisted the pane's deletion by now. Poll for the fallback's
    // marker instead of sleeping a fixed span: under a parallel test
    // run's process-spawn load, the printf can take seconds to land
    // (QA-133 measured 20/20 solo passes at the old fixed 3s, and a
    // ~1-in-20 failure only under the parallel `mux::` filter). A
    // marker that never arrives still fails below.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut text = String::new();
    loop {
        assert!(
            restored.pane_mut(pane_id).unwrap().poll_running(),
            "the pane must survive its failed resume"
        );
        let snapshot = restored
            .pane(pane_id)
            .unwrap()
            .terminal()
            .read()
            .capture_snapshot();
        text.clear();
        text.extend(snapshot.grid.scrollback_cells.iter().map(|c| c.c));
        text.extend(snapshot.grid.cells.iter().map(|c| c.c));
        if (text.contains("PRE-MARK-00")
            && text.contains("PRE-MARK-39")
            && text.contains("par-mux: agent resume failed"))
            || std::time::Instant::now() >= deadline
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    // The restored history and the fallback's own trace are both still
    // readable (screen or scrollback — the shell's startup bytes may
    // have scrolled the markers up).
    assert!(
        text.contains("PRE-MARK-00") && text.contains("PRE-MARK-39"),
        "restored scrollback survived the failed resume"
    );
    assert!(
        text.contains("par-mux: agent resume failed"),
        "the fallback announced itself in the pane"
    );

    // The agent identity survives the next persist — only an explicit
    // kill-pane (or kill-window) removes it.
    let recaptured = restored.to_persist_state();
    let session = recaptured.sessions[0].windows[0].panes[0]
        .agent_session
        .as_ref()
        .expect("agent identity survived the failed resume");
    assert_eq!(session.agent, "pi");
    assert_eq!(
        session.resume_argv.as_deref(),
        Some(r#"["par-mux-test-no-such-binary","--resume","s-1"]"#)
    );
}

/// The surviving tail covers the CLEAN exit too (card 01a13d03
/// criterion 1): a restored agent that exits 0 must also leave a live
/// pane — the reaper never sees a dead restored pane; only an explicit
/// `kill-pane` removes it. Runs the real spawn path (`sh -c` with
/// `true` as the whole invocation), unix only.
#[test]
#[cfg(unix)]
fn a_clean_agent_exit_also_leaves_a_live_pane() {
    let (tree, pane_id) = tree_with_metadata(&[
        ("agent", "pi"),
        ("agent_session_id", "s-1"),
        ("agent_resume_argv", r#"["true"]"#),
    ]);
    // Markers written straight into the terminal (not through the
    // child): content the restore must still carry after the agent
    // exits cleanly.
    {
        let terminal = tree.pane(pane_id).unwrap().terminal();
        let mut guard = terminal.write();
        guard.process(b"PRE-MARK\r\n");
    }
    let state = tree.to_persist_state();

    // Restore through the REAL factory: the pane's process is
    // `sh -c ''true''; { ...; exec shell }`. `true` exits 0 almost
    // immediately; the tail's clean-exit message decides the pass.
    let mut restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
        .expect("restore completes");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut text = String::new();
    loop {
        assert!(
            restored.pane_mut(pane_id).unwrap().poll_running(),
            "the pane must survive its agent's clean exit"
        );
        let snapshot = restored
            .pane(pane_id)
            .unwrap()
            .terminal()
            .read()
            .capture_snapshot();
        text.clear();
        text.extend(snapshot.grid.scrollback_cells.iter().map(|c| c.c));
        text.extend(snapshot.grid.cells.iter().map(|c| c.c));
        if text.contains("par-mux: agent exited") || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    assert!(
        text.contains("par-mux: agent exited"),
        "the tail announced the clean exit in the pane"
    );
    assert!(
        text.contains("PRE-MARK"),
        "restored scrollback survived the clean exit"
    );

    // The agent identity survives the next persist — only an explicit
    // kill-pane (or kill-window) removes it.
    let recaptured = restored.to_persist_state();
    let session = recaptured.sessions[0].windows[0].panes[0]
        .agent_session
        .as_ref()
        .expect("agent identity survived the clean exit");
    assert_eq!(session.agent, "pi");
    assert_eq!(session.resume_argv.as_deref(), Some(r#"["true"]"#));
}

/// Card 01a0d9b38c98: a Windows restore hands the resume argv to the
/// process verbatim, with no cmd.exe string re-parse in between (the
/// POSIX single-quote rendering made cmd treat `'claude'` as the
/// program name, so every resume failed). The observer is
/// powershell.exe — a PE, so the direct transport — printing each
/// argument between markers; a session path carrying a space, an `&`,
/// and a quote must arrive byte-identical.
#[test]
#[cfg(windows)]
fn a_windows_resume_receives_the_exact_arguments() {
    let script = std::env::temp_dir().join("par-mux-resume-observer.ps1");
    std::fs::write(
        &script,
        "foreach ($a in $args) { Write-Output \"ARG<$a>\" }\r\n",
    )
    .unwrap();
    let tricky = "C:\\my sessions\\s 1 & continue's.txt";
    // Serialize with serde_json — hand-splicing Windows paths into a
    // JSON literal produces invalid \-escapes, which the resume chain
    // would (correctly) reject back to the table.
    let argv = serde_json::to_string(&vec![
        "powershell.exe".to_string(),
        "-NoProfile".to_string(),
        // The default policy on a stock Windows blocks .ps1 files.
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-File".to_string(),
        script.display().to_string(),
        tricky.to_string(),
        "plain".to_string(),
    ])
    .unwrap();
    let (tree, pane_id) = tree_with_metadata(&[
        ("agent", "pi"),
        ("agent_session_id", "s-1"),
        ("agent_resume_argv", &argv),
    ]);
    let state = tree.to_persist_state();
    let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
        .expect("restore completes");

    // powershell's first start under a fresh ConPTY can take seconds.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    let text = loop {
        let snapshot = restored
            .pane(pane_id)
            .unwrap()
            .terminal()
            .read()
            .capture_snapshot();
        let mut text: String = snapshot.grid.scrollback_cells.iter().map(|c| c.c).collect();
        text.extend(snapshot.grid.cells.iter().map(|c| c.c));
        if text.contains("ARG<plain>") || std::time::Instant::now() >= deadline {
            break text;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    };
    assert!(
        text.contains(&format!("ARG<{tricky}>")),
        "the tricky argument arrived byte-identical, saw: {text:?}"
    );
    assert!(
        text.contains("ARG<plain>"),
        "every argument arrived, saw: {text:?}"
    );
    let _ = std::fs::remove_file(&script);
}

/// Task 6.3 criterion 3: a non-agent pane restores byte-identically to
/// Phase 3 — same spawn command through the real factory, and the
/// restore invents no agent session.
#[test]
fn non_agent_panes_restore_through_the_unchanged_phase3_path() {
    let original = populated_tree();
    let state = original.to_persist_state();
    let restored =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
    assert_same_shape(&restored, &original);
    let recaptured = restored.to_persist_state();
    assert!(recaptured
        .sessions
        .iter()
        .flat_map(|session| session.windows.iter())
        .flat_map(|window| window.panes.iter())
        .all(|pane| pane.agent_session.is_none()));
}

#[test]
fn panes_without_resumable_identity_serialize_agent_session_none() {
    // A non-agent pane…
    let bare = populated_tree();
    let state = bare.to_persist_state();
    let persisted_panes: Vec<&PersistPane> = state
        .sessions
        .iter()
        .flat_map(|s| s.windows.iter().flat_map(|w| w.panes.iter()))
        .collect();
    assert!(!persisted_panes.is_empty());
    assert!(
        persisted_panes
            .iter()
            .all(|pane| pane.agent_session.is_none()),
        "no metadata, no agent_session"
    );

    // …and a pane hook-claimed for STATE but carrying no session —
    // an agent label alone is nothing the resume path can use.
    let mut tree = tree();
    let session = tree.new_session("agents", 80, 24).unwrap();
    let pane_id = tree
        .session(session)
        .unwrap()
        .windows
        .iter()
        .filter_map(|window| tree.window(*window))
        .flat_map(|window| window.panes())
        .next()
        .unwrap();
    tree.pane_mut(pane_id)
        .unwrap()
        .set_metadata("agent", "kimi");
    let state = tree.to_persist_state();
    assert_eq!(
        state.sessions[0].windows[0].panes[0].agent_session, None,
        "label-only panes carry nothing to resume"
    );
}

/// The version this build just superseded is refused exactly like any
/// unknown one — the bump must be a boundary, not a silent partial read.
#[test]
fn previous_format_version_is_refused() {
    let original = populated_tree();
    let mut state = original.to_persist_state();
    state.format_version = FORMAT_VERSION - 1;

    let result = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()));
    assert!(
        matches!(
            result,
            Err(PersistError::UnsupportedVersion { found, supported })
                if found == FORMAT_VERSION - 1 && supported == FORMAT_VERSION
        ),
        "a v1 file must be refused, not partially read as v2"
    );
}

/// A pane created with the restore-start-command option keeps its start
/// command across a restart even after its agent reported a session —
/// the hook-reported argv and the per-agent table both lose — and the
/// reported session id reaches that command as `PAR_MUX_RESUME_SESSION_ID`
/// so a wrapper can pass `--resume` itself. The identity still rides back,
/// so the roster keeps the pane.
#[test]
fn a_flagged_pane_restores_via_its_start_command_with_the_resume_session_id() {
    let (mut tree, pane_id) =
        tree_with_metadata(&[("agent", "claude"), ("agent_session_id", "s-9")]);
    tree.pane_mut(pane_id)
        .unwrap()
        .set_restore_start_command(true);
    let mut state = tree.to_persist_state();
    state.sessions[0].windows[0].panes[0].spawn_command =
        Some("claude-wrapper --model sonnet --plugin-dir /opt/mod".to_string());

    let factory = RecordingFactory::default();
    let restored = MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    assert_eq!(
        factory.command_for(pane_id).as_deref(),
        Some("claude-wrapper --model sonnet --plugin-dir /opt/mod"),
        "the start command re-runs with every flag, not the resume shape"
    );
    assert_eq!(
        factory.resume_session_id_for(pane_id).as_deref(),
        Some("s-9"),
        "the hook-reported id reaches the start command as PAR_MUX_RESUME_SESSION_ID"
    );
    let claim = restored
        .pane(pane_id)
        .unwrap()
        .agent_claim()
        .expect("the identity still rides back");
    assert_eq!(
        claim.session_id.as_deref(),
        Some("s-9"),
        "the roster keeps the reported session"
    );
}

/// The workspace-level `start-command` policy does for every pane in the
/// workspace what the per-pane flag does for one, with no flag set.
#[test]
fn the_workspace_start_command_policy_restores_agent_panes_via_their_start_commands() {
    let (tree, pane_id) = tree_with_metadata(&[("agent", "claude"), ("agent_session_id", "s-1")]);
    let mut state = tree.to_persist_state();
    state.sessions[0].windows[0].panes[0].spawn_command = Some("bash -l".to_string());
    state.workspaces[0].restore_policy = Some("start-command".to_string());

    let factory = RecordingFactory::default();
    MuxTree::from_persist_state(&state, Box::new(factory.clone())).unwrap();
    assert_eq!(
        factory.command_for(pane_id).as_deref(),
        Some("bash -l"),
        "the workspace policy re-runs the start command, not a resume"
    );
}

/// `none` disables restore of the workspace's panes entirely: their
/// sessions never rebuild, the workspace itself restores empty (the same
/// state `new-workspace` creates), and sibling workspaces are untouched.
#[test]
fn the_workspace_none_policy_drops_its_panes_and_keeps_the_workspace() {
    let mut tree = tree();
    let keep = tree.new_session("keep", 80, 24).unwrap();
    let drop_workspace = tree.new_workspace("drop");
    let drop = tree.new_session("drop-agent", 80, 24).unwrap();
    let drop_pane = tree
        .session(drop)
        .unwrap()
        .windows
        .iter()
        .filter_map(|w| tree.window(*w))
        .flat_map(|w| w.panes())
        .next()
        .unwrap();
    let mut state = tree.to_persist_state();
    let workspace = state
        .workspaces
        .iter_mut()
        .find(|w| w.id == drop_workspace.0)
        .expect("the drop workspace was captured");
    workspace.restore_policy = Some("none".to_string());

    let restored =
        MuxTree::from_persist_state(&state, Box::new(RecordingFactory::default())).unwrap();
    assert!(
        restored.session(drop).is_none(),
        "the none workspace's session never restores"
    );
    assert!(restored.pane(drop_pane).is_none());
    assert!(restored.session(keep).is_some(), "siblings are untouched");
    assert!(
        restored
            .workspaces
            .values()
            .any(|w| w.name == "drop" && w.sessions.is_empty()),
        "the workspace itself survives, empty"
    );
    tree_consistency(&restored);
}

/// The workspace policy and the pane flag are part of the saved state:
/// the tree setter lands them on the wire, the JSON round trip keeps
/// them, and a pre-flag state file (fields stripped, the shape an older
/// daemon writes) loads with the defaults.
#[test]
fn restore_policy_and_the_pane_flag_round_trip_through_the_state_file() {
    let mut tree = tree();
    let workspace = tree.new_workspace("policy");
    let session = tree.new_session("policy", 80, 24).unwrap();
    let pane_id = tree
        .session(session)
        .unwrap()
        .windows
        .iter()
        .filter_map(|w| tree.window(*w))
        .flat_map(|w| w.panes())
        .next()
        .unwrap();
    tree.set_workspace_restore_policy(
        workspace,
        Some(crate::mux::config::RestorePolicy::StartCommand),
    )
    .unwrap();
    tree.pane_mut(pane_id)
        .unwrap()
        .set_restore_start_command(true);

    let state = tree.to_persist_state();
    let saved_workspace = state
        .workspaces
        .iter()
        .find(|w| w.id == workspace.0)
        .expect("the policy workspace was captured");
    assert_eq!(
        saved_workspace.restore_policy.as_deref(),
        Some("start-command"),
        "the workspace policy is on the wire"
    );
    assert!(
        state.sessions[0].windows[0].panes[0].restore_start_command,
        "the pane flag is on the wire"
    );

    let json = serde_json::to_string(&state).unwrap();
    let state: PersistState = serde_json::from_str(&json).unwrap();
    assert_eq!(
        state.workspaces[0].restore_policy.as_deref(),
        Some("start-command"),
        "the policy survives the JSON round trip"
    );
    assert!(
        state.sessions[0].windows[0].panes[0].restore_start_command,
        "the flag survives the JSON round trip"
    );

    // A state file written before the fields existed (an older daemon's
    // shape) loads with the defaults — serde default, not an error.
    let mut value = serde_json::to_value(&state).unwrap();
    value["workspaces"][0]
        .as_object_mut()
        .unwrap()
        .remove("restore_policy");
    value["sessions"][0]["windows"][0]["panes"][0]
        .as_object_mut()
        .unwrap()
        .remove("restore_start_command");
    let state: PersistState = serde_json::from_value(value).unwrap();
    assert_eq!(state.workspaces[0].restore_policy, None);
    assert!(!state.sessions[0].windows[0].panes[0].restore_start_command);
}
