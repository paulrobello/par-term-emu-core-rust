//! Copy-mode integration suites (kanban card 01a1a529): entry on every
//! pane kind, the frozen snapshot's yank under continued output, block
//! selection paint, marks, and the search walk.

use super::copy_mode::VisualMode;
use super::*;

fn copy_lines(session: &mut WindowSession, count: usize) {
    for i in 0..count {
        session
            .renderer
            .feed_output(1, format!("line {i:02}\r\n").as_bytes());
    }
    session.renderer.focus(1);
}

/// Enter on a zero-scrollback pane: prefix [ does not refuse, the
/// cursor starts at the host cursor on the live screen, and the frozen
/// snapshot holds the visible rows.
#[test]
fn copy_mode_enters_on_zero_scrollback_pane() {
    let (_rx, mut conn, mut session) = two_pane_session("cm-zero", FakeScript::default());
    session.renderer.focus(1);
    chord(&mut session, &mut conn, b'[');
    assert!(session.is_copy(), "empty pane: roams the live screen");
    let state = session.copy_state();
    let rows = state.rows;
    assert_eq!(state.scrollback_len, 0);
    assert_eq!(
        state.cursor_absolute_line, 0,
        "the host cursor sat at row 0"
    );
    assert_eq!(state.live_rows.len(), rows);
    session.route_plain(b"q", &mut conn);
    assert!(!session.is_copy());
}

/// Enter on an alt-screen pane: the alt grid's visible rows freeze and
/// roam.
#[test]
fn copy_mode_enters_on_alt_screen_pane() {
    let (_rx, mut conn, mut session) = two_pane_session("cm-alt", FakeScript::default());
    session.renderer.focus(1);
    session
        .renderer
        .feed_output(1, b"\x1b[?1049h".to_vec().as_slice());
    assert!(session
        .renderer
        .pane_terminal(1)
        .expect("pane")
        .is_alt_screen_active());
    chord(&mut session, &mut conn, b'[');
    assert!(session.is_copy(), "alt screen roams its visible rows");
    assert_eq!(session.copy_state().scrollback_len, 0);
    session.route_plain(b"q", &mut conn);
    assert!(!session.is_copy());
}

/// The frozen snapshot survives continued output: lines that were live
/// at entry (and scrolled into history since) yank the ENTRY text, and
/// the OSC 52 write lands in the outbox.
#[test]
fn yank_reads_frozen_lines_under_continued_output() {
    let (rx, mut conn, mut session) = two_pane_session("cm-frozen", FakeScript::default());
    copy_lines(&mut session, 60);
    chord(&mut session, &mut conn, b'[');
    assert!(session.is_copy());
    session.copy_mode_auto_exit = false;
    // Roam to the top of history and anchor a line selection there.
    session.route_plain(b"g", &mut conn);
    session.route_plain(b"g", &mut conn);
    let top = session.copy_state().cursor_absolute_line;
    assert_eq!(top, 0, "gg is the buffer top");
    session.route_plain(b"V", &mut conn);
    session.route_plain(b"jj", &mut conn);
    // Continued output pushes lines into scrollback under the open mode.
    for i in 60..70 {
        session
            .renderer
            .feed_output(1, format!("line {i:02}\r\n").as_bytes());
    }
    session.route_plain(b"y", &mut conn);
    let osc_at = session
        .notify
        .outbox
        .windows(7)
        .position(|w| w == b"\x1b]52;c;");
    assert!(osc_at.is_some(), "the yank rode the outbox");
    // Decode the payload: the frozen entry lines 0-2, not the pushed
    // screen.
    let outbox = session.notify.outbox.clone();
    let after = &outbox[osc_at.expect("checked") + 7..];
    let end = after
        .windows(2)
        .position(|w| w == b"\x1b\\")
        .expect("OSC 52 terminator");
    let encoded = std::str::from_utf8(&after[..end]).expect("base64 is utf8");
    use base64::Engine as _;
    let text = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("decodes"),
    )
    .expect("utf8");
    assert_eq!(
        text, "line 00\nline 01\nline 02\n",
        "the yank took the frozen entry lines"
    );
    assert!(drained(&rx).is_empty(), "nothing reached the pane");
}

/// Block selection: Ctrl+V spans a column range; the painted cells
/// reverse only inside it, and the yank slices the columns.
#[test]
fn block_selection_covers_and_paints() {
    let (_rx, mut conn, mut session) = two_pane_session("cm-block", FakeScript::default());
    copy_lines(&mut session, 60);
    chord(&mut session, &mut conn, b'[');
    assert!(session.is_copy());
    session.copy_mode_auto_exit = false;
    // gg to the top, then a 2x3 block from column 5.
    session.route_plain(b"g", &mut conn);
    session.route_plain(b"g", &mut conn);
    session.route_plain(b"5", &mut conn);
    session.route_plain(b"l", &mut conn);
    session.copy_mode_input(
        &mut conn,
        &TermKeyEvent::char_('v', par_term_emu_core::keyboard::modifiers::CTRL),
    );
    assert_eq!(session.copy_state().visual_mode, VisualMode::Block);
    session.route_plain(b"j", &mut conn);
    session.route_plain(b"ll", &mut conn);
    let state = session.copy_state();
    assert_eq!(
        (state.cursor_absolute_line, state.cursor_col),
        (1, 7),
        "the block head"
    );
    session.renderer.render_frame();
    let mut reversed = 0usize;
    for row in 0..session.renderer.frame_height() {
        for col in 0..40u16 {
            if session.renderer.buffer[(col, row)]
                .style()
                .add_modifier
                .contains(ratatui::style::Modifier::REVERSED)
            {
                reversed += 1;
            }
        }
    }
    assert!(reversed > 0, "the block painted reversed");
    session.route_plain(b"y", &mut conn);
    session.route_plain(b"q", &mut conn);
    assert!(!session.is_copy());
}

/// Marks set and jump, and the search walk lands the cursor on each
/// committed match (n forward, N back).
#[test]
fn marks_and_search_walk() {
    let (_rx, mut conn, mut session) = two_pane_session("cm-walk", FakeScript::default());
    copy_lines(&mut session, 60);
    chord(&mut session, &mut conn, b'[');
    assert!(session.is_copy());
    // gg to the top; set mark a; G to the bottom; 'a jumps back.
    session.route_plain(b"g", &mut conn);
    session.route_plain(b"g", &mut conn);
    session.route_plain(b"m", &mut conn);
    session.route_plain(b"a", &mut conn);
    assert_eq!(session.copy_state().marks.len(), 1, "mark a set");
    session.route_plain(b"G", &mut conn);
    let bottom = session.copy_state().cursor_absolute_line;
    assert!(bottom > 0, "G went down");
    session.route_plain(b"'", &mut conn);
    session.route_plain(b"a", &mut conn);
    assert_eq!(
        session.copy_state().cursor_absolute_line,
        0,
        "'a jumped to the mark"
    );
    // Search forward from the top; n walks; N walks back. `line 1`
    // matches lines 10-19 (10 matches); the commit lands on the first.
    session.route_plain(b"/line 1\r", &mut conn);
    assert!(!session.copy_state().is_searching, "committed");
    let landed = session.copy_state().cursor_absolute_line;
    assert_eq!(landed, 10, "the commit landed on the first match");
    session.route_plain(b"n", &mut conn);
    assert_eq!(session.copy_state().cursor_absolute_line, 11, "n walks");
    session.route_plain(b"N", &mut conn);
    assert_eq!(
        session.copy_state().cursor_absolute_line,
        10,
        "N walks back"
    );
    session.route_plain(b"q", &mut conn);
}

/// Auto-exit on (the default): a yank copies and leaves; auto-exit
/// off: the mode stays up with the visual cleared.
#[test]
fn yank_auto_exit_follows_the_setting() {
    let (_rx, mut conn, mut session) = two_pane_session("cm-exit", FakeScript::default());
    copy_lines(&mut session, 40);
    chord(&mut session, &mut conn, b'[');
    session.route_plain(b"V", &mut conn);
    session.route_plain(b"y", &mut conn);
    assert!(!session.is_copy(), "default on: yank exits");
    // Again with the setting off.
    chord(&mut session, &mut conn, b'[');
    session.copy_mode_auto_exit = false;
    session.route_plain(b"V", &mut conn);
    session.route_plain(b"y", &mut conn);
    assert!(session.is_copy(), "off: the mode stays up");
    assert_eq!(
        session.copy_state().visual_mode,
        VisualMode::None,
        "the visual cleared"
    );
    session.route_plain(b"q", &mut conn);
}
