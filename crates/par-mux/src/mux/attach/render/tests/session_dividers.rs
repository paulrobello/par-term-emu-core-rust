//! WindowSession suites: divider drags, selection release, and resize chords.

use super::*;

/// The drag affordance, headless end-to-end: a press ON the
/// two-pane divider column starts a PENDING drag (no focus, no
/// forward), motion promotes it and the pump's apply_drag sends one
/// relative resize-pane for the boundary's left pane at the full
/// delta, the divider highlights while dragging, and release clears
/// the state without forwarding anything to either pane.
#[test]
fn drag_on_divider_resizes_without_clicking_through() {
    let (rx, mut conn) = recording_conn("drag");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));

    // Drain the handshake's recorded lines first, so the quiet wire
    // assertions below see only the drag's traffic.
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}

    // Press on the divider column (x=39, 1-based col 40): pending,
    // and the press must NOT focus or forward.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 40,
            row: 6,
            release: false,
        },
    );
    assert!(
        matches!(session.drag, Some(DragState::Pending { .. })),
        "the press starts a pending drag"
    );
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "a divider press forwards nothing"
    );

    // Motion +5 cells: the drag activates and highlights the divider.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 45,
            row: 6,
            release: false,
        },
    );
    assert_eq!(
        session.renderer.drag_divider,
        Some((true, 1, 2)),
        "the dragged divider highlights"
    );

    // The pump's frame-cadence application: one resize-pane at the
    // full delta, aimed at the boundary's left/top pane.
    session.apply_drag(&mut conn);
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 5",
        "the drag maps to the wire's relative resize for pane 1"
    );
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "a drag forwards no mouse bytes to the pane"
    );

    // Release: the drag ends, the highlight clears, still no
    // click-through.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 45,
            row: 6,
            release: true,
        },
    );
    assert!(session.drag.is_none());
    assert_eq!(session.renderer.drag_divider, None);
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "the drag's release forwards nothing"
    );
}

/// A click NEAR a divider without any drag still focuses: press and
/// release at the same point land the focus on the pane under the
/// pointer (select-pane rides the wire) even though the press itself
/// was captured by the drag's pending state.
#[test]
fn click_near_divider_without_drag_still_focuses() {
    let (rx, mut conn) = recording_conn("click-div");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.focus(2);

    // Press on the divider, release without motion: the focus flips
    // to the pane under the pointer (the divider column sits in pane
    // 1's last column).
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 40,
            row: 6,
            release: false,
        },
    );
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 40,
            row: 6,
            release: true,
        },
    );
    assert_eq!(
        session.renderer.focused(),
        Some(1),
        "the bare click focuses the pane under the pointer"
    );
    assert_eq!(
        wait_recorded(&rx, "select-pane"),
        "select-pane -t %1",
        "the focus rides select-pane"
    );
}

/// A press on the pane content cell BESIDE the divider is a pane
/// click, not a drag start: the manual-pass trace's swallowed htop
/// click (a press one cell off the divider hijacked as a divider drag,
/// whose drift motions then resized the layout at pointer speed).
#[test]
fn a_press_beside_the_divider_is_a_pane_click_not_a_drag() {
    let (rx, mut conn) = recording_conn("edge-click");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.focus(2);

    // 1-based col 39 = content col 38: pane 1's second-to-last column,
    // one cell left of the divider at col 39 — a content cell.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 39,
            row: 6,
            release: false,
        },
    );
    assert!(
        session.drag.is_none(),
        "a content-cell press must not start a drag"
    );
    assert_eq!(
        session.renderer.focused(),
        Some(1),
        "the press focuses the pane under the pointer"
    );
    assert_eq!(
        wait_recorded(&rx, "select-pane"),
        "select-pane -t %1",
        "the focus rides select-pane"
    );
    // And the mirrored release is a plain pane release (no drag state).
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 39,
            row: 6,
            release: true,
        },
    );
    assert!(session.drag.is_none());
}

/// A divider drag whose RELEASE lands over the visible side panel ends
/// the drag. The panel's strip branch used to run first and
/// `sidebar_hit` consumes motion and release, so the drag stayed armed
/// forever: every later wheel was eaten by the pane-area guard (the
/// held button "owns" the pointer) and every motion re-drove
/// `resize-pane` — the "mouse stopped working until reconnect" wedge.
#[test]
fn divider_drag_release_over_the_sidebar_ends_the_drag() {
    let (rx, mut conn) = recording_conn("drag-side");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    show_sidebar(&mut session);
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}

    // Scrollback on pane 1 so a wheel has something to move.
    for i in 0..60 {
        session
            .renderer
            .feed_output(1, format!("pane one line {i:03}\r\n").as_bytes());
    }

    // Press on the divider (content col 39 = host col 60 with the
    // panel up), motion +5 to activate, then release OVER THE PANEL
    // (host col 5, inside the panel's 20-col strip).
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 60,
            row: 6,
            release: false,
        },
    );
    assert!(
        matches!(session.drag, Some(DragState::Pending { .. })),
        "the press starts a pending drag"
    );
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 65,
            row: 6,
            release: false,
        },
    );
    assert_eq!(
        session.renderer.drag_divider,
        Some((true, 1, 2)),
        "the drag activates"
    );

    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 5,
            row: 6,
            release: true,
        },
    );
    assert!(
        session.drag.is_none(),
        "the release over the side panel must end the drag"
    );
    assert_eq!(
        session.renderer.drag_divider, None,
        "the divider highlight clears with the drag"
    );

    // And the wheel works again: pane 1 does not own mouse tracking,
    // so the wheel scrolls its client-side viewport.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 64,
            col: 30,
            row: 6,
            release: false,
        },
    );
    assert_eq!(
        session.renderer.scroll_offset_of(1),
        3,
        "the wheel scrolls pane 1 once the drag is gone"
    );
}

/// A text-selection drag whose release lands over the visible side
/// panel ends the selection the same way: the panel's strip branch must
/// not swallow the release, or `text_drag` stays armed — every later
/// wheel consumed, every hover extending the selection.
#[test]
fn text_selection_release_over_the_sidebar_ends_the_selection() {
    let (rx, mut conn) = recording_conn("text-side");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    show_sidebar(&mut session);
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}
    for i in 0..6 {
        session
            .renderer
            .feed_output(1, format!("pane one line {i}\r\n").as_bytes());
    }

    // Press in pane 1 (mouse Off → the press arms a selection drag),
    // motion inside the pane activates it, release over the panel.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 30,
            row: 6,
            release: false,
        },
    );
    assert!(
        matches!(session.text_drag, Some(TextDrag::Pending { .. })),
        "the press arms a selection drag"
    );
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 34,
            row: 7,
            release: false,
        },
    );
    assert!(
        matches!(session.text_drag, Some(TextDrag::Active { .. })),
        "the motion activates the selection"
    );

    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 5,
            row: 6,
            release: true,
        },
    );
    assert!(
        session.text_drag.is_none(),
        "the release over the side panel must end the selection"
    );
    assert!(
        session.renderer.selection.is_none(),
        "the sweep over the panel copies nothing"
    );
}

/// A text-selection drag armed BEFORE a full-pointer modal (settings
/// here; the menu and the layouts dialog route identically) ends at its
/// release even while the modal is up: each of those modals consumes
/// every event, so the release used to be swallowed and the drag stayed
/// armed until the next release after the modal closed. Motion while
/// the modal is up stays modal-gated — it neither sweeps nor scrolls.
#[test]
fn text_selection_release_under_a_full_pointer_modal_ends_the_selection() {
    let (rx, mut conn) = recording_conn("text-modal");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}
    for i in 0..6 {
        session
            .renderer
            .feed_output(1, format!("pane one line {i}\r\n").as_bytes());
    }

    // Press in pane 1 (mouse Off → the press arms a selection drag),
    // motion inside the pane activates it.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 30,
            row: 6,
            release: false,
        },
    );
    assert!(
        matches!(session.text_drag, Some(TextDrag::Pending { .. })),
        "the press arms a selection drag"
    );
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 34,
            row: 7,
            release: false,
        },
    );
    assert!(
        matches!(session.text_drag, Some(TextDrag::Active { .. })),
        "the motion activates the selection"
    );
    let swept = session.renderer.selection;

    // Open the settings dialog — a full-pointer modal — while the drag
    // is armed.
    session.enter_settings();
    assert!(matches!(session.modal, Modal::Settings(_)));

    // Motion under the modal is consumed by it: the drag neither
    // extends nor scrolls.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 60,
            row: 12,
            release: false,
        },
    );
    assert_eq!(
        session.renderer.selection, swept,
        "motion under the modal must not extend the selection"
    );
    assert!(
        matches!(session.text_drag, Some(TextDrag::Active { .. })),
        "the modal-up motion leaves the drag armed"
    );

    // The first release ends the selection even under the modal.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 40,
            row: 10,
            release: true,
        },
    );
    assert!(
        session.text_drag.is_none(),
        "the release under the modal must end the selection"
    );
    assert!(
        session.renderer.selection.is_none(),
        "the release copies what was swept and clears it"
    );
    assert!(
        matches!(session.modal, Modal::Settings(_)),
        "the release is the drag's alone: the dialog stays up"
    );
}

/// The drag's resize steps are throttled: one `resize-pane` per
/// interval at most, the unapplied remainder held back while motions
/// keep arriving faster, and the release flushes what is left so the
/// drag lands exactly where it is let go.
#[test]
#[ignore = "load-sensitive: timing assertions; run via make test-rust-sensitive"]
fn drag_resizes_throttle_and_the_release_flushes_the_remainder() {
    let (rx, mut conn) = recording_conn("drag-throttle");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}

    let press = SgrMouse {
        cb: 0,
        col: 40,
        row: 6,
        release: false,
    };
    session.route_mouse(&mut conn, press);
    // Motion +5, then +3 more before the interval can pass.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 45,
            row: 6,
            release: false,
        },
    );
    session.apply_drag(&mut conn);
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 5",
        "the first step sends the full delta"
    );
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 48,
            row: 6,
            release: false,
        },
    );
    session.apply_drag(&mut conn);
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "a step inside the interval sends nothing"
    );
    // Release: the held remainder (+3) flushes immediately.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 48,
            row: 6,
            release: true,
        },
    );
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 3",
        "the release flushes the throttled remainder"
    );
    assert!(session.drag.is_none());
}

/// The tmux divider convention (the owner's manual pass): the HALF of
/// the shared divider nearer the active pane carries the highlight —
/// a side-by-side split's divider highlights its top half when the
/// left pane is focused and its bottom half when the right one is.
#[test]
fn focus_flip_changes_the_shared_dividers_color() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);

    renderer.focus(1);
    renderer.render_frame();
    assert_eq!(
        renderer.buffer[(39, 0)].fg,
        RtColor::Indexed(14),
        "pane 1 (left) focused: the divider's TOP half is the accent"
    );
    assert!(
        renderer.buffer[(39, 23)].modifier.contains(RtModifier::DIM),
        "pane 1 focused: the divider's bottom half stays dim"
    );
    renderer.focus(2);
    renderer.mark_all_dirty();
    renderer.render_frame();
    assert_eq!(
        renderer.buffer[(39, 23)].fg,
        RtColor::Indexed(14),
        "pane 2 (right) focused: the divider's BOTTOM half is the accent"
    );
    assert!(
        renderer.buffer[(39, 0)].modifier.contains(RtModifier::DIM),
        "pane 2 focused: the divider's top half stays dim"
    );
}

/// Render-mode resize chords: prefix R enters the mode and arrows
/// send the relative resize-pane for the FOCUSED pane; a non-arrow
/// key leaves the mode with nothing leaked into the pane.
#[test]
fn render_resize_chord_sends_for_the_focused_pane() {
    let (rx, conn) = recording_conn("render-resize");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    let mut conn = conn;

    assert!(
        !session.route_plain(&[crate::mux::attach::C_B, b'R'], &mut conn,),
        "the resize chord enters the mode"
    );
    assert!(session.is_resize());
    session.resize_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 1",
        "the arrow resizes the focused pane (the layout's first leaf)"
    );
    // Any other key leaves the mode, consumed.
    session.resize_key(&mut conn, &TermKeyEvent::functional(TermKey::Escape, 0));
    assert!(!session.is_resize());

    // The RIGHT pane focused: the arrows move the shared divider in
    // the pressed direction, so the wire direction inverts (the wire
    // grows the focused pane; growing the right pane would move the
    // divider left — the manual-pass report).
    session.renderer.focus(2);
    assert!(
        !session.route_plain(&[crate::mux::attach::C_B, b'R'], &mut conn,),
        "the resize chord re-enters the mode"
    );
    session.resize_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %2 -L 1",
        "Right with the right pane focused shrinks it: the divider moves right"
    );
}
