//! WindowSession suites: parked layouts and focus-pane landings.

use super::*;

/// ENH-044 parked-layout path: a `%layout-change` the handlers parked for
/// the pump (`apply_pending`) replays only panes new to the renderer —
/// the surviving panes reflow in place with their content intact (card
/// 01a1216d58dd).
#[test]
fn parked_layout_replays_only_panes_new_to_the_renderer() {
    let (rx, mut conn, mut session) = two_pane_session(
        "parked-layout-replay",
        FakeScript {
            replies: [(
                "refresh-client -t %3".to_string(),
                "pane three body".to_string(),
            )]
            .into_iter()
            .collect(),
            ..FakeScript::default()
        },
    );
    session.renderer.feed_output(1, b"left pane text");
    session.renderer.feed_output(2, b"right pane text");
    session.pending.layout = Some(
        parse_layout("0000,99x23,0,0{33x23,0,0,1,33x23,33,0,2,33x23,66,0,3}").expect("parses"),
    );
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("apply pending");
    let lines = drained(&rx);
    assert_eq!(
        replay_lines(&lines),
        vec!["refresh-client -t %3"],
        "only the new pane replays: {lines:?}"
    );
    assert!(session.renderer.emulators[&3]
        .terminal()
        .content()
        .contains("pane three body"));
    assert!(
        session.renderer.emulators[&1]
            .terminal()
            .content()
            .contains("left pane text"),
        "surviving panes keep their content"
    );
}

/// ENH-044: an `%exit` drained inside the resize's drain_around_layout
/// parks in `pending_end`, which the pump checks (and ends the session
/// on) after the refit (card 01a1216d58dd).
#[test]
fn exit_drained_in_a_resize_parks_pending_end() {
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %1 -C 100x23".to_string(),
        format!("{}\n%exit", two_pane_layout_line(100, 23)),
    );
    let (_rx, mut conn, mut session) = two_pane_session(
        "resize-drain-exit",
        FakeScript {
            notify,
            ..FakeScript::default()
        },
    );
    session
        .resize_to(&mut conn, 100, 23, &mut RecordingSink::default())
        .expect("resize");
    assert!(session.pending_end, "the drained %exit parked the end");
    assert!(
        std::mem::take(&mut session.pending_end),
        "the pump's post-drain check would end the session"
    );
}

/// A `pane:N` agent-row click on a pane in ANOTHER window: one merged
/// round trip — `switch-client -t %pane` resolves the pane's window,
/// switches, and replies the landed window, which drives the re-seed
/// straight from the reply (QA-245, card 01a1995a6).
#[test]
fn focus_pane_on_another_window_selects_looks_up_and_lands() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %5 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    let mut replies = std::collections::HashMap::new();
    replies.insert("switch-client -t %5".to_string(), "@1".to_string());
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (rx, mut conn, mut session) = two_pane_session(
        "qa245-otherwin",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    session.focus_pane(&mut conn, 5);
    settle(&mut session, &mut conn);
    let lines = drained(&rx);
    assert_eq!(
        &lines[..2],
        &["select-pane -t %5", "switch-client -t %5"],
        "wire: {lines:?}"
    );
    assert_eq!(session.window, "@1", "the view landed on the pane's window");
    let panes: Vec<u32> = session.renderer.layout().iter().map(|r| r.pane).collect();
    assert_eq!(panes, vec![5, 6], "re-seeded onto the new window's panes");
}

/// The cross-window landing focuses the CLICKED pane: a click on the
/// second pane of the target window must not land with the window's
/// first pane focused (the re-seed used to fall back to
/// `focused_pane_or_first`, which returns the new window's first pane
/// because the renderer's focus is still the old window's).
#[test]
fn focus_pane_on_another_window_lands_focused_on_the_clicked_pane() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %6 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    let mut replies = std::collections::HashMap::new();
    replies.insert("switch-client -t %6".to_string(), "@1".to_string());
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (rx, mut conn, mut session) = two_pane_session(
        "roster-second-pane",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    session.focus_pane(&mut conn, 6);
    settle(&mut session, &mut conn);
    let lines = drained(&rx);
    assert_eq!(
        &lines[..2],
        &["select-pane -t %6", "switch-client -t %6"],
        "wire: {lines:?}"
    );
    assert_eq!(session.window, "@1", "the view landed on the pane's window");
    assert_eq!(
        session.renderer.focused(),
        Some(6),
        "the landing focuses the clicked pane, not the window's first"
    );
}

/// The roster-click follow race (card 01a1989b5): the
/// `%session-window-changed` the landing's own `switch-client`
/// broadcasts parks a shared-selection follow; when the pump's
/// `apply_pending` consumes it BEFORE the `switch-client` reply is
/// dispatched (the event beats the reply to the client under load —
/// the daemon emits broadcasts before the reply), the follow re-seed
/// lands the window's FIRST pane (`focused_pane_or_first`, focus=None)
/// and the reply's clicked-pane landing is then nav-vetoed
/// (`self.window` already moved). The in-flight landing suppresses the
/// follow aimed at its own window, so the reply's landing carries the
/// clicked pane through.
#[test]
fn roster_click_landing_survives_the_parked_session_window_follow() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut notify = std::collections::HashMap::new();
    // The target window's `%layout-change`: the follow re-seed's size
    // report seeds it at %5, the landing's at %6 (whichever re-seed runs).
    notify.insert(
        "refresh-client -t %5 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    notify.insert(
        "refresh-client -t %6 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    let mut replies = std::collections::HashMap::new();
    replies.insert("switch-client -t %6".to_string(), "@1".to_string());
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (rx, mut conn, mut session) = two_pane_session(
        "roster-follow-race",
        FakeScript {
            replies,
            notify,
            // The reply lags its line by one pump beat, the way a loaded
            // daemon's broadcast beats its reply to the client: the test
            // can park the follow while the landing is genuinely in
            // flight, deterministically.
            delays: [(
                "switch-client -t %6".to_string(),
                std::time::Duration::from_millis(300),
            )]
            .into_iter()
            .collect(),
            ..FakeScript::default()
        },
    );
    session.focus_pane(&mut conn, 6);
    // The pane-info reply starts the landing (`switch-client` on the
    // wire). Dispatch while waiting for the line, and park the broadcast
    // the MOMENT it appears — the delayed reply cannot have been
    // dispatched yet. Every line the fake records lands in `wire`.
    let mut wire: Vec<String> = Vec::new();
    let started = std::time::Instant::now();
    loop {
        session.dispatch_replies(&mut conn);
        wire.extend(drained(&rx));
        if wire.iter().any(|line| line.starts_with("switch-client")) {
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the landing never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // The broadcast the daemon queues while executing the landing's own
    // `switch-client`, BEFORE its reply is written.
    session.handle_event(TmuxNotification::SessionWindowChanged {
        session_id: "$0".to_string(),
        window_id: "@1".to_string(),
    });
    // The pump order that loses the race: parked work (the follow)
    // applies before the next reply dispatch.
    let mut sink = RecordingSink::default();
    let _ = session.apply_pending(&mut conn, &mut sink);
    // The reply (delayed one pump beat) and the landing it drives.
    settle(&mut session, &mut conn);
    assert_eq!(session.window, "@1", "the view still lands on the window");
    assert_eq!(
        session.renderer.focused(),
        Some(6),
        "the landing focuses the clicked pane even when its own \
         broadcast parked a follow first"
    );
    // The suppressed follow left no follow re-seed on the wire: the only
    // size report is the landing's own, on the clicked pane (%6), not the
    // follow's first-pane seed (%5).
    wire.extend(drained(&rx));
    assert_eq!(
        wire.iter()
            .filter(|line| line.starts_with("refresh-client -t %") && line.contains(" -C "))
            .map(|line| line.split_whitespace().nth(2).unwrap().to_string())
            .collect::<Vec<_>>(),
        vec!["%6".to_string()],
        "one re-seed, on the clicked pane: {wire:?}"
    );
}

/// The same race through the cross-session flavor: the
/// `%client-session-changed` a cross-workspace click's `switch-client`
/// broadcasts parks `follow_session`; consumed before the reply, it
/// re-seeds the followed session's active window with no pane and
/// vetoes the clicked-pane landing.
#[test]
fn roster_click_landing_survives_the_parked_client_session_follow() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut notify = std::collections::HashMap::new();
    // The target window's `%layout-change`, riding either re-seed's size
    // report (see the sibling test).
    notify.insert(
        "refresh-client -t %5 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    notify.insert(
        "refresh-client -t %6 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    let mut replies = std::collections::HashMap::new();
    replies.insert("switch-client -t %6".to_string(), "@1".to_string());
    // The follow's resolution: the followed session's active window.
    replies.insert("list-windows -t $1".to_string(), "@1 * other".to_string());
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (rx, mut conn, mut session) = two_pane_session(
        "roster-follow-race-ws",
        FakeScript {
            replies,
            notify,
            // The same one-pump-beat reply lag (see the sibling test).
            delays: [(
                "switch-client -t %6".to_string(),
                std::time::Duration::from_millis(300),
            )]
            .into_iter()
            .collect(),
            ..FakeScript::default()
        },
    );
    session.focus_pane(&mut conn, 6);
    // The pane-info reply starts the landing (see the sibling test).
    let started = std::time::Instant::now();
    loop {
        session.dispatch_replies(&mut conn);
        if drained(&rx)
            .iter()
            .any(|line| line.starts_with("switch-client"))
        {
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the landing never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    session.handle_event(TmuxNotification::ClientSessionChanged {
        client: String::new(),
        session_id: "$1".to_string(),
        name: String::new(),
    });
    let mut sink = RecordingSink::default();
    let _ = session.apply_pending(&mut conn, &mut sink);
    // The reply (delayed one pump beat) and the landing it drives.
    settle(&mut session, &mut conn);
    assert_eq!(session.window, "@1", "the view still lands on the window");
    assert_eq!(
        session.renderer.focused(),
        Some(6),
        "the landing focuses the clicked pane even when its own \
         broadcast parked a follow first"
    );
}

/// Same-window click: select + lookup only, no landing; the status is
/// marked so the throttled refresh repaints the highlight (QA-245).
#[test]
fn focus_pane_on_the_shown_window_does_not_land() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("switch-client -t %2".to_string(), "@0".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "qa245-samewin",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.status_inflight.dirty = false;
    session.focus_pane(&mut conn, 2);
    settle(&mut session, &mut conn);
    let lines = drained(&rx);
    assert_eq!(lines, vec!["select-pane -t %2", "switch-client -t %2"]);
    assert_eq!(session.window, "@0");
    assert!(
        session.status_inflight.dirty,
        "the highlight repaint is scheduled"
    );
}

/// The landing's focus survives a LAGGING `%layout-change`: when the
/// switched-to window's layout event arrives after the landing — not
/// queued at the drain — the async install used to reset the focus to
/// the window's first pane, landing the roster click on the wrong pane
/// whenever the target tab hosts more than one. The landing parks the
/// clicked pane as the async install's focus (the drain case is the
/// cross-window test above).
#[test]
fn focus_pane_landing_focus_survives_a_lagging_layout_event() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut replies = std::collections::HashMap::new();
    replies.insert("switch-client -t %6".to_string(), "@1".to_string());
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    // No `notify`: the %layout-change lags the landing, riding the pump
    // stream after the re-seed has already set `window`.
    let (_rx, mut conn, mut session) = two_pane_session(
        "roster-lagging-layout",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.focus_pane(&mut conn, 6);
    settle(&mut session, &mut conn);
    assert_eq!(session.window, "@1", "the view landed on the pane's window");
    session.handle_event(TmuxNotification::LayoutChange {
        window_id: "@1".to_string(),
        window_layout: layout.to_string(),
        window_visible_layout: layout.to_string(),
        window_raw_flags: "*".to_string(),
    });
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("apply pending");
    assert_eq!(
        session.renderer.focused(),
        Some(6),
        "the async layout install keeps the clicked pane focused"
    );
}

/// A split whose `%layout-change` lags the re-seed keeps the fresh pane
/// focused: the split continuation parks it for the async install when
/// the fresh pane is not installed yet — the same lagging-event race
/// the roster landing parks for.
#[test]
fn split_focus_survives_a_lagging_layout_event() {
    let three_pane = "0000,80x23,0,0{20x23,0,0,1,20x23,20,0,3,40x23,40,0,2}";
    let mut replies = std::collections::HashMap::new();
    replies.insert("split-window -t %1 -h".to_string(), "%3".to_string());
    reseed_replies(&mut replies, "@0 * main", "@0", "%1\n%2\n%3");
    let (_rx, mut conn, mut session) = two_pane_session(
        "split-lagging-layout",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'%');
    assert_eq!(session.window, "@0");
    assert_ne!(
        session.renderer.focused(),
        Some(3),
        "pre-condition: the fresh pane is not installed yet (the layout event lags)"
    );
    session.handle_event(TmuxNotification::LayoutChange {
        window_id: "@0".to_string(),
        window_layout: three_pane.to_string(),
        window_visible_layout: three_pane.to_string(),
        window_raw_flags: "*".to_string(),
    });
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("apply pending");
    assert_eq!(
        session.renderer.focused(),
        Some(3),
        "the async install re-focuses the fresh split pane"
    );
}

/// A refused merged switch (the pane is gone) ends the click silently:
/// no landing, no busy flash (the select-pane fire-and-forget leg cannot
/// refuse visibly; the timed-out case is
/// `focus_pane_chain_stops_at_the_first_timeout`).
#[test]
fn focus_pane_stops_when_select_pane_fails() {
    let mut failing = std::collections::HashSet::new();
    failing.insert("switch-client -t %9".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "qa245-gone",
        FakeScript {
            failing,
            ..FakeScript::default()
        },
    );
    session.focus_pane(&mut conn, 9);
    settle(&mut session, &mut conn);
    // Pipelined: the select rides the wire before the refusal comes back.
    assert_eq!(
        drained(&rx),
        vec!["select-pane -t %9", "switch-client -t %9"]
    );
    assert_eq!(session.window, "@0", "the view stays put");
    assert_eq!(
        session.chrome.flash_text(),
        None,
        "a refusal is not a busy daemon"
    );
}

/// ENH-049: with a 2 s daemon delay on the click's `select-pane`, neither
/// the sidebar click nor the keystrokes typed after it make the pump wait;
/// once the daemon catches up it saw the select, the lookup and the keys
/// in order.
#[test]
fn slow_daemon_does_not_block_key_forwarding() {
    let mut script = FakeScript::default();
    script.delays.insert(
        "select-pane -t %2".to_string(),
        std::time::Duration::from_secs(2),
    );
    script
        .replies
        .insert("switch-client -t %2".to_string(), "@0".to_string());
    let (rx, mut conn, mut session) = two_pane_session("enh049-slow", script);
    show_sidebar(&mut session);
    session
        .renderer
        .set_sidebar_sections(Some(vec![super::super::super::SidebarSection {
            title: "workspaces",
            rows: vec![srow(SidebarRowId::Pane(2), "main \u{b7} agent", false)],
        }]));

    let started = std::time::Instant::now();
    session.route_mouse(&mut conn, sgr(0, 3, 2, false));
    let click = started.elapsed();
    let started = std::time::Instant::now();
    session.route_stdin_bytes(&mut conn, b"abc");
    let typed = started.elapsed();
    let limit = std::time::Duration::from_millis(200);
    assert!(click < limit, "the click blocked the pump for {click:?}");
    assert!(typed < limit, "typing blocked the pump for {typed:?}");

    settle(&mut session, &mut conn);
    let lines = drained(&rx);
    let position = |needle: &str| {
        lines
            .iter()
            .position(|l| l.starts_with(needle))
            .unwrap_or_else(|| panic!("no {needle} line: {lines:?}"))
    };
    let select = position("select-pane -t %2");
    let info = position("switch-client -t %2");
    let keys = lines
        .iter()
        .position(|l| l.starts_with("send-keys -t ") && l.ends_with("-H 61 62 63"))
        .unwrap_or_else(|| panic!("no send-keys line: {lines:?}"));
    assert!(select < info && info < keys, "wire order: {lines:?}");
}

/// ENH-049: a click on a pane of window @2 followed, before the daemon
/// answers, by the next-window chord to @3 ends on @3 and never reseeds
/// back to @2.
#[test]
fn focus_pane_chain_pipelines_and_ignores_a_stale_landing() {
    let layout = "0000,80x23,0,0,9";
    let mut notify = std::collections::HashMap::new();
    for pane in [1u32, 9] {
        notify.insert(
            format!("refresh-client -t %{pane} -C 80x23"),
            format!("%layout-change @3 {layout} {layout} *"),
        );
    }
    let mut replies = std::collections::HashMap::new();
    replies.insert("switch-client -t %5".to_string(), "@2".to_string());
    reseed_replies(&mut replies, "@0 * main\n@3 - other", "@3", "%9 @3 * z");
    let mut script = FakeScript {
        replies,
        notify,
        ..FakeScript::default()
    };
    script.delays.insert(
        "switch-client -t %5".to_string(),
        std::time::Duration::from_millis(300),
    );
    let (rx, mut conn, mut session) = two_pane_session("enh049-stale", script);

    session.focus_pane(&mut conn, 5);
    // The newer navigation: next window from @0 is @3.
    session.switch_window(&mut conn, 1);
    settle(&mut session, &mut conn);

    assert_eq!(session.window, "@3", "the newest navigation wins");
    let lines = drained(&rx);
    assert!(
        lines.iter().all(|l| !l.starts_with("switch-client -t @2")),
        "the stale pane-info reply must not land on @2: {lines:?}"
    );
    let reports = lines
        .iter()
        .filter(|l| l.starts_with("refresh-client") && l.contains(" -C "))
        .count();
    assert_eq!(reports, 1, "one reseed only: {lines:?}");
}

/// ENH-049: a parked reply never answers a later synchronous query. The
/// status refresh parses its own replies, and the continuation gets its
/// own.
#[test]
fn async_reply_never_answers_a_later_sync_query() {
    let mut script = five_session_script();
    script.delays.insert(
        "list-agents".to_string(),
        std::time::Duration::from_millis(100),
    );
    let (_rx, mut conn) = fake_daemon("enh049-mix", script);
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(FOUR_PANE).expect("parses"));
    session.window = "@0".to_string();
    let seen: std::rc::Rc<std::cell::RefCell<Option<Vec<String>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = std::rc::Rc::clone(&seen);
    session.send_then(&mut conn, "list-agents", move |_s, _c, reply| {
        *sink.borrow_mut() = Some(reply.body);
    });
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert_eq!(
        session.status.session_id.as_deref(),
        Some("$4"),
        "the status refresh parsed its own replies"
    );
    settle(&mut session, &mut conn);
    assert_eq!(
        seen.borrow().clone(),
        Some(Vec::new()),
        "the continuation received its own (empty agents) reply, not a status reply"
    );
}
