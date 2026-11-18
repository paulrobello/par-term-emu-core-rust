//! WindowSession suites: settings toggles and seeds with dying targets.

use super::*;

/// The settings dialog's `general` tab lists `session picker` (default
/// off) and its toggle writes `session-picker = true` into the file and
/// applies it to the session, for the next target-less attach to read.
#[test]
fn settings_row_toggles_the_session_picker() {
    let (_rx, mut conn, mut session) = command_menu_session("pickersettings");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    session.route_stdin_bytes(&mut conn, b"\t\t");
    // The general tab scrolls — reach the picker row before asserting.
    session.route_stdin_bytes(&mut conn, b"jjjjjjjjjj");
    let rows = overlay_text(&session);
    assert!(
        rows.iter()
            .any(|r| r.contains("session picker") && r.contains("off")),
        "the row shows the default in force: {rows:#?}"
    );
    assert!(!session.session_picker);
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("session-picker = true"), "{written}");
    assert!(session.session_picker, "applied to the session");
}

/// The settings dialog's `general` tab lists `git status badge` (default
/// on) and its toggle writes `git-status = false` into the file and
/// applies it to the session (the badge's render and poll gate).
#[test]
fn settings_row_toggles_the_git_badge() {
    let (_rx, mut conn, mut session) = command_menu_session("gitbadgesettings");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    session.route_stdin_bytes(&mut conn, b"\t\t");
    // The general tab now carries ten fixed rows (the tab-width rows
    // included) and the panel scrolls — reach the git row before
    // asserting on it.
    session.route_stdin_bytes(&mut conn, b"jjjjjjjjjjjj");
    let rows = overlay_text(&session);
    assert!(
        rows.iter()
            .any(|r| r.contains("git status badge") && r.contains("on")),
        "the row shows the default in force: {rows:#?}"
    );
    assert!(session.git_status);
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("git-status = false"), "{written}");
    assert!(!session.git_status, "applied to the session");
}

/// The daemon side of a seed whose target `%1` (window `@0`) resolved
/// and whose workspace is then torn down, leaving workspace `+1` with
/// session `$1` and its window `@5` (pane `%7`): the cascade landing's
/// replies and `@5`'s size-report layout.
fn torn_down_seed_script() -> FakeScript {
    let mut script = FakeScript::default();
    let replies = &mut script.replies;
    replies.insert("pane-info -t %1".to_string(), "%1 @0 80x23".to_string());
    replies.insert(
        "list-workspaces".to_string(),
        "+1: other active".to_string(),
    );
    replies.insert("list-sessions -t +1".to_string(), "$1: other".to_string());
    replies.insert("list-sessions".to_string(), "$1: other".to_string());
    replies.insert("list-windows -t $1".to_string(), "@5 * main".to_string());
    replies.insert("list-panes -t @5".to_string(), "%7 0 *".to_string());
    script.notify.insert(
        "refresh-client -t %7 -C 80x23".to_string(),
        "%layout-change @5 0000,80x23,0,0,7 0000,80x23,0,0,7 *".to_string(),
    );
    script
}

/// Card 01a15b6f: the shown workspace dies between the seed's target
/// resolution and its size report. The daemon refuses the report (the
/// pane is gone), so no `%layout-change` comes — the seed used to read
/// that as a pre-layout daemon and exit 1. It lands on the survivor.
#[test]
fn seed_lands_on_the_survivor_when_the_target_dies_before_its_size_report() {
    let mut script = torn_down_seed_script();
    script
        .failing
        .insert("refresh-client -t %1 -C 80x23".to_string());
    script.failing.insert("list-panes -t @0".to_string());
    let (rx, mut conn) = fake_daemon("seed-gone-report", script);
    let mut session = WindowSession::new(80, 25);
    let shown = session
        .seed(&mut conn, Some("%1"), &mut RecordingSink::default())
        .expect("a vanished target lands instead of failing the attach");
    assert!(shown, "a survivor exists, so the view is shown");
    assert_eq!(session.window, "@5", "the view landed on the survivor");
    let panes: Vec<u32> = session.renderer.layout().iter().map(|r| r.pane).collect();
    assert_eq!(panes, vec![7], "the survivor's layout is installed");
    assert!(
        drained(&rx).contains(&"refresh-client -t %7 -C 80x23".to_string()),
        "the survivor got its own size report"
    );
}

/// The same race one step later: the size report and its layout went
/// through, then the status refresh finds no session owning `@0`. The
/// seed used to end with "the target's session is gone"; it lands.
#[test]
fn seed_lands_on_the_survivor_when_the_target_session_dies_mid_seed() {
    let mut script = torn_down_seed_script();
    script.notify.insert(
        "refresh-client -t %1 -C 80x23".to_string(),
        "%layout-change @0 0000,80x23,0,0,1 0000,80x23,0,0,1 *".to_string(),
    );
    let (_rx, mut conn) = fake_daemon("seed-gone-status", script);
    let mut session = WindowSession::new(80, 25);
    let shown = session
        .seed(&mut conn, Some("%1"), &mut RecordingSink::default())
        .expect("a vanished session lands instead of failing the attach");
    assert!(shown);
    assert_eq!(session.window, "@5");
    assert_eq!(session.status.session_id.as_deref(), Some("$1"));
}

/// The all-gone boundary: the target died mid-seed and the daemon lists
/// nothing to land on, so the seed reports an empty view (a clean end),
/// not an attach failure.
#[test]
fn seed_with_nothing_left_after_the_target_dies_ends_cleanly() {
    let mut script = FakeScript::default();
    script
        .replies
        .insert("pane-info -t %1".to_string(), "%1 @0 80x23".to_string());
    script
        .failing
        .insert("refresh-client -t %1 -C 80x23".to_string());
    script.failing.insert("list-panes -t @0".to_string());
    let (_rx, mut conn) = fake_daemon("seed-gone-all", script);
    let mut session = WindowSession::new(80, 25);
    let shown = session
        .seed(&mut conn, Some("%1"), &mut RecordingSink::default())
        .expect("nothing left is a clean end, not an error");
    assert!(!shown, "nothing to show");
}

/// A target that does not exist when the seed first asks stays an
/// attach error: only a landing that vanished after it resolved walks
/// to a survivor.
#[test]
fn seed_of_a_target_missing_at_resolution_is_still_an_error() {
    let mut script = torn_down_seed_script();
    for line in ["pane-info -t %1", "list-windows -t %1", "list-panes -t %1"] {
        script.failing.insert(line.to_string());
    }
    let (_rx, mut conn) = fake_daemon("seed-missing", script);
    let mut session = WindowSession::new(80, 25);
    let err = session
        .seed(&mut conn, Some("%1"), &mut RecordingSink::default())
        .expect_err("an unknown explicit target is an error");
    assert!(err.contains("no such target: %1"), "{err}");
}

/// A `pane-info` reply past the 10 s bound proves nothing about the
/// target's kind. The resolver used to fall through to the window probe,
/// which `list-panes -t %1` passes (it accepts pane targets), and adopt
/// `%1` as the window id; the seed then waited for a `%layout-change
/// %1` no daemon sends ("no %layout-change for %0" under host load).
#[test]
fn a_timed_out_pane_info_never_adopts_the_pane_as_a_window() {
    let mut script = FakeScript::default();
    script.delays.insert(
        "pane-info -t %1".to_string(),
        std::time::Duration::from_millis(10_500),
    );
    script.failing.insert("list-windows -t %1".to_string());
    script
        .replies
        .insert("list-panes -t %1".to_string(), "%1 0 *".to_string());
    let (_rx, mut conn) = fake_daemon("paneinfo-timeout", script);
    let err = resolve_window_and_pane(&mut conn, Some("%1"))
        .expect_err("a timed-out pane-info is not a window");
    assert!(err.contains("pane-info failed"), "{err}");
}
