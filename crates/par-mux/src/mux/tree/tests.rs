use super::*;
use crate::mux::layout::{PaneChrome, ResizeDirection};
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

/// Card 01a0d9e6f012, criterion 2: an OSC 11 query inside a pane
/// answers with the CLIENT's theme background — what the client
/// actually renders — once `set-client-colors` has reported it, and a
/// pane created later inherits the same answer.
#[test]
fn client_colors_answer_osc_queries_in_existing_and_later_panes() {
    let mut tree = tree();
    let session = tree.new_session("work", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let first = tree.window(window).unwrap().panes()[0];

    fn osc_11_reply(tree: &MuxTree, pane: PaneId) -> String {
        let pane = tree.pane(pane).unwrap();
        let terminal = pane.terminal();
        let mut term = terminal.write();
        term.process(b"\x1b]11;?\x1b\\");
        String::from_utf8(term.drain_responses()).unwrap()
    }

    let before = osc_11_reply(&tree, first);
    assert!(
        !before.contains("2e2e3e1e1e") && !before.contains("1e1e/2e2e"),
        "pre-report: the core theme answers, not the client's"
    );

    tree.set_client_colors(None, Some(Color::Rgb(0x1e, 0x1e, 0x2e)));
    let after = osc_11_reply(&tree, first);
    assert!(
        after.contains("rgb:1e1e/1e1e/2e2e"),
        "OSC 11 answers with the client bg: {after}"
    );

    let later_window = tree.new_window(session, "w2", 80, 24).unwrap();
    let later = tree.window(later_window).unwrap().panes()[0];
    let inherited = osc_11_reply(&tree, later);
    assert!(
        inherited.contains("rgb:1e1e/1e1e/2e2e"),
        "a pane created after the report inherits it: {inherited}"
    );
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
    // An inline kill cannot return before SIGKILL and the reap, so a child
    // still present at return proves the kill was detached however slowly
    // a loaded machine ran the call. The wall-clock bound covers the rare
    // case where the detached thread outran the sample.
    let present_at_return = unsafe { libc::kill(pid as libc::pid_t, 0) == 0 };
    assert!(
        present_at_return || elapsed < std::time::Duration::from_millis(150),
        "kill_pane held the tree lock through the SIGHUP grace poll: {elapsed:?}, \
         child already reaped at return"
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
fn resize_pane_reaches_a_cross_orientation_ancestor() {
    // The manual-pass report: a pane nested under a stacked split inside
    // a side-by-side root could only resize along its own split's axis.
    // tmux semantics — the nearest same-axis ancestor at any depth moves.
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let left = tree.window(window_id).unwrap().panes()[0];
    tree.split_pane(left, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    let right_top = tree.window(window_id).unwrap().panes()[1];
    tree.split_pane(right_top, SplitDirection::Horizontal, 0.5, None)
        .unwrap();

    // The top-right pane's own split is stacked; -R must reach the root's
    // side-by-side divider (widths 40/40 -> 30/50 — the right stack grows
    // into the left pane's space).
    tree.resize_pane(right_top, ResizeDirection::Right, 10)
        .expect("the cross-orientation ancestor absorbs the growth");
    let window = tree.window(window_id).unwrap();
    let geo = window
        .layout
        .geometry(0, 0, window.cols as usize, window.rows as usize);
    let rect = |pane| geo.iter().find(|g| g.pane == pane).unwrap();
    assert_eq!(rect(left).width, 30);
    assert_eq!(rect(right_top).width, 50);
    let right_bottom = tree.window(window_id).unwrap().panes()[2];
    assert_eq!(rect(right_bottom).width, 50);
    assert_eq!(rect(right_top).height, 12);

    // Its own stacked split still handles the height axis: -U 2 shrinks
    // the top pane (heights 12/12 -> 10/14).
    tree.resize_pane(right_top, ResizeDirection::Up, 2).unwrap();
    let window = tree.window(window_id).unwrap();
    let geo = window
        .layout
        .geometry(0, 0, window.cols as usize, window.rows as usize);
    let rect = |pane| geo.iter().find(|g| g.pane == pane).unwrap();
    assert_eq!(rect(right_top).height, 10);
    assert_eq!(rect(right_bottom).height, 14);
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

/// `resize-pane -Z`: two panes 40x24 each; zooming takes the full
/// window grid while the hidden pane keeps its size, and unzooming
/// restores the exact prior extent — the zoom never edits the layout.
#[test]
fn zoom_toggles_full_grid_and_restores_exactly() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    let second = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (40, 24));

    tree.zoom_pane(first).unwrap();
    assert_eq!(tree.window(window_id).unwrap().zoomed, Some(first));
    assert_eq!(
        tree.pane(first).unwrap().terminal().read().size(),
        (80, 24),
        "the zoomed pane takes the full window grid"
    );
    assert_eq!(
        tree.pane(second).unwrap().terminal().read().size(),
        (40, 24),
        "the hidden pane keeps its size"
    );

    tree.zoom_pane(first).unwrap();
    assert_eq!(tree.window(window_id).unwrap().zoomed, None);
    assert_eq!(
        tree.pane(first).unwrap().terminal().read().size(),
        (40, 24),
        "unzoom restores the exact prior extent"
    );
}

/// Zooming a different pane moves the zoom to it (tmux semantics).
#[test]
fn zoom_moves_to_another_pane() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    let second = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();

    tree.zoom_pane(first).unwrap();
    tree.zoom_pane(second).unwrap();

    assert_eq!(tree.window(window_id).unwrap().zoomed, Some(second));
    assert_eq!(
        tree.pane(second).unwrap().terminal().read().size(),
        (80, 24)
    );
    assert_eq!(
        tree.pane(first).unwrap().terminal().read().size(),
        (40, 24),
        "the previous zoom target returns to its layout cell"
    );
}

/// A window resize while zoomed re-fits the zoomed pane to the NEW
/// full grid.
#[test]
fn window_resize_while_zoomed_follows_the_new_grid() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    tree.split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();

    tree.zoom_pane(first).unwrap();
    tree.resize_window(window_id, 100, 30).unwrap();

    assert_eq!(
        tree.pane(first).unwrap().terminal().read().size(),
        (100, 30),
        "the zoom tracks the window's new extent"
    );
}

/// Every layout mutation ends the zoom: split, kill, swap, and
/// select-pane to another pane unzoom first; selecting the zoomed
/// pane itself keeps it (tmux's rule).
#[test]
fn layout_mutations_unzoom_the_window() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    let second = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();

    // split-window: the new pane must be visible.
    tree.zoom_pane(first).unwrap();
    let third = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    assert_eq!(tree.window(window_id).unwrap().zoomed, None);
    let geo = tree
        .window(window_id)
        .unwrap()
        .layout
        .geometry(0, 0, 80, 24);
    let width_of = |pane| geo.iter().find(|g| g.pane == pane).unwrap().width;
    assert_eq!(
        tree.pane(first).unwrap().terminal().read().size(),
        (width_of(first), 24),
        "the split target returns to its layout cell"
    );

    // kill-pane of a non-zoomed pane still changes the layout.
    tree.zoom_pane(first).unwrap();
    tree.kill_pane(third).unwrap();
    assert_eq!(tree.window(window_id).unwrap().zoomed, None);

    // kill-pane of the zoomed pane itself.
    tree.zoom_pane(second).unwrap();
    tree.kill_pane(second).unwrap();
    assert_eq!(tree.window(window_id).unwrap().zoomed, None);

    // swap-pane: the traded geometry no longer matches the zoom.
    let second = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    tree.zoom_pane(first).unwrap();
    tree.swap_panes(first, second).unwrap();
    assert_eq!(tree.window(window_id).unwrap().zoomed, None);

    // select-pane to another pane reveals the layout; to the zoomed
    // pane itself keeps the zoom.
    tree.zoom_pane(first).unwrap();
    tree.select_pane(second).unwrap();
    assert_eq!(tree.window(window_id).unwrap().zoomed, None);
    assert_eq!(
        tree.pane(first).unwrap().terminal().read().size(),
        (40, 24),
        "unzoom on select restores the pane's layout cell"
    );
    tree.zoom_pane(first).unwrap();
    tree.select_pane(first).unwrap();
    assert_eq!(tree.window(window_id).unwrap().zoomed, Some(first));
}

/// Two 40-wide panes side by side in an 80x24 window, `first` zoomed.
fn zoomed_split() -> (MuxTree, WindowId, PaneId, PaneId) {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    let second = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    tree.zoom_pane(first).unwrap();
    (tree, window_id, first, second)
}

/// ARC-090 audit probe: `resize-pane -x` while zoomed must unzoom
/// first (tmux's `server_unzoom_window`), not silently rewrite the
/// hidden layout so the next unzoom lands on 79/1.
#[test]
fn absolute_resize_while_zoomed_unzooms_first() {
    let (mut tree, window_id, first, second) = zoomed_split();

    tree.resize_pane_absolute(first, Some(79), None).unwrap();

    assert_eq!(tree.window(window_id).unwrap().zoomed, None);
    assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (79, 24));
    assert_eq!(tree.pane(second).unwrap().terminal().read().size(), (1, 24));
}

#[test]
fn relative_resize_while_zoomed_unzooms_first() {
    let (mut tree, window_id, first, second) = zoomed_split();

    tree.resize_pane(first, ResizeDirection::Right, 5).unwrap();

    assert_eq!(tree.window(window_id).unwrap().zoomed, None);
    assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (45, 24));
    assert_eq!(
        tree.pane(second).unwrap().terminal().read().size(),
        (35, 24)
    );
}

/// A failed command changes nothing: a wrong-axis resize keeps the
/// zoom and the zoomed pane's full-grid size.
#[test]
fn a_rejected_resize_keeps_the_zoom() {
    let (mut tree, window_id, first, _second) = zoomed_split();

    let relative = tree.resize_pane(first, ResizeDirection::Up, 1);
    assert!(matches!(relative, Err(MuxError::PaneNotResizable(_))));
    let absolute = tree.resize_pane_absolute(first, None, Some(10));
    assert!(matches!(absolute, Err(MuxError::PaneNotResizable(_))));

    assert_eq!(tree.window(window_id).unwrap().zoomed, Some(first));
    assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (80, 24));
}

/// `-x 60 -y 10` where the pane spans the vertical axis: the y half
/// fails, so the x half must not be left applied (all-or-nothing).
#[test]
fn a_half_failing_absolute_resize_applies_neither_axis() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    let second = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    let before = tree.window(window_id).unwrap().layout.clone();

    let result = tree.resize_pane_absolute(first, Some(60), Some(10));

    assert!(matches!(result, Err(MuxError::PaneNotResizable(_))));
    assert_eq!(tree.window(window_id).unwrap().layout, before);
    assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (40, 24));
    assert_eq!(
        tree.pane(second).unwrap().terminal().read().size(),
        (40, 24)
    );
}

#[test]
fn zoom_rejects_an_unknown_pane() {
    let mut tree = tree();
    let result = tree.zoom_pane(PaneId(9999));
    assert!(matches!(result, Err(MuxError::NoSuchPane(_))));
}

/// `break-pane` + `join-pane` round trip: breaking a pane out of a
/// two-pane window gives it a new full-grid window (the session's
/// active one) while the survivor re-fits; joining it back beside
/// the survivor restores the two-pane layout and closes the
/// one-pane window it leaves behind.
#[test]
fn break_then_join_restores_two_panes() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let first_window = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(first_window).unwrap().panes()[0];
    let second = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (40, 24));

    let (new_window, source, source_closed) = tree.break_pane(first, "broken").unwrap();
    assert_eq!(source, first_window);
    assert!(!source_closed, "the source window keeps its other pane");
    assert_eq!(
        tree.session(session_id).unwrap().windows,
        vec![first_window, new_window],
        "the new window is appended to the session"
    );
    assert_eq!(tree.session(session_id).unwrap().active, 1);
    assert_eq!(tree.window(new_window).unwrap().name, "broken");
    assert_eq!(tree.window(new_window).unwrap().panes(), vec![first]);
    assert_eq!(
        tree.pane(first).unwrap().terminal().read().size(),
        (80, 24),
        "the broken pane takes the new window's full grid"
    );
    assert_eq!(
        tree.pane(second).unwrap().terminal().read().size(),
        (80, 24),
        "the survivor grows into the freed extent"
    );

    let (dest, src, closed, removed) = tree
        .join_pane(first, second, SplitDirection::Vertical, 0.5)
        .unwrap();
    assert_eq!(dest, first_window);
    assert_eq!(src, new_window);
    assert!(
        closed,
        "the break's one-pane window closed when its pane left"
    );
    assert_eq!(removed, None, "the session kept the destination window");
    // The split machinery puts the target first and the moved pane
    // second, exactly like a fresh split of the survivor.
    assert_eq!(
        tree.window(first_window).unwrap().panes(),
        vec![second, first]
    );
    assert_eq!(
        tree.pane(first).unwrap().terminal().read().size(),
        (40, 24),
        "the rejoined pane returns to its half"
    );
    assert_eq!(
        tree.session(session_id).unwrap().windows,
        vec![first_window],
        "the closed window left the session list"
    );
}

/// Breaking a window's only pane moves the window instead of killing
/// the session — the new window exists before the source drops.
#[test]
fn breaking_the_only_pane_closes_the_source_not_the_session() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let first_window = tree.session(session_id).unwrap().windows[0];
    let only = tree.window(first_window).unwrap().panes()[0];

    let (new_window, source, source_closed) = tree.break_pane(only, "solo").unwrap();
    assert!(source_closed, "the emptied source window closed");
    assert_eq!(source, first_window);
    assert!(tree.window(first_window).is_none());
    assert!(tree.session(session_id).is_some(), "the session survives");
    assert_eq!(
        tree.session(session_id).unwrap().windows,
        vec![new_window],
        "the pane's new window replaced the source in the list"
    );
    assert_eq!(tree.window(new_window).unwrap().panes(), vec![only]);
}

/// Joining the last pane out of a session's only window closes that
/// session — the destination belongs to the target's window, so
/// nothing backstops the source's session (the mirror image of
/// break-pane's guarantee).
#[test]
fn joining_the_last_pane_out_of_the_only_window_closes_its_session() {
    let mut tree = tree();
    let donor = tree.new_session("donor", 80, 24).unwrap();
    let donor_window = tree.session(donor).unwrap().windows[0];
    let mover = tree.window(donor_window).unwrap().panes()[0];
    let keeper = tree.new_session("keeper", 80, 24).unwrap();
    let keeper_window = tree.session(keeper).unwrap().windows[0];
    let anchor = tree.window(keeper_window).unwrap().panes()[0];

    let (dest, src, closed, removed) = tree
        .join_pane(mover, anchor, SplitDirection::Horizontal, 0.5)
        .unwrap();
    assert_eq!(dest, keeper_window);
    assert_eq!(src, donor_window);
    assert!(closed);
    assert_eq!(removed, Some(donor), "the donor session closed");
    assert!(tree.session(donor).is_none());
    assert!(tree.window(donor_window).is_none());
    // The moved pane landed next to the anchor, below it.
    assert_eq!(
        tree.window(keeper_window).unwrap().panes(),
        vec![anchor, mover]
    );
}

/// `join-pane` rejects a pane onto itself and unknown panes, and a
/// same-window join is a within-window move, not an error.
#[test]
fn join_pane_rejects_self_and_unknown_but_allows_same_window() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    let second = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();

    assert!(matches!(
        tree.join_pane(first, first, SplitDirection::Vertical, 0.5),
        Err(MuxError::SamePane(_first))
    ));
    assert!(matches!(
        tree.join_pane(PaneId(9999), first, SplitDirection::Vertical, 0.5),
        Err(MuxError::NoSuchPane(_))
    ));

    // Same window: first moves below second (target first, moved
    // pane second), and the window keeps both panes.
    let (dest, src, closed, removed) = tree
        .join_pane(first, second, SplitDirection::Horizontal, 0.5)
        .unwrap();
    assert_eq!((dest, src), (window_id, window_id));
    assert!(!closed);
    assert_eq!(removed, None);
    assert_eq!(tree.window(window_id).unwrap().panes(), vec![second, first]);
}

/// Break and join are layout mutations — a zoomed window unzooms.
#[test]
fn break_and_join_end_a_zoom() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let first_window = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(first_window).unwrap().panes()[0];
    let second = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    let other_window = tree.new_window(session_id, "other", 80, 24).unwrap();
    let other = tree.window(other_window).unwrap().panes()[0];

    tree.zoom_pane(second).unwrap();
    let (new_window, _, _) = tree.break_pane(second, "zoomed").unwrap();
    assert_eq!(tree.window(new_window).unwrap().zoomed, None);

    tree.zoom_pane(other).unwrap();
    tree.join_pane(other, first, SplitDirection::Vertical, 0.5)
        .unwrap();
    assert_eq!(tree.window(first_window).unwrap().zoomed, None);

    // A same-window join is one mutation: it ends the zoom and leaves
    // every pane fitted to the final layout.
    tree.zoom_pane(first).unwrap();
    tree.join_pane(first, other, SplitDirection::Horizontal, 0.5)
        .unwrap();
    let window = tree.window(first_window).unwrap();
    assert_eq!(window.zoomed, None);
    for geometry in window.layout.geometry(0, 0, 80, 24) {
        assert_eq!(
            tree.pane(geometry.pane).unwrap().terminal().read().size(),
            (geometry.width, geometry.height),
            "{} fits its layout cell",
            geometry.pane
        );
    }
}

/// ARC-096: the pane → window and window → session indexes match a
/// brute-force scan after every structural command.
#[test]
fn reverse_indexes_track_every_structural_command() {
    let mut tree = tree();
    let check = |tree: &MuxTree| tree.assert_indexes_consistent();

    let s0 = tree.new_session("a", 80, 24).unwrap();
    check(&tree);
    let w0 = tree.session(s0).unwrap().windows[0];
    let p0 = tree.window(w0).unwrap().panes()[0];
    let p1 = tree
        .split_pane(p0, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    check(&tree);
    let p2 = tree
        .split_pane(p1, SplitDirection::Horizontal, 0.5, None)
        .unwrap();
    check(&tree);
    assert_eq!(tree.window_of_pane(p2), Some(w0));

    tree.swap_panes(p0, p2).unwrap();
    check(&tree);

    let w1 = tree.new_window(s0, "b", 80, 24).unwrap();
    check(&tree);
    assert_eq!(tree.session_of_window(w1), Some(s0));

    // break-pane: p2 moves to a fresh window in the same session.
    let (w2, _, closed) = tree.break_pane(p2, "broken").unwrap();
    check(&tree);
    assert!(!closed);
    assert_eq!(tree.window_of_pane(p2), Some(w2));

    // join-pane across windows; w2 empties and closes.
    let (dest, _, closed, _) = tree
        .join_pane(p2, p0, SplitDirection::Vertical, 0.5)
        .unwrap();
    check(&tree);
    assert!(closed);
    assert_eq!(dest, w0);
    assert_eq!(tree.window_of_pane(p2), Some(w0));
    assert_eq!(tree.session_of_window(w2), None);

    tree.move_window(w1, 0).unwrap();
    check(&tree);
    tree.swap_windows(w0, w1).unwrap();
    check(&tree);

    // respawn swaps the pane in place under the same id.
    let factory = tree.factory();
    let plan = tree.begin_respawn(p1, true, None, None).unwrap();
    let respawned = factory
        .create_pane(plan.pane_id, plan.cols, plan.rows, None, &plan.context())
        .unwrap();
    tree.complete_respawn(plan, respawned).unwrap();
    check(&tree);
    assert_eq!(tree.window_of_pane(p1), Some(w0));

    // kill-pane of a non-last pane, then of a window's last pane.
    tree.kill_pane(p2).unwrap();
    check(&tree);
    assert_eq!(tree.window_of_pane(p2), None);
    let w1_pane = tree.window(w1).unwrap().panes()[0];
    let (_, removed) = tree.kill_pane(w1_pane).unwrap();
    check(&tree);
    assert_eq!(removed, None, "w0 keeps the session alive");
    assert_eq!(tree.session_of_window(w1), None);

    let s1 = tree.new_session("c", 80, 24).unwrap();
    let w3 = tree.new_window(s1, "d", 80, 24).unwrap();
    check(&tree);
    assert_eq!(tree.kill_window(w3).unwrap(), None);
    check(&tree);
    tree.kill_session(s1).unwrap();
    check(&tree);
    assert!(tree.session(s1).is_none());

    // Last window of the last session: the cascade clears everything.
    assert_eq!(tree.kill_window(w0).unwrap(), Some(s0));
    check(&tree);
    assert!(tree.pane_window.is_empty() && tree.window_session.is_empty());
}

/// `move-window` reorders the session's window list, clamps
/// out-of-range positions, and keeps the active window active by
/// identity rather than index.
#[test]
fn move_window_reorders_and_keeps_the_active_window() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let w0 = tree.session(session_id).unwrap().windows[0];
    let w1 = tree.new_window(session_id, "one", 80, 24).unwrap();
    let w2 = tree.new_window(session_id, "two", 80, 24).unwrap();
    assert_eq!(tree.session(session_id).unwrap().windows, vec![w0, w1, w2]);
    assert_eq!(tree.session(session_id).unwrap().active, 0);

    tree.move_window(w2, 0).unwrap();
    assert_eq!(tree.session(session_id).unwrap().windows, vec![w2, w0, w1]);
    assert_eq!(
        tree.session(session_id).unwrap().windows[tree.session(session_id).unwrap().active],
        w0,
        "the active window stayed active through the move"
    );

    // Out-of-range clamps to the end.
    tree.move_window(w0, 99).unwrap();
    assert_eq!(tree.session(session_id).unwrap().windows, vec![w2, w1, w0]);
    assert_eq!(
        tree.session(session_id).unwrap().windows[tree.session(session_id).unwrap().active],
        w0
    );

    assert!(matches!(
        tree.move_window(WindowId(9999), 0),
        Err(MuxError::NoSuchWindow(_))
    ));
}

/// `swap-window` exchanges two windows' positions in their shared
/// session and refuses windows in different sessions.
#[test]
fn swap_windows_exchanges_positions_within_a_session() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let w0 = tree.session(session_id).unwrap().windows[0];
    let w1 = tree.new_window(session_id, "one", 80, 24).unwrap();
    let w2 = tree.new_window(session_id, "two", 80, 24).unwrap();

    tree.swap_windows(w0, w2).unwrap();
    assert_eq!(tree.session(session_id).unwrap().windows, vec![w2, w1, w0]);
    // A self-swap is a no-op, not an error.
    tree.swap_windows(w1, w1).unwrap();
    assert_eq!(tree.session(session_id).unwrap().windows, vec![w2, w1, w0]);

    let other_session = tree.new_session("other", 80, 24).unwrap();
    let other_window = tree.session(other_session).unwrap().windows[0];
    assert!(matches!(
        tree.swap_windows(w0, other_window),
        Err(MuxError::WindowsInDifferentSessions(_, _))
    ));
}

/// Window order is session state — a persist round trip keeps the
/// reordered list (the restore path par-mux's restart runs).
#[test]
fn window_order_survives_a_persist_round_trip() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let w0 = tree.session(session_id).unwrap().windows[0];
    let w1 = tree.new_window(session_id, "one", 80, 24).unwrap();
    let w2 = tree.new_window(session_id, "two", 80, 24).unwrap();
    tree.move_window(w2, 0).unwrap();
    tree.swap_windows(w1, w0).unwrap();
    let order = tree.session(session_id).unwrap().windows.clone();
    assert_eq!(order, vec![w2, w1, w0]);

    let state = tree.to_persist_state();
    let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
        .expect("state rebuilds");
    assert_eq!(restored.session(session_id).unwrap().windows, order);
}

/// `respawn-pane`: a live pane refuses without `kill`; a dead pane
/// restarts in place — same id, window, and layout, the user title
/// carried to the replacement, a live process again.
#[test]
fn respawn_restarts_a_dead_pane_in_place() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let pane = tree.window(window_id).unwrap().panes()[0];
    tree.pane_mut(pane).unwrap().set_user_title("kept");

    // Live process: refused without -k.
    assert!(matches!(
        tree.begin_respawn(pane, false, None, None),
        Err(MuxError::PaneAlive(_pane))
    ));

    // Kill the process the way the reaper observes it: type exit,
    // poll until the OS agrees, mark the death.
    //
    // Enter is `\r`, not `\n`: on newer conhost builds (measured on
    // the Windows-26200 VM: both writes echoed, `exit 0exit 0` on one
    // line, never executed) a `\n`-terminated line is echoed by
    // cmd.exe but never submitted. windows CI runners were straddling
    // the 26100→26200 image rollout, which made this intermittent
    // (run 36601091589). Every other typed-line path in the suite
    // (mux_factory's `type_line`, the daemon's send-keys Enter)
    // already sends `\r`.
    tree.pane_mut(pane).unwrap().write(b"exit 0\r").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while tree.pane_mut(pane).unwrap().poll_running() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let still_running = tree.pane_mut(pane).unwrap().poll_running();
    let exit = tree.pane(pane).unwrap().exit_code();
    let screen_tail: String = tree
        .pane(pane)
        .unwrap()
        .terminal()
        .read()
        .content()
        .chars()
        .rev()
        .take(200)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    assert!(
        !still_running,
        "the shell must exit on `exit 0`; exit_code={exit:?}, screen tail \
         (an echoed command proves delivery, an absent one a dropped \
         write): {screen_tail:?}"
    );
    tree.pane_mut(pane).unwrap().mark_dead();
    assert_eq!(tree.pane(pane).unwrap().exit_code(), Some(0));
    assert!(tree.all_panes_dead());

    // Respawn with the stored command: same pane id, title carried,
    // layout untouched, a live process again.
    let factory = tree.factory();
    let plan = tree.begin_respawn(pane, false, None, None).unwrap();
    assert_eq!(plan.pane_id, pane);
    let respawned = factory
        .create_pane(
            plan.pane_id,
            plan.cols,
            plan.rows,
            plan.command.as_deref(),
            &plan.context(),
        )
        .expect("respawn spawns");
    tree.complete_respawn(plan, respawned).unwrap();
    assert_eq!(tree.window(window_id).unwrap().panes(), vec![pane]);
    assert_eq!(tree.pane(pane).unwrap().user_title(), Some("kept"));
    assert!(tree.pane(pane).unwrap().is_running());
    assert!(!tree.all_panes_dead());
}

/// A live `sleep 60` pane, fed `OSC 7 file://{host}{dir}`, and the
/// cwd `begin_respawn(-k)` plans for it (SEC-128).
#[cfg(unix)]
fn respawn_cwd_after_osc7(host: &str, dir: &std::path::Path) -> Option<PathBuf> {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    let pane = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, Some("sleep 60"))
        .unwrap();
    tree.pane(pane)
        .unwrap()
        .terminal()
        .write()
        .process(format!("\x1b]7;file://{host}{}\x1b\\", dir.display()).as_bytes());
    assert_eq!(
        tree.pane(pane)
            .unwrap()
            .terminal()
            .read()
            .current_directory(),
        Some(dir.to_str().unwrap()),
        "the OSC 7 report was recorded"
    );
    let plan = tree.begin_respawn(pane, true, None, None).unwrap();
    for id in [first, pane] {
        let _ = tree.pane_mut(id).unwrap().kill();
    }
    plan.cwd
}

/// SEC-128: an OSC 7 cwd naming another host is program output about a
/// remote machine, never a local directory respawn may run in.
#[cfg(unix)]
#[test]
fn respawn_ignores_a_remote_osc7_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = respawn_cwd_after_osc7("remote.invalid", dir.path());
    assert_ne!(cwd.as_deref(), Some(dir.path()), "remote OSC 7 was trusted");
}

/// SEC-128: a local OSC 7 report (implicit, `localhost`, or this
/// machine's own name) naming an existing directory is still used.
#[cfg(unix)]
#[test]
fn respawn_uses_a_local_osc7_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let host = crate::mux::pane::local_hostname().expect("gethostname");
    for h in ["", "localhost", host.as_str()] {
        let cwd = respawn_cwd_after_osc7(h, dir.path());
        assert_eq!(cwd.as_deref(), Some(dir.path()), "host {h:?}");
    }
}

/// SEC-128: a held dead pane with no OSC 7 has no cwd to offer (its
/// reaped PID is not read, SEC-125), so the plan defers to the
/// factory's cwd rather than the daemon's own working directory.
#[cfg(unix)]
#[test]
fn respawn_of_a_dead_pane_without_osc7_defers_the_cwd() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    let pane = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, Some("exit 3"))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let p = tree.pane_mut(pane).unwrap();
        if !p.poll_running() {
            p.mark_dead();
            if p.exit_code().is_some() {
                break;
            }
        }
        assert!(std::time::Instant::now() < deadline, "never reaped");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let plan = tree.begin_respawn(pane, false, None, None).unwrap();
    let _ = tree.pane_mut(first).unwrap().kill();
    assert_eq!(plan.cwd, None);
}

/// SEC-128: a local OSC 7 path that no longer exists is not used.
#[cfg(unix)]
#[test]
fn respawn_ignores_an_osc7_dir_that_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("gone");
    let cwd = respawn_cwd_after_osc7("localhost", &gone);
    assert_ne!(cwd.as_deref(), Some(gone.as_path()), "missing dir was used");
}

/// ARC-089: `respawn-pane -k` reuses the pane id, so the dying
/// process's SIGHUP output must not reach the id's output sink — it
/// would land on the replacement's fresh screen in every client.
#[cfg(unix)]
#[test]
fn respawn_does_not_forward_the_old_processes_exit_output() {
    type Chunks = Arc<parking_lot::Mutex<Vec<(u8, Vec<u8>)>>>;
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(window_id).unwrap().panes()[0];
    // `sleep & wait` lets the trap run inside portable-pty's SIGHUP
    // grace; see `a_killed_panes_late_output_is_not_forwarded`.
    let pane = tree
        .split_pane(
            first,
            SplitDirection::Vertical,
            0.5,
            Some("trap 'echo OLD-PANE-BYE; exit 0' HUP; echo READY-MARK; while :; do sleep 5 & wait; done"),
        )
        .unwrap();
    let chunks: Chunks = Arc::default();
    let tagged = |generation: u8| {
        let chunks = Arc::clone(&chunks);
        move |bytes: &[u8]| chunks.lock().push((generation, bytes.to_vec()))
    };
    let seen_from = |generation: u8| -> String {
        chunks
            .lock()
            .iter()
            .filter(|(g, _)| *g == generation)
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .collect()
    };
    tree.pane_mut(pane).unwrap().on_output(tagged(0));
    // Read the marker off the screen, not the sink: the shell can print
    // it before `on_output` registers, since `split_pane` spawns first.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !tree
        .pane(pane)
        .unwrap()
        .terminal()
        .read()
        .content()
        .contains("READY-MARK")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the trap's ready marker never reached the pane screen"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    let factory = tree.factory();
    let plan = tree
        .begin_respawn(pane, true, Some("sleep 60".to_string()), None)
        .unwrap();
    let replacement = factory
        .create_pane(
            plan.pane_id,
            plan.cols,
            plan.rows,
            plan.command.as_deref(),
            &plan.context(),
        )
        .expect("respawn spawns");
    tree.complete_respawn(plan, replacement).unwrap();
    tree.pane_mut(pane).unwrap().on_output(tagged(1));
    // Longer than portable-pty's SIGHUP grace plus the 500 ms reap.
    std::thread::sleep(std::time::Duration::from_millis(700));

    let old = seen_from(0);
    assert!(
        !old.contains("OLD-PANE-BYE"),
        "the old process's exit output reached the respawned id: {old:?}"
    );
    for id in [first, pane] {
        let _ = tree.pane_mut(id).unwrap().kill();
    }
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

/// QA-182: the audit probe, below the parser cap — 2000 columns of
/// 40 px cells overflowed `cols * cell_w` in u16 and panicked in
/// `resize_with_cell_pixels`. The extent now saturates.
#[test]
fn oversized_cell_pixels_refit_does_not_panic() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let window_id = tree.session(session_id).unwrap().windows[0];
    let pane = tree.window(window_id).unwrap().panes()[0];
    tree.resize_window(window_id, 2000, 50).unwrap();
    tree.set_client_cell_pixels(40, 40);
    tree.resize_window(window_id, 2000, 50).unwrap();
    assert_eq!(
        tree.pane(pane).unwrap().terminal().read().size(),
        (2000, 50)
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

/// SEC-209: names are echoed to viewers' terminals, so every setter strips
/// control characters (C0, DEL, C1) at entry instead of rejecting them.
#[test]
fn rename_strips_control_characters_from_every_tree_name() {
    let mut tree = tree();
    let session_id = tree.new_session("s\x1b[2J1", 80, 24).unwrap();
    assert_eq!(tree.session(session_id).unwrap().name, "s[2J1");
    let window_id = tree.session(session_id).unwrap().windows[0];

    tree.rename_window(window_id, "a\x1b[2Jb").unwrap();
    assert_eq!(tree.window(window_id).unwrap().name, "a[2Jb");

    tree.rename_session(session_id, "x\x07\u{9b}y\x7f").unwrap();
    assert_eq!(tree.session(session_id).unwrap().name, "xy");

    let workspace = tree.new_workspace("w\x1b]0;t\x07s");
    assert_eq!(tree.workspace(workspace).unwrap().name, "w]0;ts");
    tree.rename_workspace(workspace, "\nfresh\r").unwrap();
    assert_eq!(tree.workspace(workspace).unwrap().name, "fresh");
}

/// SEC-209: break-pane's new window name strips controls at entry.
#[test]
fn break_pane_strips_control_characters_from_the_window_name() {
    let mut tree = tree();
    let session_id = tree.new_session("main", 80, 24).unwrap();
    let first_window = tree.session(session_id).unwrap().windows[0];
    let first = tree.window(first_window).unwrap().panes()[0];
    tree.split_pane(first, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    let (new_window, _, _) = tree.break_pane(first, "b\x1b[2Jk").unwrap();
    assert_eq!(tree.window(new_window).unwrap().name, "b[2Jk");
}

/// SEC-209: a session spawn's explicit first-window name strips controls.
#[test]
fn with_window_name_strips_control_characters() {
    let mut tree = tree();
    let workspace = tree.new_workspace("ws");
    let plan = tree
        .begin_session_at(workspace, "s", 80, 24, &Default::default())
        .with_window_name("w\x07\u{9b}1");
    assert_eq!(plan.window_name.as_deref(), Some("w1"));
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

/// `split-window`'s loose target: `@N` resolves to that window's active
/// pane and `$N` to the session's active window's active pane, so both
/// follow selection; unknown window/session ids error per kind (a `%N`
/// passes through — `begin_split` reports its existence, as before).
#[test]
fn split_target_resolves_window_and_session_to_their_active_panes() {
    let mut tree = named_tree(None, None);
    // @0's active pane becomes %2, and session $0 grows window @2 whose
    // active pane is %3; the session's active window stays @0.
    tree.split_pane(PaneId(0), SplitDirection::Vertical, 0.5, None)
        .unwrap();
    let logs = tree.new_window(SessionId(0), "logs", 80, 24).unwrap();
    assert_eq!(
        tree.resolve_split_target(AnyTarget::Window(WindowId(0)))
            .unwrap(),
        PaneId(2)
    );
    assert_eq!(
        tree.resolve_split_target(AnyTarget::Window(logs)).unwrap(),
        PaneId(3)
    );
    assert_eq!(
        tree.resolve_split_target(AnyTarget::Session(SessionId(0)))
            .unwrap(),
        PaneId(2),
        "the session's active window is @0"
    );
    tree.select_window(logs).unwrap();
    assert_eq!(
        tree.resolve_split_target(AnyTarget::Session(SessionId(0)))
            .unwrap(),
        PaneId(3),
        "selection moves the session target's pane"
    );
    // Pane %1 of the OTHER session is reachable only by its own id.
    assert_eq!(
        tree.resolve_split_target(AnyTarget::Pane(PaneId(1)))
            .unwrap(),
        PaneId(1)
    );
    assert!(matches!(
        tree.resolve_split_target(AnyTarget::Window(WindowId(99))),
        Err(MuxError::NoSuchWindow(_))
    ));
    assert!(matches!(
        tree.resolve_split_target(AnyTarget::Session(SessionId(99))),
        Err(MuxError::NoSuchSession(_))
    ));
}

/// `new-window`'s loose target: `$N`/names append to the session, while
/// `@N`/`%N` carry the window the new one sits right after.
#[test]
fn new_window_target_resolves_to_session_and_insert_after() {
    let tree = named_tree(None, None);
    assert_eq!(
        tree.resolve_new_window_target(AnyTarget::Session(SessionId(0)))
            .unwrap(),
        (SessionId(0), None)
    );
    assert_eq!(
        tree.resolve_new_window_target(AnyTarget::Window(WindowId(1)))
            .unwrap(),
        (SessionId(1), Some(WindowId(1)))
    );
    assert_eq!(
        tree.resolve_new_window_target(AnyTarget::Pane(PaneId(0)))
            .unwrap(),
        (SessionId(0), Some(WindowId(0)))
    );
    assert_eq!(
        tree.resolve_new_window_target(AnyTarget::Name("beta".to_string()))
            .unwrap(),
        (SessionId(1), None),
        "names keep the session-name match, appending"
    );
    assert!(matches!(
        tree.resolve_new_window_target(AnyTarget::Window(WindowId(99))),
        Err(MuxError::NoSuchWindow(_))
    ));
    assert!(matches!(
        tree.resolve_new_window_target(AnyTarget::Pane(PaneId(99))),
        Err(MuxError::NoSuchPane(_))
    ));
}

/// Completing an insert-after `begin_window` places the window right
/// after its target in the session's list, and the session's active index
/// follows the shift so the same window stays active.
#[test]
fn window_insert_after_sits_the_new_window_right_after_its_target() {
    let mut tree = named_tree(None, None);
    let second = tree.new_window(SessionId(0), "two", 80, 24).unwrap();
    let third = tree.new_window(SessionId(0), "three", 80, 24).unwrap();
    let plan = tree
        .begin_window(SessionId(0), "inserted", 80, 24, None, Some(WindowId(0)))
        .unwrap();
    let pane = tree.spawn_from(&plan.pane_id, plan.cols, plan.rows, None, &plan.context());
    let inserted = tree.complete_window(plan, pane.unwrap()).unwrap();
    let session = tree.session(SessionId(0)).unwrap();
    assert_eq!(
        session.windows,
        vec![WindowId(0), inserted, second, third],
        "the new window took @0's next slot, not the end"
    );
    assert_eq!(
        session.windows[session.active],
        WindowId(0),
        "the window before the insertion point stays active"
    );

    // A window AFTER the insertion point shifts: with `third` active, an
    // insert after @0 moves the active index with it.
    tree.select_window(third).unwrap();
    let plan = tree
        .begin_window(SessionId(0), "another", 80, 24, None, Some(WindowId(0)))
        .unwrap();
    let pane = tree.spawn_from(&plan.pane_id, plan.cols, plan.rows, None, &plan.context());
    let another = tree.complete_window(plan, pane.unwrap()).unwrap();
    let session = tree.session(SessionId(0)).unwrap();
    assert_eq!(
        session.windows,
        vec![WindowId(0), another, inserted, second, third]
    );
    assert_eq!(
        session.windows[session.active], third,
        "the active index follows the shift"
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

#[test]
fn rename_session_updates_the_name_for_resolution_and_future_spawns() {
    let mut tree = named_tree(None, None);
    tree.rename_session(SessionId(1), "renamed").unwrap();
    assert_eq!(tree.session(SessionId(1)).unwrap().name, "renamed");
    assert_eq!(
        tree.resolve_session_target(Target::Name("renamed".to_string()))
            .unwrap(),
        SessionId(1)
    );
    assert!(matches!(
        tree.resolve_session_target(Target::Name("beta".to_string())),
        Err(MuxError::NoSuchSessionNamed(n)) if n == "beta"
    ));
    assert!(matches!(
        tree.rename_session(SessionId(9), "x"),
        Err(MuxError::NoSuchSession(_))
    ));
}

#[test]
fn kill_session_removes_every_window_pane_and_the_session() {
    let mut tree = named_tree(None, None);
    // A second window in session 1, so the kill spans multiple windows.
    tree.new_window(SessionId(1), "logs", 80, 24).unwrap();
    let killed = tree.kill_session(SessionId(1)).unwrap();
    assert_eq!(killed.len(), 2, "both windows of the session die");
    for window in &killed {
        assert!(tree.window(*window).is_none());
    }
    assert!(tree.session(SessionId(1)).is_none());
    // The other session is untouched.
    assert!(tree.session(SessionId(0)).is_some());
    assert_eq!(tree.sessions().len(), 1);
    assert!(matches!(
        tree.kill_session(SessionId(1)),
        Err(MuxError::NoSuchSession(_))
    ));
}

// --- Workspaces (first-class level above sessions) ---

#[test]
fn new_session_lands_in_the_lazily_created_default_workspace() {
    let mut tree = tree();
    assert_eq!(tree.active_workspace(), None);
    let session = tree.new_session("work", 80, 24).unwrap();
    let ws = tree.active_workspace().expect("default workspace created");
    assert_eq!(tree.workspace(ws).unwrap().name, "main");
    assert_eq!(tree.workspace(ws).unwrap().sessions, vec![session]);
    assert_eq!(tree.workspace_of_session(session), Some(ws));
    assert_eq!(tree.active_session(), Some(session));
    tree.assert_indexes_consistent();
}

#[test]
fn workspace_crud_and_active_tracking() {
    let mut tree = tree();
    let ws_a = tree.new_workspace("alpha");
    let ws_b = tree.new_workspace("beta");
    // new-workspace selects the new one.
    assert_eq!(tree.active_workspace(), Some(ws_b));

    // Sessions land in the ACTIVE workspace.
    tree.select_workspace(ws_a).unwrap();
    let s1 = tree.new_session("one", 80, 24).unwrap();
    let s2 = tree.new_session("two", 80, 24).unwrap();
    assert_eq!(
        tree.workspace(ws_a).unwrap().sessions,
        vec![s1, s2],
        "sessions join the active workspace in order"
    );
    tree.select_workspace(ws_b).unwrap();
    let s3 = tree.new_session("three", 80, 24).unwrap();
    assert_eq!(tree.workspace_of_session(s3), Some(ws_b));
    tree.assert_indexes_consistent();

    // select-workspace switches the active session to the workspace's own
    // previously-active session.
    tree.select_workspace(ws_a).unwrap();
    assert_eq!(tree.active_session(), Some(s2), "last session of ws_a");
    tree.select_workspace(ws_b).unwrap();
    assert_eq!(tree.active_session(), Some(s3));

    // Rename.
    tree.rename_workspace(ws_b, "renamed").unwrap();
    assert_eq!(tree.workspace(ws_b).unwrap().name, "renamed");
    assert!(matches!(
        tree.rename_workspace(WorkspaceId(99), "x"),
        Err(MuxError::NoSuchWorkspace(_))
    ));

    // Resolution: typed id and name, with ambiguity errors.
    assert_eq!(
        tree.resolve_workspace_target(Target::Id(ws_a)).unwrap(),
        ws_a
    );
    assert_eq!(
        tree.resolve_workspace_target(Target::Name("alpha".into()))
            .unwrap(),
        ws_a
    );
    tree.new_workspace("alpha");
    assert!(matches!(
        tree.resolve_workspace_target(Target::Name("alpha".into())),
        Err(MuxError::AmbiguousWorkspaceTarget(_, _))
    ));
}

#[test]
fn killing_a_session_removes_it_and_can_empty_its_workspace_away() {
    let mut tree = tree();
    let ws_a = tree.new_workspace("alpha");
    let ws_b = tree.new_workspace("beta");
    // alpha is no longer active (beta's creation selected it) — select it
    // back before creating its sessions.
    tree.select_workspace(ws_a).unwrap();
    let s1 = tree.new_session("one", 80, 24).unwrap();
    let s2 = tree.new_session("two", 80, 24).unwrap();
    tree.select_workspace(ws_b).unwrap();
    let s3 = tree.new_session("three", 80, 24).unwrap();

    // Killing s1 leaves ws_a with s2; the workspace active index clamps.
    tree.kill_session(s1).unwrap();
    assert_eq!(
        tree.workspace(ws_a).unwrap().sessions,
        vec![s2],
        "the survivor remains"
    );
    tree.select_workspace(ws_a).unwrap();
    assert_eq!(tree.active_session(), Some(s2), "clamped to the survivor");

    // Killing the LAST session of ws_a removes the workspace itself.
    tree.kill_session(s2).unwrap();
    assert!(tree.workspace(ws_a).is_none(), "an emptied workspace dies");
    assert_eq!(tree.workspace_of_session(s2), None);
    tree.assert_indexes_consistent();

    // The kill-pane cascade (drop_empty_window) reaches the same removal.
    let s4 = tree.new_session("four", 80, 24).unwrap();
    let window = tree.session(s4).unwrap().windows[0];
    let pane = tree.window(window).unwrap().panes()[0];
    tree.kill_pane(pane).unwrap();
    assert_eq!(tree.sessions().len(), 1, "only ws_b's session remains");
    assert_eq!(tree.workspaces().len(), 1);
    assert_eq!(tree.workspace_of_session(s3), Some(ws_b));
}

#[test]
fn kill_workspace_takes_every_session_window_and_pane_with_it() {
    let mut tree = tree();
    let ws_a = tree.new_workspace("alpha");
    let _ws_b = tree.new_workspace("beta");
    tree.select_workspace(ws_a).unwrap();
    let s1 = tree.new_session("one", 80, 24).unwrap();
    let s2 = tree.new_session("two", 80, 24).unwrap();
    // A second window in s1, so the kill spans windows.
    let w2 = tree.new_window(s1, "extra", 80, 24).unwrap();

    let killed = tree.kill_workspace(ws_a).unwrap();
    assert_eq!(killed.len(), 3, "s1's two windows + s2's one");
    assert!(killed.contains(&w2));
    assert!(tree.session(s1).is_none() && tree.session(s2).is_none());
    assert!(tree.workspace(ws_a).is_none());
    assert!(tree.windows.is_empty(), "every pane went with the sessions");
    // The active pointer moved off the killed workspace.
    assert_ne!(tree.active_workspace(), Some(ws_a));
    tree.assert_indexes_consistent();
}

#[test]
fn a_workspace_born_empty_survives_until_a_session_dies_inside_it() {
    let mut tree = tree();
    let ws = tree.new_workspace("empty");
    assert!(tree.workspace(ws).unwrap().sessions.is_empty());
    assert_eq!(tree.active_workspace(), Some(ws));
    // No session removal happened, so the empty workspace stays.
    assert!(tree.workspace(ws).is_some());
}

// ---- Smallest-attached-client window sizing ----

fn extent(tree: &MuxTree, window: WindowId) -> (u16, u16) {
    let window = tree.window(window).unwrap();
    (window.cols, window.rows)
}

#[test]
fn window_size_is_the_componentwise_minimum_of_viewers_reports() {
    let mut tree = tree();
    let session = tree.new_session("sync", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];

    // The first viewer's report sizes the window outright.
    assert_eq!(
        tree.set_client_view(1, window, 100, 30, Default::default()),
        vec![window]
    );
    assert_eq!(extent(&tree, window), (100, 30));

    // A smaller viewer pulls the window down to the componentwise minimum.
    assert_eq!(
        tree.set_client_view(2, window, 60, 20, Default::default()),
        vec![window]
    );
    assert_eq!(extent(&tree, window), (60, 20));

    // A viewer larger than the current minimum contributes nothing.
    assert!(tree
        .set_client_view(3, window, 90, 28, Default::default())
        .is_empty());
    assert_eq!(extent(&tree, window), (60, 20));

    // Repeated identical reports are idempotent — the property that keeps
    // the re-fit from feeding back into a resize loop.
    for _ in 0..3 {
        assert!(tree
            .set_client_view(1, window, 100, 30, Default::default())
            .is_empty());
    }
    assert_eq!(extent(&tree, window), (60, 20));
}

#[test]
fn disconnect_removes_a_contribution_and_the_window_grows() {
    let mut tree = tree();
    let session = tree.new_session("sync", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    tree.set_client_view(1, window, 100, 30, Default::default());
    tree.set_client_view(2, window, 60, 20, Default::default());
    assert_eq!(extent(&tree, window), (60, 20));

    // The constraining viewer leaves: the window re-fits to the remaining
    // viewers' minimum.
    assert_eq!(tree.clear_client_view(2), vec![window]);
    assert_eq!(extent(&tree, window), (100, 30));
    // Clearing an unknown connection is a no-op.
    assert!(tree.clear_client_view(2).is_empty());
    // The last viewer leaving leaves the extent where it is (it re-fits
    // when next displayed).
    assert!(tree.clear_client_view(1).is_empty());
    assert_eq!(extent(&tree, window), (100, 30));
}

#[test]
fn switching_away_releases_the_old_window_from_the_switched_client() {
    let mut tree = tree();
    let session = tree.new_session("sync", 80, 24).unwrap();
    let w1 = tree.session(session).unwrap().windows[0];
    let w2 = tree.new_window(session, "second", 80, 24).unwrap();

    tree.set_client_view(1, w1, 100, 30, Default::default());
    tree.set_client_view(2, w1, 60, 20, Default::default());
    assert_eq!(extent(&tree, w1), (60, 20));

    // Client 2's renderer lands on the other window: its contribution
    // moves, and the old window grows back to the remaining minimum.
    let resized = tree.set_client_view(2, w2, 60, 20, Default::default());
    assert!(
        resized.contains(&w2),
        "the new window takes the report: {resized:?}"
    );
    assert!(
        resized.contains(&w1),
        "the old window re-fits too: {resized:?}"
    );
    assert_eq!(extent(&tree, w1), (100, 30));
    assert_eq!(extent(&tree, w2), (60, 20));
}

#[test]
fn a_selection_follow_moves_every_viewer_of_the_session() {
    let mut tree = tree();
    let session = tree.new_session("sync", 80, 24).unwrap();
    let w1 = tree.session(session).unwrap().windows[0];
    let w2 = tree.new_window(session, "second", 80, 24).unwrap();
    tree.set_client_view(1, w1, 100, 30, Default::default());
    tree.set_client_view(2, w1, 60, 20, Default::default());

    // The shared selection lands the session on w2: both viewers follow,
    // and the target window re-fits to the moved minimum. The left window
    // has no viewer left, so it keeps its extent (it re-fits when next
    // displayed) — neither client constrains it anymore.
    let resized = tree.follow_session_window(session, w2);
    assert!(resized.contains(&w2), "the target re-fits: {resized:?}");
    assert_eq!(extent(&tree, w2), (60, 20));
    assert_eq!(
        extent(&tree, w1),
        (60, 20),
        "kept, not grown: nobody views it"
    );
}

#[test]
fn a_window_without_viewers_keeps_its_extent() {
    let mut tree = tree();
    let session = tree.new_session("sync", 80, 24).unwrap();
    let w1 = tree.session(session).unwrap().windows[0];
    let w2 = tree.new_window(session, "second", 100, 30).unwrap();
    tree.set_client_view(1, w1, 60, 20, Default::default());
    // w2 has no viewer: the rule does not touch it.
    assert!(tree.follow_session_window(session, w1).is_empty());
    assert_eq!(extent(&tree, w2), (100, 30));
}

/// Records whether a zone eviction reached the observer and whether the
/// tree mutex was free while the callback ran (it is not re-entrant, so a
/// delivery under the lock fails `try_lock` on the delivering thread).
#[cfg(unix)]
struct TreeLockProbe {
    tree: std::sync::Weak<parking_lot::Mutex<MuxTree>>,
    scrolled_out: std::sync::atomic::AtomicBool,
    tree_lock_was_free: std::sync::atomic::AtomicBool,
}

#[cfg(unix)]
impl crate::terminal::observer::TerminalObserver for TreeLockProbe {
    fn on_zone_event(&self, event: &crate::terminal::TerminalEvent) {
        use std::sync::atomic::Ordering;
        if matches!(
            event,
            crate::terminal::TerminalEvent::ZoneScrolledOut { .. }
        ) {
            let free = self
                .tree
                .upgrade()
                .is_some_and(|tree| tree.try_lock().is_some());
            self.tree_lock_was_free.store(free, Ordering::SeqCst);
            self.scrolled_out.store(true, Ordering::SeqCst);
        }
    }
}

/// A full-width pane running a quiet child, its terminal holding one
/// completed command zone at the top of a scrollback a few lines short of
/// its 10 000-line cap, so halving the width reflows the zone out. The
/// quiet child keeps the reader thread from delivering events of its own,
/// so every event the probe sees comes from the resize. Unix-only: ConPTY
/// writes setup output after spawn, which would land past the fill and
/// evict the zone on the reader thread instead.
#[cfg(unix)]
fn quiet_pane_with_evictable_zone(tree: &mut MuxTree) -> PaneId {
    let session = tree.new_session("observers", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let first = tree.window(window).unwrap().panes()[0];
    let quiet = tree
        .split_pane(first, SplitDirection::Horizontal, 0.5, Some("sleep 60"))
        .unwrap();
    let terminal = tree.pane(quiet).unwrap().terminal();
    let mut term = terminal.write();
    assert_eq!(term.size().0, 80, "the quiet pane starts full width");
    let mut bytes =
        b"\x1b]133;A\x07$ \x1b]133;B\x07cmd\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07".to_vec();
    let line = "x".repeat(80);
    // Only the visible rows reflow into scrollback on a narrowing, so park
    // the scrollback two lines short of its cap: the halved width pushes a
    // whole screen past it, evicting the zone at the top. The `- 3` covers
    // the lines the zone itself scrolls.
    let rows = term.size().1;
    let fill = term.grid().max_scrollback() - term.grid().scrollback_len() + rows - 2 - 3;
    for _ in 0..fill {
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(line.as_bytes());
    }
    term.process(&bytes);
    let headroom = term.grid().max_scrollback() - term.grid().scrollback_len();
    assert_eq!(headroom, 2, "scrollback parked two lines short of its cap");
    assert_eq!(term.get_zones().len(), 3, "no zone evicted yet");
    drop(term);
    quiet
}

#[cfg(unix)]
fn attach_probe(tree: &Arc<parking_lot::Mutex<MuxTree>>, pane: PaneId) -> Arc<TreeLockProbe> {
    let probe = Arc::new(TreeLockProbe {
        tree: Arc::downgrade(tree),
        scrolled_out: std::sync::atomic::AtomicBool::new(false),
        tree_lock_was_free: std::sync::atomic::AtomicBool::new(false),
    });
    let terminal = tree.lock().pane(pane).unwrap().terminal();
    terminal.write().add_observer(probe.clone());
    probe
}

/// Card 01a114e4: a layout mutation re-fits pane terminals under the tree
/// mutex, and the observer events that re-fit raises (here a zone the
/// shrink evicts) are delivered by the dispatcher only after the mutex is
/// released, so an observer that touches the tree cannot deadlock on it.
#[cfg(unix)]
#[test]
fn layout_resize_observer_events_arrive_after_the_tree_lock_drops() {
    use std::sync::atomic::Ordering;
    let mut owned = tree();
    let quiet = quiet_pane_with_evictable_zone(&mut owned);
    let tree = Arc::new(parking_lot::Mutex::new(owned));
    let probe = attach_probe(&tree, quiet);
    assert!(!probe.scrolled_out.load(Ordering::SeqCst));

    let clients: crate::mux::server::Clients = Arc::default();
    let ctx = crate::mux::dispatch::Ctx {
        client_id: None,
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
    };
    let command =
        crate::mux::command::parse_command(&format!("split-window -t {quiet} -h")).unwrap();
    let reply = crate::mux::dispatch::dispatch_command(command, &ctx, None, None);
    assert!(!reply.contains("%error"), "split succeeds: {reply}");
    let width = tree.lock().pane(quiet).unwrap().terminal().read().size().0;
    assert!(width < 80, "the split narrowed the quiet pane: {width}");

    assert!(
        probe.scrolled_out.load(Ordering::SeqCst),
        "the shrink's ZoneScrolledOut reached the observer by the time dispatch returned"
    );
    assert!(
        probe.tree_lock_was_free.load(Ordering::SeqCst),
        "the observer ran while the tree mutex was held"
    );
    assert!(tree.lock().take_observer_batches().is_empty());
}

/// The direct-tree side of the same contract: a tree method that re-fits
/// panes parks the observer batch instead of delivering it, and
/// `take_observer_batches` hands it to the caller.
#[cfg(unix)]
#[test]
fn a_direct_tree_resize_parks_observer_events_until_taken() {
    use std::sync::atomic::Ordering;
    let mut owned = tree();
    let quiet = quiet_pane_with_evictable_zone(&mut owned);
    let tree = Arc::new(parking_lot::Mutex::new(owned));
    let probe = attach_probe(&tree, quiet);

    let batches = {
        let mut guard = tree.lock();
        guard
            .split_pane(quiet, SplitDirection::Vertical, 0.5, Some("sleep 60"))
            .unwrap();
        assert!(
            !probe.scrolled_out.load(Ordering::SeqCst),
            "nothing is delivered under the tree lock"
        );
        guard.take_observer_batches()
    };
    assert!(!batches.is_empty(), "the shrink parked a batch");
    for batch in batches {
        batch.deliver();
    }
    assert!(probe.scrolled_out.load(Ordering::SeqCst));
    assert!(probe.tree_lock_was_free.load(Ordering::SeqCst));
}

/// Spawns `sleep 60` panes. Once armed, the next pane it builds carries a
/// [`TreeLockProbe`] and a terminal parked exactly at its scrollback cap
/// with a completed zone at the top, so the first line the spawn's
/// start-dir note scrolls evicts that zone.
#[cfg(unix)]
struct NoteProbeFactory {
    tree: std::sync::Weak<parking_lot::Mutex<MuxTree>>,
    armed: Arc<std::sync::atomic::AtomicBool>,
    probe: Arc<std::sync::OnceLock<Arc<TreeLockProbe>>>,
}

#[cfg(unix)]
impl PaneFactory for NoteProbeFactory {
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
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let probe = Arc::new(TreeLockProbe {
                tree: self.tree.clone(),
                scrolled_out: std::sync::atomic::AtomicBool::new(false),
                tree_lock_was_free: std::sync::atomic::AtomicBool::new(false),
            });
            let terminal = pane.terminal();
            let mut term = terminal.write();
            let mut bytes =
                b"\x1b]133;A\x07$ \x1b]133;B\x07cmd\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07"
                    .to_vec();
            let line = "x".repeat(term.size().0);
            let rows = term.size().1;
            // The zone itself scrolls three lines; the fill tops scrollback
            // up to exactly its cap with the cursor on the bottom row.
            let fill = term.grid().max_scrollback() - term.grid().scrollback_len() + rows - 3;
            for _ in 0..fill {
                bytes.extend_from_slice(b"\r\n");
                bytes.extend_from_slice(line.as_bytes());
            }
            term.process(&bytes);
            assert_eq!(
                term.grid().scrollback_len(),
                term.grid().max_scrollback(),
                "scrollback parked at its cap"
            );
            assert_eq!(term.cursor().row, rows - 1, "cursor on the bottom row");
            assert_eq!(term.get_zones().len(), 3, "no zone evicted yet");
            term.add_observer(probe.clone());
            drop(term);
            let _ = self.probe.set(probe);
        }
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

/// Card 01a11989: a spawn whose start dir is gone writes a note into the
/// new pane while the dispatcher holds the tree mutex. The observer events
/// that write raises (here a zone the note scrolls out) are delivered only
/// after the mutex is released, as for the resize paths.
#[cfg(unix)]
#[test]
fn start_dir_note_observer_events_arrive_after_the_tree_lock_drops() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let armed = Arc::new(AtomicBool::new(false));
    let slot: Arc<std::sync::OnceLock<Arc<TreeLockProbe>>> = Arc::default();
    let tree = Arc::new_cyclic(|weak| {
        parking_lot::Mutex::new(MuxTree::new(Box::new(NoteProbeFactory {
            tree: weak.clone(),
            armed: armed.clone(),
            probe: slot.clone(),
        })))
    });
    let clients: crate::mux::server::Clients = Arc::default();
    let ctx = crate::mux::dispatch::Ctx {
        client_id: None,
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
    };
    let run = |line: &str| {
        let command = crate::mux::command::parse_command(line).unwrap();
        crate::mux::dispatch::dispatch_command(command, &ctx, None, None)
    };
    let reply = run("new-session -s main");
    assert!(!reply.contains("%error"), "new-session succeeds: {reply}");

    armed.store(true, Ordering::SeqCst);
    let reply = run("new-window -c /par-mux-test-no-such-dir");
    assert!(!reply.contains("%error"), "new-window succeeds: {reply}");
    let probe = slot.get().expect("the armed spawn attached the probe");

    assert!(
        probe.scrolled_out.load(Ordering::SeqCst),
        "the note's ZoneScrolledOut reached the observer by the time dispatch returned"
    );
    assert!(
        probe.tree_lock_was_free.load(Ordering::SeqCst),
        "the observer ran while the tree mutex was held"
    );
    assert!(tree.lock().take_observer_batches().is_empty());
}

/// The border ring's one-cell inset (card 01a11c5b), as declared.
fn ring() -> PaneChrome {
    PaneChrome {
        border: true,
        ..PaneChrome::default()
    }
}

/// Card 01a11c5b, the reserve-border-lines model: a declared ring on a
/// stacked pair sizes each pane's PTY to its rect less the ring — two
/// rows and two columns per pane — while the rects (the layout string)
/// keep their full geometry.
#[test]
fn declared_ring_sizes_stacked_panes_to_the_interior() {
    let mut tree = tree();
    let session = tree.new_session("ring", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let top = tree.window(window).unwrap().panes()[0];
    let bottom = tree
        .split_pane(top, SplitDirection::Horizontal, 0.5, None)
        .unwrap();
    let size_of = |tree: &MuxTree, pane| tree.pane(pane).unwrap().terminal().read().size();
    // No declaration: today's exact sizes.
    assert_eq!(size_of(&tree, top), (80, 12));
    assert_eq!(size_of(&tree, bottom), (80, 12));

    assert_eq!(
        tree.set_client_view(1, window, 80, 24, ring()),
        vec![window]
    );
    assert_eq!(size_of(&tree, top), (78, 10), "rect 80x12 less the ring");
    assert_eq!(size_of(&tree, bottom), (78, 10), "rect 80x12 less the ring");
    // The rects themselves are untouched.
    let rects = {
        let w = tree.window(window).unwrap();
        w.layout.geometry(0, 0, w.cols as usize, w.rows as usize)
    };
    assert_eq!(
        rects
            .iter()
            .map(|g| (g.width, g.height))
            .collect::<Vec<_>>(),
        vec![(80, 12), (80, 12)]
    );
}

/// Side by side: the ring takes two columns from each pane.
#[test]
fn declared_ring_sizes_side_by_side_panes_to_the_interior() {
    let mut tree = tree();
    let session = tree.new_session("ring", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let left = tree.window(window).unwrap().panes()[0];
    let right = tree
        .split_pane(left, SplitDirection::Vertical, 0.5, None)
        .unwrap();
    tree.set_client_view(1, window, 80, 24, ring());
    let size_of = |pane| tree.pane(pane).unwrap().terminal().read().size();
    assert_eq!(size_of(left), (38, 22));
    assert_eq!(size_of(right), (38, 22));
}

/// Gaps multiply per side, compounding with the ring and the gutter.
#[test]
fn declared_gaps_and_gutter_compound_with_the_ring() {
    let mut tree = tree();
    let session = tree.new_session("gaps", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane = tree.window(window).unwrap().panes()[0];
    let chrome = PaneChrome {
        border: true,
        gap: 2,
        gutter: true,
    };
    tree.set_client_view(1, window, 80, 24, chrome);
    // 80 - 2*2 gap - 2 ring - 1 gutter = 73; 24 - 2*2 gap - 2 ring = 18.
    assert_eq!(tree.pane(pane).unwrap().terminal().read().size(), (73, 18));
}

/// A pane too small to carry a ring keeps its full rect (the client's
/// content_view clamp: the ring needs a rect larger than 2x2).
#[test]
fn a_pane_too_small_for_the_ring_keeps_its_full_rect() {
    let mut tree = tree();
    let session = tree.new_session("tiny", 80, 2).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane = tree.window(window).unwrap().panes()[0];
    tree.set_client_view(1, window, 80, 2, ring());
    assert_eq!(tree.pane(pane).unwrap().terminal().read().size(), (80, 2));
}

/// A chrome-only change (a border toggle at an unchanged host size)
/// re-fits — the extent check alone would leave the PTYs stale — and
/// dropping the declaration restores today's full-rect sizing.
#[test]
fn a_chrome_only_report_refits_and_undeclaring_restores_full_rects() {
    let mut tree = tree();
    let session = tree.new_session("toggle", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane = tree.window(window).unwrap().panes()[0];
    let size_of = |tree: &MuxTree| tree.pane(pane).unwrap().terminal().read().size();
    tree.set_client_view(1, window, 80, 24, PaneChrome::default());
    assert_eq!(size_of(&tree), (80, 24));
    assert_eq!(
        tree.set_client_view(1, window, 80, 24, ring()),
        vec![window]
    );
    assert_eq!(size_of(&tree), (78, 22));
    assert!(
        tree.set_client_view(1, window, 80, 24, ring()).is_empty(),
        "an identical re-declaration is idempotent"
    );
    assert_eq!(
        tree.set_client_view(1, window, 80, 24, PaneChrome::default()),
        vec![window]
    );
    assert_eq!(size_of(&tree), (80, 24));
}

/// Several viewers: the window reserves the union, so the PTY is never
/// larger than any declaring client's painted interior.
#[test]
fn viewers_chrome_merges_to_the_union() {
    let mut tree = tree();
    let session = tree.new_session("merge", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let pane = tree.window(window).unwrap().panes()[0];
    tree.set_client_view(1, window, 80, 24, PaneChrome::default());
    tree.set_client_view(2, window, 80, 24, ring());
    assert_eq!(tree.window(window).unwrap().chrome, ring());
    assert_eq!(tree.pane(pane).unwrap().terminal().read().size(), (78, 22));
    // The ring-declaring viewer leaves: back to full rects.
    tree.clear_client_view(2);
    assert_eq!(tree.pane(pane).unwrap().terminal().read().size(), (80, 24));
}

/// A zoomed pane takes the full window grid less the chrome.
#[test]
fn a_zoomed_pane_reserves_the_chrome_too() {
    let (mut tree, window, first, _second) = zoomed_split();
    tree.set_client_view(1, window, 80, 24, ring());
    assert_eq!(tree.pane(first).unwrap().terminal().read().size(), (78, 22));
}
