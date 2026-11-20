//! WindowSession suites: snapshot refresh, toasts, and cached landings.

use super::*;

/// ENH-043: against a daemon advertising `client-snapshot v1`, one
/// status refresh — bar, pane titles, and side panel — is exactly one
/// round trip, and every fact lands from that one reply.
#[test]
fn snapshot_refresh_is_one_round_trip() {
    let (rx, mut conn) = fake_daemon("snaponce", snapshot_script());
    let mut session = snapshot_session();
    drained(&rx);

    session
        .refresh_status_facts(&mut conn)
        .expect("snapshot refresh");
    assert_eq!(
        received_within_300ms(&rx),
        vec!["client-snapshot -t @0".to_string()]
    );
    assert_eq!(
        session.status.windows(),
        &[
            ("@0".to_string(), "edit".to_string()),
            ("@1".to_string(), "logs".to_string()),
        ]
    );
    assert_eq!(session.status.session_id.as_deref(), Some("$1"));
    assert_eq!(session.status.pane_title(), "build output");
    assert_eq!(session.renderer.border_label(1), "build output");
    assert_eq!(session.renderer.border_label(2), "tests");
    assert_eq!(session.renderer.border_label(3), "");
    assert_eq!(
        session.renderer.sidebar_sections,
        Some(vec![
            crate::mux::attach::panels::SidebarSection {
                title: "workspaces",
                rows: vec![
                    srow(SidebarRowId::Workspace("+0".into()), "main", true),
                    srow(SidebarRowId::Workspace("+1".into()), "side", false),
                ],
            },
            crate::mux::attach::panels::SidebarSection {
                title: "agents",
                rows: vec![
                    // Grouped sub-sorts by status priority: blocked first.
                    arow(3, None, "kimi", "blocked", false),
                    arow(1, None, "claude", "working", true),
                ],
            },
        ])
    );
    let text: String = session
        .status
        .compose(200, None, None, false)
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(!text.contains("claude:working"), "{text}");
    assert!(!text.contains("kimi:blocked"), "{text}");
    assert!(text.contains("$2:three"), "{text}");
}

/// Without the feature the legacy per-fact round still runs, and the
/// client never sends `client-snapshot`.
#[test]
fn legacy_refresh_still_runs_without_the_feature() {
    let mut script = snapshot_script();
    script.replies.remove("list-commands");
    script
        .replies
        .insert("list-sessions".to_string(), "+0: main: $0: one".to_string());
    script
        .replies
        .insert("list-windows -t $0".to_string(), "@0 * edit".to_string());
    let (rx, mut conn) = fake_daemon("snaplegacy", script);
    let mut session = snapshot_session();
    drained(&rx);

    session
        .refresh_status_facts(&mut conn)
        .expect("legacy refresh");
    let sent = received_within_300ms(&rx);
    assert!(sent.iter().any(|l| l == "list-sessions"), "{sent:?}");
    assert!(sent.iter().any(|l| l == "list-agents"), "{sent:?}");
    for pane in 1..=3 {
        let line = format!("pane-title -t %{pane}");
        assert_eq!(
            sent.iter().filter(|l| **l == line).count(),
            1,
            "one {line}: {sent:?}"
        );
    }
    assert!(
        !sent.iter().any(|l| l.starts_with("client-snapshot")),
        "{sent:?}"
    );
}

/// A pushed transition into a selected state queues one notification
/// escape the next frame writes to the host; the refresh that seeded the
/// roster, a repeat of the same state, an unselected state, and
/// delivery `off` queue nothing.
#[test]
fn inside_delivery_shows_a_toast_that_expires_and_clicks_to_the_pane() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snaptoast", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    session.refresh_status_facts(&mut conn).expect("seed");
    // Put the agents in the background (the shown window holds pane 9).
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    drained(&rx);
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%1".to_string(),
        agent: "claude".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    });
    assert_eq!(
        session.notify.outbox,
        vec![0x07],
        "no terminal escape, only the bell that flags the host tab"
    );
    // The bell reaches the host with the next frame.
    let mut sink = super::RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(sink.raw, vec![0x07], "the frame rings the host bell");
    assert!(session.notify.outbox.is_empty(), "rung once");
    let toast = session.renderer.toast.clone().expect("a toast is up");
    assert_eq!(toast.title, "claude blocked");
    assert_eq!(toast.body, "main \u{b7} claude");
    assert_eq!(toast.pane, Some(1));
    // It paints at the content area's bottom-right, over the panes.
    let (tx, ty, tw, th) = session.renderer.toast_rect().expect("on screen");
    assert_eq!(tx + tw, session.renderer.frames.width, "flush right");
    assert_eq!(ty + th, session.renderer.frames.height, "flush bottom");
    session.renderer.render_frame();
    let title_row: String = (tx..tx + tw)
        .map(|x| {
            session
                .renderer
                .cell(x, ty + 1)
                .unwrap()
                .symbol()
                .to_string()
        })
        .collect();
    assert!(title_row.contains("claude blocked"), "{title_row:?}");
    // A click on the card focuses the pane and dismisses the toast.
    let (hx, hy) = session.geometry.frame_to_host(tx + 2, ty + 1);
    session.route_mouse(&mut conn, sgr(0, hx + 1, hy + 1, false));
    assert!(session.renderer.toast.is_none(), "the click dismisses it");
    settle(&mut session, &mut conn);
    assert!(
        drained(&rx).iter().any(|line| line == "select-pane -t %1"),
        "the click lands on the agent's pane"
    );
    // A later toast expires on its own.
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%3".to_string(),
        agent: "kimi".to_string(),
        state: "idle".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    });
    session.notify.states.push("idle".to_string());
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%3".to_string(),
        agent: "kimi".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    });
    assert!(session.renderer.toast.is_some());
    let until = session.notify.toast_until.expect("deadline set");
    session.tick_flash(until);
    assert!(session.renderer.toast.is_none(), "the toast expired");
}

/// A blocked push's `msg=` renders as the toast's reason line: the why
/// is the point of a blocked card (the TUI audit's F5 — the reason rode
/// the wire end to end and rendered nowhere).
#[test]
fn toast_renders_the_blocked_reason_line() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snapreason", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    session.refresh_status_facts(&mut conn).expect("seed");
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    drained(&rx);
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%1".to_string(),
        agent: "claude".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: "needs approval".to_string(),
    });
    let toast = session.renderer.toast.clone().expect("a toast is up");
    assert_eq!(toast.message.as_deref(), Some("needs approval"));
    session.renderer.render_frame();
    let (tx, ty, tw, th) = session.renderer.toast_rect().expect("on screen");
    assert_eq!(th, 5, "the reason adds the card's fourth row");
    let reason_row: String = (tx..tx + tw)
        .map(|x| {
            session
                .renderer
                .cell(x, ty + 3)
                .unwrap()
                .symbol()
                .to_string()
        })
        .collect();
    assert!(
        reason_row.contains("needs approval"),
        "the reason paints: {reason_row:?}"
    );
}

/// The lost-toast race (card 01a15b6f): the daemon applies a state
/// report, then broadcasts it, so a status refresh that runs between the
/// two (any `%sessions-changed`, a timed-out refresh's retry, the project
/// poll) answers with the NEW state while the push still waits in the
/// notification queue. The refresh must not consume the transition: the
/// push drained after it still toasts and rings the bell. The fake daemon
/// writes the push ahead of the snapshot reply — the wire order — and the
/// pump's own steps run in their own order (6 refresh, 9 drain). The
/// refresh now writes its snapshot without waiting (the async status
/// path); `settle` runs the reply's continuation, the same dispatch the
/// pump's step 1b performs, before the drain.
#[test]
fn a_refresh_that_already_saw_the_new_state_still_lets_its_push_toast() {
    use super::super::notify::NotifyDelivery;
    let mut script = snapshot_script();
    let snapshot = script
        .replies
        .get("client-snapshot -t @0")
        .expect("snapshot reply")
        .replace(
            "agent %1 claude working hook",
            "agent %1 claude blocked hook ws=+0",
        );
    script
        .replies
        .insert("client-snapshot -t @0".to_string(), snapshot);
    script.notify.insert(
        "client-snapshot -t @0".to_string(),
        "%agent-state-changed %1 claude blocked source=hook ws=+0".to_string(),
    );
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    // The roster before the report (claude working), committed by an
    // earlier refresh the way the session seed commits it.
    let (seed_rx, mut seed_conn) = fake_daemon("snaprace-seed", snapshot_script());
    session.refresh_status_facts(&mut seed_conn).expect("seed");
    drained(&seed_rx);
    let (rx, mut conn) = fake_daemon("snaprace", script);
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    drained(&rx);

    session.status_inflight.dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    settle(&mut session, &mut conn);
    assert_eq!(
        session.status.agents()[0].state,
        "blocked",
        "the refresh saw the new state first"
    );
    assert!(session.renderer.toast.is_none(), "a refresh never toasts");
    assert!(session.drain_pushes(&mut conn).is_continue());

    let toast = session
        .renderer
        .toast
        .clone()
        .expect("the push still toasts");
    assert_eq!(toast.title, "claude blocked");
    assert_eq!(toast.pane, Some(1));
    assert_eq!(session.notify.outbox, vec![0x07], "and rings the bell");
}

/// The fully async status path: the pump's snapshot writes without
/// waiting — a late reply parks no input — and the reply applies as a
/// continuation on the dispatch tick, committing the same facts the
/// sync refresh committed.
#[test]
fn status_snapshot_issues_without_blocking_and_applies_on_dispatch() {
    let mut script = snapshot_script();
    script.delays.insert(
        "client-snapshot -t @0".to_string(),
        std::time::Duration::from_millis(500),
    );
    let (rx, mut conn) = fake_daemon("asyncsnap", script);
    let mut session = snapshot_session();
    drained(&rx);

    session.status_inflight.dirty = true;
    let started = std::time::Instant::now();
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert!(
        started.elapsed() < std::time::Duration::from_millis(200),
        "the snapshot writes without waiting: {:?}",
        started.elapsed()
    );
    assert!(
        !session.status_inflight.dirty,
        "the issue consumed the dirty mark"
    );
    assert!(
        session.status_inflight.flight.is_some(),
        "the reply is owed: one snapshot in flight"
    );
    wait_recorded(&rx, "client-snapshot");
    assert!(
        drained(&rx).is_empty(),
        "one snapshot on the wire, nothing else"
    );

    settle(&mut session, &mut conn);
    assert_eq!(
        session.status.agents().len(),
        2,
        "the reply's roster committed"
    );
    assert!(
        session.status_inflight.flight.is_none(),
        "the application releases the in-flight slot"
    );
    assert!(
        session.pending_cmds.is_empty(),
        "the reply drained exactly once"
    );
}

/// The nothing-left-to-show `End` rides the continuation: the `%error`
/// snapshot reply lands `SessionGone`, the landing resolves nothing,
/// and the continuation raises the pump-visible `status_inflight.end` that the
/// status step checks on that tick.
#[test]
fn status_snapshot_end_lands_through_the_continuation() {
    let mut script = snapshot_script();
    script.failing.insert("client-snapshot -t @0".to_string());
    let (rx, mut conn) = fake_daemon("asyncend", script);
    let mut session = snapshot_session();
    drained(&rx);

    session.status_inflight.dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert!(
        !session.status_inflight.end,
        "nothing ends before the reply"
    );
    settle(&mut session, &mut conn);
    assert!(
        session.status_inflight.end,
        "the SessionGone landing found nothing left to show"
    );
}

/// The re-seed's status snapshot rides the same machinery the seed's
/// does: written without waiting — no inline snapshot round trip left
/// on snapshot daemons outside the explicit continuations — and the
/// reply applies as a pump-tick continuation committing the same facts
/// the sync refresh committed.
#[test]
fn a_reseed_issues_its_snapshot_without_blocking_and_applies_on_dispatch() {
    let mut script = snapshot_script();
    reseed_replies(&mut script.replies, "@0 * main", "@0", "%1\n%2");
    let (rx, mut conn) = fake_daemon("reseedasync", script);
    let mut session = snapshot_session();
    drained(&rx);

    session.reseed_window(&mut conn, "@0", false);
    assert_eq!(session.window, "@0", "the view re-seeded onto @0");
    assert!(
        session.status_inflight.flight.is_some(),
        "the snapshot rides the machinery, not an inline round trip"
    );
    assert!(
        !session.status_inflight.dirty,
        "the issue consumed the dirty mark"
    );
    let wire = received_within_300ms(&rx);
    assert_eq!(
        wire.iter()
            .filter(|l| l.starts_with("client-snapshot"))
            .count(),
        1,
        "one snapshot on the wire: {wire:?}"
    );

    settle(&mut session, &mut conn);
    assert!(
        session.status_inflight.flight.is_none(),
        "the application releases the in-flight slot"
    );
    assert_eq!(
        session.status.agents().len(),
        2,
        "the reply's roster committed"
    );
    assert!(
        session.notify.transitions.is_some(),
        "the transition baseline seeded"
    );
}

/// The cached-landing fixture: a session showing `@0` whose status
/// cache already lists `@1` beside it (the last snapshot's roster,
/// applied test-side), and a snapshot daemon whose `client-snapshot -t
/// @0` now refuses (the window is gone). `extra_failing` names the
/// landing queries the daemon also refuses.
fn cached_landing_fixture(
    tag: &str,
    extra_failing: &[&str],
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
    WindowSession,
) {
    let mut script = snapshot_script();
    script
        .replies
        .insert("list-panes -t @1".to_string(), "%4 0 *".to_string());
    script.notify.insert(
        "refresh-client -t %4 -C 90x24".to_string(),
        "%layout-change @1 0000,90x24,0,0,4 0000,90x24,0,0,4 *".to_string(),
    );
    script.failing.insert("client-snapshot -t @0".to_string());
    for line in extra_failing {
        script.failing.insert(line.to_string());
    }
    let (rx, conn) = fake_daemon(tag, script);
    let mut session = snapshot_session();
    session.status.apply_snapshot_for_test(&[
        "snapshot 1".to_string(),
        "workspace +0 * main".to_string(),
        "session $1 * +0 two".to_string(),
        "window @0 * edit".to_string(),
        "window @1 - logs".to_string(),
    ]);
    (rx, conn, session)
}

/// The SessionGone landing resolves from the last snapshot's cached
/// window list when it names a survivor of the gone window: the view
/// reseeds onto the cached sibling with no landing query on the wire,
/// so a pathological daemon parks nothing extra on the landing path.
#[test]
fn a_gone_window_lands_on_the_cached_sibling_without_queries() {
    let (rx, mut conn, mut session) = cached_landing_fixture("cachedlanding", &[]);
    drained(&rx);

    session.status_inflight.dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    settle(&mut session, &mut conn);
    let wire = drained(&rx);
    for query in [
        "list-windows",
        "list-workspaces",
        "list-sessions",
        "switch-client",
    ] {
        assert!(
            !wire.iter().any(|line| line.starts_with(query)),
            "the landing resolved from the cache: {query} never left the client"
        );
    }
    assert_eq!(
        session.window, "@1",
        "the view landed on the cached sibling"
    );
    assert!(
        !session.status_inflight.end,
        "a survivor existed, so the view stands"
    );
}

/// A cached landing that died before it could be shown (the session
/// itself is gone) falls to the interactive ladder: the failed reseed
/// reports the view did not move, the ladder runs, and nothing left to
/// show ends the view the usual way.
#[test]
fn a_dead_cached_landing_falls_back_to_the_interactive_ladder() {
    let (rx, mut conn, mut session) = cached_landing_fixture(
        "cachedeadlanding",
        &[
            "client-snapshot -t @1",
            "list-windows -t $1",
            "list-workspaces",
            "list-sessions -t +0",
        ],
    );
    drained(&rx);

    session.status_inflight.dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    settle(&mut session, &mut conn);
    let wire = drained(&rx);
    for query in [
        "list-windows -t $1",
        "list-workspaces",
        "list-sessions -t +0",
    ] {
        assert!(
            wire.iter().any(|line| line.starts_with(query)),
            "the ladder ran: {query} was asked"
        );
    }
    assert!(
        session.status_inflight.end,
        "the ladder found nothing left to show"
    );
}

/// The storm rule survives the async path: a snapshot still owed past
/// the status bound is abandoned in place — the stale bar stands, no
/// re-issue queues behind the wedged reply, and the late reply (still
/// token-aligned) applies when dispatched.
#[test]
fn status_snapshot_timeout_abandons_without_re_issuing() {
    let mut script = snapshot_script();
    script.delays.insert(
        "client-snapshot -t @0".to_string(),
        std::time::Duration::from_millis(900),
    );
    let (rx, mut conn) = fake_daemon("asyncstorm", script);
    let mut session = snapshot_session();
    drained(&rx);

    session.status_inflight.dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    wait_recorded(&rx, "client-snapshot");

    // Mid-flight dirty mark: nothing re-issues while the reply is owed,
    // and the mark survives for a re-issue once the reply applies. No
    // sleep gates the leg: only dispatch_replies applies the reply and
    // clears the slot, and none has run — the flag itself proves the
    // refresh cannot reach the abandon branch.
    assert!(session.status_inflight.flight.is_some());
    session.status_inflight.dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert!(
        session.status_inflight.dirty,
        "the mid-flight mark survives for the post-reply re-issue"
    );
    assert!(drained(&rx).is_empty(), "no re-issue while in flight");

    // Past the bound: the abandon clears the in-flight slot and the
    // dirty mark — nothing re-queues behind the wedged reply (the
    // storm rule; the next poll tick or event re-queries).
    let flight = session
        .status_inflight
        .flight
        .expect("the reply is still owed");
    session.status_inflight.flight = Some(flight.backdated(status::STATUS_TIMEOUT));
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert!(
        !session.status_inflight.dirty,
        "the abandon leaves the re-query to the next poll or event"
    );
    assert!(drained(&rx).is_empty(), "no re-issue after the abandon");

    // The late reply is still owed its continuation: it applies when
    // dispatched (tokens keep the reply stream aligned).
    settle(&mut session, &mut conn);
    assert_eq!(
        session.status.agents().len(),
        2,
        "the late reply applies once it lands"
    );
}
