//! Render mode's agent and notification surfaces: the roster, state
//! notifications and their delivery, and the settings dialog.

use super::*;

/// Acceptance criterion 1 (render mode, PTY level): the status bar. The
/// client's output carries a bottom-row draw containing the session name,
/// and an agent state change broadcast (%agent-state-changed) triggers the
/// throttled re-query — the roster chip appears/updates on the row.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_status_bar_shows_sessions_and_updates_on_agent_changes() {
    let (fixture, _daemon, mut client) = fixture_with_session("statusbar");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    // Settle: alt-screen enter + first frame.
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );

    // Agent churn: a hook report claims the pane; %agent-state-changed
    // marks the status stale and the throttled re-query pulls the roster
    // row onto the side panel.
    let reply = hook_report(
        fixture.socket(),
        &format!(
            r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"kimi","state":"blocked","seq":1,"source":"par-mux:test"}}}}"#
        ),
    );
    assert!(
        reply.contains(r#""result":"ok""#),
        "claim accepted: {reply}"
    );
    // The refresh runs at the next pump pass (16 ms cadence); give it a
    // generous beat, then reconstruct the row from everything received.
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_secs(2));
    let mut all = Vec::new();
    while let Ok(bytes) = host.output_rx.try_recv() {
        all.extend_from_slice(&bytes);
    }
    let (_final, ever) = reconstructed_screen(&all, 24, 80);
    let ever_row: String = ever[23].iter().map(|r| format!("{r:?}\n")).collect();
    assert!(
        ever[23].iter().any(|r| r.contains("$0:att")),
        "the status bar must show the shown session on row 24. snapshots:\n{ever_row}\nstderr: {}",
        stderr.lock().unwrap()
    );
    // The roster left the bar: this fixture runs sidebar-off, so the
    // claim must NOT land here — the side panel carries it (the panel
    // test below).
    let ever_grid: String = ever
        .iter()
        .map(|row| row.iter().map(|r| r.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !ever_grid.contains("kimi"),
        "the roster left the bar (the side panel renders it). snapshots:\n{ever_grid}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// The side panel carries the agent roster: a hook report claims the
/// pane, the throttled re-query fills the panel's agents section, and
/// the agent's name lands in the strip columns (the sidebar-on fixture;
/// the sidebar-off bar contract is the test above).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_side_panel_carries_the_agent_roster() {
    let (fixture, _daemon, mut client) = fixture_with_session("panelroster");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach_with_config(
        &fixture,
        &["-t", &pane],
        "render",
        "[client]\nsidebar-on-launch = true\n",
    );
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );

    // The panel's workspace row painted, so the first status refresh
    // landed; then let startup refreshes, the launch refit, and the first
    // cursor heartbeat settle so no incidental refresh lands after the
    // claim and repaints the panel for it.
    let mut all = Vec::new();
    assert!(
        panel_ever_contains(&host, &mut all, 19, "main", Duration::from_secs(10)),
        "the side panel never painted its workspace row; stderr: {}",
        stderr.lock().unwrap()
    );
    screen_stable(
        &host,
        &mut all,
        Duration::from_millis(1500),
        Duration::from_secs(10),
    );
    all.clear();

    let reply = hook_report(
        fixture.socket(),
        &format!(
            r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"kimi","state":"blocked","seq":1,"source":"par-mux:test"}}}}"#
        ),
    );
    assert!(
        reply.contains(r#""result":"ok""#),
        "claim accepted: {reply}"
    );
    // 1.5 s: the scrape liveness sweep may release the test claim after
    // about 2 s, and `ever` still sees a row a later repaint removed.
    let seen = panel_ever_contains(&host, &mut all, 19, "kimi", Duration::from_millis(1500));
    let (_final, ever) = reconstructed_screen(&all, 24, 80);
    let ever_grid: String = ever
        .iter()
        .map(|row| row.iter().map(|r| r.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        seen,
        "the side panel did not show the claim within 1.5 s of %agent-state-changed. \
         snapshots:\n{ever_grid}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// The change-cursor-shape pointer cue END TO END (card 01a13d8c): the
/// live render client under a PTY writes OSC 22 to the host terminal —
/// the bytes ghostty/kitty turn into the resize pointer — when the
/// pointer hovers or drags a divider, and restores `text` after.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_change_cursor_shape_writes_osc22_to_the_host() {
    let (fixture, _daemon, mut client) = fixture_with_session("ptr22");
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
    assert!(
        pane_b.trim().starts_with('%'),
        "split returned a pane id: {pane_b:?}"
    );
    let (mut host, stderr) = spawn_attach_with_config(
        &fixture,
        &["-t", &pane_a],
        "render",
        "[client]\nchange-cursor-shape = true\nsidebar-on-launch = false\n",
    );
    let mut seed = expect_output(
        &host,
        b"\x1b[?1003h",
        common::CEILING,
        "render client up (any-event mouse reporting)",
    );
    // Two side-by-side panes in the 80-col window put the divider at
    // host col 40 (SGR col 40); content rows sit under the tab strip.
    // Probe once the divider is laid out where the hover lands (row 5,
    // 0-based col 39), so the layout the client hit-tests is the split.
    expect_screen(
        &host,
        &mut seed,
        |s| cells(s, 5, 39, 1) != " ",
        "the divider between the two panes",
    );
    // The seed's trailing bytes must not reach the hover's collector.
    while host.output_rx.try_recv().is_ok() {}

    // 1) A no-button hover ON the divider (cb 35).
    host.to_child
        .write_all(b"\x1b[<35;40;6M")
        .expect("hover divider");
    host.to_child.flush().ok();
    // The X11 synonym rides the SAME write (iTerm2's OSC 22 map knows
    // only X11 cursor-font names and resets on unknown ones), so it is
    // asserted on the matched buffer — a fresh expect_output starts a
    // new collector and would never see bytes that arrived earlier.
    let hover_out = expect_output(
        &host,
        b"]22;ew-resize",
        common::CEILING,
        "hover on the divider writes the resize pointer",
    );
    assert!(
        hover_out
            .windows(b"]22;sb_h_double_arrow".len())
            .any(|w| w == b"]22;sb_h_double_arrow"),
        "the X11 synonym rides the hover write: {:?}",
        String::from_utf8_lossy(&hover_out)
    );
    // 2) The same hover off the divider restores the terminal's own.
    host.to_child
        .write_all(b"\x1b[<35;10;6M")
        .expect("hover off");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"]22;text",
        common::CEILING,
        "leaving the divider restores the pointer",
    );
    // 3) A live drag: press on the divider, motion with the button
    //    held over a pane (cb 32) — the drag keeps the shape wherever
    //    the pointer wanders.
    host.to_child
        .write_all(b"\x1b[<0;40;6M")
        .expect("press divider");
    host.to_child
        .write_all(b"\x1b[<32;36;6M")
        .expect("drag motion");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"]22;ew-resize",
        common::CEILING,
        "a live drag keeps the resize pointer",
    );
    // 4) Release ends the drag off the divider: the pointer restores.
    host.to_child.write_all(b"\x1b[<0;36;6m").expect("release");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"]22;text",
        common::CEILING,
        "release restores the pointer",
    );

    // Detach clean: prefix d, exit 0 — and the exit path's own restore
    // rides the same bytes.
    host.to_child.write_all(&[0x02, b'd']).expect("prefix d");
    host.to_child.flush().ok();
    let code = child_exit(&mut host, common::CEILING);
    assert_eq!(
        code,
        Some(0),
        "prefix d must end the client with exit 0. stderr: {}",
        stderr.lock().unwrap()
    );
}

/// An agent in a window the client is NOT showing announces its
/// selected state through the host terminal: the real client writes
/// OSC 9 with the agent's name when the push lands, and the claim's own
/// first state (a background claim straight into `blocked`) notifies
/// too, while an agent in the shown window stays quiet.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_background_agent_state_notifies_the_host() {
    let (fixture, _daemon, mut client) = fixture_with_session("notifyhost");
    let shown = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let background_window = client
        .send("new-window -d")
        .expect("new-window")
        .join("")
        .trim()
        .to_string();
    let background = client
        .send(&format!("list-panes -t {background_window}"))
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("the background pane")
        .to_string();
    // The pin is for the OSC 9 arm: the spawn pins a non-kitty host, so
    // the suite's expectation holds whatever terminal launched it (a
    // kitty host delivers the same notification as chunked OSC 99).
    let (mut host, stderr) = spawn_attach_with_env(
        &fixture,
        &["-t", &shown],
        "render",
        "[client]\n",
        &[("TERM_PROGRAM", "iTerm.app")],
    );
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up",
    );
    let mut all = Vec::new();
    screen_stable(
        &host,
        &mut all,
        Duration::from_millis(1000),
        Duration::from_secs(10),
    );
    let report = |pane: &str, agent: &str, state: &str, seq: u32| {
        hook_report(
            fixture.socket(),
            &format!(
                r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"{agent}","state":"{state}","seq":{seq},"source":"par-mux:test"}}}}"#
            ),
        )
    };
    // The shown pane's agent: in view, no notification.
    assert!(report(&shown, "fgagent", "blocked", 1).contains(r#""result":"ok""#));
    // The background pane's agent goes blocked: OSC 9 with its name.
    assert!(report(&background, "bgagent", "blocked", 1).contains(r#""result":"ok""#));
    let got = wait_for_output(&host, b"bgagent: blocked", Duration::from_secs(10));
    let text = String::from_utf8_lossy(&got);
    assert!(
        text.contains("\x1b]9;par-mux: ") && text.contains("bgagent: blocked\x07"),
        "the background agent's state reaches the host as OSC 9. stderr: {} bytes: {text:?}",
        stderr.lock().unwrap()
    );
    let all_text = String::from_utf8_lossy(&all).to_string() + &text;
    assert!(
        !all_text.contains("fgagent: blocked"),
        "the shown window's agent stays quiet: {all_text:?}"
    );
    host.killer.kill().ok();
}

/// Investigating Paul's report (2026-10-15: `notify-delivery =
/// "terminal"` never showed in ghostty or iTerm2 while `inside` and
/// `system` worked) found a latent misdetection: a `KITTY_WINDOW_ID`
/// leaked into a non-kitty host's environment (a host started from a
/// kitty shell) switched the escape to kitty's OSC 99, which neither
/// ghostty nor iTerm2 parses. Paul's own live attach processes carried no
/// `KITTY_*` variables, so this is a bug class, not proven to be his
/// cause. Drives the reload path (delivery off at launch, `terminal`
/// written to the file, the reload chord) under such an environment and
/// asserts the background agent's transition reaches the host as OSC 9.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_terminal_delivery_after_reload_is_osc9_under_a_leaked_kitty_env() {
    let (fixture, _daemon, mut client) = fixture_with_session("notifyleak");
    let shown = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let background_window = client
        .send("new-window -d")
        .expect("new-window")
        .join("")
        .trim()
        .to_string();
    let background = client
        .send(&format!("list-panes -t {background_window}"))
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("the background pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_with_env(
        &fixture,
        &["-t", &shown],
        "render",
        "[client]\nnotify-delivery = \"off\"\n",
        &[
            ("TERM_PROGRAM", "ghostty"),
            ("KITTY_WINDOW_ID", "1"),
            ("KITTY_PID", "696"),
        ],
    );
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up",
    );
    let mut all = Vec::new();
    screen_stable(
        &host,
        &mut all,
        Duration::from_millis(800),
        Duration::from_secs(10),
    );
    // The settings dialog's write, then its reload (the C-b C-r chord
    // runs the same `reload_config`).
    std::fs::write(
        fixture.client_config_path(),
        "[client]\nnotify-delivery = \"terminal\"\n",
    )
    .expect("write terminal delivery");
    host.to_child
        .write_all(&[0x02, 0x12])
        .expect("reload chord");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"config reloaded",
        Duration::from_secs(10),
        "the reload landed",
    );
    let report = hook_report(
        fixture.socket(),
        &format!(
            r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{background}","agent":"leakagent","state":"blocked","seq":1,"source":"par-mux:test"}}}}"#
        ),
    );
    assert!(report.contains(r#""result":"ok""#), "{report}");
    let got = wait_for_output(&host, b"leakagent", Duration::from_secs(10));
    let mut got_more = got.clone();
    got_more.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(300),
    ));
    let text = String::from_utf8_lossy(&got_more);
    assert!(
        text.contains("\x1b]9;par-mux: ") && text.contains("leakagent: blocked\x07"),
        "the reloaded terminal delivery reaches a ghostty host as OSC 9. stderr: {} bytes: {text:?}",
        stderr.lock().unwrap()
    );
    assert!(
        !text.contains("\x1b]99;"),
        "a leaked KITTY_WINDOW_ID must not switch a ghostty host to OSC 99: {text:?}"
    );
    host.killer.kill().ok();
}

/// Card 01a1425582ef7103a61647925496d387: a settings change one render
/// client makes (file write + its reload) reaches every OTHER attached
/// client through the daemon's `%config-changed` broadcast. Client B
/// re-applies the new `notify-delivery = "terminal"` (its next
/// background agent transition reaches its host as OSC 9) and flashes
/// `config re-applied` once; the writing client A, whose own reload
/// already applied the file, never flashes the follower cue.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_settings_change_reaches_every_other_attached_client() {
    let (fixture, _daemon, mut client) = fixture_with_session("cfgprop");
    let (shown, background) = shown_and_background_panes(&mut client);
    let off = "[client]\nnotify-delivery = \"off\"\n";
    let ghostty = [("TERM_PROGRAM", "ghostty")];
    let (mut a, a_stderr) =
        spawn_attach_with_env(&fixture, &["-t", &shown], "render", off, &ghostty);
    expect_output(&a, b"\x1b[?1002h", Duration::from_secs(10), "client A up");
    let (mut b, b_stderr) =
        spawn_attach_with_env(&fixture, &["-t", &shown], "render", off, &ghostty);
    expect_output(&b, b"\x1b[?1002h", Duration::from_secs(10), "client B up");
    let mut settle_a = Vec::new();
    screen_stable(
        &a,
        &mut settle_a,
        Duration::from_millis(800),
        Duration::from_secs(10),
    );
    let mut settle_b = Vec::new();
    screen_stable(
        &b,
        &mut settle_b,
        Duration::from_millis(800),
        Duration::from_secs(10),
    );

    // A's settings dialog: write the key, then the reload chord's path.
    std::fs::write(
        fixture.client_config_path(),
        "[client]\nnotify-delivery = \"terminal\"\n",
    )
    .expect("write terminal delivery");
    a.to_child.write_all(&[0x02, 0x12]).expect("reload chord");
    a.to_child.flush().ok();
    let mut a_bytes = expect_output(
        &a,
        b"config reloaded",
        Duration::from_secs(10),
        "A's own reload landed",
    );
    expect_output(
        &b,
        b"config re-applied",
        Duration::from_secs(10),
        "B re-applied the config A changed",
    );
    // B's positive proves the broadcast landed; A had the same cue.
    a_bytes.extend(wait_for_output(
        &a,
        b"\x00-never",
        Duration::from_millis(500),
    ));
    let a_text = plain_text(&a_bytes);
    assert!(
        !a_text.contains("-applied"),
        "the writing client does not re-apply its own change: {a_text:?} stderr: {}",
        a_stderr.lock().unwrap()
    );

    // The observable rebind: B's background agent now notifies as OSC 9.
    let report = hook_report(
        fixture.socket(),
        &format!(
            r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{background}","agent":"propagent","state":"blocked","seq":1,"source":"par-mux:test"}}}}"#
        ),
    );
    assert!(report.contains(r#""result":"ok""#), "{report}");
    let mut got = wait_for_output(&b, b"propagent: blocked\x07", Duration::from_secs(10));
    got.extend(wait_for_output(
        &b,
        b"\x00-never",
        Duration::from_millis(300),
    ));
    let text = String::from_utf8_lossy(&got);
    assert!(
        text.contains("\x1b]9;par-mux: ") && text.contains("propagent: blocked\x07"),
        "B's re-applied terminal delivery reaches its host as OSC 9. stderr: {} bytes: {text:?}",
        b_stderr.lock().unwrap()
    );
    a.killer.kill().ok();
    b.killer.kill().ok();
}

/// A client that attaches AFTER a reload (it never saw that
/// `%config-changed`) still follows the next one: any client's reload —
/// here a plain control client's `reload-config` after an out-of-band
/// file edit — reaches it, it re-applies, and it keeps running.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_config_changed_reaches_a_client_attached_after_a_reload() {
    let (fixture, _daemon, mut client) = fixture_with_session("cfglate");
    let (shown, _background) = shown_and_background_panes(&mut client);
    let (mut a, _a_stderr) = spawn_attach_with_config(
        &fixture,
        &["-t", &shown],
        "render",
        "[client]\nnotify-delivery = \"off\"\n",
    );
    expect_output(&a, b"\x1b[?1002h", Duration::from_secs(10), "client A up");
    let terminal = "[client]\nnotify-delivery = \"terminal\"\n";
    std::fs::write(fixture.client_config_path(), terminal).expect("write terminal delivery");
    a.to_child.write_all(&[0x02, 0x12]).expect("reload chord");
    a.to_child.flush().ok();
    expect_output(
        &a,
        b"config reloaded",
        Duration::from_secs(10),
        "the first reload landed",
    );

    // C attaches after that broadcast, over the post-reload file.
    let (mut c, c_stderr) = spawn_attach_with_config(&fixture, &["-t", &shown], "render", terminal);
    expect_output(&c, b"\x1b[?1002h", Duration::from_secs(10), "client C up");
    let mut settle_c = Vec::new();
    screen_stable(
        &c,
        &mut settle_c,
        Duration::from_millis(800),
        Duration::from_secs(10),
    );

    // A further change, reloaded by a control client: C follows it.
    std::fs::write(
        fixture.client_config_path(),
        "[client]\nnotify-delivery = \"inside\"\n",
    )
    .expect("write inside delivery");
    let reply = client.send("reload-config").expect("reload-config");
    assert!(
        !reply.join("\n").contains("%error"),
        "the daemon re-read its config: {reply:?}"
    );
    expect_output(
        &c,
        b"config re-applied",
        Duration::from_secs(10),
        "C re-applied the later change",
    );
    assert!(
        c.child.try_wait().expect("try_wait").is_none(),
        "C survives the cue. stderr: {}",
        c_stderr.lock().unwrap()
    );
    a.killer.kill().ok();
    c.killer.kill().ok();
}

/// Paul's flow (2026-10-14): an agent claimed while its workspace is
/// shown, then the view switches to another workspace, then the agent
/// changes state — the now-background agent must notify.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_agent_in_a_left_workspace_notifies_the_host() {
    let (fixture, _daemon, mut client) = fixture_with_session("notifyws");
    client.send("new-workspace -n lab").expect("new-workspace");
    client
        .send("new-session -s work -t lab")
        .expect("session in lab");
    std::thread::sleep(Duration::from_millis(300));
    let pane_main = session_first_pane(&mut client, "$0");
    let pane_lab = session_first_pane(&mut client, "$2");
    // The pin is for the OSC 9 arm: the spawn pins a non-kitty host, so
    // the suite's expectation holds whatever terminal launched it (a
    // kitty host delivers the same notification as chunked OSC 99).
    let (mut host, stderr) = spawn_attach_with_env(
        &fixture,
        &["-t", &pane_lab],
        "render",
        "[client]\n",
        &[("TERM_PROGRAM", "iTerm.app")],
    );
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up on the lab workspace",
    );
    let mut all = Vec::new();
    screen_stable(
        &host,
        &mut all,
        Duration::from_millis(1000),
        Duration::from_secs(10),
    );
    let report = |state: &str, seq: u32| {
        hook_report(
            fixture.socket(),
            &format!(
                r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{pane_lab}","agent":"labagent","state":"{state}","seq":{seq},"source":"par-mux:test"}}}}"#
            ),
        )
    };
    // Claimed while shown: quiet.
    assert!(report("working", 1).contains(r#""result":"ok""#));
    std::thread::sleep(Duration::from_millis(500));
    // The user switches to `main` (the side panel's workspace row press
    // lands the view: select-workspace + the session's window).
    client
        .send("select-workspace -t main")
        .expect("select main");
    client
        .send(&format!("select-pane -t {pane_main}"))
        .expect("focus main");
    let mut moved = wait_for_output(&host, b"\x00-never", Duration::from_millis(1500));
    all.append(&mut moved);
    // Now in the background: working -> idle is a finish nobody saw, so
    // the daemon stores `done`, which the default notify-states selects.
    // The client's re-seed onto `main` lands asynchronously; until its
    // size report moves the daemon-side view off the lab window, a finish
    // is (correctly) seen. Retry the finish until the daemon calls it done.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut seq = 2;
    loop {
        assert!(report("idle", seq).contains(r#""result":"ok""#));
        let roster = client.send("list-agents").expect("list-agents").join("\n");
        if roster.contains("labagent done") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon never saw the lab pane leave view: {roster}"
        );
        std::thread::sleep(Duration::from_millis(300));
        assert!(report("working", seq + 1).contains(r#""result":"ok""#));
        seq += 2;
    }
    let got = wait_for_output(&host, b"labagent: done", Duration::from_secs(10));
    let text = String::from_utf8_lossy(&got);
    assert!(
        text.contains("\x1b]9;par-mux: ") && text.contains("labagent: done"),
        "the left-behind agent notifies. stderr: {} bytes: {text:?}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// `notify-delivery = "inside"` paints the notification as a toast card
/// in the real client — no host escape — when a background agent goes
/// blocked.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_inside_delivery_paints_a_toast() {
    let (fixture, _daemon, mut client) = fixture_with_session("notifytoast");
    let shown = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let background_window = client
        .send("new-window -d")
        .expect("new-window")
        .join("")
        .trim()
        .to_string();
    let background = client
        .send(&format!("list-panes -t {background_window}"))
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("the background pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_with_config(
        &fixture,
        &["-t", &shown],
        "render",
        "[client]\nnotify-delivery = \"inside\"\n",
    );
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up",
    );
    let mut all = Vec::new();
    screen_stable(
        &host,
        &mut all,
        Duration::from_millis(1000),
        Duration::from_secs(20),
    );
    all.clear();
    let reply = hook_report(
        fixture.socket(),
        &format!(
            r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{background}","agent":"toastagent","state":"blocked","seq":1,"source":"par-mux:test"}}}}"#
        ),
    );
    assert!(
        reply.contains(r#""result":"ok""#),
        "claim accepted: {reply}"
    );
    // Under load the client's frames pace out past one second. A quiet
    // window (screen_stable 800 ms) returned between two frames of the
    // toast's progressive paint (`w`, `wo`, `wor`, … at load ~360), so
    // the barrier is the toast itself plus the bell it rings, within the
    // hang ceiling.
    let painted = |bytes: &[u8]| {
        reconstructed_screen(bytes, 24, 80)
            .1
            .iter()
            .any(|snaps| snaps.iter().any(|r| r.contains("toastagent blocked")))
            && bytes.contains(&0x07)
    };
    let end = Instant::now() + common::CEILING;
    while !painted(&all) && Instant::now() < end {
        if let Ok(bytes) = host.output_rx.recv_timeout(Duration::from_millis(50)) {
            all.extend_from_slice(&bytes);
            for more in host.output_rx.try_iter() {
                all.extend_from_slice(&more);
            }
        }
    }
    let screen = reconstructed_screen(&all, 24, 80).1;
    let ever: String = screen
        .iter()
        .map(|row| row.iter().cloned().collect::<Vec<_>>().join("|"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        ever.contains("toastagent blocked"),
        "the toast card paints over the panes. stderr: {} screen: {ever}",
        stderr.lock().unwrap()
    );
    assert!(
        !String::from_utf8_lossy(&all).contains("\x1b]9;"),
        "inside delivery writes no host notification escape"
    );
    // The toast rings the host bell so the host flags the tab unread;
    // `all` was cleared after the settle, so only the toast's frames are
    // in it (an idle shell writes nothing).
    assert!(
        all.contains(&0x07),
        "inside delivery rings the host bell alongside the toast"
    );
    host.killer.kill().ok();
}

/// The command menu's `settings` row opens herdr's tabbed settings
/// dialog in the real client, and Escape closes it.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_settings_dialog_opens_from_the_panel_menu() {
    let (fixture, _daemon, mut client) = fixture_with_session("settingsdlg");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) =
        spawn_attach_with_config(&fixture, &["-t", &pane_a], "render", "[client]\n");
    let mut got = expect_output(
        &host,
        b"\x1b[?1002h",
        common::CEILING,
        "render client up (mouse reporting enabled)",
    );
    // The panel is up at launch: click the chip once it is laid out.
    expect_screen(&host, &mut got, menu_chip_painted, "the menu chip");
    // The ` menu ` chip on the workspace area's last row (SGR 17;12).
    host.to_child
        .write_all(b"\x1b[<0;17;12M")
        .expect("menu chip press");
    host.to_child.flush().ok();
    // `settings` is panel row 1 of the menu anchored beside its chip
    // (x0 = 19, y0 = 2: the 9-row box with its `Layouts…` row): frame row
    // y0 + 2, host row y0 + 3, SGR y0 + 4.
    let (x0, y0) = (19usize, 2usize);
    expect_screen(
        &host,
        &mut got,
        |s| {
            s.iter().any(|r| r.contains("reload config"))
                && cells(s, y0 + 3, x0 + 2, 8) == "settings"
        },
        "the chip opens the command menu",
    );
    host.to_child
        .write_all(format!("\x1b[<0;{};{}M", x0 + 5, y0 + 4).as_bytes())
        .expect("settings click");
    host.to_child.flush().ok();
    expect_screen(
        &host,
        &mut got,
        |s| {
            let screen = s.join("\n");
            screen.contains("integrations") && screen.contains("side panel")
        },
        &format!(
            "the dialog paints its tabs and the first section. stderr: {}",
            stderr.lock().unwrap()
        ),
    );
    host.to_child.write_all(b"\x1b").expect("escape");
    host.to_child.flush().ok();
    // Escape closes it: the panes repaint over the box, so the screen no
    // longer shows the dialog's tab row (a condition, not a quiet window:
    // under load an 800 ms lull fell before the close repaint).
    expect_screen(
        &host,
        &mut got,
        |s| !s.join("\n").contains("integrations"),
        "Escape closes the dialog",
    );
    host.killer.kill().ok();
}

/// The notify tab's `delivery` row cycles and the write lands in the
/// config the client loaded — driven with the MOUSE on the user's real
/// config shape (the live report: the method seemed unchangeable from
/// the dialog).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_settings_notify_delivery_cycles() {
    let (fixture, _daemon, mut client) = fixture_with_session("setnotify");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_with_config(
        &fixture,
        &["-t", &pane_a],
        "render",
        "[client]\nborder-lines = \"herdr\"\nnotify-states = [\"working\", \"blocked\", \"idle\"]\nnotify-delivery = \"off\"\nscrollbar-gutter = true\nchange-cursor-shape = true\npane-gaps = 0\n",
    );
    let mut got = expect_output(
        &host,
        b"\x1b[?1002h",
        common::CEILING,
        "render client up (mouse reporting enabled)",
    );
    expect_screen(&host, &mut got, menu_chip_painted, "the menu chip");
    host.to_child
        .write_all(b"\x1b[<0;17;12M")
        .expect("menu chip press");
    host.to_child.flush().ok();
    let (x0, y0) = (19usize, 2usize);
    expect_screen(
        &host,
        &mut got,
        |s| {
            s.iter().any(|r| r.contains("reload config"))
                && cells(s, y0 + 3, x0 + 2, 8) == "settings"
        },
        "the chip opens the command menu",
    );
    host.to_child
        .write_all(format!("\x1b[<0;{};{}M", x0 + 5, y0 + 4).as_bytes())
        .expect("settings click");
    host.to_child.flush().ok();
    // The box centers over the 22-row frame: 14 panel rows + ring →
    // y0 = frame 3; the tab row (panel row 0) paints frame 4 = host 5 =
    // SGR 6. Inner cols are box x0+1.. : x0 = (80-66)/2 = 7, so inner
    // col c is host col 8+c = SGR col 9+c. ` notify ` spans inner
    // 27..35 — click SGR col 38.
    expect_screen(
        &host,
        &mut got,
        |s| s.join("\n").contains("side panel") && cells(s, 5, 8 + 27, 8) == " notify ",
        &format!(
            "the dialog is up on its first section. stderr: {}",
            stderr.lock().unwrap()
        ),
    );
    host.to_child
        .write_all(b"\x1b[<0;38;6M")
        .expect("notify tab click");
    host.to_child.flush().ok();
    // The delivery row is the section's only row: body row 0 = panel
    // row 5 = frame 9 = host 10 = SGR 11.
    expect_screen(
        &host,
        &mut got,
        |s| s.join("\n").contains("how agent state") && s[10].contains("delivery"),
        "the notify tab is up",
    );
    let config_reads = |want: &str, what: &str| {
        common::test_wait::until(
            what,
            common::CEILING,
            || {
                let written = std::fs::read_to_string(fixture.client_config_path()).ok()?;
                written.contains(want).then_some(())
            },
            || std::fs::read_to_string(fixture.client_config_path()).unwrap_or_default(),
        )
    };
    // Click it: off cycles to inside.
    host.to_child
        .write_all(b"\x1b[<0;20;11M")
        .expect("delivery row click");
    host.to_child.flush().ok();
    expect_screen(
        &host,
        &mut got,
        |s| s[10].contains("inside"),
        &format!(
            "the dialog shows the cycled value. stderr: {}",
            stderr.lock().unwrap()
        ),
    );
    config_reads(
        "notify-delivery = \"inside\"",
        "the config carries the new value",
    );
    // The arrow gesture: → cycles again (inside → terminal).
    host.to_child.write_all(b"\x1b[C").expect("right arrow");
    host.to_child.flush().ok();
    expect_screen(
        &host,
        &mut got,
        |s| s[10].contains("terminal"),
        "the arrow cycles the shown value",
    );
    config_reads(
        "notify-delivery = \"terminal\"",
        "the arrow cycled the value",
    );
    host.killer.kill().ok();
}

/// The respawn chord ships in the settings dialog's keys tab (project
/// rule: every TUI chord is editable there): the tab's last row is
/// `respawn dead pane` at its live binding.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_settings_keys_tab_lists_the_respawn_chord() {
    let (fixture, _daemon, mut client) = fixture_with_session("setrespawn");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) =
        spawn_attach_with_config(&fixture, &["-t", &pane], "render", "[client]\n");
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up (mouse reporting enabled)",
    );
    std::thread::sleep(Duration::from_millis(600));
    host.to_child
        .write_all(b"\x1b[<0;17;12M")
        .expect("menu chip press");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"reload config",
        Duration::from_secs(10),
        "the chip opens the command menu",
    );
    let (x0, y0) = (19usize, 2usize);
    host.to_child
        .write_all(format!("\x1b[<0;{};{}M", x0 + 5, y0 + 4).as_bytes())
        .expect("settings click");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"integrations",
        Duration::from_secs(10),
        "the dialog is up",
    );
    // Five Tabs reach the keys section; 30 downs clamp at its last row.
    host.to_child.write_all(b"\t\t\t\t\t").expect("tabs");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"management chord",
        Duration::from_secs(10),
        "the keys tab is up",
    );
    host.to_child.write_all(&[b'j'; 30]).expect("downs");
    host.to_child.flush().ok();
    let mut got = wait_for_output(&host, b"respawn dead pane", Duration::from_secs(10));
    screen_stable(
        &host,
        &mut got,
        Duration::from_millis(500),
        Duration::from_secs(10),
    );
    let screen = reconstructed_screen(&got, 24, 80).0.join("\n");
    let row = screen
        .lines()
        .find(|line| line.contains("respawn dead pane"))
        .unwrap_or_default()
        .to_string();
    assert!(
        row.trim_end().trim_end_matches('\u{2502}').trim_end().ends_with(" r"),
        "the keys tab lists the respawn chord at its binding. stderr: {} row: {row:?} screen: {screen}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}
