//! WindowSession suites: paste handling, render options, border colors, pump ticks.

use super::*;

/// The new-tab prompt's default name: one past the highest window
/// ordinal, bumped past any window NAME that already claims the
/// number (the manual-pass ask: the next non-conflicting index).
#[test]
fn next_window_name_bumps_past_conflicts() {
    // One past the highest window ordinal.
    assert_eq!(
        super::super::super::next_window_name(&[
            ("@0".to_string(), "0".to_string()),
            ("@1".to_string(), "1".to_string()),
        ]),
        "2"
    );
    // A rename claiming the next index bumps the default past it.
    assert_eq!(
        super::super::super::next_window_name(&[
            ("@0".to_string(), "0".to_string()),
            ("@1".to_string(), "2".to_string()),
        ]),
        "3"
    );
    // No numeric names in use: still the ordinal successor.
    assert_eq!(
        super::super::super::next_window_name(&[("@0".to_string(), "demo".to_string())]),
        "1"
    );
}

/// A name crossing the wire spelled so the daemon's quoting grammar
/// keeps it one word: unconditional single quotes, embedded quotes
/// via the `'\''` idiom.
#[test]
fn wire_quote_survives_spaces_and_quotes() {
    assert_eq!(super::super::super::wire_quote("notes"), "'notes'");
    assert_eq!(super::super::super::wire_quote("my notes"), "'my notes'");
    assert_eq!(super::super::super::wire_quote("it's"), "'it'\\''s'");
    assert_eq!(super::super::super::wire_quote(""), "''");
}

/// The help overlay's overflow scrollbar: a `┃` thumb on the ring's
/// right column, sized/positioned from the scroll state; a panel
/// that fits paints nothing on the ring.
#[test]
fn help_overlay_paints_a_scrollbar_thumb_when_it_overflows() {
    use super::super::super::{compose_help_panel, help_rows};
    let rows = help_rows(0x02, 0x12, Default::default(), 1);
    let total = super::super::super::help_content(&rows, "").len();
    let mut renderer = PaneRenderer::new(80, 40, Glyphs::Unicode);
    renderer.apply_layout(parse_layout("0000,80x40,0,0,1").expect("parses"));
    // A window small enough that the panel must scroll: thumb at the
    // top border column.
    renderer.set_overlay(Some((
        crate::mux::attach::HELP_OVERLAY_TITLE,
        compose_help_panel(&rows, "", false, 6, 0),
        Some((0, 6, total)),
    )));
    renderer.render_frame();
    let (x0, y0, inner, height) = renderer.overlay_geometry().expect("geometry");
    let ring_x = (x0 + inner + 1) as u16;
    let thumbs: Vec<u16> = (y0 as u16 + 1..y0 as u16 + 1 + height as u16)
        .filter(|y| renderer.cell(ring_x, *y).is_some_and(|c| c.symbol() == "┃"))
        .collect();
    assert!(
        !thumbs.is_empty(),
        "the thumb paints on the ring: rows {thumbs:?}"
    );
    assert_eq!(
        thumbs.len(),
        (height * 6 / total).max(1),
        "the thumb spans its proportional share"
    );
    // Scroll to the bottom: the thumb parks at the track's end.
    renderer.set_overlay(Some((
        crate::mux::attach::HELP_OVERLAY_TITLE,
        compose_help_panel(&rows, "", false, 6, total),
        Some((total.saturating_sub(6), 6, total)),
    )));
    renderer.render_frame();
    let last_thumb = (y0 as u16 + 1..y0 as u16 + 1 + height as u16)
        .filter(|y| renderer.cell(ring_x, *y).is_some_and(|c| c.symbol() == "┃"))
        .max()
        .expect("a bottom thumb");
    assert_eq!(
        last_thumb,
        y0 as u16 + height as u16,
        "the thumb parks at the track's end"
    );
    // A panel that fits: nothing on the ring.
    let (x0s, y0s, inners, heights) = {
        let short: Vec<crate::mux::attach::HelpRow> = (0..4)
            .map(|i| crate::mux::attach::HelpRow {
                text: format!("row {i}"),
                accent: false,
                footer: false,
                render_only: false,
            })
            .chain(std::iter::once(crate::mux::attach::HelpRow {
                text: "footer".to_string(),
                accent: false,
                footer: true,
                render_only: false,
            }))
            .collect();
        renderer.set_overlay(Some((" t ", short, Some((0, 6, 4)))));
        renderer.render_frame();
        renderer.overlay_geometry().expect("geometry")
    };
    assert!(
        (y0s as u16 + 1..y0s as u16 + 1 + heights as u16).all(|y| renderer
            .cell((x0s + inners + 1) as u16, y)
            .is_some_and(|c| c.symbol() != "┃"))
    );
}

/// The `send-keys -H` hex spelling of `bytes` (what `forward_chunked`
/// puts on the wire for one chunk).
fn hex_wire(pane: &str, bytes: &[u8]) -> String {
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("send-keys -t {pane} -H {}", hex.join(" "))
}

/// A two-pane session over a recording conn, focused on pane 1.
fn paste_session(
    tag: &str,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
    WindowSession,
) {
    let (rx, conn) = recording_conn(tag);
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.focus(1);
    drained(&rx);
    (rx, conn, session)
}

/// SEC-208 (a): an SGR mouse report split across two stdin bursts (two
/// pump ticks) is one mouse route — the session's parser holds the
/// partial — and no fragment of it reaches the pane as typed text.
#[ignore = "load-sensitive: timing assertions; run via make test-rust-sensitive"]
#[test]
fn split_mouse_report_across_pump_ticks_routes_once() {
    let (rx, mut conn, mut session) = paste_session("split-mouse");
    session.renderer.focus(2);
    assert!(!session.route_stdin_bytes(&mut conn, b"\x1b[<0;10;"));
    settle(&mut session, &mut conn);
    assert!(!session.route_stdin_bytes(&mut conn, b"5M"));
    settle(&mut session, &mut conn);
    assert_eq!(
        session.renderer.focused(),
        Some(1),
        "the reassembled press focused the pane under the pointer"
    );
    let lines = drained(&rx);
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("select-pane"))
            .count(),
        1,
        "exactly one mouse route: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with("send-keys")),
        "no mouse fragment forwarded as text: {lines:?}"
    );
}

/// SEC-208 (b): a bracketed paste split across two pump ticks forwards
/// its body opaque — the embedded prefix + `x` never kills the pane — and
/// a pane that enabled DECSET 2004 gets the body re-framed.
#[ignore = "load-sensitive: timing assertions; run via make test-rust-sensitive"]
#[test]
fn split_paste_across_pump_ticks_is_opaque_and_rewrapped() {
    let (rx, mut conn, mut session) = paste_session("split-paste");
    session.renderer.feed_output(1, b"\x1b[?2004h");
    assert!(!session.route_stdin_bytes(&mut conn, b"\x1b[200~ab"));
    settle(&mut session, &mut conn);
    assert!(!session.route_stdin_bytes(&mut conn, b"\x02xcd\x1b[201~"));
    settle(&mut session, &mut conn);
    assert!(
        !session.input_route.prefix_pending,
        "a pasted prefix byte never arms the prefix"
    );
    let lines = drained(&rx);
    assert!(
        !lines.iter().any(|l| l.starts_with("kill-pane")),
        "the pasted chord did not fire: {lines:?}"
    );
    assert_eq!(
        lines,
        vec![hex_wire("%1", b"\x1b[200~ab\x02xcd\x1b[201~")],
        "the body forwards whole, framed for the bracketed-paste pane"
    );
}

/// SEC-208 (b'): the same paste into a pane WITHOUT bracketed paste
/// forwards the body bare (no framing the app did not ask for).
#[test]
fn paste_into_a_plain_pane_forwards_the_body_bare() {
    let (rx, mut conn, mut session) = paste_session("plain-paste");
    session.route_stdin_bytes(&mut conn, b"\x1b[200~ab\x02xcd\x1b[201~");
    settle(&mut session, &mut conn);
    assert_eq!(drained(&rx), vec![hex_wire("%1", b"ab\x02xcd")]);
}

/// The paste bug's exact shape (card 01a18f0437): a host with 2004 on
/// (the guard now requests it) delivers a MULTI-LINE paste framed, and
/// the frame carries the interior newlines into a 2004 pane whole — a
/// pane app like claude code inserts the block instead of submitting
/// line-by-line. The unframed pane keeps the bare body.
#[test]
fn multi_line_paste_forwards_framed_with_interior_newlines() {
    let (rx, mut conn, mut session) = paste_session("multiline-paste");
    session.renderer.feed_output(1, b"\x1b[?2004h");
    session.route_stdin_bytes(&mut conn, b"\x1b[200~first\r\nsecond\x1b[201~");
    settle(&mut session, &mut conn);
    assert_eq!(
        drained(&rx),
        vec![hex_wire("%1", b"\x1b[200~first\r\nsecond\x1b[201~")]
    );

    let (rx, mut conn, mut session) = paste_session("multiline-plain");
    session.route_stdin_bytes(&mut conn, b"\x1b[200~first\r\nsecond\x1b[201~");
    settle(&mut session, &mut conn);
    assert_eq!(drained(&rx), vec![hex_wire("%1", b"first\r\nsecond")]);
}

/// SEC-208: a paste cancels a pending prefix (tmux's rule), so the
/// paste's first byte is not taken as a chord key.
#[test]
fn paste_cancels_a_pending_prefix() {
    let (rx, mut conn, mut session) = paste_session("paste-prefix");
    session.route_stdin_bytes(&mut conn, &[crate::mux::attach::C_B]);
    settle(&mut session, &mut conn);
    assert!(session.input_route.prefix_pending);
    session.route_stdin_bytes(&mut conn, b"\x1b[200~xd\x1b[201~");
    settle(&mut session, &mut conn);
    assert!(!session.input_route.prefix_pending);
    let lines = drained(&rx);
    assert_eq!(lines, vec![hex_wire("%1", b"xd")], "{lines:?}");
}

/// SEC-208 (c): an embedded terminator in a paste body is stripped
/// before the re-frame, so pasted text cannot close the pane's paste
/// early — including a terminator spliced together by the removal.
#[test]
fn embedded_paste_terminator_is_stripped() {
    let (rx, mut conn, mut session) = paste_session("paste-strip");
    session.renderer.feed_output(1, b"\x1b[?2004h");
    session.route_paste(&mut conn, b"a\x1b[201~b\x1b[201\x1b[201~~c");
    settle(&mut session, &mut conn);
    assert_eq!(drained(&rx), vec![hex_wire("%1", b"\x1b[200~abc\x1b[201~")]);
}

/// ARC-122: `with_options` applies every option and `options` reads
/// them all back — a field one side forgets fails here.
#[test]
fn render_options_round_trip_through_the_renderer() {
    let opts = RenderOptions {
        bg: Some(RtColor::Rgb(9, 9, 9)),
        glyphs: Glyphs::Heavy,
        pane_borders: true,
        show_label_in_border: true,
        pane_gaps: 2,
        scrollbar_gutter: true,
        sidebar_width: 17,
        border_active: Some(RtColor::Rgb(1, 2, 3)),
        border_plain: Some(RtColor::Rgb(4, 5, 6)),
    };
    assert_eq!(PaneRenderer::with_options(80, 24, &opts).options(), opts);
    assert_eq!(
        PaneRenderer::new(80, 24, Glyphs::Ascii).options(),
        RenderOptions {
            glyphs: Glyphs::Ascii,
            sidebar_width: 0,
            ..RenderOptions::default()
        },
        "new() keeps no side panel"
    );
}

/// ARC-122: the session's effective options hide the configured panel
/// width while the panel is off.
#[test]
fn effective_render_opts_zero_the_hidden_sidebar() {
    let mut session = WindowSession::new(80, 25);
    assert_eq!(session.render_opts.sidebar_width, 20);
    assert_eq!(session.effective_render_opts().sidebar_width, 0);
    session.sidebar.set(true);
    assert_eq!(session.effective_render_opts().sidebar_width, 20);
}

/// ARC-122: a host resize re-fits the renderer; the configured border
/// colors survive the refit (they used to revert to cyan/dim).
#[test]
fn border_colors_survive_a_resize_rebuild() {
    let active = RtColor::Rgb(1, 2, 3);
    let plain = RtColor::Rgb(4, 5, 6);
    let (_rx, mut conn, mut session) = two_pane_session(
        "arc122-resize",
        FakeScript {
            notify: two_pane_notify(100, 40),
            ..FakeScript::default()
        },
    );
    session.set_pane_borders(true);
    session.set_border_colors(Some(active), Some(plain));
    session
        .resize_to(&mut conn, 100, 40, &mut RecordingSink::default())
        .expect("resize");
    assert_eq!(session.renderer.layout().len(), 2, "the refit re-laid");
    assert_ring_colors(&mut session, 0, 100, active, plain);
}

/// ARC-122: a window re-seed re-fits the renderer too; the colors
/// survive it.
#[test]
fn border_colors_survive_a_reseed_rebuild() {
    let active = RtColor::Rgb(1, 2, 3);
    let plain = RtColor::Rgb(4, 5, 6);
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 * main", "@0", "%1\n%2");
    let (_rx, mut conn, mut session) = two_pane_session(
        "arc122-reseed",
        FakeScript {
            replies,
            notify: two_pane_notify(80, 23),
            ..FakeScript::default()
        },
    );
    session.set_pane_borders(true);
    session.set_border_colors(Some(active), Some(plain));
    session.reseed_window(&mut conn, "@0", false);
    assert_eq!(session.renderer.layout().len(), 2, "the reseed re-laid");
    assert_ring_colors(&mut session, 0, 80, active, plain);
}

/// ARC-122: toggling the side panel (which parks a refit the pump runs
/// as a resize refit) keeps the colors, and the configured panel
/// width rides the refit.
#[test]
fn border_colors_survive_a_sidebar_toggle_refit() {
    let active = RtColor::Rgb(1, 2, 3);
    let plain = RtColor::Rgb(4, 5, 6);
    let (_rx, mut conn, mut session) = two_pane_session(
        "arc122-sidebar",
        FakeScript {
            notify: two_pane_notify(60, 23),
            ..FakeScript::default()
        },
    );
    session.set_pane_borders(true);
    session.set_border_colors(Some(active), Some(plain));
    session.toggle_sidebar(&mut conn);
    assert!(session.pending.grid_refit);
    session
        .resize_to(&mut conn, 80, 23, &mut RecordingSink::default())
        .expect("refit");
    assert_eq!(
        session.renderer.sidebar_width(),
        20,
        "the panel width survives"
    );
    assert_eq!(session.renderer.layout().len(), 2, "the refit re-laid");
    assert_ring_colors(&mut session, 20, 60, active, plain);
}

/// Pump step 9's wake, event-wise (card 01a19d78): the wait's stdin arm
/// consumes a host click the moment it arrives instead of the wait
/// blocking on the daemon channel alone until the 16 ms tick — input
/// used to wait out the poll remainder. The tick enters as a channel the
/// test fires itself, so the assertion is ordering (the chunk stashed by
/// the stdin arm while the test's tick is still pending), not a
/// wall-clock bound racing the gate's own load — the fixed bound flaked
/// at load 9-100 (card 01a1d8e1). Unix-only: a real pipe carries the
/// click through the production reader thread.
#[test]
#[cfg(unix)]
fn pump_wait_wakes_on_stdin_off_the_poll_tick() {
    let (_replies, mut conn) = recording_conn("pump-wait-stdin");
    let mut session = WindowSession::new(80, 25);

    // The host-tty shape: a real pipe whose read end the production
    // reader thread drains into the wake channel.
    let mut fds = [0 as libc::c_int; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let (read_fd, write_fd) = (fds[0], fds[1]);
    struct FdReader(libc::c_int);
    impl std::io::Read for FdReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            loop {
                let n = unsafe { libc::read(self.0, buf.as_mut_ptr().cast(), buf.len()) };
                if n < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(err);
                }
                return Ok(n as usize);
            }
        }
    }
    impl Drop for FdReader {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }
    let (tx, rx) = crossbeam_channel::bounded::<crate::mux::attach::StdinMsg>(64);
    crate::mux::attach::Stdin::spawn_reader(FdReader(read_fd), tx);
    let mut stdin = crate::mux::attach::Stdin { rx, front: None };

    // The test's own tick: pending, undelivered. The thread behind it is
    // only the fail-safe that bounds a regressed daemon-only wait (which
    // would otherwise hang the suite) — a stdin wake never waits it out.
    let (tick_tx, tick_rx) = crossbeam_channel::bounded::<std::time::Instant>(1);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let _ = tick_tx.send(std::time::Instant::now());
    });

    // The click lands ahead of the wait; the reader thread delivers it
    // whenever scheduled, and the wait wakes on that delivery. There is
    // no tick boundary left to race.
    let bytes = b"\x1b[<0;10;5M";
    assert_eq!(
        unsafe { libc::write(write_fd, bytes.as_ptr().cast(), bytes.len()) },
        bytes.len() as isize
    );

    let outcome = session.wait_next_push_with_tick(&mut conn, &mut stdin, tick_rx.clone());
    assert!(
        matches!(outcome, std::ops::ControlFlow::Continue(())),
        "the stdin wake must not end the view"
    );

    // The stdin arm consumed the click into the stash while the test's
    // tick is still pending — the wake off-tick, asserted event-wise.
    let stashed = stdin
        .front
        .take()
        .expect("the wait's stdin arm parked the click");
    assert_eq!(stashed.expect("the chunk is Ok"), bytes.to_vec());
    assert_eq!(
        tick_rx.try_recv().unwrap_err(),
        crossbeam_channel::TryRecvError::Empty,
        "the test's tick must still be pending"
    );
    unsafe { libc::close(write_fd) };
}
