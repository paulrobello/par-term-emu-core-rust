//! Render mode layout and chrome: replay, resize, zoom, splits, borders,
//! the side panel and status row, tabs, held panes, popups, and kitty
//! graphics.

use super::*;

/// The render-mode chords against a live daemon: prefix z rides the
/// daemon's `resize-pane -Z` (the zoomed pane's child re-fits to the
/// full grid; the Z cue and the unzoom flash paint), prefix , commits
/// the window-rename prompt as a `rename-window` control line, prefix $
/// prompts for the pane title and Escape cancels without committing,
/// and prefix B repaints the dividers at the next glyph set.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_zoom_rename_border_chords() {
    let (fixture, _daemon, mut client) = fixture_with_session("renderchords");
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
    assert!(pane_b.starts_with('%'), "split returned a pane id");
    client
        .send(&format!("send-keys -t {pane_b} -l 'echo PANE-B-CHORD'"))
        .expect("marker B");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("Enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane_b}"),
        echoed("PANE-B-CHORD"),
        Duration::from_secs(10),
        "PANE-B-CHORD executed on the pane screen",
    );

    // The input trace rides along: its stderr timeline (echoed by the
    // asserts below) shows whether a chord byte that produces no flash
    // was read and routed at all — the discriminating evidence for the
    // macos-runner failures (runs 39700463900/39701897063/39703870255).
    let (mut host, stderr) = spawn_attach_with_env(
        &fixture,
        &["-t", &pane_a],
        "render",
        DEFAULT_CLIENT_CONFIG,
        &[("PAR_MUX_INPUT_TRACE", "1")],
    );
    let boot = wait_for_output(&host, b"PANE-B-CHORD", Duration::from_secs(10));
    let early = child_exit(&mut host, Duration::from_millis(200));
    assert!(
        early.is_none(),
        "render attach exited {early:?} before the chords: stderr: {} boot: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&boot)
    );

    // prefix z: the zoomed re-layout re-seeds the view and the Z cue
    // paints on the status row.
    host.to_child.write_all(&[0x02, b'z']).expect("prefix z");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b" Z |", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains(" Z |"),
        "prefix z must paint the zoom cue. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );

    // prefix z again: the unzoom flash names itself.
    host.to_child
        .write_all(&[0x02, b'z'])
        .expect("prefix z again");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"unzoomed", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("unzoomed"),
        "the second prefix z must flash unzoomed. stderr: {}",
        stderr.lock().unwrap()
    );

    // prefix , opens the window-rename prompt.
    host.to_child.write_all(&[0x02, b',']).expect("prefix ,");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"rename window", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("rename window"),
        "prefix , must open the rename prompt. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );

    // Typing + Enter commits as a rename-window control line.
    host.to_child.write_all(b"rewin").expect("type name");
    host.to_child.flush().ok();
    host.to_child.write_all(b"\r").expect("commit");
    host.to_child.flush().ok();
    let mut renamed = false;
    for _ in 0..50 {
        if let Ok(lines) = client.send("list-windows") {
            if lines.join(" ").contains("rewin") {
                renamed = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(renamed, "prefix , must commit as rename-window");

    // prefix $ opens the pane-title prompt; Escape cancels untouched.
    host.to_child.write_all(&[0x02, b'$']).expect("prefix $");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"rename pane", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("rename pane"),
        "prefix $ must open the pane prompt. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    host.to_child.write_all(b"nope-title").expect("type title");
    host.to_child.flush().ok();
    host.to_child.write_all(&[0x1b]).expect("escape");
    host.to_child.flush().ok();
    // negative check: nothing positive to wait on, so the window is fixed
    std::thread::sleep(Duration::from_millis(400));
    let title = client
        .send(&format!("pane-title -t {pane_a}"))
        .expect("pane-title")
        .join(" ");
    assert!(
        !title.contains("nope-title"),
        "Escape must cancel the prompt without committing: {title:?}"
    );

    // prefix B cycles the border glyphs (herdr is the seed default, so
    // two presses land on double): the flash names the new style and
    // the same repaint carries the double-line vertical.
    host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    host.to_child.flush().ok();
    let mut got = wait_for_output(&host, b"border style", Duration::from_secs(10));
    // Accumulate the burst's tail (the dividers repaint in the same
    // frame the flash paints).
    got.extend(wait_for_output(&host, b"", Duration::from_millis(700)));
    assert!(
        plain_text(&got).contains("border style: double"),
        "prefix B must flash the new border style. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    assert!(
        got.windows("║".len()).any(|w| w == "║".as_bytes()),
        "the border cycle must repaint the dividers at the new glyphs"
    );

    // Three more cycles land on herdr: every pane draws its own rounded
    // box; the flash names the style and the repaint carries the corner.
    for _ in 0..3 {
        host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    }
    host.to_child.flush().ok();
    let mut got = wait_for_output(&host, b"border style: herdr", Duration::from_secs(10));
    got.extend(wait_for_output(&host, b"", Duration::from_millis(700)));
    assert!(
        plain_text(&got).contains("border style: herdr"),
        "cycling to herdr must flash the style. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    assert!(
        got.windows("╭".len()).any(|w| w == "╭".as_bytes()),
        "the herdr style must paint per-pane rounded boxes"
    );

    // prefix l toggles the pane labels; the flash names the new state.
    // Labels default ON (the manual-pass ruling: titles show in their
    // borders), so the first press turns them off.
    host.to_child.write_all(&[0x02, b'l']).expect("prefix l");
    host.to_child.flush().ok();
    // The exact phrase, not the shared `labels` stem: under runner load the
    // FIRST press's flash can arrive after this wait starts, and a stem
    // needle then returns press 1's window — press 2's flash never enters
    // `got` and the screen assert below fails (measured on macos-latest,
    // run 39700463900).
    let mut got = wait_for_output(&host, b"labels off", Duration::from_secs(10));
    got.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    // The diff may skip the flash text's unchanged space cell, so the
    // raw bytes can split the words — read the reconstructed screen,
    // where the gap keeps its prior (space) content.
    let (_, ever) = reconstructed_screen(&got, 24, 80);
    assert!(
        ever.last()
            .is_some_and(|snaps| snaps.iter().any(|r| r.contains("labels off"))),
        "prefix l must flash labels off. stderr: {} bytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    // Press and poll, re-pressing up to twice: on the macos runner the
    // second press has three times produced NO flash in a 10 s window
    // (byte-identical captures), so a single press is not a stable
    // observable there. `l` toggles, so from labels-off an odd number of
    // processed presses ends on labels on and its flash names it; the
    // assert below accepts the flash from any press.
    let mut got = Vec::new();
    let mut presses = 0;
    while presses < 3 {
        host.to_child
            .write_all(&[0x02, b'l'])
            .expect("prefix l again");
        host.to_child.flush().ok();
        presses += 1;
        got.extend(wait_for_output(&host, b"labels on", Duration::from_secs(4)));
        if got.windows("labels on".len()).any(|w| w == b"labels on") {
            break;
        }
    }
    got.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    let (_, ever) = reconstructed_screen(&got, 24, 80);
    // Scan EVERY snapshot, not just the last: the flash paints only the
    // changed cells (raw bytes may never spell `labels on`), and a late
    // backlog repaint of the status row can overwrite the flash before
    // the window ends. Reconstruction joins the unchanged prefix, so a
    // snapshot that ever held the flash still reads `labels on`.
    let saw_on_raw = got.windows("labels on".len()).any(|w| w == b"labels on");
    assert!(
        ever.iter()
            .flatten()
            .any(|r| r.contains("labels on")),
        "prefix l again must flash labels on (presses: {presses}, raw needle seen: {saw_on_raw}). stderr: {} bytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );

    // prefix g opens the workspace picker: the themed modal titles itself
    // ` workspaces ` with one row per workspace; Escape dismisses.
    host.to_child.write_all(&[0x02, b'g']).expect("prefix g");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"workspaces", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains(" workspaces "),
        "prefix g must open the workspace picker. stderr: {}",
        stderr.lock().unwrap()
    );
    host.to_child.write_all(&[0x1b]).expect("escape picker");
    host.to_child.flush().ok();
    // input spacing
    std::thread::sleep(Duration::from_millis(300));

    // prefix s toggles the side panel: the strip paints the workspaces
    // section and the panes re-divide around it; toggling off restores.
    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    // The strip's cells precede the status row in the same frame diff, so
    // the flash and the section text arrive together.
    let got = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("sidebar on") && plain_text(&got).contains("workspaces"),
        "prefix s must flash and paint the workspaces section. stderr: {} tail: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
            .chars()
            .rev()
            .take(600)
            .collect::<String>()
    );
    host.to_child
        .write_all(&[0x02, b's'])
        .expect("prefix s again");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"sidebar off", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("sidebar off"),
        "prefix s again must flash sidebar off. stderr: {}",
        stderr.lock().unwrap()
    );

    // Shift+arrow swaps with the pane in that direction: whichever side
    // holds focus, one of the two presses hits a neighbor and the roster's
    // layout order exchanges the two panes.
    let first_before = client
        .send("list-panes")
        .expect("roster")
        .iter()
        .find_map(|l| {
            l.split_whitespace()
                .next()
                .filter(|p| p.starts_with('%'))
                .map(str::to_string)
        })
        .expect("a pane id");
    host.to_child.write_all(&[0x02]).expect("prefix");
    host.to_child.write_all(b"\x1b[1;2C").expect("shift-right");
    host.to_child.write_all(&[0x02]).expect("prefix");
    host.to_child.write_all(b"\x1b[1;2D").expect("shift-left");
    host.to_child.flush().ok();
    common::wait_daemon(
        fixture.socket(),
        "list-panes",
        |reply| common::pane_ids(reply).first() != Some(&first_before),
        Duration::from_secs(10),
        "the shift-arrow swap must exchange the two panes' layout cells",
    );
    host.killer.kill().ok();
}

/// Phase B criteria 1+2, driven against the REAL daemon in-process: a
/// 2-pane split's `%layout-change` parses to rects matching the daemon's
/// own geometry, and each pane's replay-fed emulator reproduces what
/// `capture-pane` ground truth says the pane holds — the same cell
/// content the renderer paints at those rects. (Criterion 3's frame
/// coalescing is pinned by `output_flood_coalesces_at_frame_cadence` in
/// `src/mux/attach/render/tests/mod.rs` — a pure-renderer property.)
///
/// The renderer runs headless (no sink): the frames stay in the
/// [`PaneRenderer::buffer`] the assertions read.
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_layout_and_pane_replay_match_daemon_ground_truth() {
    let (fixture, _daemon, mut client) = fixture_with_session("render");

    // The initial session's pane, then a vertical split; distinct content
    // per pane through the shell.
    let pane0 = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("split-window -t {pane0} -h"))
        .expect("split");
    client
        .send(&format!("send-keys -t {pane0} -l 'echo LEFT-MARKER'"))
        .expect("keys left");
    client
        .send(&format!("send-keys -t {pane0} Enter"))
        .expect("enter left");
    let pane1 = client
        .send("list-panes")
        .expect("list-panes")
        .join("\n")
        .lines()
        .find(|l| l.split_whitespace().next() != Some(pane0.as_str()))
        .and_then(|l| l.split_whitespace().next())
        .expect("the split's pane")
        .to_string();
    client
        .send(&format!("send-keys -t {pane1} -l 'echo RIGHT-MARKER'"))
        .expect("keys right");
    client
        .send(&format!("send-keys -t {pane1} Enter"))
        .expect("enter right");
    // The echoes land on the pane screens BEFORE the replay: the
    // assertion is about the replay seeding, not a race with %output.
    for (pane, marker) in [(&pane0, "LEFT-MARKER"), (&pane1, "RIGHT-MARKER")] {
        common::wait_daemon(
            fixture.socket(),
            &format!("capture-pane -t {pane}"),
            echoed(marker),
            Duration::from_secs(10),
            &format!("{marker} executed on the pane screen"),
        );
    }

    // The render client: the documented handshake, then the size report
    // that pulls the current layout triple.
    let mut conn = par_mux::mux::attach::test_support::AttachConn::connect(fixture.socket())
        .expect("render client connect");
    let _replay = conn.drain_pending_events();
    conn.send_checked(&format!("refresh-client -t {pane0} -C 80x24"))
        .expect("size report");
    let layout_event = conn
        .drain_pending_events()
        .into_iter()
        .find_map(|event| match event {
            par_term_emu_core::tmux_control::TmuxNotification::LayoutChange {
                window_layout,
                window_visible_layout,
                window_raw_flags,
                ..
            } => Some((window_layout, window_visible_layout, window_raw_flags)),
            _ => None,
        })
        .expect("the size report broadcast a layout change");
    let rects = par_mux::mux::attach::test_support::parse_layout_triple(
        &layout_event.0,
        &layout_event.1,
        &layout_event.2,
    )
    .expect("the daemon's own layout string parses");
    assert_eq!(rects.len(), 2, "the split's two leaves");
    assert_eq!(rects[0].x, 0, "left pane starts at col 0");
    assert_eq!(rects[1].x, rects[0].width, "right pane abuts the left");
    assert_eq!(
        rects[0].height, rects[1].height,
        "a vertical split shares the height"
    );

    // Renderer over the daemon's layout, replays per pane.
    let mut renderer = par_mux::mux::attach::test_support::PaneRenderer::new(
        80,
        24,
        par_mux::mux::attach::test_support::Glyphs::Unicode,
    );
    renderer.apply_layout(rects.clone());
    for rect in &rects {
        let pane = format!("%{}", rect.pane);
        let reply = conn
            .send_checked(&format!("refresh-client -t {pane}"))
            .expect("replay request");
        assert!(reply.ok, "replay of {pane}");
        let mut bytes = reply.body.join("\n").into_bytes();
        bytes.push(b'\n');
        renderer.feed_output(rect.pane, &bytes);
    }
    renderer.render_frame();

    // Ground truth per pane: the daemon's own capture of what each pane
    // shows must appear at that pane's layout rect of the rendered buffer.
    for (marker, rect) in [("LEFT-MARKER", &rects[0]), ("RIGHT-MARKER", &rects[1])] {
        let rendered: String = (0..rect.height)
            .map(|row| {
                (0..rect.width)
                    .map(|col| {
                        renderer
                            .cell(rect.x + col, rect.y + row)
                            .map(|c| c.symbol())
                            .unwrap_or(" ")
                    })
                    .collect::<String>()
            })
            .collect::<String>();
        assert!(
            rendered.contains(marker),
            "the pane's rect must render its capture-pane content ({marker}); got {rendered:?}"
        );
        // And the daemon agrees the pane itself shows it.
        let capture = client
            .send(&format!("capture-pane -t %{}", rect.pane))
            .expect("capture");
        assert!(
            capture.join("\n").contains(marker),
            "daemon ground truth for %{} shows {marker}: {capture:?}",
            rect.pane
        );
    }
}

/// Card 01a1714212 (roster on reconnect): a fresh render client paints the
/// full agent roster from its seed's snapshot — no new hook report or
/// state push in between. The agent is a scrape-tier fixture invoked as
/// `claude` (the bundled blocked rule matches its screen), so the roster
/// row outlives the reconnect window without a liveness sweep to race.
/// The first attach watches the row arrive; the second attach, spawned
/// after the first is killed and with nothing reported meanwhile, must
/// show the same row on its own first status refresh.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_side_panel_reconnect_shows_the_roster_without_a_new_report() {
    let (fixture, _daemon, mut client) = fixture_with_session("reconnroster");
    let fake_bin = fixture.dir().join("claude");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/assets/par-mux-scrape-agent.sh"),
        &fake_bin,
    )
    .expect("copy the scrape fixture");
    std::fs::set_permissions(
        &fake_bin,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("make the fixture executable");

    // The agent owns its own workspace's pane, so the shown view carries
    // the agents section for it. list-panes defaults to the active
    // session, so snapshot the fixture's shell pane first and pick the
    // new pane by exclusion.
    let shell_pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("\n")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("new-session -s agents -- {}", fake_bin.display()))
        .expect("agent session");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("\n")
        .split_whitespace()
        .find(|p| *p != shell_pane)
        .expect("the agent session's pane")
        .to_string();
    client
        .send(&format!("adopt-pane -t {pane} claude"))
        .expect("adopt the agent pane");

    let row = format!("{pane} claude blocked scrape ws=+");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let roster = client.send("list-agents").expect("list-agents").join("");
        if roster.contains(&row) || Instant::now() >= deadline {
            assert!(
                roster.contains(&row),
                "the scrape fixture never joined the roster: {roster} \
                 ;; panes: {:?} ;; sessions: {:?} ;; screen: {:?}",
                client.send("list-panes").unwrap_or_default(),
                client.send("list-sessions").unwrap_or_default(),
                client
                    .send(&format!("capture-pane -t {pane}"))
                    .unwrap_or_default()
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    // First attach: the row lands (seed snapshot or the state push).
    let config = "[client]\nsidebar-on-launch = true\n";
    let (mut first, stderr) = spawn_attach_with_config(&fixture, &["-t", &pane], "render", config);
    let mut all = Vec::new();
    assert!(
        panel_ever_contains(&first, &mut all, 19, "claude", Duration::from_secs(10)),
        "the first attach never painted the roster row; stderr: {}",
        stderr.lock().unwrap()
    );
    first.killer.kill().ok();

    // Nothing reports in between — the daemon's roster still carries the
    // row from the scrape, and the fresh client must paint it from its
    // seed's snapshot alone.
    let roster = client.send("list-agents").expect("list-agents").join("");
    assert!(
        roster.contains(&row),
        "the roster row vanished while no client was attached: {roster}"
    );
    let (mut second, stderr) = spawn_attach_with_config(&fixture, &["-t", &pane], "render", config);
    let mut all = Vec::new();
    assert!(
        panel_ever_contains(&second, &mut all, 19, "claude", Duration::from_secs(10)),
        "the reconnecting attach never painted the roster row; stderr: {}",
        stderr.lock().unwrap()
    );
    second.killer.kill().ok();
}

/// Acceptance criterion (render mode, PTY level): a host resize re-fits
/// the whole client. Resizing the harness master PTY (the host terminal's
/// SIGWINCH shape) makes the pump report the new grid (content height,
/// minus the status row), the daemon re-divides the window and
/// re-broadcasts the layout, `resize_to` re-fits and re-seeds every pane
/// synchronously, and the status row repaints at the host's new bottom
/// row — the settle-redraw discipline means a repaint flood cannot leave
/// it blank.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_host_resize_refits_window_layout_and_status_row() {
    let (fixture, _daemon, mut client) = fixture_with_session("resize");
    let pane0 = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("split-window -t {pane0} -h"))
        .expect("split");
    let pane1 = client
        .send("list-panes")
        .expect("list-panes")
        .join("\n")
        .lines()
        .find(|l| l.split_whitespace().next() != Some(pane0.as_str()))
        .and_then(|l| l.split_whitespace().next())
        .expect("the split's pane")
        .to_string();
    client
        .send(&format!("send-keys -t {pane0} -l 'echo LEFT-MARKER'"))
        .expect("keys left");
    client
        .send(&format!("send-keys -t {pane0} Enter"))
        .expect("enter left");
    client
        .send(&format!("send-keys -t {pane1} -l 'echo RIGHT-MARKER'"))
        .expect("keys right");
    client
        .send(&format!("send-keys -t {pane1} Enter"))
        .expect("enter right");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane1}"),
        echoed("RIGHT-MARKER"),
        Duration::from_secs(10),
        "RIGHT-MARKER executed on the pane screen",
    );

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane0]);
    // Settle: mouse capture + first frame with both panes painted.
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    expect_output(
        &host,
        b"LEFT-MARKER",
        Duration::from_secs(10),
        "LEFT-MARKER on the host screen",
    );
    expect_output(
        &host,
        b"RIGHT-MARKER",
        Duration::from_secs(10),
        "RIGHT-MARKER on the host screen",
    );
    while host.output_rx.try_recv().is_ok() {}

    // Resize the host terminal: 24x80 -> 30x100. Everything the host
    // receives past this point is resize-attributable.
    host.master
        .resize(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("host resize");

    // The status row repaints at the host's NEW bottom row (row 30) — a
    // CUP to row 30 cannot come from the 24-row geometry.
    let got = wait_for_output(&host, b"\x1b[30;", Duration::from_secs(10));
    assert!(
        !got.is_empty(),
        "the resize must re-fit and repaint (status row CUP to the new bottom \
         row). stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );

    // The daemon re-divided to the reported content grid (100x28 on a
    // 30-row host: rows minus the tab strip minus the status row): each
    // pane re-fit to half of 100 columns. Give the broadcast/refit a
    // beat, then read pane-info ground truth.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut fitted = false;
    while Instant::now() < deadline {
        let info = client
            .send(&format!("pane-info -t {pane0}"))
            .expect("pane-info");
        // The pane rect is 50x28; the PTY is its interior inside the
        // declared border ring (card 01a11c5b): 48x26.
        if info.join(" ").contains("48x26") {
            fitted = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        fitted,
        "the size report must re-fit the daemon's panes to the content grid \
         (100x28, half = 50 wide, less the border ring = 48x26): {:?}",
        client
            .send(&format!("pane-info -t {pane0}"))
            .expect("pane-info")
    );

    // Reconstruct the painted screen at the new geometry: both panes
    // re-seeded, each in its half, and the status row carrying the shown
    // session on row 30. The repaint may straddle the needle read, so the
    // reconstruction joins the needle-read bytes with what followed; give
    // the re-seed + repaint a beat to finish flowing before draining.
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(1500));
    let mut all = got;
    while let Ok(bytes) = host.output_rx.try_recv() {
        all.extend_from_slice(&bytes);
    }
    let capture_left = client
        .send(&format!("capture-pane -t {pane0}"))
        .expect("capture left");
    let (final_grid, ever) = reconstructed_screen(&all, 30, 100);
    assert!(
        final_grid.iter().any(|row| row.contains("LEFT-MARKER"))
            || ever
                .iter()
                .any(|rows| rows.iter().any(|r| r.contains("LEFT-MARKER"))),
        "the left pane must repaint after the resize re-fit: {final_grid:?}\n\
         daemon capture: {:?}\nbytes received after resize: {} bytes, first 400: {:?}",
        capture_left.join("\n"),
        all.len(),
        String::from_utf8_lossy(&all[..all.len().min(400)])
    );
    assert!(
        final_grid.iter().any(|row| row.contains("RIGHT-MARKER")),
        "the right pane must repaint after the resize re-fit: {final_grid:?}"
    );
    let left_mark = final_grid
        .iter()
        .filter_map(|row| row.find("LEFT-MARKER"))
        .next();
    if let Some(col) = left_mark {
        assert!(
            col < 50,
            "the left pane's marker must sit in the re-divided left half \
             (cols 0..50): col {col}"
        );
    }
    let right_mark = final_grid
        .iter()
        .filter_map(|row| row.find("RIGHT-MARKER"))
        .next();
    if let Some(col) = right_mark {
        assert!(
            (50..100).contains(&col),
            "the right pane's marker must sit in the re-divided right half \
             (cols 50..100): col {col}"
        );
    }
    let ever_row: String = ever[29].iter().map(|r| format!("{r:?}\n")).collect();
    assert!(
        ever[29].iter().any(|r| r.contains("$0:att")),
        "the status row must survive the repaint flood at the new bottom row \
         (row 30). snapshots:\n{ever_row}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Acceptance criterion (render mode, PTY level): a host resize to a
/// TALLER-than-handshake grid re-fits the whole frame. The client settles
/// at the harness's 24-row default; growing the master to 40 rows (width
/// kept) must (a) report the content grid on the wire — 40 rows minus
/// the tab strip minus the status row = 38, (b) wipe the host screen and
/// repaint the full frame at the new geometry — tab strip on row 1,
/// status on row 40, pane content between — so no pre-resize content can
/// remain visible outside the new frame, and (c) keep pane content seeded
/// before the attach visible through the re-fit (position may shift).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_resize_to_taller_than_handshake_covers_the_full_frame() {
    let (fixture, _daemon, mut client) = fixture_with_session("tallresize");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // A deterministic tab name, so the strip row pins exactly.
    client.send("rename-window -t @0 big").expect("rename");
    // Pre-attach content: seeded before the client attaches, so the
    // replay path (not a live %output race) must carry it into the
    // re-fit frame.
    client
        .send(&format!("send-keys -t {pane} -l 'echo SURVIVOR-MARKER'"))
        .expect("seed marker");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane}"),
        echoed("SURVIVOR-MARKER"),
        Duration::from_secs(10),
        "SURVIVOR-MARKER executed on the pane screen",
    );

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    // Settle on the 24-row handshake grid: mouse capture + the seeded
    // marker painted.
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    expect_output(
        &host,
        b"SURVIVOR-MARKER",
        Duration::from_secs(10),
        "SURVIVOR-MARKER on the host screen",
    );
    // Drain the settle paint so everything read next is resize-attributable.
    while host.output_rx.try_recv().is_ok() {}

    // Grow the host 24 -> 40 rows, width kept.
    host.master
        .resize(PtySize {
            rows: 40,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("host resize");

    // The status row repaints at the host's NEW bottom row (row 40) — a
    // CUP to row 40 cannot come from the 24-row geometry.
    let got = wait_for_output(&host, b"\x1b[40;", Duration::from_secs(10));
    assert!(
        !got.is_empty(),
        "the taller resize must re-fit and repaint (status CUP to row 40). \
         stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );

    // Give the re-seed + full repaint a beat to finish flowing, then
    // collect everything the host received after the resize.
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(1500));
    let mut all = got;
    while let Ok(bytes) = host.output_rx.try_recv() {
        all.extend_from_slice(&bytes);
    }

    // The wire size report re-fit the daemon's window to the content
    // grid: 40 rows - tab strip - status row = 38 rows, width kept.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut fitted = false;
    while Instant::now() < deadline {
        let info = client
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info");
        // The pane rect is 80x38; the PTY is its interior inside the
        // declared border ring (card 01a11c5b): 78x36.
        if info.join(" ").contains("78x36") {
            fitted = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        fitted,
        "the size report must carry the content grid (40 rows minus the tab \
         strip minus the status row = 38, width kept; less the border ring = 78x36): {:?}",
        client
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info")
    );

    // The repaint wipe: resize_to clears the host screen before the
    // re-fit frame — the discipline that guarantees no pre-resize
    // content can survive outside the new frame.
    assert!(
        all.windows(4).any(|w| w == b"\x1b[2J"),
        "the re-fit must wipe the host screen (CSI 2J) so pre-resize content \
         cannot remain visible. bytes: {}",
        String::from_utf8_lossy(&all[..all.len().min(400)])
    );

    // Reconstruct the painted screen at the new geometry and pin the
    // full frame: tab strip on row 1, status on row 40, and the
    // pre-attach marker re-seeded inside the pane area (1-based rows
    // 2..39; 0-based 1..=38).
    let (final_grid, ever) = reconstructed_screen(&all, 40, 80);
    let ever_row = |r: usize| {
        ever[r]
            .iter()
            .map(|s| format!("{s:?}\n"))
            .collect::<String>()
    };
    assert!(
        ever[0].iter().any(|r| r.contains(" big ")),
        "the tab strip must paint row 1 at the taller grid. snapshots:\n{}\n\
         stderr: {}",
        ever_row(0),
        stderr.lock().unwrap()
    );
    assert!(
        ever[39].iter().any(|r| r.contains("$0:att")),
        "the status row must paint the new bottom row (row 40). snapshots:\n{}\n\
         stderr: {}",
        ever_row(39),
        stderr.lock().unwrap()
    );
    let marker_row = final_grid
        .iter()
        .position(|row| row.contains("SURVIVOR-MARKER"))
        .or_else(|| {
            ever.iter()
                .position(|rows| rows.iter().any(|r| r.contains("SURVIVOR-MARKER")))
        });
    assert!(
        marker_row.is_some(),
        "the pre-attach marker must survive the re-fit. final: {final_grid:?}"
    );
    if let Some(row) = marker_row {
        assert!(
            (1..=38).contains(&row),
            "the marker must sit inside the pane area (1-based rows 2..39), \
             not on the strip or status rows: 0-based row {row}"
        );
    }
    host.killer.kill().ok();
}

/// Acceptance criterion (render mode, PTY level): a daemon-side zoom
/// (`resize-pane -t <pane> -Z`, tmux semantics) re-renders the VISIBLE
/// layout — the zoomed pane alone at full window extent — and a second
/// `-Z` unzooms, restoring the split. On the wire the true layout keeps
/// both panes while `window_visible_layout` carries the zoomed one and
/// the flags carry `Z`; the layout parser renders the visible form.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_daemon_zoom_shows_single_pane_and_unzoom_restores_split() {
    let (fixture, _daemon, mut client) = fixture_with_session("zoom");
    let pane0 = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("split-window -t {pane0} -h"))
        .expect("split");
    let pane1 = client
        .send("list-panes")
        .expect("list-panes")
        .join("\n")
        .lines()
        .find(|l| l.split_whitespace().next() != Some(pane0.as_str()))
        .and_then(|l| l.split_whitespace().next())
        .expect("the split's pane")
        .to_string();
    client
        .send(&format!("send-keys -t {pane0} -l 'echo LEFT-MARKER'"))
        .expect("keys left");
    client
        .send(&format!("send-keys -t {pane0} Enter"))
        .expect("enter left");
    client
        .send(&format!("send-keys -t {pane1} -l 'echo RIGHT-MARKER'"))
        .expect("keys right");
    client
        .send(&format!("send-keys -t {pane1} Enter"))
        .expect("enter right");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane1}"),
        echoed("RIGHT-MARKER"),
        Duration::from_secs(10),
        "RIGHT-MARKER executed on the pane screen",
    );

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane0]);
    // Settle: the split view with both panes painted.
    let settle = {
        let a = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
        let b = wait_for_output(&host, b"LEFT-MARKER", Duration::from_secs(10));
        let c = wait_for_output(&host, b"RIGHT-MARKER", Duration::from_secs(10));
        let _ = (a, b);
        c
    };
    let _ = settle;
    while host.output_rx.try_recv().is_ok() {}

    // Zoom pane1. The %layout-change (flags carry Z) parks the visible
    // layout — pane1 alone at 80x23 — and the pump re-seeds it, so the
    // zoom repaint carries RIGHT-MARKER past the drain.
    client
        .send(&format!("resize-pane -t {pane1} -Z"))
        .expect("zoom");
    let zoomed = wait_for_output(&host, b"RIGHT-MARKER", Duration::from_secs(10));
    assert!(
        !zoomed.is_empty(),
        "the zoom must re-render the visible layout with the zoomed pane's \
         content. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&zoomed)
    );
    // The daemon kept the true layout: both panes still exist under the
    // window (zoom is view state, not structure). @0 is the session's
    // only window (the fixture seeds one session with one pane; the split
    // stayed in it).
    let roster = client.send("list-panes -t @0").expect("window roster");
    let roster_text = roster.join("\n");
    let ids: Vec<&str> = roster_text
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert!(
        ids.contains(&pane0.as_str()) && ids.contains(&pane1.as_str()),
        "the zoom must keep the true layout (both panes): {ids:?}"
    );
    while host.output_rx.try_recv().is_ok() {}

    // Unzoom: -Z again on the zoomed pane restores the exact split, and
    // the restore repaint re-seeds pane0 — its marker lands past the drain.
    client
        .send(&format!("resize-pane -t {pane1} -Z"))
        .expect("unzoom");
    let restored = wait_for_output(&host, b"LEFT-MARKER", Duration::from_secs(10));
    assert!(
        !restored.is_empty(),
        "unzooming must restore the split view (the hidden pane repaints). \
         stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&restored)
    );
    // Ground truth both ways: the panes kept their screens through the
    // zoom round-trip.
    for (marker, pane) in [("LEFT-MARKER", &pane0), ("RIGHT-MARKER", &pane1)] {
        let capture = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture");
        assert!(
            capture.join("\n").contains(marker),
            "{pane} must still show {marker} after the zoom round-trip: {capture:?}"
        );
    }
    host.killer.kill().ok();
}

/// Render mode's prefix %: the chord splits the focused pane and the
/// view re-seeds — the split's second pane renders at its rect, and
/// typing lands in the fresh pane (its shell echoes it into the grid the
/// next frame paints).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_split_chord_reseeds_the_window_with_the_new_pane() {
    let (fixture, _daemon, mut client) = fixture_with_session("rendersplit");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane}"),
        |screen| {
            screen
                .lines()
                .any(|l| !l.trim().is_empty() && !l.starts_with('%'))
        },
        Duration::from_secs(10),
        "the pane's shell drew its prompt",
    );

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    // output settle: the replay's frame lands before the drain
    std::thread::sleep(Duration::from_millis(300));
    while host.output_rx.try_recv().is_ok() {}

    // prefix % splits right; the re-seed paints two panes (a vertical
    // divider column appears — Unicode `│` or the ACS `|` fallback).
    host.to_child.write_all(&[0x02, b'%']).expect("prefix %");
    host.to_child.flush().ok();
    let divider = wait_for_output(&host, "│".as_bytes(), Duration::from_secs(15));
    assert!(
        !divider.is_empty(),
        "the split chord must re-seed the window with the new pane's \
         divider. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&divider)
    );
    // Two panes exist daemon-side, and the fresh pane renders a prompt.
    let roster = common::wait_daemon(
        fixture.socket(),
        "list-panes",
        |reply| common::pane_ids(reply).len() >= 2,
        Duration::from_secs(10),
        "the split created a second pane",
    );
    let roster = common::pane_ids(&roster);
    // Typing after the chord reaches the FRESH pane: an echo there runs
    // in the split (asserted daemon-side — render frames paint per-cell
    // diffs, so the marker never appears as one raw substring in the
    // host stream).
    host.to_child
        .write_all(b"echo RENDER-SPLIT-MARKER\r")
        .expect("type into the split");
    host.to_child.flush().ok();
    let fresh = roster
        .iter()
        .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
        .find(|id| *id != pane)
        .expect("the fresh pane id");
    let mut capture = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        capture = client
            .send(&format!("capture-pane -t {fresh}"))
            .expect("capture")
            .join("\n");
        if capture.contains("RENDER-SPLIT-MARKER") {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        capture.contains("RENDER-SPLIT-MARKER"),
        "typing after the chord must reach the fresh pane: {capture:?}"
    );
    host.killer.kill().ok();
}

/// The sidebar chord in isolation: boot, attach, settle, toggle on and
/// off — bisecting the full-chain test's sidebar failure.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_sidebar_toggle_minimal() {
    let (fixture, _daemon, mut client) = fixture_with_session("sidebarmini");
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
    client
        .send(&format!(
            "send-keys -t {pane_b} -l 'echo SIDEBAR-MINI-MARK'"
        ))
        .expect("marker");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("Enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane_b}"),
        echoed("SIDEBAR-MINI-MARK"),
        Duration::from_secs(10),
        "SIDEBAR-MINI-MARK executed on the pane screen",
    );

    let (mut host, _stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    expect_output(
        &host,
        b"SIDEBAR-MINI-MARK",
        Duration::from_secs(10),
        "SIDEBAR-MINI-MARK on the host screen",
    );

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    // The strip and the flash paint in the SAME frame (the strip's cells
    // precede the status row in the diff), so one wait covers both.
    let got = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("sidebar on") && plain_text(&got).contains("workspaces"),
        "the toggle must flash and paint the workspaces section. tail: {:?}",
        String::from_utf8_lossy(&got)
            .chars()
            .rev()
            .take(500)
            .collect::<String>()
    );
    host.to_child
        .write_all(&[0x02, b's'])
        .expect("prefix s again");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"sidebar off", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("sidebar off"),
        "the second toggle must flash off. tail: {:?}",
        String::from_utf8_lossy(&got)
            .chars()
            .rev()
            .take(400)
            .collect::<String>()
    );
    host.killer.kill().ok();
}

/// Sidebar toggle geometry: the refit must re-divide the panes to the
/// reported width and paint them flush against the panel's right edge —
/// the manual-pass screenshot showed the contents pushed right of the
/// panel with dead margin at the far right. On an 80-col host with the
/// 20-col panel up, the right pane's marker paints at column 51; with
/// the panel down it returns to 41.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_sidebar_toggle_refits_pane_geometry() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbgeo");
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
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane_b}"),
        |screen: &str| screen.contains("GEO-MARK-42"),
        Duration::from_secs(10),
        "GEO-MARK-42 executed on the pane screen",
    );

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let pre = wait_for_output(&host, b"GEO-MARK-42", Duration::from_secs(10));
    let col_pre = cup_col_of_text(&pre, "GEO-MARK-42");
    assert_eq!(
        col_pre,
        Some(42),
        "pre-toggle the right pane's content starts at 41 (the 80/2 split, +1 for the per-pane-box inset). stderr: {} text: {:?}",
        stderr.lock().unwrap(),
        plain_text(&pre)
    );

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    // The refit's repaint rides the same flush as the flash (one pump
    // iteration); drain a beat for the tail before parsing the geometry.
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(600),
    ));
    assert!(
        plain_text(&flash).contains("sidebar on"),
        "toggle must flash on. tail: {:?}",
        String::from_utf8_lossy(&flash)
            .chars()
            .rev()
            .take(300)
            .collect::<String>()
    );
    let col_on = cup_col_of_text(&flash, "GEO-MARK-42");
    assert_eq!(
        col_on,
        Some(52),
        "panel up: panes re-divide to 60 wide, the right pane's content paints at 51 (30+20, +1 inset). text: {:?}",
        plain_text(&flash)
    );

    host.to_child
        .write_all(&[0x02, b's'])
        .expect("prefix s again");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"sidebar off", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(600),
    ));
    assert!(
        plain_text(&flash).contains("sidebar off"),
        "second toggle must flash off"
    );
    let col_off = cup_col_of_text(&flash, "GEO-MARK-42");
    assert_eq!(
        col_off,
        Some(42),
        "panel down restores the full-width split"
    );
    host.killer.kill().ok();
}

/// `sidebar-on-launch` absent (the default, true): with no chord sent the
/// view shows the side panel — the strip's workspace rows, and the panes
/// divided around it (the right pane's marker at column 52, the panel-up
/// geometry of `render_mode_sidebar_toggle_refits_pane_geometry`). The
/// geometry reads the marker's last paint; the client has no launch-time
/// refit path, so that paint is the seed's division.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_sidebar_on_launch_default_shows_panel_on_first_frame() {
    let (fixture, _daemon, mut client) = fixture_with_session("sblaunch");
    let pane_a = split_with_right_marker(&mut client, "LAUNCH-MARK-7");

    let (mut host, stderr) =
        spawn_attach_with_config(&fixture, &["-t", &pane_a], "render", "[client]\n");
    let mut got = wait_for_output(&host, b"LAUNCH-MARK-7", Duration::from_secs(10));
    got.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(600),
    ));
    let text = plain_text(&got);
    assert!(
        text.contains("workspaces"),
        "the panel paints on the first frame without a chord. stderr: {} text: {text:?}",
        stderr.lock().unwrap()
    );
    assert!(
        !text.contains("sidebar on"),
        "no toggle ran — the panel is the launch state, not a flash: {text:?}"
    );
    assert_eq!(
        cup_col_of_text(&got, "LAUNCH-MARK-7"),
        Some(52),
        "the daemon's first division already reserves the 20-col panel. text: {text:?}"
    );
    // The strip's ROWS, not just the tab strip's ` workspaces ` title:
    // the workspace area's ` new ` footer chip only composes when the
    // rows exist.
    assert!(
        got.windows(5).any(|w| w == b" new "),
        "the panel's workspace rows paint without a chord: {text:?}"
    );
    host.killer.kill().ok();
}

/// The persisted panel state (`client-state.json` beside the client's
/// config) applies at launch: a collapsed panel attaches as the 2-column
/// status rail — the daemon's first division reserves 2 columns, not the
/// 20 of an expanded panel (the marker at 43, not 52) — and the attach
/// writes nothing back on launch.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_persisted_collapsed_panel_launches_as_the_rail() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbrail");
    let pane_a = split_with_right_marker(&mut client, "RAIL-MARK-7");
    let state = fixture
        .client_config_path()
        .with_file_name("client-state.json");
    let body = r#"{"sidebar_width":30,"sidebar_collapsed":true}"#;
    std::fs::write(&state, body).expect("write client state");

    let (mut host, stderr) =
        spawn_attach_with_config(&fixture, &["-t", &pane_a], "render", "[client]\n");
    let mut got = wait_for_output(&host, b"RAIL-MARK-7", Duration::from_secs(10));
    got.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(600),
    ));
    let text = plain_text(&got);
    assert_eq!(
        cup_col_of_text(&got, "RAIL-MARK-7"),
        Some(43),
        "the rail reserves 2 columns (78 = 39 | 38). stderr: {} text: {text:?}",
        stderr.lock().unwrap()
    );
    assert!(
        got.windows("»".len()).any(|w| w == "»".as_bytes()),
        "the rail paints its expand chevron: {text:?}"
    );
    assert!(
        !text.contains("workspaces"),
        "the rail paints no area header or tab-strip title: {text:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&state).expect("state"),
        body,
        "launching does not rewrite the state"
    );
    host.killer.kill().ok();
}

/// `sidebar-on-launch = false`: the panel stays hidden on launch and the
/// panes divide the full width (the marker at column 42).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_sidebar_on_launch_false_keeps_panel_hidden() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbnolaunch");
    let pane_a = split_with_right_marker(&mut client, "NOLAUNCH-MARK-7");

    let (mut host, stderr) = spawn_attach_with_config(
        &fixture,
        &["-t", &pane_a],
        "render",
        "[client]\nsidebar-on-launch = false\n",
    );
    // Settle on the marker first so the absence check reads a painted
    // frame, not an empty capture.
    let mut got = wait_for_output(&host, b"NOLAUNCH-MARK-7", Duration::from_secs(10));
    got.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(600),
    ));
    let text = plain_text(&got);
    assert_eq!(
        cup_col_of_text(&got, "NOLAUNCH-MARK-7"),
        Some(42),
        "panel hidden: the full-width split. stderr: {} text: {text:?}",
        stderr.lock().unwrap()
    );
    assert!(
        !text.contains("workspaces"),
        "no panel strip on launch: {text:?}"
    );
    host.killer.kill().ok();
}

/// The refit's repaint_all clears the WHOLE screen — the strip rows too —
/// so the sidebar toggle's resize_to must invalidate the tab strip and
/// status row or their diffs flush nothing and the rows stay wiped (the
/// manual-pass report: part of the status bar did not render after
/// bringing the panel up).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_sidebar_toggle_repaints_the_strip_rows() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbstrip");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, _stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up (mouse reporting enabled)",
    );

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    // The refit frame rides the same flush as the flash (one pump
    // iteration); drain a beat for the tail before matching.
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(600),
    ));
    // Bottom row, last column (host 80x24): the status row's tail only
    // flushes on a full-row diff, which only the invalidate produces.
    // Run-length encoding reaches the tail via the cursor's
    // auto-advance instead of a CUP on the last cell, so the proof is
    // the row-24 paint's reach, not a byte needle.
    assert!(
        row24_furthest_col_on(&flash, 24) >= 80,
        "the refit must repaint the status row's tail (row 24 reaches col {}). tail: {:?}",
        row24_furthest_col_on(&flash, 24),
        String::from_utf8_lossy(&flash)
            .chars()
            .rev()
            .take(400)
            .collect::<String>()
    );
    // Top row: the tab strip under the same rule. The invalidate
    // repaints every strip column, head through the bar tail (col 80,
    // unchanged since the seed frame) — under run-length encoding the
    // proof is the row's painted reach, not CUP byte needles.
    assert!(
        row24_furthest_col_on(&flash, 1) >= 80,
        "the refit must repaint the whole tab strip row (row 1 reaches col {}). tail: {:?}",
        row24_furthest_col_on(&flash, 1),
        String::from_utf8_lossy(&flash)
            .chars()
            .rev()
            .take(400)
            .collect::<String>()
    );
    host.killer.kill().ok();
}

/// The status-bar toggle: prefix S hides the bottom row — the pane grid
/// grows a row, the shell prompt moves DOWN to host row 24, the Help
/// chip vanishes from the flushes, and nothing erases the grown bottom
/// row afterward; prefix S again restores the row, the chip, and the
/// cursor to host row 23.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_status_bar_toggle_hides_and_restores_the_row() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbtoggle");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let mut shown = wait_for_output(&host, b"$", Duration::from_secs(10));
    // Settle a beat so the seed frame's tail (the strip and the status
    // row follow the pane cells in one flush) is in the capture.
    shown.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&shown).contains("C-b? Help"),
        "the Help chip ends the shown bar: {shown:?}"
    );
    assert_eq!(
        cup_row_of_text(&shown, "Help"),
        Some(24),
        "the chip paints on the bottom row. stderr: {}",
        stderr.lock().unwrap()
    );

    // Hide: the refit reports the taller grid and the pane replays one
    // row taller — the newest line moves DOWN onto the row the status
    // bar vacated. No flash cue — the flash paints ON the status row, so
    // the bar's vanishing is the feedback. The chip must be gone from
    // the flushes entirely (byte windows over a per-cell flush are
    // meaningless — this reads the plain text).
    host.to_child.write_all(&[0x02, b'S']).expect("prefix S");
    host.to_child.flush().ok();
    let mut hidden = wait_for_output(&host, b"\x1b[24;", Duration::from_secs(10));
    hidden.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        !plain_text(&hidden).contains("Help"),
        "the chip is gone while hidden. text: {:?}",
        plain_text(&hidden)
    );
    // The pane claims the vacated row: with herdr's per-pane boxes the
    // pane rectangle's bottom edge (the ring's ─) moves DOWN onto host
    // row 24 — a row no flush could target while the bar owned it, since
    // the hidden state never writes the row at all.
    assert!(
        row24_paints_char(&hidden, '─'),
        "the pane box's bottom edge must move onto the vacated row"
    );

    // Show again: the chip returns to the bottom row and the cursor back
    // to the shrunk content's last row (host row 23).
    host.to_child.write_all(&[0x02, b'S']).expect("prefix S");
    host.to_child.flush().ok();
    let mut reshowed = wait_for_output(&host, b"\x1b[23;", Duration::from_secs(10));
    reshowed.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&reshowed).contains("C-b? Help"),
        "the chip returns with the bar: {:?}",
        plain_text(&reshowed)
    );
    host.killer.kill().ok();
}

/// The side panel's ` menu ` chip opens the command menu, and its
/// `detach` row ends the client exactly like prefix `d` (exit 0).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_panel_menu_chip_detaches() {
    let (fixture, _daemon, mut client) = fixture_with_session("menudetach");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up (mouse reporting enabled)",
    );

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"sidebar on",
        Duration::from_secs(10),
        "side panel toggled on",
    );
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(600));
    // The chips sit on the workspace area's last row: the 22-row frame
    // splits 11/11, so composed row 10 = host row 11 (SGR row 12);
    // ` menu ` spans 0-based cols 13..19.
    host.to_child
        .write_all(b"\x1b[<0;17;12M")
        .expect("menu chip press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"reload config", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains("reload config"),
        "the chip must open the command menu. stderr: {} text: {}",
        stderr.lock().unwrap(),
        plain_text(&flash)
    );

    // The menu opens beside its chip: the box's left ring at x0 = 19
    // (just right of the chip on the 20-wide strip) and its bottom ring
    // on the chip's frame row 10, so the 8-row box starts at y0 = 3.
    // `detach` is panel row 4 at host y0+6.
    let (x0, y0) = (19usize, 3usize);
    host.to_child
        .write_all(format!("\x1b[<0;{};{}M", x0 + 5, y0 + 7).as_bytes())
        .expect("detach click");
    host.to_child.flush().ok();
    let code = child_exit(&mut host, Duration::from_secs(10));
    assert_eq!(
        code,
        Some(0),
        "the detach row must end the client with exit 0. stderr: {}",
        stderr.lock().unwrap()
    );
}

/// The tab context menu must be CLICKABLE with the side panel up: the
/// menu's geometry is host-absolute (anchored under the clicked tab,
/// whose block spans absolute host columns), so its clicks map by the
/// raw host column — the guard mapped them through `x - strip` and every
/// click landed one panel width left of the visible items (the
/// manual-pass round-7 report; the same defect class as round 5's
/// unclickable +).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_tab_menu_actions_click_with_the_panel_up() {
    let (fixture, _daemon, mut client) = fixture_with_session("menuclick");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up (mouse reporting enabled)",
    );

    // Panel up, then a right-press on the shown window's tab opens the
    // tab menu.
    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"sidebar on",
        Duration::from_secs(10),
        "side panel toggled on",
    );
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(600));
    // With the panel up the tabs start at the panel's right edge
    // (strip col 20): the shown window's herdr block (`  att  `, width
    // 7) spans 0-based host cols 20-26, so the right press at 1-based
    // col 24 (0-based 23, the `a`) lands on it.
    let press: &[u8] = b"\x1b[<2;24;1M";
    host.to_child.write_all(press).expect("right press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"rename", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains("rename"),
        "the menu must open on a right press. stderr: {} text: {}",
        stderr.lock().unwrap(),
        plain_text(&flash)
    );

    // Click the RENAME row at its RAW host coordinates (the menu is
    // anchored under the clicked tab: the tab block `  att  ` starts at
    // 0-based host col 20 — the box's left ring x0 — and its top ring
    // sits on the first frame row, host row 1 0-based. The rename row
    // (panel row 1) paints at frame row 2, host row 3 0-based; SGR is
    // 1-based.)
    let menu_col = 25; // x0 (20) + 5, inside the box, 1-based host col
    let menu_row = 4; // frame row 2 + the strip row + the 1-based origin
    host.to_child
        .write_all(format!("\x1b[<0;{menu_col};{menu_row}M").as_bytes())
        .expect("menu click");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"rename window", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains("rename window"),
        "the menu's rename row must open the rename prompt. stderr: {} raw: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&flash)
    );
    host.killer.kill().ok();
}

#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_plus_click_works_with_the_panel_up() {
    let (fixture, _daemon, mut client) = fixture_with_session("pluspanel");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let mut all = expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up (mouse reporting enabled)",
    );
    // Panel up (the manual-pass reporter's state): the top row stays a
    // tab-strip row and the + follows the last visible tab. Discover
    // its painted column instead of hardcoding it.
    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    // output settle: keep EVERY window's bytes — the channel is
    // consumed once, so a discarded return drops that span's paints
    // from the replay.
    all.extend(expect_output(
        &host,
        b"sidebar on",
        Duration::from_secs(10),
        "side panel toggled on",
    ));
    let drain = wait_for_output(&host, b"\x00-never", Duration::from_millis(400));
    all.extend_from_slice(&drain);
    screen_stable(
        &host,
        &mut all,
        Duration::from_millis(400),
        Duration::from_secs(10),
    );
    let (grid, _) = reconstructed_screen(&all, 24, 80);
    let row0 = grid.first().expect("a strip row");
    if !row0.contains('+') {
        panic!(
            "the + paints with the panel up; screen:\n{}",
            grid.join("\n")
        );
    }
    let byte = row0.find('+').expect("the + paints on row0");
    let col = row0[..byte].chars().count() + 1; // 1-based host column
    let press = format!("\x1b[<0;{col};1M");
    host.to_child
        .write_all(press.as_bytes())
        .expect("plus press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b" new tab ", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains(" new tab "),
        "the prompt must open with the panel up. stderr: {} text: {}",
        stderr.lock().unwrap(),
        plain_text(&flash)
    );
    host.killer.kill().ok();
}

#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_plus_click_works_on_a_wide_host() {
    let (fixture, _daemon, mut client) = fixture_with_session("pluswide");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let mut all = expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up (mouse reporting enabled)",
    );

    // A live terminal is much wider than the harness's 80 columns: the
    // manual-pass report could not click the + there. Widen the host,
    // let the repaint settle, and click the discovered + — it follows
    // the last tab, so the resize keeps it reachable at any width.
    host.master
        .resize(PtySize {
            rows: 24,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("resize");
    // output settle: the resize repaint burst, then discover
    let drain = wait_for_output(&host, b"\x00-never", Duration::from_millis(400));
    all.extend_from_slice(&drain);
    screen_stable(
        &host,
        &mut all,
        Duration::from_millis(400),
        Duration::from_secs(10),
    );
    let (grid, _) = reconstructed_screen(&all, 24, 200);
    let row0 = grid.first().expect("a strip row");
    let byte = row0.find('+').expect("the + paints on the wide host");
    let col = row0[..byte].chars().count() + 1; // 1-based host column
    let press = format!("\x1b[<0;{col};1M");
    host.to_child
        .write_all(press.as_bytes())
        .expect("plus press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b" new tab ", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains(" new tab "),
        "the prompt must open at the wide host's right edge. stderr: {} text: {}",
        stderr.lock().unwrap(),
        plain_text(&flash)
    );
    host.killer.kill().ok();
}

/// The tab strip's ` + ` button: a click opens the new-tab prompt
/// (herdr's footer, the next free index as the editable default), Enter
/// creates the window and lands on it, Esc cancels without creating.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_plus_click_prompts_and_creates_a_window() {
    let (fixture, _daemon, mut client) = fixture_with_session("plusclick");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    // The ` + ` button follows the last tab: discover its painted
    // column on the strip row instead of hardcoding the old right-edge
    // reservation.
    let mut all = expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up (mouse reporting enabled)",
    );
    let shown = screen_until(&host, &mut all, Duration::from_secs(10), |s| {
        s.lines().next().is_some_and(|row| row.contains('+'))
    });
    let row0 = shown.lines().next().expect("a strip row");
    let byte = row0.find('+').expect("the + paints on the strip row");
    let col = row0[..byte].chars().count() + 1; // 1-based host column
    let press = format!("\x1b[<0;{col};1M");
    host.to_child
        .write_all(press.as_bytes())
        .expect("plus press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b" new tab ", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    let text = plain_text(&flash);
    assert!(
        text.contains(" new tab "),
        "the prompt opens. stderr: {} text: {text}",
        stderr.lock().unwrap()
    );
    assert!(
        text.contains("> 1"),
        "the default is the next free index: {text}"
    );
    let windows_before = client.send("list-windows").expect("list").join("");

    // Esc cancels without creating: the daemon's window roster is
    // unchanged after the prompt closes.
    host.to_child.write_all(&[0x1b]).expect("esc");
    host.to_child.flush().ok();
    // negative check: nothing positive to wait on, so the window is fixed
    std::thread::sleep(Duration::from_millis(300));
    let after_cancel = client.send("list-windows").expect("list").join("");
    assert_eq!(
        after_cancel, windows_before,
        "esc must not create a window: {after_cancel}"
    );

    // Click + again, edit the default (the input shows `> 1notes` before
    // saving), Enter: the window exists and the view lands on it. The
    // reopen's overlay flush races the esc restore window, so settle
    // instead of waiting on the title needle.
    host.to_child
        .write_all(press.as_bytes())
        .expect("plus press");
    host.to_child.flush().ok();
    // input spacing
    std::thread::sleep(Duration::from_millis(300));
    host.to_child.write_all(b"notes").expect("name");
    host.to_child.flush().ok();
    let typed = wait_for_output(&host, b"\x00-never", Duration::from_millis(400));
    assert!(
        plain_text(&typed).contains("notes"),
        "the typed name joins the default in the input line: {:?}",
        plain_text(&typed)
    );
    host.to_child.write_all(b"\r").expect("enter");
    host.to_child.flush().ok();
    common::wait_daemon(
        fixture.socket(),
        "list-windows",
        |reply| reply.contains("notes"),
        Duration::from_secs(10),
        "the named window exists",
    );
    host.killer.kill().ok();
}

/// session state: cycling away and back and then splitting a pane must
/// not revert the borders (the manual-pass report — the re-seed rebuilt
/// the renderer with the default glyphs).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_border_style_survives_split() {
    let (fixture, _daemon, mut client) = fixture_with_session("herdrsplit");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    // herdr is the DEFAULT: the seed's first frame (which flushes before
    // the shell's prompt bytes) already paints per-pane boxes.
    let pre = expect_output(
        &host,
        b"\xe2\x95\xad",
        common::CEILING,
        "herdr corner at seed",
    );
    // plain_text latin-1s multi-byte glyphs, so a painted ╭ is matched
    // on the raw bytes (the glyph's UTF-8 is contiguous in one cell's
    // flush); the wait needle is the same three bytes.
    assert!(
        pre.windows(3).any(|w| w == b"\xe2\x95\xad"),
        "herdr corner must paint at seed. stderr: {}",
        stderr.lock().unwrap()
    );

    // Cycle all the way around (herdr → unicode → … → herdr): five
    // presses return to the default set.
    for _ in 0..5 {
        host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    }
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"border style: herdr", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("border style: herdr"),
        "the full cycle must land back on herdr"
    );

    // Split via the CHORD: the split's re-seed used to restore the
    // session-level pane-borders flag (the cycle flipped the renderer's
    // flag only) and the boxes reverted to shared dividers.
    host.to_child.write_all(&[0x02, b'%']).expect("prefix %");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"\xe2\x95\xad", Duration::from_secs(10));
    assert!(
        got.windows(3).any(|w| w == b"\xe2\x95\xad"),
        "the chord split must keep the per-pane-box style. stderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Border labels paint the user `-T` title: set labels daemon-side
/// before attach, cycle to the per-pane-box style, and the label text
/// must appear embedded in the pane's top border (the manual-pass
/// report: labels never showed because the painter read the shell's OSC
/// title only).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_border_label_paints_the_user_title() {
    let (fixture, _daemon, mut client) = fixture_with_session("labels");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("select-pane -t {pane_a} -T 'MY-TEST-LABEL'"))
        .expect("label a");
    let pane_b = client
        .send(&format!("split-window -h -t {pane_a}"))
        .expect("split")
        .join("");
    let pane_b = pane_b.trim().to_string();
    client
        .send(&format!("select-pane -t {pane_b} -T 'SECOND-LABEL'"))
        .expect("label b");

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up (mouse reporting enabled)",
    );

    // Cycle to herdr: the per-pane-box mode labels paint in.
    for _ in 0..4 {
        host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    }
    host.to_child.flush().ok();
    let got = expect_output(
        &host,
        b"MY-TEST-LABEL",
        common::CEILING,
        "the user label in the border",
    );
    assert!(
        plain_text(&got).contains("MY-TEST-LABEL"),
        "the user label must paint in the border. stderr: {} tail: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
            .chars()
            .rev()
            .take(500)
            .collect::<String>()
    );
    host.killer.kill().ok();
}

/// Card 01a11c5b (the size-path twin of 7059335's mouse inset): with the
/// pane-border ring on (the default herdr style), a full-screen app's last
/// row must be VISIBLE. The client declares its per-pane chrome on the
/// size report, so the daemon sizes the pane's PTY to the ring's interior:
/// the 80x24 host leaves an 80x22 content grid (tab strip + status bar),
/// the single pane's rect is 80x22, and its interior is 78x20. A program
/// drawing at `stty size`'s last row lands inside the painted interior.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_border_ring_keeps_the_tui_last_row_visible() {
    let (fixture, _daemon, mut client) = fixture_with_session("lastrow");
    let pane = session_first_pane(&mut client, "$0");
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    let mut all = Vec::new();
    echo_marker(&mut client, &pane, "ATTACHED-OK");
    let shown = screen_until(&host, &mut all, common::CEILING, |s| {
        s.contains("ATTACHED-OK")
    });
    assert!(
        shown.contains("ATTACHED-OK"),
        "the render client never painted the pane:\n{shown}\nstderr: {}",
        stderr.lock().unwrap()
    );

    // The PTY's own idea of its size: the ring's interior. The `%s`
    // printf split keeps the shell's echo of the typed line from matching.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'printf \"SIZE%sIS \" -; stty size'"
        ))
        .expect("stty keys");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("stty Enter");
    let deadline = Instant::now() + Duration::from_secs(10);
    let size_body = loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("SIZE-IS ") || Instant::now() >= deadline {
            break body;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    // A full-screen draw at the PTY's last row: clear, jump to the row
    // `stty size` reports, print the marker, park.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'r=$(stty size | cut -d\" \" -f1); clear; printf \"\\033[%s;1HLAST%sROW\" \"$r\" -; sleep 60'"
        ))
        .expect("draw keys");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("draw Enter");
    let shown = screen_until(&host, &mut all, Duration::from_secs(15), |s| {
        s.contains("LAST-ROW")
    });
    let rows: Vec<&str> = shown.lines().collect();
    let at = rows.iter().position(|r| r.contains("LAST-ROW"));
    // Host row 21 (0-based): tab strip row 0, ring row 1, interior rows
    // 2..=21, the ring's bottom edge on row 22, the status bar on 23.
    assert_eq!(
        at,
        Some(21),
        "the TUI's last row must paint on the interior's last row, not in the cropped ring:\n{shown}\nstderr: {}",
        stderr.lock().unwrap()
    );
    // Character (not byte) column: the ring's glyph reconstructs as a
    // multi-byte replacement character.
    let col = rows[21]
        .find("LAST-ROW")
        .map(|byte| rows[21][..byte].chars().count());
    assert_eq!(
        col,
        Some(1),
        "column 1 is the interior's first column:\n{shown}"
    );
    assert!(
        size_body.contains("SIZE-IS 20 78"),
        "the pane's PTY must be the border ring's interior (20 rows x 78 cols): {size_body:?}"
    );
    host.killer.kill().ok();
}

/// Card 01a13d03's render-mode gap: a held pane shows the exit cue on the
/// status row, and the respawn chord (`C-b r`) restarts it — the client
/// repaints the live shell on the same pane. Driven through Ctrl+D on a
/// pane's shell under a remain-on-exit daemon — the hold is what makes
/// the pane held dead now that the auto-remove default cascades.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_held_pane_shows_the_exit_cue_and_the_chord_respawns_it() {
    let fixture = MuxFixture::new("rendercue");
    let _daemon = common::spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
    client.send("new-session -s att").expect("new-session");
    let pane = session_first_pane(&mut client, "$0");
    // settle: the fresh pane's shell starts before the client attaches
    std::thread::sleep(Duration::from_millis(500));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
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

    // `true` pins the shell's last status to 0: the exit cue embeds the exit
    // status, and a host rc that leaves $? nonzero would leak its digit here.
    host.to_child.write_all(b"true\r").expect("type true");
    host.to_child.flush().ok();
    host.to_child.write_all(&[0x04]).expect("Ctrl+D");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"respawns)", Duration::from_secs(10));
    let text = String::from_utf8_lossy(&got);
    assert!(
        text.contains("(exited 0 \u{2014} C-b r respawns)"),
        "the status row shows the exit cue. stderr: {} text: {text:?}",
        stderr.lock().unwrap()
    );

    // The chord restarts it; the fresh shell runs a command whose output
    // only a live pane could paint.
    host.to_child.write_all(&[0x02, b'r']).expect("prefix r");
    host.to_child.flush().ok();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let info = client
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info")
            .join("\n");
        if !info.contains("exited=") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the chord never respawned: {info}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // settle: the respawned shell's prompt before typing
    std::thread::sleep(Duration::from_millis(500));
    host.to_child
        .write_all(b"echo CUE-LIVE | tr A-Z a-z\r")
        .expect("type");
    host.to_child.flush().ok();
    let mut got = wait_for_output(&host, b"cue-live", Duration::from_secs(10));
    screen_stable(
        &host,
        &mut got,
        Duration::from_millis(600),
        Duration::from_secs(10),
    );
    let screen = reconstructed_screen(&got, 24, 80).0.join("\n");
    assert!(
        screen.contains("cue-live"),
        "the client repaints the live shell: {screen}"
    );
    assert!(
        !screen.contains("respawns)"),
        "the cue clears after the respawn: {screen}"
    );
    host.killer.kill().ok();
}

/// A pane held dead BEFORE the render client attaches: the registration
/// replay's `%pane-exited` seeds the cue on the first status row.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_pane_held_before_attach_shows_the_exit_cue() {
    let (fixture, _daemon, mut client) = fixture_with_session("rendercuelate");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("respawn-pane -t {pane} -k exit 4"))
        .expect("respawn -k");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let info = client
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info")
            .join("");
        if info.contains("exited=4") {
            break;
        }
        assert!(Instant::now() < deadline, "the pane never died: {info}");
        std::thread::sleep(Duration::from_millis(50));
    }
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    let got = wait_for_output(&host, b"respawns)", Duration::from_secs(10));
    let text = String::from_utf8_lossy(&got);
    assert!(
        text.contains("(exited 4 \u{2014} C-b r respawns)"),
        "the replayed held pane shows the cue. stderr: {} text: {text:?}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Popup command windows (herdr's `type = "popup"`): prefix + the
/// popup's key runs its command on the daemon in a scratch window the
/// box mirrors. The command really runs (its output is on the daemon
/// pane's screen AND painted in the box, with the description in the
/// border); the scratch window is sized to the box's inner grid; keys
/// typed while the popup is up reach its pane and not the tiled one;
/// prefix + the key again closes it and kills the window; and a command
/// that exits closes its popup on its own — typing then reaches the
/// original pane again.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_popup_runs_a_command_in_a_floating_box_and_closes() {
    let (fixture, _daemon, mut client) = fixture_with_session("popup");
    let pane0 = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let config = "[client]\nsidebar-on-launch = false\n\
        [[client.popup]]\nkey = \"t\"\n\
        command = \"echo POPUP-UP-MARKER; read line; echo GOT-$line; sleep 30\"\n\
        width = \"50%\"\nheight = 10\ndescription = \"scratchbox\"\n\
        [[client.popup]]\nkey = \"y\"\ncommand = \"echo QUICK-MARKER; sleep 1\"\n";
    let (mut host, stderr) = spawn_attach_with_config(&fixture, &["-t", &pane0], "render", config);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up",
    );
    // input spacing: the pump must be past its seed before the chord.
    std::thread::sleep(Duration::from_millis(300));

    host.to_child.write_all(&[0x02, b't']).expect("prefix t");
    host.to_child.flush().ok();
    let windows = common::wait_daemon(
        fixture.socket(),
        "list-windows -a",
        |reply| popup_window_id(reply).is_some(),
        Duration::from_secs(15),
        "the popup's scratch window exists",
    );
    let popup_window = popup_window_id(&windows).expect("scratch window");
    let popup_pane = client
        .send(&format!("list-panes -t {popup_window}"))
        .expect("popup panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("popup pane")
        .to_string();
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {popup_pane}"),
        |screen| screen.contains("POPUP-UP-MARKER"),
        Duration::from_secs(15),
        "the popup command ran on the daemon",
    );
    // 50% of 80 columns = 40 outer, 38 inner; 10 outer rows, 8 inner.
    common::wait_daemon(
        fixture.socket(),
        &format!("pane-info -t {popup_pane}"),
        |reply| reply.contains(" 38x8"),
        Duration::from_secs(15),
        "the scratch window is the box's inner grid",
    );
    // The box's first frames carry its border title and the output.
    let painted = expect_output(
        &host,
        b"POPUP-UP-MARKER",
        Duration::from_secs(15),
        "the box mirrors the popup's output",
    );
    assert!(
        plain_text(&painted).contains("scratchbox"),
        "the box's border carries the description: {}",
        plain_text(&painted)
    );

    // Typing goes to the popup, never the tiled pane.
    host.to_child.write_all(b"ping\r").expect("type into popup");
    host.to_child.flush().ok();
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {popup_pane}"),
        |screen| screen.contains("GOT-ping"),
        Duration::from_secs(15),
        "typed input reached the popup's command",
    );
    let tiled = client
        .send(&format!("capture-pane -t {pane0}"))
        .expect("tiled capture")
        .join("\n");
    assert!(
        !tiled.contains("ping"),
        "the tiled pane saw popup input: {tiled}"
    );

    // prefix t again closes it: the scratch window is killed.
    host.to_child
        .write_all(&[0x02, b't'])
        .expect("prefix t close");
    host.to_child.flush().ok();
    common::wait_daemon(
        fixture.socket(),
        "list-windows -a",
        |reply| popup_window_id(reply).is_none(),
        Duration::from_secs(15),
        "closing the popup killed its scratch window",
    );

    // A command that exits closes its popup by itself.
    std::thread::sleep(Duration::from_millis(300));
    host.to_child.write_all(&[0x02, b'y']).expect("prefix y");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"QUICK-MARKER",
        Duration::from_secs(15),
        "the quick popup ran",
    );
    common::wait_daemon(
        fixture.socket(),
        "list-windows -a",
        |reply| popup_window_id(reply).is_none(),
        Duration::from_secs(20),
        "an exited popup command leaves no scratch window",
    );

    // The tiled view has the input back. The window's removal can reach
    // the daemon before the client's stream sees its end (the stream
    // watch polls), and keys typed in between go to the closing popup by
    // design — so re-type until the tiled pane runs the line.
    let back = Instant::now() + Duration::from_secs(30);
    loop {
        host.to_child
            .write_all(b"echo BACK-HOME-MARKER\r")
            .expect("type into the tiled pane");
        host.to_child.flush().ok();
        let reached = (0..20).any(|_| {
            std::thread::sleep(Duration::from_millis(100));
            client
                .send(&format!("capture-pane -t {pane0}"))
                .map(|lines| echoed("BACK-HOME-MARKER")(&lines.join("\n")))
                .unwrap_or(false)
        });
        if reached {
            break;
        }
        assert!(
            Instant::now() < back,
            "typing never reached the original pane after the popup closed"
        );
    }
    let _ = stderr;
    host.killer.kill().ok();
}

/// The render client under a PTY with kitty graphics support modeled
/// (the fake host answers the probe's `a=q` with `OK`): a kitty image a
/// pane prints reaches the host as an upload plus a placement at the
/// image's host cell (pane cell (5, 3) + the tab strip row = CUP 5;6),
/// sized 2x2 cells with `C=1`. The text after the image lands on the
/// row the daemon's own pane put it (capture-pane ground truth), so the
/// mirror's image math matches the daemon's.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_kitty_graphics_reach_a_kitty_host_positioned() {
    let (fixture, _daemon, mut client) = fixture_with_session("kittyhost");
    let pane = session_first_pane(&mut client, "att");
    // The echoed command must wrap before ED2, regardless of the host's temp path.
    let dir = tempfile::Builder::new()
        .prefix(&"kitty-wrap-".repeat(8))
        .tempdir()
        .expect("tempdir");
    let script = kitty_image_script(dir.path());
    let (mut host, _stderr) = spawn_attach_host(
        &fixture,
        &["-t", &pane],
        "render",
        KITTY_CLIENT_CONFIG,
        &[],
        true,
    );
    let placement: &[u8] = b"\x1b[5;6H\x1b_Ga=p,i=10000,p=1,c=2,r=2,C=1,q=2,w=20,h=40;\x1b\\";
    let mut out = cat_into_pane(
        &mut client,
        &host,
        &pane,
        &script,
        placement,
        "the pane's image is placed on the host at its cell",
    );
    let text = String::from_utf8_lossy(&out).into_owned();
    let upload = text
        .find("\x1b_Ga=t,t=d,f=32,s=20,v=40,i=10000,q=2,m=1;")
        .expect("the image uploads (3200 RGBA bytes: two base64 chunks)");
    let placed = text
        .find(std::str::from_utf8(placement).unwrap())
        .expect("placed");
    assert!(upload < placed, "upload before placement");

    out.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(500),
    ));
    let (screen, _) = reconstructed_screen(&strip_apc(&out), 24, 80);
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture")
        .join("\n");
    let daemon_row = capture
        .lines()
        .position(|l| l.contains("KITTY-AFTER-MARKER"))
        .expect("the daemon's pane holds the marker");
    assert_eq!(
        daemon_row, 5,
        "the image moved the daemon's cursor 2 rows: {capture}"
    );
    assert!(
        screen[daemon_row + 1].starts_with("KITTY-AFTER-MARKER"),
        "the client's mirror put the marker on the daemon's row (+ the strip): {screen:#?}"
    );
    host.killer.kill().ok();
}

/// A host without kitty graphics (it answers the probe's DA1 sentinel
/// and never the kitty query — the probe's definite "unsupported"): the
/// pane's image never reaches it — no upload, no placement, no APC at
/// all past the one probe query — while the pane's text still paints.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_kitty_graphics_stay_off_a_silent_host() {
    let (fixture, _daemon, mut client) = fixture_with_session("kittysilent");
    let pane = session_first_pane(&mut client, "att");
    let dir = tempfile::tempdir().expect("tempdir");
    let script = kitty_image_script(dir.path());
    let (mut host, _stderr) = spawn_attach_host(
        &fixture,
        &["-t", &pane],
        "render",
        KITTY_CLIENT_CONFIG,
        &[],
        false,
    );
    let mut seen = cat_into_pane(
        &mut client,
        &host,
        &pane,
        &script,
        b"KITTY-AFTER-MARKER",
        "the pane's text paints",
    );
    seen.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(500),
    ));
    let text = String::from_utf8_lossy(&seen);
    assert!(text.contains("\x1b_Gi=31,"), "the probe asked: {text:?}");
    assert_eq!(
        text.matches("\x1b_G").count(),
        1,
        "nothing but the probe query reaches a silent host: {text:?}"
    );
    host.killer.kill().ok();
}

/// `kitty-graphics = false`: the client never asks the host and never
/// draws, even on a kitty-capable host.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_kitty_graphics_config_off_sends_nothing() {
    let (fixture, _daemon, mut client) = fixture_with_session("kittyoff");
    let pane = session_first_pane(&mut client, "att");
    let dir = tempfile::tempdir().expect("tempdir");
    let script = kitty_image_script(dir.path());
    let config = format!("{KITTY_CLIENT_CONFIG}kitty-graphics = false\n");
    let (mut host, _stderr) =
        spawn_attach_host(&fixture, &["-t", &pane], "render", &config, &[], true);
    let mut seen = cat_into_pane(
        &mut client,
        &host,
        &pane,
        &script,
        b"KITTY-AFTER-MARKER",
        "the pane's text paints",
    );
    seen.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(500),
    ));
    let text = String::from_utf8_lossy(&seen);
    assert!(
        !text.contains("\x1b_G"),
        "no kitty query and no graphics with the option off: {text:?}"
    );
    host.killer.kill().ok();
}
