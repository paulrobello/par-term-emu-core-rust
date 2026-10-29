//! Render mode input: key re-encoding, mouse and wheel routing, scroll
//! and copy mode, and click repaint framing.

use super::*;

/// Frame coalescing (pump step 9): a click's big pane redraw must land
/// as ONE frame. Before the hold, htop's post-click redraw streamed as
/// dozens of %output chunks, the pump framed a partial state mid-burst
/// and re-framed the rest a millisecond later — two whole-screen frames
/// 1 ms apart, the visible flash-and-repaint of the manual-pass report.
/// PAR_MUX_INPUT_TRACE rides the PTY with the frames; within 400 ms of
/// the click's release at most one frame may exceed 500 cells (htop's
/// periodic 1.5 s meter refresh stays outside the window).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_click_repaint_coalesces_into_one_frame() {
    use portable_pty::CommandBuilder as PtyCommand;

    if !htop_available() {
        eprintln!("htop not found; skipping");
        return;
    }
    let (fixture, _daemon, mut client) = fixture_with_session("coalesce");
    let panes = client.send("list-panes").expect("list-panes");
    let pane = panes.first().cloned().unwrap_or_default();
    client
        .send(&format!("split-window -t {pane} -h"))
        .expect("split");
    client
        .send(&format!("send-keys -t {pane} htop Enter"))
        .expect("start htop");

    std::fs::write(
        fixture.client_config_path(),
        "[client]\nsidebar-on-launch = false\n",
    )
    .expect("client config");
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            ..Default::default()
        })
        .expect("openpty");
    let mut cmd = PtyCommand::new(env!("CARGO_BIN_EXE_par-mux"));
    cmd.arg("attach");
    cmd.arg("--socket");
    cmd.arg(fixture.socket());
    cmd.arg("--mode");
    cmd.arg("render");
    cmd.env("PAR_MUX_CONFIG", fixture.client_config_path());
    cmd.env("PAR_MUX_INPUT_TRACE", "1");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn attach");
    let mut to_child = pair.master.take_writer().expect("writer");
    let mut from_child = pair.master.try_clone_reader().expect("reader");
    let (tx, rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match from_child.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    tx.send(buf[..n].to_vec()).ok();
                }
            }
        }
    });
    let drain = |budget: Duration| -> Vec<u8> {
        let mut out = Vec::new();
        let end = Instant::now() + budget;
        while Instant::now() < end {
            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(c) => out.extend_from_slice(&c),
                Err(_) => break,
            }
        }
        out
    };
    let wait_for = |needle: &[u8], budget: Duration| -> Vec<u8> {
        let mut out = Vec::new();
        let end = Instant::now() + budget;
        while Instant::now() < end {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(c) => {
                    let hit = out.windows(needle.len()).any(|w| w == needle);
                    out.extend_from_slice(&c);
                    if hit {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        out
    };

    // Boot: htop's legend names Setup.
    let _ = wait_for(b"Setup", Duration::from_secs(10));
    let _ = drain(Duration::from_millis(500));

    // The legend's Setup cell opens the dialog; the Screens category row
    // is the measured click (host 1-based (4,17) in the 80x24 grid).
    for (col, row, settle) in [(14u16, 22u16, 600u64), (4, 17, 0)] {
        to_child
            .write_all(format!("\x1b[<0;{col};{row}M").as_bytes())
            .expect("press");
        to_child.flush().ok();
        std::thread::sleep(Duration::from_millis(150));
        to_child
            .write_all(format!("\x1b[<0;{col};{row}m").as_bytes())
            .expect("release");
        to_child.flush().ok();
        if settle > 0 {
            let _ = wait_for(b"[Setup]", Duration::from_secs(5));
            let _ = drain(Duration::from_millis(500));
        }
    }
    // The release's trace line anchors the window; drain 400 ms of it.
    let _ = wait_for(b"release=true", Duration::from_secs(5));
    let window = drain(Duration::from_millis(400));

    let text = String::from_utf8_lossy(&window);
    let big = text
        .lines()
        .filter(|l| l.contains("frame flush"))
        .filter_map(|l| {
            let toks: Vec<&str> = l.split_whitespace().collect();
            toks.iter()
                .position(|t| *t == "flush")
                .and_then(|i| toks.get(i + 1))
                .and_then(|n| n.parse::<usize>().ok())
        })
        .filter(|&cells| cells > 500)
        .count();
    assert!(
        big <= 1,
        "the click's repaint must coalesce into one big frame, saw {big}. window: {text:?}"
    );
    child.kill().ok();
    let _ = child.wait();
}

/// ARC-123: render mode (the default) honors `--prefix`: under
/// `--prefix C-a`, the old prefix `C-b d` does NOT detach (it forwards
/// to the pane), and `C-a d` does, with exit 0. Render mode used to
/// seed `C-b` and ignore the flag.
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_prefix_flag_rebinds_the_detach_chord() {
    let (fixture, _daemon, mut client) = fixture_with_session("render-prefix");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane, "--prefix", "C-a"]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up (mouse reporting enabled)",
    );

    // The old prefix is an ordinary key now: no detach.
    host.to_child.write_all(&[0x02, b'd']).expect("C-b d");
    host.to_child.flush().ok();
    assert_eq!(
        child_exit(&mut host, Duration::from_millis(1500)),
        None,
        "C-b d must not detach under --prefix C-a. stderr: {}",
        stderr.lock().unwrap()
    );

    host.to_child.write_all(&[0x01, b'd']).expect("C-a d");
    host.to_child.flush().ok();
    assert_eq!(
        child_exit(&mut host, Duration::from_secs(10)),
        Some(0),
        "C-a d must detach with exit 0. stderr: {}",
        stderr.lock().unwrap()
    );
}

/// Acceptance criterion 1 (render mode, PTY level): the pane's DECCKM
/// state re-encodes arrow keys. The pane's shell enables application
/// cursor keys by emitting DECCKM through `printf`, the render client
/// parses the host arrow key from stdin and re-encodes it against the
/// pane's tracked state, and the pane's `cat -v` prints the received
/// spelling onto its screen — which the daemon's capture-pane verifies.
/// An app reading arrow keys sees `ESC O A` (application) or `ESC [ A`
/// (normal), not whatever the host terminal sent.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_arrow_keys_reencode_per_the_panes_decckm() {
    let (fixture, _daemon, mut client) = fixture_with_session("renderkeys");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    // The pane: enable DECCKM, then echo every received byte visibly.
    // `cat -v` shows ESC as `^[`.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'printf \"\\033[?1hCKM%sON\\n\" -; cat -v'"
        ))
        .expect("start key reader");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    // Wait for the pane to print its DECCKM (the marker printf assembles
    // follows it, so the shell's echo never matches) so the replay
    // carries it before the client mirrors the pane — a fixed sleep raced
    // a slow shell prompt under full-gate load.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("CKM-ON") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never ran the DECCKM printf: {body:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    // Settle: the renderer painted the pane's marker, so its emulator
    // consumed the DECCKM before it. Then send an Up arrow (the
    // harness's stdin is the client's host stdin).
    let deadline = Instant::now() + common::CEILING;
    let mut painted: Vec<u8> = Vec::new();
    while !plain_text(&painted).contains("CKM-ON") {
        assert!(
            Instant::now() < deadline,
            "the render client never painted the pane's marker: {:?}\nstderr: {}",
            plain_text(&painted),
            stderr.lock().unwrap()
        );
        painted.extend(wait_for_output(&host, b"", Duration::from_millis(100)));
    }
    host.to_child.write_all(b"\x1b[A").expect("arrow up");
    host.to_child.flush().ok();

    // `ESC O A` prints as `^[OA` under `cat -v`. The authority is the
    // pane itself: poll its capture for the SS3 spelling.
    let deadline = Instant::now() + Duration::from_secs(10);
    let body = loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("^[OA") || body.contains("^[[A") || Instant::now() >= deadline {
            break body;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        body.contains("^[OA"),
        "the pane must receive the application-cursor spelling ESC O A for Up \
         (DECCKM on): {body:?}\nstderr: {}",
        stderr.lock().unwrap()
    );
    assert!(
        !body.contains("0;11;6"),
        "sanity: no stray mouse SGR reached the pane: {body:?}"
    );
    host.killer.kill().ok();
}

/// The report that only Backspace worked for claude panes: forward
/// delete must reach the pane as `ESC [ 3 ~`. The pane's `cat -v`
/// prints the received spelling — the pane is the authority.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_forward_delete_reaches_the_pane() {
    let (fixture, _daemon, mut client) = fixture_with_session("fwdel");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    // The pane: print the marker, then echo every received byte
    // visibly (`cat -v` shows ESC as `^[`).
    client
        .send(&format!(
            "send-keys -t {pane} -l 'printf \"DEL%sREADY\\n\" -; cat -v'"
        ))
        .expect("start key reader");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("DEL-READY") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never ran the marker printf: {body:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    let deadline = Instant::now() + common::CEILING;
    let mut painted: Vec<u8> = Vec::new();
    while !plain_text(&painted).contains("DEL-READY") {
        assert!(
            Instant::now() < deadline,
            "the render client never painted the pane's marker: {:?}\nstderr: {}",
            plain_text(&painted),
            stderr.lock().unwrap()
        );
        painted.extend(wait_for_output(&host, b"", Duration::from_millis(100)));
    }
    // The two spellings a host terminal sends for forward delete: the
    // xterm tilde form and the kitty-protocol CSI-u form.
    host.to_child.write_all(b"\x1b[3~").expect("tilde delete");
    host.to_child
        .write_all(b"\x1b[57427u")
        .expect("csi-u delete");
    host.to_child.flush().ok();

    let deadline = Instant::now() + Duration::from_secs(10);
    let body = loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("^[[3~") || Instant::now() >= deadline {
            break body;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        body.contains("^[[3~"),
        "the pane must receive forward delete as ESC [ 3 ~: {body:?}\nstderr: {}",
        stderr.lock().unwrap()
    );

    // The claude scenario: the pane app pushes the kitty keyboard
    // flags (disambiguate). The spec's functional-key table keeps the
    // tilde spelling in kitty mode too (57426..57435 are KP/media codes,
    // and a real Claude Code pane ignores `CSI 57427u` outright), so the
    // pane must still receive `ESC [ 3 ~`. The printf first clears the
    // screen so stage 1's output cannot satisfy the later capture, making
    // the assertion non-vacuous. `cat -v` owns the pane's stdin, so
    // interrupt it first to get the shell back before typing the printf.
    client
        .send(&format!("send-keys -t {pane} C-c"))
        .expect("stop cat");
    client
        .send(&format!(
            "send-keys -t {pane} -l 'printf \"\\033[2J\\033[H\\033[>1uKITTY%sON\\n\" -; cat -v'"
        ))
        .expect("push kitty flags");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("KITTY-ON") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never pushed the kitty flags: {body:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    host.to_child.write_all(b"\x1b[3~").expect("delete again");
    host.to_child.flush().ok();
    let deadline = Instant::now() + Duration::from_secs(10);
    let body = loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("^[[3~") || Instant::now() >= deadline {
            break body;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        body.contains("^[[3~"),
        "under pushed kitty flags the pane must still receive the tilde delete \
         per the spec's functional-key table: {body:?}\nstderr: {}",
        stderr.lock().unwrap()
    );
    assert!(
        !body.contains("57427"),
        "the PUA spelling CSI 57427u must never reach the pane (it is KP_BEGIN, \
         not Delete, and real Claude Code ignores it): {body:?}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Acceptance criterion 3 (render mode, PTY level): the wheel scrolls the
/// CLIENT's scrollback when the pane does not own mouse mode — the
/// renderer repaints the pane rect from its scrollback, and the pane
/// itself receives nothing.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_wheel_scrolls_client_scrollback_without_mouse_mode() {
    let (fixture, _daemon, mut client) = fixture_with_session("wheelscroll");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // Fill the pane's history: 40 numbered lines through the 24-row pane.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'for i in $(seq 1 40); do echo HISTLINE-$i; done'"
        ))
        .expect("fill");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane}"),
        |s| s.contains("HISTLINE-40"),
        Duration::from_secs(10),
        "the HISTLINE loop ran to its last line",
    );

    let (mut host, _stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    // Settle on the live paint.
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    expect_output(
        &host,
        b"HISTLINE-40",
        Duration::from_secs(10),
        "HISTLINE-40 on the host screen",
    );
    // Drain the settle paint so the post-wheel read is wheel-attributable.
    while host.output_rx.try_recv().is_ok() {}

    // Wheel up over the pane: SGR wheel-up at (10, 5), 1-based. The view
    // shifts up 3 lines, so the pane rect's TOP view row repaints with
    // scrollback-only content. (Which history line lands there depends on
    // the pane's prompt/echo lines, so the exact number is pinned by the
    // renderer's unit suite, not asserted here — the routing behavior is.)
    // The live pane's daemon-side view must not move: scrolling is
    // client-side only, so the capture is identical before and after the
    // wheel. (Render mode's size report carries content height — grid
    // minus the status row — so the visible line count is one fewer than
    // the host grid; never pin a specific history line here.)
    // The shell's next prompt lands AFTER the loop's last line: baselining
    // `before` while the prompt was still arriving made the post-wheel
    // `after` capture differ spuriously (card 01a13c1f0c). Baseline only
    // once the daemon grid holds still — two identical captures 300ms
    // apart — so `after` can only differ by a real wheel effect.
    let settle_deadline = Instant::now() + Duration::from_secs(10);
    let before = loop {
        let a = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        std::thread::sleep(Duration::from_millis(300));
        let b = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if a == b {
            break a;
        }
        assert!(
            Instant::now() < settle_deadline,
            "the pane never settled on a prompt: {a:?} vs {b:?}"
        );
    };
    assert!(!before.is_empty(), "the pane settled with a live view");
    host.to_child
        .write_all(b"\x1b[<64;10;5M")
        .expect("wheel up");
    host.to_child.flush().ok();
    let scrolled = wait_for_output(&host, b"\x1b[1;", Duration::from_secs(5));
    assert!(
        !scrolled.is_empty(),
        "the wheel must repaint the pane rect's top rows from scrollback"
    );
    let after = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture")
        .join("\n");
    assert_eq!(after, before, "the pane's live view is intact");
    assert!(
        !after.contains("\x1b[<64"),
        "no wheel SGR was forwarded to the pane: {after:?}"
    );
    host.killer.kill().ok();
}

/// Acceptance criterion 2+3 (forward side): a pane that owns mouse mode
/// receives pane-relative SGR mouse reports through the render client, and
/// a click on a rendered pane issues the daemon-side select-pane. The pane
/// enables SGR mouse mode (DECSET 1000 + 1006) via its own output; the
/// harness sends a left press at window (10, 5); the pane's `cat -v`
/// prints the re-encoded report.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_mouse_forwards_pane_relative_when_pane_owns_mouse() {
    let (fixture, _daemon, mut client) = fixture_with_session("mousefwd");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // The DECSET is followed by a marker printf assembles (`MOUSE%sON`
    // with `-`), so the shell's echo of the typed line never matches it.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'printf \"\\033[?1000h\\033[?1006hMOUSE%sON\\n\" -; cat -v'"
        ))
        .expect("enable mouse + reader");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    // A fixed sleep raced the shell under full-gate load: the click
    // landed before the pane owned the mouse and was only a focus click.
    // Wait for the daemon's pane to show the marker (the DECSET precedes
    // it), then for the render client to paint it (its emulator consumed
    // the DECSET too) before clicking.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("MOUSE-ON") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never ran the mouse-enable printf: {body:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    let deadline = Instant::now() + common::CEILING;
    let mut painted: Vec<u8> = Vec::new();
    while !plain_text(&painted).contains("MOUSE-ON") {
        assert!(
            Instant::now() < deadline,
            "the render client never painted the pane's marker: {:?}\nstderr: {}",
            plain_text(&painted),
            stderr.lock().unwrap()
        );
        painted.extend(wait_for_output(&host, b"", Duration::from_millis(100)));
    }

    // Left press at host col 10 row 5 (1-based): the tab strip shifts the
    // content down one row and the pane-border ring (the default config)
    // insets it one more, so the single pane's rect (0,0) maps the click
    // to pane-relative (8, 2) 0-based and the wire spelling is
    // ESC[<0;9;3M.
    host.to_child.write_all(b"\x1b[<0;10;5M").expect("click");
    host.to_child.flush().ok();
    // Poll the daemon's view of the pane: the client forwards the report,
    // `cat -v` echoes it, and the pane emulator records it.
    let deadline = Instant::now() + Duration::from_secs(10);
    let body = loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("^[[<0;9;3M") || Instant::now() >= deadline {
            break body;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        body.contains("^[[<0;9;3M"),
        "the owning pane must receive the pane-relative SGR click: {body:?}\n\
         stderr: {}",
        stderr.lock().unwrap()
    );

    // The click also focused the pane daemon-side (the single-pane window's
    // marked pane is this pane either way; the select-pane command itself
    // was issued — a two-pane focus assertion lives in the unit suite, and
    // pane-relative re-encoding during scroll mode is a non-goal (the
    // viewport is a modal view).
    host.killer.kill().ok();
}

/// omp's fullscreen TUI (pi-mono) enables `?1000 ?1002 ?1004 ?1006` and
/// scrolls on SGR wheel reports — the same enable shape as this repro.
/// The wheel must forward to a mouse-owning pane exactly like a press
/// does, re-encoded SGR pane-relative (wheel-up is button code 64).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_wheel_forwards_to_a_pane_that_owns_mouse() {
    let (fixture, _daemon, mut client) = fixture_with_session("wheelfwd");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!(
            "send-keys -t {pane} -l 'printf \"\\033[?1000h\\033[?1002h\\033[?1004h\\033[?1006hMOUSE%sON\\n\" -; cat -v'"
        ))
        .expect("enable mouse + reader");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("MOUSE-ON") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never ran the mouse-enable printf: {body:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    let deadline = Instant::now() + common::CEILING;
    let mut painted: Vec<u8> = Vec::new();
    while !plain_text(&painted).contains("MOUSE-ON") {
        assert!(
            Instant::now() < deadline,
            "the render client never painted the pane's marker: {:?}\nstderr: {}",
            plain_text(&painted),
            stderr.lock().unwrap()
        );
        painted.extend(wait_for_output(&host, b"", Duration::from_millis(100)));
    }

    // Wheel-up at host col 10 row 5, the same geometry the click test
    // maps to pane-relative (8, 2) 0-based: the pane must receive
    // ESC[<64;9;3M.
    host.to_child.write_all(b"\x1b[<64;10;5M").expect("wheel");
    host.to_child.flush().ok();
    let deadline = Instant::now() + Duration::from_secs(10);
    let body = loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("^[[<64;9;3M") || Instant::now() >= deadline {
            break body;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        body.contains("^[[<64;9;3M"),
        "the owning pane must receive the pane-relative SGR wheel: {body:?}\n\
         stderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// A scrolling gesture must deliver ONLY the forwarded reports to the
/// pane: a burst of wheel-ups plus a pointer motion (what a trackpad
/// gesture produces in iTerm2), then nothing else for a settle window.
/// Filed from a report that omp under par-mux attach scrolls up and
/// immediately snaps back to the bottom — if anything else is written
/// to the pane after the gesture (a synthesized wheel-down, a focus
/// report, a repaint), this prints the pane's full received bytes.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_scroll_gesture_sends_nothing_but_the_reports() {
    let (fixture, _daemon, mut client) = fixture_with_session("gesture");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // cat -n -v prefixes every line, so an injected bare sequence that
    // arrives without a newline is still visible in the capture.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'printf \"\\033[?1000h\\033[?1002h\\033[?1003h\\033[?1004h\\033[?1006hMOUSE%sON\\n\" -; cat -n -v'"
        ))
        .expect("enable mouse + reader");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let body = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        if body.contains("MOUSE-ON") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never ran the mouse-enable printf: {body:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut painted: Vec<u8> = Vec::new();
    while !plain_text(&painted).contains("MOUSE-ON") {
        assert!(
            Instant::now() < deadline,
            "the render client never painted the marker: {:?}\nstderr: {}",
            plain_text(&painted),
            stderr.lock().unwrap()
        );
        painted.extend(wait_for_output(&host, b"", Duration::from_millis(100)));
    }

    // The gesture: three wheel-ups, one no-button hover motion (cb 35,
    // what 1003 all-event tracking produces as the pointer drifts), one
    // more wheel-up.
    for seq in [
        "\x1b[<64;10;5M",
        "\x1b[<64;10;5M",
        "\x1b[<64;10;5M",
        "\x1b[<35;12;6M",
        "\x1b[<64;10;5M",
    ] {
        host.to_child.write_all(seq.as_bytes()).expect("gesture");
        host.to_child.flush().ok();
        std::thread::sleep(Duration::from_millis(30));
    }
    // Let any follow-on writer fire (heartbeats, sweeps), then read the
    // pane's full received stream.
    std::thread::sleep(Duration::from_millis(1500));
    let body = client
        .send(&format!("capture-pane -t {pane} -S -80"))
        .expect("capture")
        .join("\n");
    host.killer.kill().ok();
    let expected = [
        "^[[<64;9;3M",
        "^[[<64;9;3M",
        "^[[<64;9;3M",
        "^[[<35;11;4M",
        "^[[<64;9;3M",
    ];
    for want in expected {
        assert!(
            body.contains(want),
            "the pane missed forwarded report {want}: {body:?}\n\
             stderr: {}",
            stderr.lock().unwrap()
        );
    }
    // The exact input the pane received, minus what it echoed to itself:
    // every SGR report cat -n -v shows after the marker is input WE
    // caused — count REPORTS, not lines (cat buffers a burst onto one
    // line). The five reports must be the whole of it.
    let reports: usize = body
        .lines()
        .filter(|l| l.contains("^[[<"))
        .map(|l| l.matches("M").count())
        .sum();
    assert_eq!(
        reports, 5,
        "extra input reached the pane after the gesture (omp snaps to \
         its bottom on unexpected input): {body:?}"
    );
}

/// Acceptance criterion 2 (render mode, PTY level): prefix-[ enters the
/// scroll viewport — the pane rect's top rows repaint from scrollback —
/// and q exits cleanly (live view restored, keys reach the pane again).
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_prefix_bracket_enters_scroll_mode_and_q_exits() {
    let (fixture, _daemon, mut client) = fixture_with_session("scrollmode");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // Fill history: 40 numbered lines through the 24-row pane.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'for i in $(seq 1 40); do echo HISTLINE-$i; done'"
        ))
        .expect("fill");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane}"),
        |s| s.contains("HISTLINE-40"),
        Duration::from_secs(10),
        "the HISTLINE loop ran to its last line",
    );

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client enabled host mouse reporting",
    );
    expect_output(
        &host,
        b"HISTLINE-40",
        Duration::from_secs(10),
        "HISTLINE-40 on the host screen",
    );
    while host.output_rx.try_recv().is_ok() {}

    // prefix [ (C-b then '['). The viewport jumps one viewport up and the
    // pane rect's top rows repaint from scrollback.
    host.to_child.write_all(&[0x02, b'[']).expect("prefix [");
    host.to_child.flush().ok();
    let scrolled = wait_for_output(&host, b"scroll", Duration::from_secs(10));
    assert!(
        !scrolled.is_empty(),
        "prefix [ must repaint the pane rect from scrollback (the [scroll +N] \
         status cue or the history rows). stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&scrolled)
    );

    // q exits: the live view returns (HISTLINE-40 repaints) and keys reach
    // the pane again.
    host.to_child.write_all(b"q").expect("q");
    host.to_child.flush().ok();
    let restored = wait_for_output(&host, b"HISTLINE-40", Duration::from_secs(10));
    assert!(
        !restored.is_empty(),
        "q must snap the view back to live (HISTLINE-40 repaints): {:?}",
        String::from_utf8_lossy(&restored)
    );

    // Keys reach the pane again: an `echo` runs, the marker lands.
    host.to_child
        .write_all(b"echo POST-SCROLL-MARKER\r")
        .expect("type after exit");
    host.to_child.flush().ok();
    expect_output(
        &host,
        b"POST-SCROLL-MARKER",
        Duration::from_secs(10),
        "POST-SCROLL-MARKER on the host screen",
    );
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture");
    let body = capture.join("\n");
    assert!(
        body.contains("POST-SCROLL-MARKER"),
        "keys must reach the pane after exiting scroll mode: {body:?}"
    );
    host.killer.kill().ok();
}

/// Copy mode's `e` against a live daemon (herdr's `edit_scrollback`):
/// prefix [ then a typed `e` writes the pane's text to a private temp
/// file and runs `$VISUAL` on it in a zoomed pane split off the focused
/// one. The "editor" is a script that copies the file it was handed out;
/// the launcher removes the temp files when the editor exits.
#[cfg(unix)]
#[ignore = "load-sensitive: PTY/timing suite; run via make test-rust-sensitive or checkall-ci"]
#[test]
fn render_mode_copy_mode_e_opens_the_scrollback_in_the_editor() {
    use std::os::unix::fs::PermissionsExt as _;
    let (fixture, _daemon, mut client) = fixture_with_session("editsb");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!(
            "send-keys -t {pane} -l 'for i in $(seq 1 40); do echo EDITLINE-$i; done'"
        ))
        .expect("fill");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {pane}"),
        |s| s.contains("EDITLINE-40"),
        Duration::from_secs(10),
        "the EDITLINE loop ran",
    );

    let work = tempfile::tempdir().expect("work dir");
    let tmp = work.path().join("tmp");
    std::fs::create_dir(&tmp).expect("tmp dir");
    let copy = work.path().join("edited.txt");
    let editor = work.path().join("editor.sh");
    std::fs::write(
        &editor,
        format!("#!/bin/sh\ncp \"$1\" '{}'\n", copy.display()),
    )
    .expect("editor script");
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let (mut host, stderr) = spawn_attach_with_env(
        &fixture,
        &["-t", &pane],
        "render",
        DEFAULT_CLIENT_CONFIG,
        &[
            ("VISUAL", editor.to_str().expect("utf-8 path")),
            ("TMPDIR", tmp.to_str().expect("utf-8 path")),
        ],
    );
    expect_output(
        &host,
        b"EDITLINE-40",
        Duration::from_secs(10),
        "EDITLINE-40 on the host screen",
    );
    host.to_child.write_all(&[0x02, b'[']).expect("prefix [");
    host.to_child.flush().ok();
    // input spacing: the chord and the copy-mode key are separate keypresses
    std::thread::sleep(Duration::from_millis(200));
    host.to_child.write_all(b"e").expect("e");
    host.to_child.flush().ok();

    let deadline = Instant::now() + Duration::from_secs(15);
    while !copy.exists() {
        assert!(
            Instant::now() < deadline,
            "the editor never ran on the scrollback. panes: {:?} tmp: {:?} stderr: {}",
            client.send("list-panes"),
            std::fs::read_dir(&tmp)
                .map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
                .ok(),
            stderr.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // cp may still be writing when the file first appears.
    std::thread::sleep(Duration::from_millis(200));
    let edited = std::fs::read_to_string(&copy).expect("editor copy");
    assert!(
        edited.contains("EDITLINE-1\n") && edited.contains("EDITLINE-40\n"),
        "the editor saw the scrollback + screen: {edited:?}"
    );
    let cleanup = Instant::now() + Duration::from_secs(5);
    loop {
        let left: Vec<_> = std::fs::read_dir(&tmp)
            .expect("tmp")
            .flatten()
            .map(|e| e.file_name())
            .filter(|n| n.to_string_lossy().starts_with("par-mux-scrollback-"))
            .collect();
        if left.is_empty() {
            break;
        }
        assert!(Instant::now() < cleanup, "temp files left behind: {left:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
    host.killer.kill().ok();
}
