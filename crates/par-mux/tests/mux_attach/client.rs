//! The attach client itself: passthrough mode, detach and terminal
//! restore, prefix chords, target resolution, the session picker, and the
//! byte-fidelity floods.

use super::*;

/// The output matcher finds a phrase the cell diff split with a cursor
/// jump over an unchanged space — the wire shape captured when
/// `render_mode_settings_dialog_opens_from_the_panel_menu` failed at host
/// load ~200 with the menu fully painted.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn output_matcher_reads_a_phrase_split_by_a_skipped_cell() {
    let wire = b"\x1b[9;22H\x1b[0mreload\x1b[9;29Hconfig\x1b[9;49H";
    assert!(
        !plain_text(wire).contains("reload config"),
        "the escape-stripped text loses the skipped space"
    );
    assert!(output_contains(wire, b"reload config"));
    assert!(output_contains(wire, b"reload"));
    assert!(!output_contains(wire, b"reload  config"));
    assert!(!output_contains(wire, b"\x00-never"));
}

/// The smallest-attached-client window size with two differently-shaped
/// attach clients (a wide-short and a narrow-tall terminal): the window
/// takes the minimum in EACH dimension, so neither client's screen runs
/// past its host, and each larger dimension pads with dim `.` cells. The
/// size holds still with both attached — before reports that moved
/// nothing were answered to the issuer alone, the two clients' re-reports
/// coupled into a %layout-change re-seed flip-flop (the manual-pass htop
/// sort-panel oscillation) — and the constraining client detaching grows
/// the window back.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn two_attach_clients_do_not_war_over_the_window_size() {
    let (fixture, _daemon, mut client) = fixture_with_session("sizewar");
    let panes = common::send(&mut client, "list-panes").expect("list-panes");
    let pane = panes.first().cloned().unwrap_or_default();
    fn pane_size(client: &mut par_mux::mux::MuxClient, pane: &str) -> (u16, u16) {
        let body = common::send(client, &format!("pane-info -t {pane}")).expect("pane-info");
        let row = body.first().expect("pane-info row");
        let size = row.split_whitespace().nth(2).expect("size token");
        let (cols, rows) = size.split_once('x').expect("WxH");
        (cols.parse().expect("cols"), rows.parse().expect("rows"))
    }
    // Poll until `done` holds, then until the size stops moving (the
    // chrome report lands just after the grid report, so a transient
    // pre-ring size is never the value to pin).
    fn settle(
        client: &mut par_mux::mux::MuxClient,
        pane: &str,
        what: &str,
        done: impl Fn((u16, u16)) -> bool,
    ) -> (u16, u16) {
        let end = Instant::now() + Duration::from_secs(15);
        let mut last = pane_size(client, pane);
        let mut since = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let now = pane_size(client, pane);
            if now != last {
                last = now;
                since = Instant::now();
            } else if done(now) && since.elapsed() >= Duration::from_millis(600) {
                return now;
            }
            assert!(Instant::now() < end, "{what}: pane size {last:?}");
        }
    }

    let spawn_sized = |fixture: &MuxFixture, cols: u16, rows: u16| {
        let (host, stderr) = spawn_attach_with_config(
            fixture,
            &[],
            "render",
            "[client]\nsidebar-on-launch = false\n",
        );
        host.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("host resize to the client's size");
        (host, stderr)
    };

    // The narrow-tall client alone sizes the window (90 wide, not the 80
    // default, so the report's landing is observable).
    let (tall, _tall_stderr) = spawn_sized(&fixture, 90, 30);
    let alone = settle(&mut client, &pane, "the first client's seed lands", |s| {
        s.0 > 80 && s.1 > 24
    });

    // The wide-short client attaches: the window keeps the narrow
    // client's width and takes the short client's height.
    let (wide, _wide_stderr) = spawn_sized(&fixture, 130, 14);
    let both = settle(&mut client, &pane, "the componentwise minimum", |s| {
        s.1 < 14
    });
    assert_eq!(
        both.0, alone.0,
        "the wider client must not widen the window"
    );

    // Several status-refresh cycles with no input: the size must not move
    // again — the war's re-seed churn showed up as exactly this window
    // re-fitting between the two clients' reports.
    let end = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(
            pane_size(&mut client, &pane),
            both,
            "the size must hold with both clients attached"
        );
    }

    // Each client pads the dimension it has to spare: the wide client to
    // the right of the window, the tall one below it.
    for (host, what) in [(&wide, "the wide client"), (&tall, "the tall client")] {
        let out = wait_for_output(host, b"..........", Duration::from_secs(10));
        assert!(
            out.windows(10).any(|w| w == b".........."),
            "{what} pads past the window with dots"
        );
    }

    // The constraining client shrinks further: its report applies.
    wide.master
        .resize(PtySize {
            rows: 11,
            cols: 130,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("resize the wide client's host");
    let shorter = settle(&mut client, &pane, "the smaller report applies", |s| {
        s.1 == both.1 - 3
    });
    assert_eq!(shorter.0, alone.0);

    // The constraining client detaches: the window grows back to the
    // remaining client's report.
    let mut wide = wide;
    wide.killer.kill().ok();
    wide.child.wait().ok();
    settle(&mut client, &pane, "the window grows back", |s| s == alone);

    let mut tall = tall;
    tall.killer.kill().ok();
    tall.child.wait().ok();
}

/// Acceptance criterion 1: a known escape-sequence payload round-trips
/// byte-identical through attach — the pane emits a distinctive SGR+CUP
/// sequence; the host reads exactly those bytes out of the client.
///
/// Unix-only: the pane payload pipeline is POSIX shell syntax.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn escape_sequence_payload_round_trips_byte_identical() {
    let (fixture, _daemon, mut client) = fixture_with_session("fidelity");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    // The payload: a CUP + SGR + literal marker + reset. Distinctive bytes
    // that must survive the daemon's octal %output escaping and the
    // client's octal decode untouched.
    const PAYLOAD: &str = "\x1b[5;3H\x1b[38;5;196mATTACH-FIDELITY-MARKER\x1b[0m";

    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    // Settle on the resync + status draw, so everything the host reads
    // after this point is the LIVE %output path (the criterion: the raw
    // pane bytes survive the daemon's octal %output escaping AND the
    // client's octal decode, byte-identical).
    expect_output(
        &host,
        b"\x1b[1;23r",
        Duration::from_secs(10),
        "passthrough status-row reserve",
    );

    // Emit the payload. Base64 carries the bytes through both quoting
    // layers untouched (the daemon's bounded grammar and the pane shell's
    // own word splitting); the whole command rides as ONE quoted send-keys
    // token (tokens otherwise concatenate without spaces), and Enter goes
    // separately as a key.
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(PAYLOAD);
    client
        .send(&format!(
            "send-keys -t {pane} -l 'echo {encoded} | base64 -d'"
        ))
        .expect("send-keys payload");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("press Enter");

    let got = wait_for_output(&host, b"ATTACH-FIDELITY-MARKER", Duration::from_secs(15));
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture");
    assert!(
        got.windows(PAYLOAD.len()).any(|w| w == PAYLOAD.as_bytes()),
        "the exact escape-sequence payload must appear byte-identical in the \
         client's output (%output raw forward): {:?}\nstderr: {}\ncapture: {capture:?}",
        String::from_utf8_lossy(&got),
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Acceptance criterion 2: prefix d detaches — the client exits 0, restores
/// the terminal (the reset sequence appears in its output), and the pane is
/// still running afterwards (a later capture still sees its content).
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn prefix_d_detaches_and_leaves_the_pane_running() {
    let (fixture, _daemon, mut client) = fixture_with_session("detach");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    wait_for_output(&host, b"attach-detach-marker", Duration::from_secs(1)); // settle
                                                                             // Put a marker on the pane's screen first.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'echo attach-detach-marker'"
        ))
        .expect("send marker");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("press Enter");
    expect_output(
        &host,
        b"attach-detach-marker",
        Duration::from_secs(15),
        "attach-detach-marker on the host screen",
    );

    // Prefix d = C-b 0x02 then 'd'.
    host.to_child.write_all(&[0x02, b'd']).expect("prefix d");
    host.to_child.flush().ok();

    let code = child_exit(&mut host, Duration::from_secs(10));
    assert_eq!(
        code,
        Some(0),
        "prefix d must detach with exit 0. stderr: {}",
        stderr.lock().unwrap()
    );

    // The pane outlived the client: an ordinary capture still answers and
    // the marker is on the pane's screen.
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture after detach");
    assert!(
        capture.join("").contains("attach-detach-marker"),
        "the pane must still be running after detach: {capture:?}"
    );
}

/// Acceptance criterion 2b (clean restore): the client's output carries a
/// terminal-restore sequence after detach — raw mode off is crossterm's
/// disable_raw_mode (no bytes), and the pump leaves the scroll region
/// reset; the DECSTBM reserve appeared at startup, so a detach must reset
/// the region (our status redraw writes `\x1b[r` on the way out via the
/// last draw, and detach itself re-emits a final reset).
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn detach_restores_the_terminal_region() {
    let (fixture, _daemon, mut client) = fixture_with_session("restore");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    // Startup: the screen clear must come before anything else
    // the client draws — the replay must not overwrite the host's prior
    // content — and the status-line DECSTBM reserve must appear. Both are
    // asserted on UNIX only. Windows ConPTY re-encodes the client's
    // output: it INTERPRETS DECSTBM (the region is applied conhost-side)
    // and consumes alt-screen switches into its own buffer model, so
    // neither raw sequence passes through to the pty master even though
    // the client emitted them (the re-encoded status row draw does
    // appear). The client-side emits are pinned by the unix run; the
    // Windows run asserts the status draw ConPTY preserves.
    if cfg!(unix) {
        let startup = wait_for_output(&host, b"\x1b[1;23r", Duration::from_secs(10));
        assert!(
            startup.windows(4).any(|w| w == b"\x1b[2J"),
            "attach must clear the screen before the replay: {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
        );
        assert!(
            !startup.windows(8).any(|w| w == b"\x1b[?1049h"),
            "attach must NOT enter the alternate screen (a pane app's own 1049 \
             pair would pop it and strand later draws on the host main): \
             {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
        );
        assert!(
            startup.windows(7).any(|w| w == b"\x1b[1;23r"),
            "attach must reserve the status row with DECSTBM (ESC[1;23r on a 24-row \
             terminal): {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
        );
        assert!(
            startup.windows(4).position(|w| w == b"\x1b[2J")
                < startup.windows(7).position(|w| w == b"\x1b[1;23r"),
            "the clear precedes the status draw: {:?}",
            String::from_utf8_lossy(&startup)
        );
        // The status draw must leave the content region reserved: a
        // full-screen region reset (ESC[1;24r) after the draw lets the
        // next pane scroll carry the status row away (the manual-pass
        // scroll-away bug). Only rows-1 regions may appear.
        assert!(
            !startup.windows(9).any(|w| w == b"\x1b[1;24r"),
            "the status draw must not reset the scroll region to the full \
             screen: {:?}",
            String::from_utf8_lossy(&startup)
        );
    } else {
        let startup = wait_for_output(&host, b"\x1b[24;1H", Duration::from_secs(10));
        assert!(
            startup.windows(7).any(|w| w == b"\x1b[24;1H"),
            "attach must draw the status row (CUP to row 24): {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
        );
    }
    let _ = client.send(&format!("send-keys -t {pane} -l x Enter"));
    expect_output(&host, b"x", Duration::from_secs(5), "typed byte echoed");

    host.to_child.write_all(&[0x02, b'd']).expect("prefix d");
    host.to_child.flush().ok();
    let code = child_exit(&mut host, Duration::from_secs(10));
    assert_eq!(code, Some(0));
    // Drain what remains: the final reset must be present.
    let mut tail = Vec::new();
    while let Ok(bytes) = host.output_rx.try_recv() {
        tail.extend_from_slice(&bytes);
    }
    // The last things the client writes before exit are the region restore
    // and the final screen clear (no alt screen to leave — a pane app's
    // own 1049 pair would pop the client's). (crossterm's raw-mode disable
    // is a termios call, not bytes.) On Windows ConPTY likewise absorbs
    // ESC[r and erase-display, so the restore is asserted through the
    // client's exit code there (already checked above) — the bytes are
    // pinned on unix.
    if cfg!(unix) {
        assert!(
            tail.windows(3).any(|w| w == b"\x1b[r"),
            "detach must restore the scroll region (ESC[r): {tail:?}"
        );
        assert!(
            tail.windows(4).any(|w| w == b"\x1b[2J"),
            "detach must clear the screen (no alt screen to leave): {tail:?}"
        );
    }
}

/// The restart-reattach contract (2026-10-28 garble report): a pane app's
/// DECSTBM and the other modes its passthrough bytes set on the host
/// outlive the daemon that pane ran under, and `ESC[2J` does not touch a
/// scroll region. Everything the reconnect then draws — the seed replay,
/// the status row, the renderer's frames — scrolled or wrapped inside the
/// stale region, sized for the pre-restart screen. Every entry path must
/// reset the borrowed state before the clear, in both modes.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn attach_entry_resets_stale_host_terminal_state() {
    for mode in ["passthrough", "render"] {
        let (fixture, _daemon, _client) = fixture_with_session("stale-region");
        let (mut host, stderr) = match mode {
            "passthrough" => spawn_attach(&fixture, &[]),
            _ => spawn_attach_render(&fixture, &[]),
        };
        let startup = wait_for_output(&host, b"\x1b[2J", Duration::from_secs(10));
        let reset: &[u8] = b"\x1b[r\x1b[?6l\x1b[0m\x1b[?25h\x1b[?7h";
        assert!(
            startup.windows(reset.len()).any(|w| w == reset),
            "{mode} attach must reset the host terminal state (DECSTBM, origin \
             mode, SGR, cursor visibility, autowrap) at entry: {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
        );
        assert!(
            startup.windows(reset.len()).position(|w| w == reset)
                < startup.windows(4).position(|w| w == b"\x1b[2J"),
            "{mode} attach must reset before the clear: {:?}",
            String::from_utf8_lossy(&startup)
        );
        host.killer.kill().ok();
    }
}

/// Acceptance criterion 3: prefix pane switching issues a daemon-side
/// select and a resync redraw. Split the pane, then prefix o; the client's
/// output shows the NEW pane's content (the resync of the other pane).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn prefix_o_switches_panes_and_resyncs() {
    let (fixture, _daemon, mut client) = fixture_with_session("switch");
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
    assert!(
        pane_b.starts_with('%'),
        "split returned a pane id: {pane_b:?}"
    );

    // Distinct markers on each pane.
    client
        .send(&format!("send-keys -t {pane_b} -l 'echo PANE-B-MARKER'"))
        .expect("marker B");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("press Enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane_b}"),
        echoed("PANE-B-MARKER"),
        Duration::from_secs(10),
        "PANE-B-MARKER executed on the pane screen",
    );

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane_a]);
    // Attach to pane A: settling output arrives; B's marker must NOT be
    // the reason we proceed (B is a different pane).
    expect_output(
        &host,
        b"\x1b[1;23r",
        Duration::from_secs(10),
        "passthrough client up (status row reserved)",
    );

    // Prefix o cycles to pane B; the resync redraw carries B's marker.
    host.to_child.write_all(&[0x02, b'o']).expect("prefix o");
    host.to_child.flush().ok();
    let got = expect_output(
        &host,
        b"PANE-B-MARKER",
        common::CEILING,
        "prefix o resyncs to pane B",
    );
    assert!(
        got.windows(b"PANE-B-MARKER".len())
            .any(|w| w == b"PANE-B-MARKER"),
        "prefix o must resync to the other pane's screen. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    // The daemon-side select landed: the window's active pane is B now.
    let active = client.send("list-panes").expect("global roster");
    let _ = active; // roster carries no marker globally; the resync above is the proof
    host.killer.kill().ok();
}

/// prefix %: the chord splits the FOCUSED pane and the client lands on
/// the fresh split — the new pane's distinct marker reaches the client
/// through the landing resync, and the pane it split from stays alive.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn split_chord_lands_the_client_on_the_new_pane() {
    let (fixture, _daemon, mut client) = fixture_with_session("chordsplit");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane_a]);
    expect_output(
        &host,
        b"\x1b[1;23r",
        Duration::from_secs(10),
        "passthrough client up (status row reserved)",
    );

    // Prefix % splits right; the client lands on the new pane. The pane
    // starts a fresh shell, whose prompt is the landing evidence — but
    // the pane it came from shows one too, so drive the new pane
    // directly: typing AFTER the chord reaches the fresh pane only.
    host.to_child.write_all(&[0x02, b'%']).expect("prefix %");
    host.to_child.flush().ok();
    common::wait_daemon(
        fixture.socket(),
        "list-panes",
        |reply| common::pane_ids(reply).len() == 2,
        Duration::from_secs(10),
        "the split chord created a second pane",
    );
    // input spacing: the client's landing follows the daemon's split reply
    std::thread::sleep(Duration::from_millis(200));
    host.to_child
        .write_all(b"echo SPLIT-LANDED-MARKER\n")
        .expect("type into the new pane");
    host.to_child.flush().ok();

    let got = wait_for_output(&host, b"SPLIT-LANDED-MARKER", Duration::from_secs(15));
    assert!(
        got.windows(b"SPLIT-LANDED-MARKER".len())
            .any(|w| w == b"SPLIT-LANDED-MARKER"),
        "the split chord must land the pump on the fresh pane (its stdin \
         carries the marker echo). stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    // Two panes now exist under the session.
    let roster = client.send("list-panes").expect("roster");
    assert!(
        roster.len() >= 2,
        "the split created a second pane: {roster:?}"
    );
    host.killer.kill().ok();
}

/// prefix x: the chord kills the focused pane and the pump follows the
/// window's survivor — the survivor's marker resyncs in.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn kill_chord_lands_the_client_on_the_survivor() {
    let (fixture, _daemon, mut client) = fixture_with_session("chordkill");
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
        .send(&format!("send-keys -t {pane_b} -l 'echo SURVIVOR-MARKER'"))
        .expect("marker B");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("Enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane_b}"),
        echoed("SURVIVOR-MARKER"),
        Duration::from_secs(10),
        "SURVIVOR-MARKER executed on the pane screen",
    );

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane_a]);
    expect_output(
        &host,
        b"\x1b[1;23r",
        Duration::from_secs(10),
        "passthrough client up (status row reserved)",
    );

    // Prefix x kills pane A; the pump must land on the survivor B and
    // resync its marker in.
    host.to_child.write_all(&[0x02, b'x']).expect("prefix x");
    host.to_child.flush().ok();

    let got = wait_for_output(&host, b"SURVIVOR-MARKER", Duration::from_secs(15));
    assert!(
        got.windows(b"SURVIVOR-MARKER".len())
            .any(|w| w == b"SURVIVOR-MARKER"),
        "prefix x must land the pump on the surviving pane. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    let roster = client.send("list-panes").expect("roster");
    assert!(
        !roster.iter().any(|l| l.starts_with(&pane_a)),
        "the focused pane is gone from the roster: {roster:?}"
    );
    host.killer.kill().ok();
}

/// The client exits 0 when the daemon shuts down (%exit), restoring the
/// terminal.
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn daemon_exit_ends_the_client_cleanly() {
    let (fixture, _daemon, mut client) = fixture_with_session("exit");
    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    // Settle on the status draw, so %exit is the only pending signal. On
    // Windows the settle keys on the re-encoded status-row draw (ConPTY
    // absorbs DECSTBM — see detach_restores_the_terminal_region).
    let settle_needle: &[u8] = if cfg!(unix) {
        b"\x1b[1;23r"
    } else {
        b"\x1b[24;1H"
    };
    let startup = wait_for_output(&host, settle_needle, Duration::from_secs(10));
    let _ = startup;
    client.send_checked("kill-server").expect("kill-server");
    let code = child_exit(&mut host, Duration::from_secs(10));
    assert_eq!(
        code,
        Some(0),
        "a daemon shutdown must end attach with exit 0. stderr: {}",
        stderr.lock().unwrap()
    );
}

/// Held-dead pane: typing into a dead pane must not flood the screen — the
/// client drops the bytes (the daemon answers every send-keys to a dead
/// pane with the NotStartedError `%error`, and the bytes would echo into
/// the pane's frozen screen either way), the status row names the respawn
/// chord, and prefix `r` respawns the pane and resumes forwarding.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn dead_pane_takes_no_typing_and_prefix_r_respawns() {
    let (fixture, _daemon, mut client) = fixture_with_session("deadpane");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // Kill the pane's shell: it is held dead (remain-on-exit) before the
    // client attaches, the deterministic shape (the %pane-exited-mid-view
    // path sets the same `exited` flag — pinned at the router level).
    client
        .send(&format!("send-keys -t {pane} -l 'exit 0'"))
        .expect("exit");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let info = client
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info")
            .join("");
        assert!(Instant::now() < deadline, "the pane never died: {info}");
        if info.contains("exited=") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane]);
    // Settle on the status draw; it must carry the respawn hint (the raw
    // status text passes through the pty untouched; the em dash is its
    // UTF-8 spelling \xe2\x80\x94).
    let startup = wait_for_output(&host, b"respawns)", Duration::from_secs(10));
    assert!(
        startup.windows(14).any(|w| w == b"(exited 0 \xe2\x80\x94 "),
        "the status row must show the held-dead cue: {:?}\nstderr: {}",
        String::from_utf8_lossy(&startup),
        stderr.lock().unwrap()
    );
    assert!(
        startup.windows(15).any(|w| w == b"C-b r respawns)"),
        "the status row must name the respawn chord: {:?}",
        String::from_utf8_lossy(&startup)
    );

    // Type a burst: the client must drop the bytes. The only output that
    // may follow is the settle-redraw status row (the one-shot redraw the
    // client owes after the startup burst — same bytes as the first draw,
    // no pane content), so assert no ECHO of the typed bytes and no
    // send-keys error text renders.
    host.to_child
        .write_all(b"garbage typing \x1b[A x")
        .expect("type");
    host.to_child.flush().ok();
    // negative check: nothing positive to wait on, so the window is fixed
    std::thread::sleep(Duration::from_millis(500));
    let mut after = startup.clone();
    while let Ok(bytes) = host.output_rx.try_recv() {
        after.extend_from_slice(&bytes);
    }
    let extra = &after[startup.len()..];
    assert!(
        !extra.windows(14).any(|w| w == b"garbage typing"),
        "typed bytes must never echo to a held-dead pane's screen: {:?}",
        String::from_utf8_lossy(&after)
    );
    assert!(
        !extra.windows(7).any(|w| w == b"par-mux"),
        "no error text may render over the dead pane: {:?}",
        String::from_utf8_lossy(&after)
    );
    // Only status-row redraws may appear after typing: every extra byte
    // must sit inside a status-draw block — the ESC7..ESC8 wrap plus the
    // absolute tracked-cell CUP (ESC[<r>;<c>H) that closes each draw (the
    // settle/redraw discipline) — no pane output, no flood. Byte-COUNT
    // budgets flake on slow machines where more redraws land in the window.
    let mut residue: Vec<u8> = Vec::new();
    let mut inside_draw = false;
    let mut awaiting_placement_h = false;
    let mut i = 0;
    while i < extra.len() {
        if extra[i..].starts_with(b"\x1b7") {
            inside_draw = true;
            i += 2;
        } else if extra[i..].starts_with(b"\x1b8") {
            // The wrap's restore is immediately followed by the draw's
            // absolute tracked-cell CUP — still draw bytes until its H.
            awaiting_placement_h = true;
            inside_draw = true;
            i += 2;
        } else if awaiting_placement_h {
            if extra[i] == b'H' {
                awaiting_placement_h = false;
                inside_draw = false;
            }
            i += 1;
        } else {
            if !inside_draw {
                residue.push(extra[i]);
            }
            i += 1;
        }
    }
    assert!(
        residue.iter().all(|b| b.is_ascii_whitespace()),
        "a held-dead pane must take no stdin bytes: only status-row redraws \
         (ESC7..ESC8 blocks) may follow, found {} bytes of other output: {:?}. \
         stderr: {}",
        residue.len(),
        String::from_utf8_lossy(&residue),
        stderr.lock().unwrap()
    );

    // Prefix r respawns the pane and resumes forwarding: the client wipes
    // the frozen dead-pane screen (clear + home) before the fresh replay,
    // then a typed echo round-trips through the revived pane.
    host.to_child.write_all(&[0x02, b'r']).expect("prefix r");
    host.to_child.flush().ok();
    let wiped = wait_for_output(&host, b"\x1b[2J\x1b[H", Duration::from_secs(10));
    assert!(
        wiped.windows(7).any(|w| w == b"\x1b[2J\x1b[H"),
        "respawn must clear the frozen screen before the resync replay. \
         stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&wiped)
    );
    host.to_child
        .write_all(b"echo RESPAWNED-ROUNDTRIP\r")
        .expect("type after respawn");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"RESPAWNED-ROUNDTRIP", Duration::from_secs(15));
    assert!(
        got.windows(b"RESPAWNED-ROUNDTRIP".len())
            .any(|w| w == b"RESPAWNED-ROUNDTRIP"),
        "prefix r must respawn the pane and resume forwarding. stderr: {}\n\
         bytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    host.killer.kill().ok();
}

/// Regression (the manual-pass ESC7/ESC8 race): pane output flowing WHILE a
/// status draw ran made the ESC8 restore land one line off, and every later
/// output painted over the wrong row (observed on Terminal.app: a new
/// prompt overwriting the middle of the previous line). The status draw now
/// CLOSES with an absolute CUP at the shadow emulator's tracked cell, so a
/// scroll landing between the draw and the placement cannot make the final
/// position wrong. The PTY-level proof: flood the pane with scrolling
/// output while the client's status draws land (the settle-redraw cadence
/// draws during the flood), assert every draw the client emitted closes
/// with its absolute tracked-cell CUP (a bare-ESC8 close would be the raced
/// shape), then drive an echo through the pane — it must round-trip. The
/// exact tracked cell is pinned headless by
/// `status_draw_places_the_cursor_at_the_tracked_cell` (`src/mux/attach/mod.rs`).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn status_draw_tracks_the_cursor_under_output_flood() {
    let (fixture, _daemon, mut client) = fixture_with_session("cursorrace");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    // Flood: numbered lines scrolled fast enough that scrolls land while
    // the client's status redraws are in flight.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'for i in $(seq 1 60); do echo FLOOD-LINE-$i; done'"
        ))
        .expect("flood");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane}"),
        |s| s.contains("FLOOD-LINE-60"),
        Duration::from_secs(10),
        "the FLOOD-LINE loop ran to its last line",
    );

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane]);
    let startup = wait_for_output(&host, b"FLOOD-LINE-60", Duration::from_secs(15));
    assert!(
        !startup.is_empty(),
        "the flood must reach the host. stderr: {}",
        stderr.lock().unwrap()
    );
    // The settle-redraw draws AFTER the flood's last line; give it time to
    // flow, then join what followed so the scan sees whole draw blocks.
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(700));
    let mut b = startup;
    while let Ok(bytes) = host.output_rx.try_recv() {
        b.extend_from_slice(&bytes);
    }

    // Every draw closes with its absolute tracked-cell CUP: scan the whole
    // capture for draw blocks and require the placement CUP right after
    // each restore. (The exact cell at draw time is the emulator's
    // business — the headless suite pins it; here the structural shape is
    // the contract.)
    let mut draws = 0usize;
    let mut i = 0usize;
    while i + 1 < b.len() {
        if b[i] == 0x1b && b[i + 1] == 0x37 {
            // ESC7: find the ESC8 close, then require ESC[ ... H after it.
            let mut j = i + 2;
            while j + 1 < b.len() && !(b[j] == 0x1b && b[j + 1] == 0x38) {
                j += 1;
            }
            assert!(
                j + 1 < b.len(),
                "a draw never closed (no ESC8): bytes[{i}..]: {:?}",
                String::from_utf8_lossy(&b[i..(i + 80).min(b.len())])
            );
            j += 2;
            assert!(
                j + 1 < b.len() && b[j] == 0x1b && b[j + 1] == b'[',
                "a draw closed with a bare ESC8 (the raced shape): \
                 bytes[{j}..]: {:?}",
                String::from_utf8_lossy(&b[j..(j + 80).min(b.len())])
            );
            // The placement CUP's params end at 'H'.
            while j < b.len() && b[j] != b'H' {
                j += 1;
            }
            assert!(j < b.len(), "the placement CUP never closed");
            draws += 1;
            i = j + 1;
        } else {
            i += 1;
        }
    }
    assert!(
        draws > 0,
        "at least one status draw must appear in the flood capture. stderr: {}",
        stderr.lock().unwrap()
    );
    // The placement stays inside the content region: a cursor parked on
    // the status row (24) sits below the 1;23 scroll region, where later
    // output overwrites one line instead of scrolling (card 01a11d62).
    let rows = status_draw_placement_rows(&b);
    assert!(
        rows.iter().all(|&r| (1..=23).contains(&r)),
        "status draws must park the cursor inside rows 1..=23: {rows:?}"
    );

    // With draws landing under live scrolling output, an echo typed now
    // must round-trip intact through the pane.
    host.to_child
        .write_all(b"echo RACED-ROUNDTRIP\r")
        .expect("type echo");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"RACED-ROUNDTRIP", Duration::from_secs(15));
    assert!(
        got.windows(15).any(|w| w == b"RACED-ROUNDTRIP"),
        "an echo must round-trip intact after draws under a flood. stderr: {}\n\
         bytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    host.killer.kill().ok();
}

/// The `-t` target forms resolve: a pane id attaches to that pane (its
/// content arrives), and an unknown target is a clean failure with exit 1
/// and a message, not a crash.
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn target_resolution_and_clean_failure() {
    let (fixture, _daemon, mut client) = fixture_with_session("targets");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("send-keys -t {pane} -l 'echo TARGETED-MARKER'"))
        .expect("marker");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("press Enter");
    // The echo lands on the pane screen BEFORE attach starts: the
    // assertion is about the resync replay, not a race with %output.
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane}"),
        echoed("TARGETED-MARKER"),
        Duration::from_secs(10),
        "TARGETED-MARKER executed on the pane screen",
    );

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane]);
    let got = wait_for_output(&host, b"TARGETED-MARKER", Duration::from_secs(15));
    assert!(
        got.windows(b"TARGETED-MARKER".len())
            .any(|w| w == b"TARGETED-MARKER"),
        "attaching with -t <pane> must resync that pane. stderr: {}; bytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    host.killer.kill().ok();

    // Unknown target: exit 1, message on stderr, no hang.
    let output = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("attach")
        .arg("--socket")
        .arg(fixture.socket())
        .arg("-t")
        .arg("%999")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run attach with a bad target");
    assert_eq!(output.status.code(), Some(1));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        err.contains("no such target"),
        "the failure names the target problem: {err}"
    );
}

/// `[client] session-picker` absent, off, or on with stdio that is not a
/// terminal (stdin null, stdout and stderr piped — a script or agent
/// shell): a target-less attach against two sessions prints exactly the
/// refusal on stderr, exits 1, and never draws the picker. Both modes.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn session_picker_off_or_without_a_terminal_keeps_the_refusal_bytes() {
    let (fixture, _daemon, mut client) = fixture_with_two_sessions("pickrefuse");
    let expected = expected_refusal(&mut client);
    for mode in ["render", "passthrough"] {
        for config in [
            "[client]\n",
            "[client]\nsession-picker = false\n",
            "[client]\nsession-picker = true\n",
        ] {
            std::fs::write(fixture.client_config_path(), config).expect("write client config");
            let output = Command::new(env!("CARGO_BIN_EXE_par-mux"))
                .args(["attach", "--mode", mode, "--socket"])
                .arg(fixture.socket())
                .env("PAR_MUX_CONFIG", fixture.client_config_path())
                .stdin(std::process::Stdio::null())
                .output()
                .expect("run attach");
            assert_eq!(output.status.code(), Some(1), "{mode} {config:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stderr),
                expected,
                "{mode} {config:?}: the refusal, byte for byte"
            );
            assert!(
                !String::from_utf8_lossy(&output.stdout).contains("choose one"),
                "{mode} {config:?}: no picker on a non-terminal stdout"
            );
        }
    }
}

/// `[client] session-picker = true` with ONE stdio stream a terminal:
/// stdin redirected (stdout on the PTY), then stdout redirected to a file
/// (stdin on the PTY). Either way the picker stays off — no footer is
/// drawn, nothing waits for a key — and the refusal prints, exit 1.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn session_picker_needs_both_stdin_and_stdout_on_a_terminal() {
    let (fixture, _daemon, _client) = fixture_with_two_sessions("pickhalf");
    std::fs::write(
        fixture.client_config_path(),
        "[client]\nsession-picker = true\n",
    )
    .expect("write client config");
    let dir = tempfile::tempdir().expect("tempdir");
    let out_file = dir.path().join("stdout.txt");
    let attach = format!(
        "exec '{}' attach --mode render --socket '{}'",
        env!("CARGO_BIN_EXE_par-mux"),
        fixture.socket().display()
    );
    for (what, script) in [
        ("stdin redirected", format!("{attach} </dev/null")),
        (
            "stdout redirected",
            format!("{attach} >'{}'", out_file.display()),
        ),
    ] {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", &script]);
        cmd.env("PAR_MUX_CONFIG", fixture.client_config_path());
        let mut child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let (tx, rx) = channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let end = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = child.try_wait().expect("try_wait") {
                break status;
            }
            assert!(
                Instant::now() < end,
                "{what}: the client must not wait in a picker"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut seen = Vec::new();
        while let Ok(bytes) = rx.recv_timeout(Duration::from_millis(300)) {
            seen.extend_from_slice(&bytes);
        }
        let mut text = String::from_utf8_lossy(&seen).into_owned();
        if what == "stdout redirected" {
            text.push_str(&std::fs::read_to_string(&out_file).unwrap_or_default());
        }
        assert_eq!(status.exit_code(), 1, "{what}: {text}");
        assert!(
            text.contains("multiple sessions exist")
                && text.contains("par-mux attach -t <session>"),
            "{what}: the refusal prints: {text:?}"
        );
        assert!(
            !text.contains("enter attach"),
            "{what}: no picker: {text:?}"
        );
    }
}

#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn session_picker_pick_attaches_the_chosen_session_render() {
    session_picker_pick_attaches("render", "pickrender", b"\x1b[?1049h");
}

#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn session_picker_pick_attaches_the_chosen_session_passthrough() {
    session_picker_pick_attaches("passthrough", "pickpass", b"\x1b[1;23r");
}

/// Aborting the picker (`q`) prints the refusal so the `-t` syntax stays
/// on screen, exits 1, and leaves the terminal out of raw mode with the
/// cursor shown.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn session_picker_abort_prints_the_refusal_and_exits_nonzero() {
    let (fixture, _daemon, _client) = fixture_with_two_sessions("pickquit");
    let (mut host, _stderr) = spawn_attach_with_config(
        &fixture,
        &[],
        "render",
        "[client]\nsidebar-on-launch = false\nsession-picker = true\n",
    );
    expect_output(
        &host,
        b"enter attach",
        Duration::from_secs(10),
        "the picker's footer",
    );
    host.to_child.write_all(b"q").expect("q");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"<server> -t <session>", Duration::from_secs(10));
    assert_eq!(
        child_exit(&mut host, Duration::from_secs(10)),
        Some(1),
        "an aborted pick exits 1: {}",
        String::from_utf8_lossy(&got)
    );
    let text = String::from_utf8_lossy(&got);
    assert!(
        text.contains("multiple sessions exist") && text.contains("par-mux attach -t <session>"),
        "the refusal follows the abort: {text}"
    );
    assert!(
        text.contains("\x1b[?25h"),
        "the cursor is shown again: {text:?}"
    );
    assert!(
        !text.contains("\x1b[?1049h"),
        "no attach screen was entered: {text:?}"
    );
}

/// The passthrough half of the default-tree create: same daemon shape,
/// same rule, passthrough's byte-pump seed.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn passthrough_without_target_creates_the_default_session() {
    let fixture = MuxFixture::new("passdefault");
    let _daemon = common::spawn_daemon_auto_remove(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");

    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    expect_output(
        &host,
        b"\x1b[1;23r",
        Duration::from_secs(10),
        "passthrough client up — the create path attached",
    );
    // The create lands during the seed, after the status-row byte the
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

/// The passthrough status line gains the workspaces segment: workspace
/// names in id order, the daemon's active one bracketed.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn passthrough_status_line_carries_the_workspaces_segment() {
    let (fixture, _daemon, mut client) = fixture_with_session("wspass");
    client.send("new-workspace -n lab").expect("new-workspace");
    client
        .send("new-session -s work -t lab")
        .expect("session in lab");
    // settle: the fresh panes' shells start before the client attaches
    std::thread::sleep(Duration::from_millis(300));

    // A pinned target: the target-less rule refuses when several sessions
    // exist, and this test's subject is the status line, not the pick.
    let (mut host, stderr) = spawn_attach(&fixture, &["-t", "$0"]);
    let got = wait_for_output(&host, b"[lab]", Duration::from_secs(15));
    assert!(
        got.windows(b"[lab]".len()).any(|w| w == b"[lab]"),
        "the passthrough status line must carry the workspaces segment \
         with the active workspace bracketed. bytes: {:?}\nstderr: {}",
        String::from_utf8_lossy(&got),
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// prefix W (passthrough): the chord sends `select-workspace` and lands
/// the pump on the target workspace's pane — typing after the chord
/// reaches the new workspace's pane only.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn passthrough_workspace_chord_selects_and_lands() {
    let (fixture, _daemon, mut client) = fixture_with_session("wspasschord");
    client.send("new-workspace -n lab").expect("new-workspace");
    client
        .send("new-session -s work -t lab")
        .expect("session in lab");
    client
        .send("select-workspace -t main")
        .expect("select main");
    // settle: the fresh panes' shells start before the client attaches
    std::thread::sleep(Duration::from_millis(300));

    // A pinned target: the target-less rule refuses when several sessions
    // exist, and this test's subject is the workspace chord, not the pick.
    let (mut host, stderr) = spawn_attach(&fixture, &["-t", "$0"]);
    expect_output(
        &host,
        b"\x1b[1;23r",
        Duration::from_secs(10),
        "passthrough client up (status row reserved)",
    );

    // Prefix W: next workspace (main -> lab). Typing AFTER the chord
    // reaches the lab session's pane only — the landing evidence.
    host.to_child.write_all(&[0x02, b'W']).expect("prefix W");
    host.to_child.flush().ok();
    common::wait_daemon(
        fixture.socket(),
        "list-workspaces",
        |reply| {
            reply
                .lines()
                .any(|l| l.starts_with("+1:") && l.contains(" active"))
        },
        Duration::from_secs(10),
        "the chord must move the daemon's active-workspace marker",
    );
    // input spacing: the client's landing follows the daemon's reply
    std::thread::sleep(Duration::from_millis(200));
    host.to_child
        .write_all(b"echo WS-LANDED-MARKER\n")
        .expect("type into the landed pane");
    host.to_child.flush().ok();

    let got = wait_for_output(&host, b"WS-LANDED-MARKER", Duration::from_secs(15));
    assert!(
        got.windows(b"WS-LANDED-MARKER".len())
            .any(|w| w == b"WS-LANDED-MARKER"),
        "prefix W must land the pump on the new workspace's pane. \
         bytes: {:?}\nstderr: {}",
        String::from_utf8_lossy(&got),
        stderr.lock().unwrap()
    );
    let workspaces = client.send("list-workspaces").expect("list-workspaces");
    assert!(
        workspaces
            .iter()
            .any(|l| l.starts_with("+1:") && l.contains(" active")),
        "the chord must move the daemon's active-workspace marker: {workspaces:?}"
    );
    host.killer.kill().ok();
}

/// ARC-124: the passthrough half of card 01a11bd1. A passthrough session
/// switch (prefix `(`) must land through `switch-client`, so the
/// daemon's persisted displayed-session pointer follows; after a restart
/// the pointer still names the session passthrough landed on, and an
/// attach pinned to it lands there. (A bare `select-window` on a
/// background session never moved the pointer, so the restored daemon
/// still named the old session.)
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn passthrough_session_landing_survives_a_restart_for_the_next_attach() {
    let (fixture, mut daemon, client) = fixture_two_session_workspace("passrestart");
    drop(client);
    let (mut a, a_err) = spawn_attach(&fixture, &["-t", "$2"]);
    expect_output(
        &a,
        b"SOLO-MARK",
        Duration::from_secs(10),
        "SOLO-MARK on the host screen",
    );
    a.to_child.write_all(&[0x02, b'(']).expect("prefix (");
    a.to_child.flush().ok();
    let landed = expect_output(&a, b"MULTI-", common::CEILING, "the multi-pane session");
    assert!(
        landed.windows(b"MULTI-".len()).any(|w| w == b"MULTI-"),
        "passthrough lands on the multi-pane session. stderr: {}",
        a_err.lock().unwrap()
    );
    a.to_child.write_all(&[0x02, b'd']).expect("prefix d");
    a.to_child.flush().ok();
    assert_eq!(child_exit(&mut a, Duration::from_secs(10)), Some(0));

    common::sigterm_clean(&mut daemon);
    let _restarted = common::spawn_daemon(&fixture);
    wait_listening(fixture.socket());

    let mut query = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
    let displayed = query
        .send("switch-client")
        .expect("displayed-session query");
    assert_eq!(
        displayed.join("").trim(),
        "$1",
        "the restored daemon's displayed session is the passthrough landing"
    );
    let (mut c, c_err) = spawn_attach_render(&fixture, &["-t", "$1"]);
    let mut c_all = Vec::new();
    let shown = screen_until(&c, &mut c_all, Duration::from_secs(10), shows_multi);
    assert!(
        shows_multi(&shown) && !shown.contains("SOLO-MARK"),
        "an attach pinned to the persisted landing session shows its \
         window. screen:\n{shown}\nstderr: {}",
        c_err.lock().unwrap()
    );
    c.killer.kill().ok();
}

/// Card 01a11d62 (passthrough drops the rich demo's colored paragraphs):
/// a passthrough client attached to a pane that floods rich-like output
/// must forward EVERY pane byte to the host. Both byte streams are
/// recorded — the daemon side through an observer control client's
/// `%output` (the PTY reader's exact reads), the host side through the
/// attach client's PTY master — and must be byte-identical across the
/// marked payload region. A cargo-build-like scrolling flood runs first,
/// the shape Paul's pane had just before the demo.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn passthrough_forwards_every_byte_of_a_rich_like_flood() {
    passthrough_flood_round_trip(HostShape::Fast);
}

/// The flood against a slow host terminal (small, paced reads — a host
/// painting truecolor glyphs drains far slower than the pane writes):
/// backpressure must reach the pane, never cost bytes.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn passthrough_forwards_every_byte_to_a_slow_host() {
    passthrough_flood_round_trip(HostShape::Slow);
}

/// Card 01a11d62, the HOST's view (Paul's check): after a scroll flood,
/// pane output must land in the host terminal's screen or scrollback.
/// The pane PTY is the host grid minus the status row; a shadow emulator
/// one row taller tracked a scrolled pane's cursor onto the status row,
/// every status draw parked the host cursor there — below the `1;23`
/// scroll region, where line feeds never scroll — and every later line
/// overwrote that one row, never reaching the host's scrollback. The
/// bytes all arrived (the byte-exact tests above prove the transport);
/// the host could not keep them.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn passthrough_output_after_a_scroll_reaches_the_host_scrollback() {
    let (fixture, _daemon, mut client) = fixture_with_session("ptscroll");
    let pane = session_first_pane(&mut client, "$0");
    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane]);
    expect_output(
        &host,
        b"\x1b[1;23r",
        Duration::from_secs(10),
        "passthrough status-row reserve",
    );

    // A scroll flood, a pause long enough for status draws to land with
    // the pane scrolled, then numbered paragraphs (the `%s` split keeps
    // the shell's echo of the typed line from matching the end marker).
    client
        .send(&format!(
            "send-keys -t {pane} -l 'seq 1 200; sleep 1; \
             for i in $(seq 1 40); do printf \"PARA-%03d lorem ipsum\\n\" $i; done; \
             sleep 1; echo PARAS%sDONE -'"
        ))
        .expect("send-keys");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("Enter");
    let got = wait_for_output(&host, b"PARAS-DONE", Duration::from_secs(30));
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(700));
    let mut host_bytes = got;
    while let Ok(bytes) = host.output_rx.try_recv() {
        host_bytes.extend_from_slice(&bytes);
    }

    let rows = status_draw_placement_rows(&host_bytes);
    assert!(!rows.is_empty(), "status draws must appear in the capture");
    assert!(
        rows.iter().all(|&r| (1..=23).contains(&r)),
        "every status draw must park the host cursor inside the content \
         region (rows 1..=23), never on the status row: {rows:?}"
    );

    // Replay the host's bytes into a host-sized terminal and read back
    // what the user would see: screen plus scrollback.
    let mut host_term = par_term_emu_core::terminal::Terminal::with_scrollback(80, 24, 10_000);
    host_term.process(&host_bytes);
    let grid = host_term.grid();
    let mut seen: Vec<String> = (0..grid.scrollback_len())
        .filter_map(|i| grid.scrollback_line(i))
        .map(|cells| cells.iter().map(|c| c.c).collect())
        .collect();
    seen.extend((0..24).map(|r| grid.row_text(r)));
    let missing: Vec<u32> = (1..=40)
        .filter(|i| {
            let needle = format!("PARA-{i:03} lorem ipsum");
            !seen.iter().any(|line| line.contains(&needle))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "paragraphs missing from the host's screen AND scrollback: {missing:?}\n\
         stderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Card 01a1216d4e3e — the post-reply race measurement. The output-race
/// rule (`drain_around_layout`) only catches `%output` already queued
/// when the drain runs; bytes the daemon pushes AFTER the size-report
/// reply feed at the new size through the pump. This test measures
/// whether real resizes under continuous output hit that window: a pane
/// emits 90-char lines (which wrap differently at 80 and 100 columns)
/// while the host resizes back and forth; afterwards the client's
/// painted pane rows must equal the daemon's own grid row-for-row. A
/// line applied at the old size on the daemon but fed at the new size
/// in the client wraps differently and fails the comparison.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn resize_storm_post_reply_output_matches_daemon_ground_truth() {
    let (fixture, _daemon, mut client) = fixture_with_session("race");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    // The emitter: 300 lines of exactly 90 visible chars (4-char marker
    // + 85 padded digits), one every ~5ms, then the end anchor. At 100
    // cols each line is one row; at 80 cols it wraps — the divergence
    // signal an old-size-fed byte would leave. The anchor rides shell
    // EXPANSION — the echoed command carries the literal $(date …)
    // spelling, only the executed OUTPUT holds ANCHOR-<epoch>-DONE, so
    // the anchor can never fire on the echo (a plain-needle anchor fired
    // on the echoed command line under load and compared mid-stream).
    client
        .send(&format!(
            "send-keys -t {pane} -l 'for i in $(seq 1 300); do printf \"R%03d-%085d\\n\" $i $i; \
             sleep 0.005; done; echo \"ANCHOR-$(date +%s)-DONE\"'"
        ))
        .expect("keys emitter");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter emitter");

    let (host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    // The emitter is already running: settle on its live output before
    // the storm, so the resizes land under genuine output pressure. Any
    // emitted line will do, not the first ones: under load the client
    // attached after `R099` had scrolled away (its seed showed `R265-…`).
    // The zero padding exists only in executed output, never in the
    // echoed `%085d` command line.
    expect_output(
        &host,
        b"-0000000",
        common::CEILING,
        "emitter output on the host screen",
    );

    // The storm: alternate the host grid 12 times. Every resize rides
    // the SIGWINCH -> size-report -> drain path the card names.
    for i in 0..12 {
        let (cols, rows) = if i % 2 == 0 { (100, 30) } else { (80, 24) };
        host.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("storm resize");
        std::thread::sleep(Duration::from_millis(100));
    }
    // End at the wide grid so the comparison geometry is deterministic.
    host.master
        .resize(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("final resize");

    // The anchor: the emitter's last OUTPUT seen by BOTH sides (the
    // expanded ANCHOR-<digits>-DONE, never the echoed literal form).
    // Past it only the fresh prompt follows; give that a beat to land
    // everywhere. Every host byte is accumulated — the anchor may paint
    // in one poll and the comparison needs the full stream anyway.
    let anchor_done = |text: &str| -> bool {
        let mut rest = text;
        while let Some(at) = rest.find("ANCHOR-") {
            let tail = &rest[at + "ANCHOR-".len()..];
            let digits = tail.bytes().take_while(|b| b.is_ascii_digit()).count();
            if digits >= 8 && tail[digits..].starts_with("-DONE") {
                return true;
            }
            rest = &rest[at + 1..];
        }
        false
    };
    let mut all_host: Vec<u8> = Vec::new();
    // 60s, not 30s: under load the emitter's 300 shell sleeps overshoot
    // past the old bound while the test remains healthy (card
    // 01a13c1464); nextest's ci profile now grants this test 30s x8, so
    // the in-test bound sits under the harness kill instead of racing it.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        while let Ok(bytes) = host.output_rx.try_recv() {
            all_host.extend_from_slice(&bytes);
        }
        let capture = client
            .send(&format!("capture-pane -t {pane} -e"))
            .expect("capture");
        // The render client paints DIFFS (changed cells only), so the
        // anchor line exists in the raw stream as fragments interleaved
        // with other rows' repaints — flattening the raw bytes garbles
        // the needle (a failing run showed 7 of 10 digits before -DONE).
        // Search the reconstructed SCREEN instead: the final geometry is
        // 100x30 and the client's refit to it is part of what this loop
        // waits out; a not-yet-refit screen simply fails the needle and
        // retries until the deadline.
        let (host_grid, _) = reconstructed_screen(&all_host, 30, 100);
        if anchor_done(&capture.join("\n")) && anchor_done(&host_grid.join("\n")) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "emitter never finished: capture={:?} stderr={} host_plain_tail={}",
            capture.join("\n"),
            stderr.lock().unwrap(),
            plain_text(&all_host[all_host.len().saturating_sub(4000)..])
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(800));

    // The daemon must have APPLIED the final size before its capture is
    // ground truth — under load the re-fit lags the resize (the existing
    // host-resize test's fitted wait). The single pane fills the 100x30
    // host's 100x28 content grid; its PTY is the interior inside the
    // default border ring: 98x26 (card 01a11c5b's geometry).
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let info = client
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info");
        if info.join(" ").contains("98x26") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the daemon never re-fit the pane to 98x26: {:?}",
            info
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // Ground truth: the daemon's own pane grid (one line per row, SGR
    // inline), stripped to plain text.
    let capture = client
        .send(&format!("capture-pane -t {pane} -e"))
        .expect("capture -e");
    let truth: Vec<String> = capture
        .iter()
        .map(|row| strip_sgr(row).trim_end().to_string())
        .collect();

    // The client's painted screen at the final 100x30 geometry: the pane
    // fills rows 1..=28 (tab strip at 0, status row at 29). Keep pulling
    // the stream until a quiet beat, then reconstruct.
    let mut quiet = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let mut got = 0;
        while let Ok(bytes) = host.output_rx.try_recv() {
            all_host.extend_from_slice(&bytes);
            got += bytes.len();
        }
        if got == 0 {
            quiet += 1;
            if quiet >= 3 {
                break;
            }
        } else {
            quiet = 0;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let (grid, _ever) = reconstructed_screen(&all_host, 30, 100);
    // The reconstructor mangles multibyte glyphs (one byte per cell), so
    // compare ASCII-graphic text only; chrome rows and prompt glyphs
    // normalize to empty and drop. Row ORDER is preserved on both sides.
    let norm = |rows: Vec<String>| -> Vec<String> {
        rows.into_iter()
            .map(|row| {
                row.chars()
                    .filter(|c| c.is_ascii_graphic() || *c == ' ')
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .filter(|row| !row.is_empty())
            .collect()
    };
    let painted = norm(
        grid[1..grid.len() - 1]
            .iter()
            .map(|row| row.trim_end().to_string())
            .collect::<Vec<_>>(),
    );
    let truth = norm(truth);
    assert_eq!(
        truth,
        painted,
        "the client's painted pane diverged from the daemon's grid — a \
         post-reply %output fed at the wrong size (card 01a1216d4e3e). \
         stderr: {}",
        stderr.lock().unwrap()
    );
}
