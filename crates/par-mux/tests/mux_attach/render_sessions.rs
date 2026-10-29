//! Render mode across sessions and workspaces: targetless attach,
//! landing, cross-client follow, and pane/session/workspace teardown.

use super::*;

/// Regression (render-mode target-less resolution): `par-mux attach
/// --mode render` with no `-t` attaches to the newest session's newest
/// pane — the documented default — and renders it. `list-sessions`
/// replies `$N: name`; a whitespace split kept the colon (`$0:`), the
/// daemon's id parser rejected it, and the client exited 1 with "the
/// session has no windows". One session, one pane: the client must come
/// up (alt-screen enter + the pane's marker in the rendered frame) and
/// keys must still reach the pane — not exit.
/// The target-less render client attaches THE ONLY session: alt-screen
/// up (alt-screen enter + the pane's marker in the rendered frame) and
/// keys must still reach the pane — not exit.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_without_target_attaches_the_only_session() {
    let (fixture, _daemon, mut client) = fixture_with_session("rendernotarget");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // A distinctive line on the pane's screen: the rendered frame must
    // carry it once the client seeds from the replay.
    client
        .send(&format!("send-keys -t {pane} -l 'echo NO-TARGET-MARKER'"))
        .expect("seed marker");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane}"),
        echoed("NO-TARGET-MARKER"),
        Duration::from_secs(10),
        "NO-TARGET-MARKER executed on the pane screen",
    );

    // No -t anywhere: resolution is entirely the client's job.
    let (mut host, stderr) = spawn_attach_render(&fixture, &[]);
    let got = wait_for_output(&host, b"NO-TARGET-MARKER", Duration::from_secs(15));
    assert!(
        !got.is_empty(),
        "the render client with no -t must attach the session's pane and \
         render its content (alt-screen enter + the replayed marker). \
         stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    // Alt-screen enter: the render client took over the screen rather
    // than exiting with an error banner.
    assert!(
        got.windows(8).any(|w| w == b"\x1b[?1049h"),
        "the client must enter the alternate screen (render mode): {:?}",
        String::from_utf8_lossy(&got)
    );

    // It stays up and routes keys: an echo lands on the pane's screen
    // through the client's input router.
    host.to_child
        .write_all(b"echo STILL-UP-MARKER\r")
        .expect("type");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"STILL-UP-MARKER",
        Duration::from_secs(10),
        "STILL-UP-MARKER on the host screen",
    );
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture");
    assert!(
        capture.join("\n").contains("STILL-UP-MARKER"),
        "keys must reach the pane through the target-less client: {capture:?}"
    );
    host.killer.kill().ok();
}

/// The target-less render client REFUSES when several sessions exist: it
/// exits non-zero without attaching, and its stderr names every session
/// plus the `-t` hint (Paul, 2026-10-17: the user picks, not the client).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_without_target_refuses_when_several_sessions_exist() {
    let (fixture, _daemon, mut client) = fixture_with_session("renderoldest");
    client
        .send("new-session -s second")
        .expect("second session");
    // settle: the fresh panes' shells start before the client attaches
    std::thread::sleep(Duration::from_millis(500));

    let (mut host, stderr) = spawn_attach_render(&fixture, &[]);
    // The PTY harness wires the client's stderr into the slave, so the
    // refusal text rides the OUTPUT stream (the `stderr` buffer is a
    // placeholder the harness never fills).
    let got = wait_for_output(&host, b"multiple sessions exist", Duration::from_secs(10));
    assert_eq!(
        child_exit(&mut host, Duration::from_secs(10)),
        Some(1),
        "the no-target client must refuse several sessions and exit 1. output: {}",
        String::from_utf8_lossy(&got)
    );
    let text = String::from_utf8_lossy(&got).to_string();
    assert!(
        text.contains("multiple sessions exist")
            && text.contains("att")
            && text.contains("second")
            && text.contains("par-mux attach -t"),
        "the refusal must list the sessions and the hint: {text}"
    );
    let _ = stderr;
}

/// The target-less render client on a daemon with NO sessions creates the
/// default tree and attaches to it: `new-workspace -n default` builds
/// workspace + session `default`, a window named `1`, one unnamed pane.
/// The tree survives the client's detach (a session, not a client-scoped
/// view), so the daemon stays serving.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_without_target_creates_the_default_session() {
    let fixture = MuxFixture::new("renderdefault");
    let _daemon = common::spawn_daemon_auto_remove(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");

    let (mut host, stderr) = spawn_attach_render(&fixture, &[]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up — the create path attached",
    );
    // The create lands during the seed, after the mouse-capture byte the
    // expect above waited on: poll rather than assume.
    let deadline = Instant::now() + Duration::from_secs(10);
    let sessions = loop {
        let sessions = client.send("list-sessions").expect("list-sessions");
        if sessions.iter().any(|l| l.contains("$0: default")) {
            break sessions;
        }
        assert!(
            Instant::now() < deadline,
            "the default tree must be created by the attach. stderr: {}",
            stderr.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        sessions.iter().any(|l| l.contains("default: $0")),
        "the default tree's session must exist in the default workspace: {sessions:?}"
    );
    let windows = client.send("list-windows -t $0").expect("list-windows");
    assert!(
        windows
            .iter()
            .any(|l| l.split_whitespace().next() == Some("@0") && l.contains(" 1")),
        "the default window must be named \"1\": {windows:?}"
    );
    host.killer.kill().ok();
}

/// The tab strip, PTY level: a click on the second tab in the strip row
/// switches the shown window daemon-side — the client sends
/// `select-window` and the daemon's active-window marker moves — and no
/// `send-keys` reaches any pane from a strip click (no forwarding).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_tab_click_switches_the_active_window() {
    let (fixture, _daemon, mut client) = fixture_with_session("tabstrip");
    // Two windows with deterministic names: the herdr block "  one  " is
    // 7 cols (0-based 0..7), one gap column (7), so the second tab's
    // block "  two  " spans 0-based host cols 8..15 (1-based 9..15).
    client.send("new-window -t $0").expect("new-window");
    client.send("rename-window -t @0 one").expect("rename @0");
    client.send("rename-window -t @1 two").expect("rename @1");
    client.send("select-window -t @0").expect("back to @0");
    // settle: the fresh panes' shells start before the client attaches
    std::thread::sleep(Duration::from_millis(400));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", "@0"]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    expect_output(
        &host,
        b"one",
        Duration::from_secs(10),
        "tab strip names window one",
    );
    // Drain the settle paint so the post-click reads are click-attributable.
    while host.output_rx.try_recv().is_ok() {}

    // A press on the second tab (host row 1 = the strip).
    host.to_child
        .write_all(b"\x1b[<0;12;1M")
        .expect("click press");
    host.to_child.flush().ok();

    // The authority: the daemon's active-window marker moved to @1.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut activated = false;
    while std::time::Instant::now() < deadline {
        let windows = client.send("list-windows -t $0").expect("list-windows");
        if windows
            .iter()
            .any(|l| l.split_whitespace().next() == Some("@1") && l.contains('*'))
        {
            activated = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        activated,
        "the click must activate the clicked window daemon-side. stderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// prefix W (render mode): the chord sends `select-workspace` and lands
/// the view on the target workspace's session — the daemon's active-
/// workspace marker moves, and the target pane's seeded content re-seeds
/// into the frame (the select+resync contract's redraw half).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_workspace_chord_selects_and_lands() {
    let (fixture, _daemon, mut client) = fixture_with_session("wschord");
    // A second workspace with its own session; `new-workspace` makes it
    // the daemon's active one. Back to `main` first, so prefix W has one
    // deterministic next: lab.
    client.send("new-workspace -n lab").expect("new-workspace");
    client
        .send("new-session -s work -t lab")
        .expect("session in lab");
    client
        .send("select-workspace -t main")
        .expect("select main");
    // settle: the fresh panes' shells start before the client attaches
    std::thread::sleep(Duration::from_millis(300));

    // The rosters: `list-panes -t` takes a window or pane target, not a
    // session, so each pane is reached through its window. `lab` holds
    // its auto-spawned `$1` and the newer `$2 work` — the session lab
    // resumes as active, so the one the landing must show (card 01a11bd1:
    // the landing follows the resumed session, not the first listed).
    let pane_a = session_first_pane(&mut client, "$0");
    let pane_work = session_first_pane(&mut client, "$2");
    // The landing evidence, seeded before the chord: the lab pane's
    // screen carries the marker, and the post-chord re-seed must bring
    // it into the frame.
    client
        .send(&format!("send-keys -t {pane_work} -l 'echo WS-LAB-MARKER'"))
        .expect("marker");
    client
        .send(&format!("send-keys -t {pane_work} Enter"))
        .expect("Enter");

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    expect_output(
        &host,
        b"main",
        Duration::from_secs(10),
        "status row names the main workspace",
    );
    // Drain the settle paint so the post-chord reads are attributable.
    while host.output_rx.try_recv().is_ok() {}

    // prefix W: next workspace (main -> lab).
    host.to_child.write_all(&[0x02, b'W']).expect("prefix W");
    host.to_child.flush().ok();

    // The authority: the daemon's active-workspace marker moved to lab.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut selected = false;
    while std::time::Instant::now() < deadline {
        let workspaces = client.send("list-workspaces").expect("list-workspaces");
        if workspaces
            .iter()
            .any(|l| l.starts_with("+1:") && l.contains(" active"))
        {
            selected = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        selected,
        "prefix W must select the next workspace daemon-side. stderr: {}",
        stderr.lock().unwrap()
    );

    // The resync half: the chord-attributable output re-seeds the target
    // pane, whose seeded marker must appear in the reconstructed frame.
    let end = std::time::Instant::now() + Duration::from_secs(10);
    let mut all: Vec<u8> = Vec::new();
    while std::time::Instant::now() < end {
        match host.output_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => all.extend_from_slice(&bytes),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let (final_grid, _ever) = reconstructed_screen(&all, 24, 80);
    let shown = final_grid.join("\n");
    assert!(
        shown.contains("WS-LAB-MARKER"),
        "the chord must land the view on the new workspace's pane (its \
         seeded marker re-seeds in). screen:\n{shown}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// %workspaces-changed (render mode): a workspace add broadcast marks
/// the status stale, and the throttled re-query pulls the new
/// workspace's name onto the workspaces segment — the same discipline
/// %agent-state-changed follows for the roster chips.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_workspaces_changed_refreshes_the_status_segment() {
    let (fixture, _daemon, mut client) = fixture_with_session("wsstatus");
    let (mut host, stderr) = spawn_attach_render(&fixture, &[]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    expect_output(
        &host,
        b"$0:att",
        Duration::from_secs(10),
        "status row names the attached session",
    );
    // Drain the settle paint so the post-broadcast reads are
    // broadcast-attributable.
    while host.output_rx.try_recv().is_ok() {}

    // Workspace churn: a new workspace rides %workspaces-changed.
    client.send("new-workspace -n beta").expect("new-workspace");
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_secs(2));
    let mut all = Vec::new();
    while let Ok(bytes) = host.output_rx.try_recv() {
        all.extend_from_slice(&bytes);
    }
    let (_final, ever) = reconstructed_screen(&all, 24, 80);
    let ever_row: String = ever[23].iter().map(|r| format!("{r:?}\n")).collect();
    assert!(
        ever[23].iter().any(|r| r.contains("beta")),
        "the workspaces segment must show the new workspace after \
         %workspaces-changed. snapshots:\n{ever_row}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Clicking a workspace row in the side panel re-seeds the view (the
/// select+resync contract every switch follows). The re-seed must keep
/// the panel: its width rides the fresh renderer and its rows re-query —
/// the manual-pass report was the re-seed rebuilding the renderer
/// without either, so the panel vanished and the panes painted under
/// where it had been.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_workspace_click_reseeds_with_the_panel() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbsplit");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let pane_b = client
        .send(&format!("split-window -h -t {pane_a}"))
        .expect("split")
        .join("");
    let pane_b = pane_b.trim().to_string();
    print_marker_line(&mut client, &pane_b, "GEO-MARK-42");

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    expect_output(
        &host,
        b"GEO-MARK-42",
        Duration::from_secs(10),
        "GEO-MARK-42 on the host screen",
    );

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("workspaces"),
        "panel must paint on open. stderr: {}",
        stderr.lock().unwrap()
    );

    // A press ON the painted workspace row lands on it: 1-based host
    // rows are 1 = the tab strip (its lead carries the panel title),
    // 2 = the FIRST workspace row (the panel opens with its rows — no
    // in-panel header). The select+resync re-seed must
    // re-emit the panel's full frame (a fresh renderer diffs against an
    // empty buffer) with the panes still offset by the panel's width —
    // and REPEATED clicks must not ratchet the layout left: the re-seed
    // used to rebuild the renderer at the REPORTED width (grid less the
    // panel), so each re-seed narrowed the buffer and each report then
    // shrank again (the manual-pass click-shrinks-the-panes report).
    for click in 1..=2 {
        host.to_child
            .write_all(&[0x1b, b'[', b'<', b'0', b';', b'3', b';', b'2', b'M'])
            .expect("panel click");
        host.to_child.flush().ok();
        let mut got = wait_for_output(&host, b"workspaces", Duration::from_secs(10));
        got.extend(wait_for_output(
            &host,
            b"\x00-never",
            Duration::from_millis(600),
        ));
        assert!(
            plain_text(&got).contains("workspaces"),
            "click {click}: the re-seed must keep the panel painted"
        );
        let col = cup_col_of_text(&got, "GEO-MARK-42");
        assert_eq!(
            col,
            Some(52),
            "click {click}: the right pane must stay put (no ratchet). text: {:?}",
            plain_text(&got)
        );
    }
    host.killer.kill().ok();
}

/// An explicit session target lands on that session's multi-pane window:
/// the seeded markers render, and the background workspace's session does
/// not leak into the frame.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_explicit_target_lands_on_its_sessions_window() {
    let (fixture, _daemon, mut client) = fixture_with_session("dispseed");
    seed_multi_pane(&mut client, "$0");
    client
        .send("new-workspace -n other")
        .expect("new-workspace");
    let other = session_first_pane(&mut client, "$1");
    echo_marker(&mut client, &other, "OTHER-MARK");
    client
        .send("select-workspace -t main")
        .expect("back to main");
    // settle: the fresh panes' shells start before the client attaches
    std::thread::sleep(Duration::from_millis(500));

    // A pinned target: the target-less rule refuses when several sessions
    // exist, and this test's subject is the landing, not the pick.
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", "$0"]);
    let mut all = Vec::new();
    let shown = screen_until(&host, &mut all, Duration::from_secs(10), shows_multi);
    assert!(
        shows_multi(&shown) && !shown.contains("OTHER-MARK"),
        "a `-t $0` attach must land on that session's multi-pane \
         window. screen:\n{shown}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Card 01a11bd1 symptom 2: client A lands on the workspace's sibling
/// session (prefix `(`) — that landing moves the daemon's displayed
/// session, so client B follows to it instead of staying on (or, after
/// a later follow, reverting to) the session the pointer still named.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_session_landing_moves_the_display_and_the_other_client_follows() {
    let (fixture, _daemon, mut client) = fixture_two_session_workspace("dispfollow");
    // Both clients pin the solo session: the target-less rule refuses
    // when several sessions exist; the chord landing below moves the
    // display and B follows through the broadcast.
    let (mut a, a_err) = spawn_attach_render(&fixture, &["-t", "$2"]);
    let mut a_all = Vec::new();
    let _ = screen_until(&a, &mut a_all, Duration::from_secs(10), |s| {
        s.contains("SOLO-MARK")
    });
    let (mut b, b_err) = spawn_attach_render(&fixture, &["-t", "$2"]);
    let mut b_all = Vec::new();
    let shown = screen_until(&b, &mut b_all, Duration::from_secs(10), |s| {
        s.contains("SOLO-MARK")
    });
    assert!(
        shown.contains("SOLO-MARK"),
        "B seeds on the displayed solo session:\n{shown}"
    );
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(300));
    while b.output_rx.try_recv().is_ok() {}
    b_all.clear();

    // A: prefix ( — the previous session ($2 -> $1).
    a.to_child.write_all(&[0x02, b'(']).expect("prefix (");
    a.to_child.flush().ok();
    let a_shown = screen_until(&a, &mut a_all, Duration::from_secs(10), shows_multi);
    assert!(
        shows_multi(&a_shown),
        "A lands on the multi-pane session:\n{a_shown}\nstderr: {}",
        a_err.lock().unwrap()
    );
    let b_shown = screen_until(&b, &mut b_all, Duration::from_secs(10), shows_multi);
    assert!(
        shows_multi(&b_shown) && !b_shown.contains("SOLO-MARK"),
        "B must follow A's landing to the multi-pane session. screen:\n{b_shown}\nstderr: {}",
        b_err.lock().unwrap()
    );
    let displayed = client
        .send("switch-client")
        .expect("displayed-session query");
    assert_eq!(
        displayed.join("").trim(),
        "$1",
        "the daemon's displayed session moved"
    );
    a.killer.kill().ok();
    b.killer.kill().ok();
}

/// The landing is persisted across a daemon restart: the restored
/// daemon's displayed session is the one the client last landed on
/// (`switch-client` names it), and an attach pinned to it lands on the
/// multi-pane window. (The target-less attach now refuses on several
/// sessions — the persistence the pointer carries is queried, not
/// silently attached to.)
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_landing_survives_a_restart_for_the_next_attach() {
    let (fixture, mut daemon, client) = fixture_two_session_workspace("disprestart");
    drop(client);
    let (mut a, a_err) = spawn_attach_render(&fixture, &["-t", "$2"]);
    let mut a_all = Vec::new();
    let _ = screen_until(&a, &mut a_all, Duration::from_secs(10), |s| {
        s.contains("SOLO-MARK")
    });
    a.to_child.write_all(&[0x02, b'(']).expect("prefix (");
    a.to_child.flush().ok();
    let a_shown = screen_until(&a, &mut a_all, Duration::from_secs(10), shows_multi);
    assert!(
        shows_multi(&a_shown),
        "A lands on the multi-pane session:\n{a_shown}\nstderr: {}",
        a_err.lock().unwrap()
    );
    a.killer.kill().ok();
    let _ = child_exit(&mut a, Duration::from_secs(5));

    common::sigterm_clean(&mut daemon);
    let _restarted = common::spawn_daemon(&fixture);
    wait_listening(fixture.socket());

    let (mut c, c_err) = spawn_attach_render(&fixture, &["-t", "$1"]);
    let mut c_all = Vec::new();
    let shown = screen_until(&c, &mut c_all, Duration::from_secs(10), shows_multi);
    assert!(
        shows_multi(&shown) && !shown.contains("SOLO-MARK"),
        "an attach pinned to the persisted landing session shows its \
         multi-pane window. screen:\n{shown}\nstderr: {}",
        c_err.lock().unwrap()
    );
    c.killer.kill().ok();
}

/// Card 01a11bd1 symptom 3, the landing half: client A's prefix `c` in
/// a sibling session it landed on moves the display there too, so B —
/// on the session the pointer used to name — follows to A's new tab.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_new_window_in_another_client_reaches_this_client() {
    let (fixture, _daemon, _client) = fixture_two_session_workspace("newwinfollow");
    // Both clients pin the solo session: the target-less rule refuses
    // when several sessions exist; the landing chord below drives the
    // display moves the follow machinery tests.
    let (mut a, a_err) = spawn_attach_render(&fixture, &["-t", "$2"]);
    let mut a_all = Vec::new();
    let _ = screen_until(&a, &mut a_all, Duration::from_secs(10), |s| {
        s.contains("SOLO-MARK")
    });
    let (mut b, b_err) = spawn_attach_render(&fixture, &["-t", "$2"]);
    let mut b_all = Vec::new();
    let _ = screen_until(&b, &mut b_all, Duration::from_secs(10), |s| {
        s.contains("SOLO-MARK")
    });
    // A lands on the multi-pane sibling first (prefix `(`), then opens
    // a tab there.
    a.to_child.write_all(&[0x02, b'(']).expect("prefix (");
    a.to_child.flush().ok();
    let _ = screen_until(&a, &mut a_all, Duration::from_secs(10), shows_multi);
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(500));
    while b.output_rx.try_recv().is_ok() {}
    b_all.clear();
    a.to_child.write_all(&[0x02, b'c']).expect("prefix c");
    a.to_child.flush().ok();
    // The chord opens the same seeded new-tab prompt the tab strip's
    // ` + ` opens (the seed is one past the highest window id — "@1"
    // here, so "2"); committing the seed is what creates the tab B
    // follows.
    let _ = wait_for_output(&a, b" new tab ", Duration::from_secs(10));
    a.to_child.write_all(b"\r").expect("enter commits the seed");
    // B's strip names A's session's two tabs (`1` and the seeded `2`)
    // and its screen leaves the multi-pane view for the fresh window.
    let end = std::time::Instant::now() + Duration::from_secs(10);
    let mut followed = false;
    while std::time::Instant::now() < end && !followed {
        if let Ok(bytes) = b.output_rx.recv_timeout(Duration::from_millis(100)) {
            b_all.extend_from_slice(&bytes);
        }
        let (grid, _) = reconstructed_screen(&b_all, 24, 80);
        followed =
            grid[0].contains(" 1 ") && grid[0].contains(" 2 ") && !shows_multi(&grid.join("\n"));
    }
    let (grid, _) = reconstructed_screen(&b_all, 24, 80);
    assert!(
        followed,
        "B must follow A's new tab. screen:\n{}\nA stderr: {}\nB stderr: {}",
        grid.join("\n"),
        a_err.lock().unwrap(),
        b_err.lock().unwrap()
    );
    a.killer.kill().ok();
    b.killer.kill().ok();
}

/// Card 01a11bd1 symptom 3: a window added by another client (here the
/// CLI) reaches this render client's tab strip — `%window-add` marks the
/// status stale like every other roster cue.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_window_add_elsewhere_refreshes_the_tab_strip() {
    let (fixture, _daemon, mut client) = fixture_with_session("tabadd");
    let (mut host, stderr) = spawn_attach_render(&fixture, &[]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    expect_output(
        &host,
        b"$0:att",
        Duration::from_secs(10),
        "status row names the attached session",
    );
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(300));
    while host.output_rx.try_recv().is_ok() {}

    client
        .send("new-window -t $0 -n cliwin")
        .expect("new-window elsewhere");
    let end = std::time::Instant::now() + Duration::from_secs(5);
    let mut all = Vec::new();
    let mut seen = false;
    while std::time::Instant::now() < end && !seen {
        if let Ok(bytes) = host.output_rx.recv_timeout(Duration::from_millis(100)) {
            all.extend_from_slice(&bytes);
        }
        let (_final, ever) = reconstructed_screen(&all, 24, 80);
        seen = ever[0].iter().any(|r| r.contains("cliwin"));
    }
    assert!(
        seen,
        "the tab strip must gain the window another client added. stderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Card 01a11c3a (Phase B manual pass): Ctrl+D exits the focused pane's
/// shell under the product default (`remain-on-exit = false`, the pane is
/// auto-removed). The window keeps its other panes, so BOTH render
/// clients must stay attached and repaint the survivors — the pane's
/// death is not the session's.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_ctrl_d_in_a_pane_keeps_both_clients_on_the_survivors() {
    let fixture = MuxFixture::new("ctrldpane");
    let _daemon = common::spawn_daemon_auto_remove(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
    client.send("new-session -s att").expect("new-session");
    seed_multi_pane(&mut client, "$0");
    let window = client.send("list-windows -t $0").expect("list-windows")[0]
        .split_whitespace()
        .next()
        .expect("a window")
        .to_string();
    // settle: the fresh panes' shells start before the client attaches
    std::thread::sleep(Duration::from_millis(500));

    let (mut a, a_err) = spawn_attach_render(&fixture, &[]);
    let mut a_all = Vec::new();
    let _ = screen_until(&a, &mut a_all, Duration::from_secs(10), shows_multi);
    let (mut b, b_err) = spawn_attach_render(&fixture, &[]);
    let mut b_all = Vec::new();
    let _ = screen_until(&b, &mut b_all, Duration::from_secs(10), shows_multi);
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(500));
    while client.notifications().try_recv().is_ok() {}

    // Ctrl+D through client A reaches its focused pane's shell at an
    // empty prompt: the shell exits, the daemon reaps and removes it.
    a.to_child.write_all(&[0x04]).expect("Ctrl+D");
    a.to_child.flush().ok();
    assert!(
        saw_pane_exited(&client, Duration::from_secs(10)),
        "the shell's exit must reach observers as %pane-exited"
    );
    let end = Instant::now() + Duration::from_secs(5);
    while pane_count(&mut client, &window) != 2 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        pane_count(&mut client, &window),
        2,
        "the dead pane is removed"
    );

    // Both clients must survive the removal (give the status refresh and
    // reseed time to run their course).
    // negative check: nothing positive to wait on, so the window is fixed
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        child_exit(&mut a, Duration::from_millis(200)),
        None,
        "client A must stay attached after its pane's shell exited. stderr: {}",
        a_err.lock().unwrap()
    );
    assert_eq!(
        child_exit(&mut b, Duration::from_millis(200)),
        None,
        "client B must stay attached after a pane's shell exited. stderr: {}",
        b_err.lock().unwrap()
    );
    // The survivors keep painting: a marker typed into a surviving pane
    // reaches both clients' screens.
    let survivor = client
        .send(&format!("list-panes -t {window}"))
        .expect("list-panes")
        .iter()
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .find(|p| p.starts_with('%'))
        .expect("a survivor");
    echo_marker(&mut client, &survivor, "AFTER-EXIT");
    for (host, all, name) in [(&a, &mut a_all, "A"), (&b, &mut b_all, "B")] {
        let shown = screen_until(host, all, Duration::from_secs(10), |s| {
            s.contains("AFTER-EXIT")
        });
        assert!(
            shown.contains("AFTER-EXIT"),
            "client {name} must keep rendering the surviving panes:\n{shown}"
        );
    }
    a.killer.kill().ok();
    b.killer.kill().ok();
}

/// Card 01a13d03, the exit cascade lands up the tree: when the exiting
/// shell is its session's LAST pane, the cascade removes the session, but
/// the workspace keeps a sibling session — so both render clients land on
/// the sibling's active window and repaint it instead of ending.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_ctrl_d_on_a_sessions_last_pane_lands_on_the_sibling_session() {
    let fixture = MuxFixture::new("ctrldlast");
    let _daemon = common::spawn_daemon_auto_remove(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
    client.send("new-session -s att").expect("new-session");
    // A second session in the same workspace: the landing site once the
    // shown session cascades away.
    let pane = session_first_pane(&mut client, "$0");
    client.send("new-session -s other").expect("second session");
    let sibling = session_first_pane(&mut client, "$1");
    // settle: the fresh panes' shells start before the client attaches
    std::thread::sleep(Duration::from_millis(500));

    let (mut a, a_err) = spawn_attach_render(&fixture, &["-t", &pane]);
    expect_output(
        &a,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    let (mut b, b_err) = spawn_attach_render(&fixture, &["-t", &pane]);
    expect_output(
        &b,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(800));
    while client.notifications().try_recv().is_ok() {}

    a.to_child.write_all(&[0x04]).expect("Ctrl+D");
    a.to_child.flush().ok();
    assert!(
        saw_pane_exited(&client, Duration::from_secs(10)),
        "the shell's exit must reach observers as %pane-exited"
    );
    let end = Instant::now() + Duration::from_secs(5);
    while client
        .send("list-sessions")
        .expect("list-sessions")
        .iter()
        .any(|l| l.contains("$0:"))
    {
        assert!(
            Instant::now() < end,
            "the emptied session must cascade away"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // negative check: nothing positive to wait on, so the window is fixed
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        child_exit(&mut a, Duration::from_millis(200)),
        None,
        "client A must land on the sibling session, not end. stderr: {}",
        a_err.lock().unwrap()
    );
    assert_eq!(
        child_exit(&mut b, Duration::from_millis(200)),
        None,
        "client B must land on the sibling session, not end. stderr: {}",
        b_err.lock().unwrap()
    );
    // Both repaint the sibling session's window.
    echo_marker(&mut client, &sibling, "LANDED-SIBLING");
    for (host, name) in [(&a, "A"), (&b, "B")] {
        let mut all = Vec::new();
        let shown = screen_until(host, &mut all, Duration::from_secs(10), |s| {
            s.contains("LANDED-SIBLING")
        });
        assert!(
            shown.contains("LANDED-SIBLING"),
            "client {name} must land on the sibling session's window:\n{shown}"
        );
    }
    a.killer.kill().ok();
    b.killer.kill().ok();
}

/// Paul's rule at the client rung (2026-10-18): Ctrl+D exits the ONLY
/// pane of the only workspace under auto-remove, the full cascade takes
/// the pane, its session, and its workspace, and with nothing left to
/// show the daemon exits after its exit-when-empty grace — so the render
/// client ends instead of holding a dead pane. The keep-one-tab hold is
/// the `set-workspace-respawn` opt-in now.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_ctrl_d_on_the_workspaces_last_pane_cascades_and_ends() {
    let fixture = MuxFixture::new("ctrldkeep");
    let _daemon = common::spawn_daemon_auto_remove(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
    client.send("new-session -s att").expect("new-session");
    // settle: the fresh pane's shell starts before the client attaches
    std::thread::sleep(Duration::from_millis(500));

    let (mut a, a_err) = spawn_attach_render(&fixture, &[]);
    expect_output(
        &a,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(800));
    while client.notifications().try_recv().is_ok() {}

    a.to_child.write_all(&[0x04]).expect("Ctrl+D");
    a.to_child.flush().ok();
    assert!(
        saw_pane_exited(&client, Duration::from_secs(10)),
        "the shell's exit must reach observers as %pane-exited"
    );
    // The cascade empties the tree; the daemon exits after its
    // exit-when-empty grace (connected client or not) and the client ends
    // with it.
    child_exit(&mut a, Duration::from_secs(20)).unwrap_or_else(|| {
        panic!(
            "the client ends when the emptied daemon exits. stderr: {}",
            a_err.lock().unwrap()
        )
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if par_mux::mux::MuxClient::connect(fixture.socket()).is_err() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon is gone after the grace"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    a.killer.kill().ok();
}

/// Card 01a13d03, the workspace rung of the landing walk: an external
/// `kill-workspace` of the SHOWN workspace leaves another workspace, so the
/// render client lands on the daemon's active workspace's window and
/// repaints it instead of ending (the cached workspace is gone, so the walk
/// falls through to the daemon's active one).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_kill_of_the_shown_workspace_lands_on_the_surviving_one() {
    let fixture = MuxFixture::new("killshownws");
    let _daemon = common::spawn_daemon_auto_remove(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
    client.send("new-session -s att").expect("new-session");
    let shown_pane = session_first_pane(&mut client, "$0");
    let shown_ws = client
        .send("list-workspaces")
        .expect("list-workspaces")
        .first()
        .and_then(|l| l.split(':').next())
        .expect("the first workspace")
        .to_string();
    // A second workspace (with its own first session), then select the
    // first again so the client attaches to it.
    let other_ws = client
        .send("new-workspace -n other")
        .expect("new-workspace")
        .first()
        .expect("workspace id")
        .trim()
        .to_string();
    let other_session = client
        .send(&format!("list-sessions -t {other_ws}"))
        .expect("list-sessions")
        .iter()
        .find_map(|l| {
            l.rfind('$')
                .map(|i| l[i..].split(':').next().unwrap_or("").to_string())
        })
        .expect("the other workspace's session");
    let survivor = session_first_pane(&mut client, &other_session);
    client
        .send(&format!("select-workspace -t {shown_ws}"))
        .expect("select-workspace");
    echo_marker(&mut client, &shown_pane, "SHOWN-WORKSPACE");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {shown_pane}"),
        |s| s.contains("SHOWN-WORKSPACE"),
        Duration::from_secs(10),
        "the shown pane printed its marker",
    );

    let (mut a, _a_err) = spawn_attach_render(&fixture, &["-t", &shown_pane]);
    // Paint barrier: the kill must hit the workspace the client SHOWS.
    // The mouse-mode enable is written before the seed even resolves
    // the target, and a kill that beats the resolution is an unknown
    // explicit target — an attach error by contract, not a landing.
    let mut all = Vec::new();
    let painted = screen_until(&a, &mut all, common::CEILING, |s| {
        s.contains("SHOWN-WORKSPACE")
    });
    assert!(
        painted.contains("SHOWN-WORKSPACE"),
        "the client painted the shown workspace before the kill:\n{painted}"
    );
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(800));
    while a.output_rx.try_recv().is_ok() {}

    let killed = client
        .send_checked(&format!("kill-workspace -t {shown_ws}"))
        .expect("kill-workspace");
    assert!(killed.ok, "kill-workspace succeeds: {:?}", killed.body);

    // negative check: nothing positive to wait on, so the window is fixed
    std::thread::sleep(Duration::from_secs(2));
    if let Some(code) = child_exit(&mut a, Duration::from_millis(200)) {
        // The client's own error line rides the pty stream, not stderr.
        let mut tail = Vec::new();
        while let Ok(bytes) = a.output_rx.recv_timeout(Duration::from_millis(300)) {
            tail.extend_from_slice(&bytes);
        }
        panic!(
            "the client must land on the surviving workspace, not end (exit {code}): {:?}",
            String::from_utf8_lossy(&tail)
        );
    }
    echo_marker(&mut client, &survivor, "LANDED-WORKSPACE");
    let mut all = Vec::new();
    let shown = screen_until(&a, &mut all, Duration::from_secs(10), |s| {
        s.contains("LANDED-WORKSPACE")
    });
    assert!(
        shown.contains("LANDED-WORKSPACE"),
        "the client must repaint the surviving workspace's window:\n{shown}"
    );
    a.killer.kill().ok();
}

/// Card 01a13d03, the all-gone boundary: an explicit `kill-workspace` of
/// the daemon's only workspace leaves nothing to land on, so the render
/// client ends cleanly. It must end well inside the daemon's 5 s
/// exit-when-empty grace — the daemon still serving proves the end came
/// from the client's own landing walk, not the broadcast `%exit`.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_kill_workspace_of_the_only_workspace_ends_the_client() {
    let fixture = MuxFixture::new("killallws");
    let _daemon = common::spawn_daemon_auto_remove(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
    client.send("new-session -s att").expect("new-session");
    // settle: the fresh pane's shell starts before the client attaches
    std::thread::sleep(Duration::from_millis(500));

    let (mut a, a_err) = spawn_attach_render(&fixture, &[]);
    expect_output(
        &a,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    // output settle: collect the trailing redraw burst
    std::thread::sleep(Duration::from_millis(800));

    let workspace = client
        .send("list-workspaces")
        .expect("list-workspaces")
        .first()
        .and_then(|l| l.split(':').next())
        .expect("one workspace")
        .to_string();
    let killed = client
        .send_checked(&format!("kill-workspace -t {workspace}"))
        .expect("kill-workspace");
    assert!(killed.ok, "kill-workspace succeeds: {:?}", killed.body);
    assert_eq!(
        child_exit(&mut a, Duration::from_secs(3)),
        Some(0),
        "the client must end cleanly when nothing is left to show. stderr: {}",
        a_err.lock().unwrap()
    );
    assert!(
        client.send("list-workspaces").is_ok(),
        "the daemon still serves: the client ended on its own landing walk"
    );
}

/// Card 01a11c3a, the multi-tab shape: Ctrl+D exits the ONLY pane of the
/// shown window, so the daemon closes that window — but the session keeps
/// its other window. tmux semantics: both clients stay attached and land
/// on the session's surviving window instead of ending the view.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_ctrl_d_closing_a_tab_keeps_both_clients_in_the_session() {
    let fixture = MuxFixture::new("ctrldtab");
    let _daemon = common::spawn_daemon_auto_remove(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
    client.send("new-session -s att").expect("new-session");
    let first = session_first_pane(&mut client, "$0");
    echo_marker(&mut client, &first, "KEEP-TAB");
    // A second, single-pane window — selected, so both clients show it.
    let doomed_window = client
        .send("new-window -t $0")
        .expect("new-window")
        .first()
        .expect("window id")
        .trim()
        .to_string();
    client
        .send(&format!("select-window -t {doomed_window}"))
        .expect("select-window");
    // settle: the fresh panes' shells start before the client attaches
    std::thread::sleep(Duration::from_millis(500));

    let (mut a, a_err) = spawn_attach_render(&fixture, &[]);
    expect_output(
        &a,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    let (mut b, b_err) = spawn_attach_render(&fixture, &[]);
    expect_output(
        &b,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(800));
    while a.output_rx.try_recv().is_ok() {}
    while b.output_rx.try_recv().is_ok() {}
    while client.notifications().try_recv().is_ok() {}

    a.to_child.write_all(&[0x04]).expect("Ctrl+D");
    a.to_child.flush().ok();
    assert!(
        saw_pane_exited(&client, Duration::from_secs(10)),
        "the shell's exit must reach observers as %pane-exited"
    );
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let windows = client.send("list-windows -t $0").expect("list-windows");
        if !windows.iter().any(|l| l.starts_with(&doomed_window)) {
            assert_eq!(
                windows.len(),
                1,
                "the session keeps one window: {windows:?}"
            );
            break;
        }
        assert!(Instant::now() < end, "the emptied window must close");
        std::thread::sleep(Duration::from_millis(50));
    }

    // negative check: nothing positive to wait on, so the window is fixed
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        child_exit(&mut a, Duration::from_millis(200)),
        None,
        "client A must stay attached when its tab closed but the session lives. stderr: {}",
        a_err.lock().unwrap()
    );
    assert_eq!(
        child_exit(&mut b, Duration::from_millis(200)),
        None,
        "client B must stay attached when the shown tab closed but the session lives. stderr: {}",
        b_err.lock().unwrap()
    );
    // Both land on the surviving window and repaint it.
    echo_marker(&mut client, &first, "LANDED-TAB");
    for (host, name) in [(&a, "A"), (&b, "B")] {
        let mut all = Vec::new();
        let shown = screen_until(host, &mut all, Duration::from_secs(10), |s| {
            s.contains("LANDED-TAB")
        });
        assert!(
            shown.contains("LANDED-TAB"),
            "client {name} must land on the session's surviving window:\n{shown}"
        );
    }
    a.killer.kill().ok();
    b.killer.kill().ok();
}
