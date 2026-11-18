//! WindowSession suites: notification transitions, sounds, and agent events.

use super::*;

/// A shown pane's death announces the respawn affordance even under the
/// default auto-remove, where the held exit cue never gets to show (the
/// TUI audit's F4).
#[test]
fn a_shown_pane_exit_flashes_the_respawn_opt_in() {
    let (rx, _conn) = fake_daemon("snapexit", snapshot_script());
    let mut session = snapshot_session();
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,1").expect("parses"));
    drained(&rx);
    session.on_pane_exited("%1", Some(7));
    let flash = session.chrome.flash_text().expect("the flash is up");
    assert!(flash.contains("exited (7)"), "{flash}");
    assert!(flash.contains("remain-on-exit"), "{flash}");
}

#[test]
fn agent_state_push_notifies_selected_transitions_only() {
    use super::super::notify::NotifyDelivery;
    let (_rx, mut conn) = fake_daemon("snapnotify", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;
    session.notify.host_is_kitty = false;
    session.notify.states = vec!["blocked".to_string(), "idle".to_string()];
    session.refresh_status_facts(&mut conn).expect("seed");
    assert!(
        session.notify.outbox.is_empty(),
        "the refresh that loads the roster notifies nothing"
    );
    let push = |session: &mut WindowSession, pane: &str, state: &str| {
        session.handle_event(TmuxNotification::AgentStateChanged {
            pane_id: pane.to_string(),
            agent: "claude".to_string(),
            state: state.to_string(),
            source: "hook".to_string(),
            ws: "+0".to_string(),
            msg: String::new(),
        });
    };
    // The snapshot session shows %1..%3, so their agents are in view and
    // never notify; drop the layout to put them in the background.
    push(&mut session, "%1", "blocked");
    assert!(
        session.notify.outbox.is_empty(),
        "an agent in the shown window does not notify"
    );
    session
        .renderer
        .apply_layout(parse_layout("0000,80x24,0,0,9").expect("parses"));
    // %1 claude blocked -> idle: selected, one OSC 9.
    push(&mut session, "%1", "idle");
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;par-mux: main ? claude: idle\x07"
    );
    assert!(session.notify.outbox.is_empty(), "the frame drained it");
    // The same state again, then an unselected one: nothing.
    push(&mut session, "%1", "idle");
    push(&mut session, "%1", "working");
    assert!(session.notify.outbox.is_empty());
    // Off: a selected transition queues nothing.
    session.notify.delivery = NotifyDelivery::Off;
    push(&mut session, "%1", "blocked");
    assert!(session.notify.outbox.is_empty());
}

/// The default `notify-states` selects `done` (finished unseen); the
/// daemon's seen flip (done -> idle, another client viewed the pane)
/// never notifies, even with `idle` selected.
#[test]
fn done_notifies_and_the_seen_flip_does_not() {
    use super::super::notify::NotifyDelivery;
    let (_rx, mut conn) = fake_daemon("snapdone", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;
    session.notify.host_is_kitty = false;
    session.notify.states = crate::mux::config::Chords::with_defaults().notify_states;
    assert_eq!(session.notify.states, vec!["blocked", "done"]);
    session.refresh_status_facts(&mut conn).expect("seed");
    session
        .renderer
        .apply_layout(parse_layout("0000,80x24,0,0,9").expect("parses"));
    let push = |session: &mut WindowSession, state: &str| {
        session.handle_event(TmuxNotification::AgentStateChanged {
            pane_id: "%1".to_string(),
            agent: "claude".to_string(),
            state: state.to_string(),
            source: "hook".to_string(),
            ws: "+0".to_string(),
            msg: String::new(),
        });
    };
    push(&mut session, "working");
    push(&mut session, "done");
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;par-mux: main ? claude: done\x07"
    );
    session.notify.states.push("idle".to_string());
    push(&mut session, "idle");
    assert!(
        session.notify.outbox.is_empty(),
        "done -> idle is the seen flip, not news"
    );
}

/// Sounds ride their own channel (herdr's `[ui.sound]`): with delivery
/// `off` a blocked push still plays the blocked file and a background
/// finish the done file; a muted agent and an unselected sound key stay
/// silent; a missing file falls back to the bell.
#[test]
fn transition_sounds_play_per_event_and_respect_the_mute_list() {
    use super::super::notify::NotifyDelivery;
    use crate::mux::attach::sound::SPAWNED;
    let (_rx, mut conn) = fake_daemon("snapsound", snapshot_script());
    let mut session = snapshot_session();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("done.wav"), b"RIFF").unwrap();
    std::fs::write(dir.path().join("blocked.wav"), b"RIFF").unwrap();
    session.settings_path = Some(dir.path().join("config.toml"));
    session.notify.delivery = NotifyDelivery::Off;
    session.notify.sound = crate::mux::config::SoundSettings {
        enabled: true,
        player: "true".to_string(),
        done: "done.wav".to_string(),
        blocked: "blocked.wav".to_string(),
        mute: vec!["droid".to_string()],
    };
    session.refresh_status_facts(&mut conn).expect("seed");
    session
        .renderer
        .apply_layout(parse_layout("0000,80x24,0,0,9").expect("parses"));
    let push = |session: &mut WindowSession, agent: &str, state: &str| {
        session.handle_event(TmuxNotification::AgentStateChanged {
            pane_id: "%1".to_string(),
            agent: agent.to_string(),
            state: state.to_string(),
            source: "hook".to_string(),
            ws: "+0".to_string(),
            msg: String::new(),
        });
    };
    let take = || SPAWNED.with(|s| std::mem::take(&mut *s.borrow_mut()));
    take();
    push(&mut session, "claude", "working");
    assert!(take().is_empty(), "working never sounds");
    push(&mut session, "claude", "blocked");
    push(&mut session, "claude", "done");
    let played: Vec<std::ffi::OsString> = take().into_iter().map(|argv| argv[1].clone()).collect();
    assert_eq!(
        played,
        vec![
            dir.path().join("blocked.wav").into_os_string(),
            dir.path().join("done.wav").into_os_string()
        ]
    );
    assert!(
        session.notify.outbox.is_empty(),
        "delivery off: no escape, no bell"
    );
    push(&mut session, "droid", "blocked");
    assert!(take().is_empty(), "a muted agent never sounds");
    // A missing custom file falls back to the built-in OS sound; with
    // no built-in on this host the bell rings in its place.
    session.notify.sound.mute.clear();
    session.notify.sound.blocked = "missing.wav".to_string();
    let builtin_present =
        crate::mux::attach::sound::builtin_file(crate::mux::attach::sound::Sound::Blocked)
            .is_some_and(|f| std::path::Path::new(f).is_file());
    push(&mut session, "codex", "working");
    push(&mut session, "codex", "blocked");
    if builtin_present {
        assert_eq!(take().len(), 1, "the built-in plays");
        assert!(session.notify.outbox.is_empty());
    } else {
        assert!(take().is_empty());
        assert_eq!(session.notify.outbox, vec![0x07], "the bell stands in");
    }
    session.notify.outbox.clear();
    session.notify.sound.enabled = false;
    push(&mut session, "codex", "working");
    push(&mut session, "codex", "blocked");
    assert!(take().is_empty(), "sound off");
    assert!(session.notify.outbox.is_empty());
}

/// The notification-jump chord (prefix `j`, herdr's
/// `open_notification_target`) focuses the pane behind the most recent
/// agent notification this client delivered and dismisses the toast;
/// with nothing delivered it flashes and sends nothing. A suppressed
/// transition (an in-view agent) never becomes the target.
#[test]
fn notify_jump_chord_focuses_the_last_notified_pane() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snapjump", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    session.refresh_status_facts(&mut conn).expect("seed");
    drained(&rx);
    assert_eq!(
        session.management.notify_jump,
        crate::mux::config::ChordKey::plain(b'j')
    );
    // Nothing delivered yet: a flash, no command.
    chord(&mut session, &mut conn, b'j');
    assert_eq!(
        session.chrome.flash_text(),
        Some("no notification to jump to")
    );
    assert!(
        !drained(&rx).iter().any(|l| l.starts_with("select-pane")),
        "no target, no select"
    );
    let push = |session: &mut WindowSession, pane: &str, state: &str| {
        session.handle_event(TmuxNotification::AgentStateChanged {
            pane_id: pane.to_string(),
            agent: "claude".to_string(),
            state: state.to_string(),
            source: "hook".to_string(),
            ws: "+0".to_string(),
            msg: String::new(),
        });
    };
    // %1 is in the shown window: no notification, so no target.
    push(&mut session, "%1", "blocked");
    assert!(session.notify.jump_targets.is_empty());
    // Background the agents; %3 then %1 notify — the latest wins.
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    push(&mut session, "%3", "working");
    push(&mut session, "%3", "blocked");
    push(&mut session, "%1", "working");
    assert_eq!(
        session.notify.jump_targets.last(),
        Some(&3),
        "unselected keeps it"
    );
    push(&mut session, "%1", "blocked");
    assert_eq!(session.notify.jump_targets.last(), Some(&1));
    assert!(session.renderer.toast.is_some());
    drained(&rx);
    chord(&mut session, &mut conn, b'j');
    assert!(session.renderer.toast.is_none(), "the jump dismisses it");
    assert!(
        drained(&rx).iter().any(|line| line == "select-pane -t %1"),
        "the chord lands on the last notified pane"
    );
    // The jump CONSUMED %1: a second jump walks back to %3 (herdr's
    // consume-and-promote), a third finds the backlog empty.
    chord(&mut session, &mut conn, b'j');
    assert!(
        drained(&rx).iter().any(|line| line == "select-pane -t %3"),
        "the second jump walks to the previous notification"
    );
    chord(&mut session, &mut conn, b'j');
    assert_eq!(
        session.chrome.flash_text(),
        Some("no notification to jump to")
    );
}

/// An ad-hoc `%notification` (`show-notification`) rides the client's
/// delivery channel with the script's own title — ignoring notify-states
/// and the background-only rule — never moves the jump target, and its
/// toast, which has no pane, only dismisses on a click.
#[test]
fn adhoc_notification_delivers_through_the_channel_without_a_target() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snapadhoc", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;
    session.notify.host_is_kitty = false;
    session.notify.states = Vec::new();
    session.refresh_status_facts(&mut conn).expect("seed");
    drained(&rx);
    let adhoc = |session: &mut WindowSession, body: &str| {
        // The delivery's 1/s rate gate (herdr's bound) is a clock, so the
        // test clears it between calls — each delivery below is its own
        // first.
        session.notify.last_adhoc_at = None;
        session.handle_event(TmuxNotification::Notification {
            title: "build failed".to_string(),
            body: body.to_string(),
        });
    };
    adhoc(&mut session, "3 errors");
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;build failed: 3 errors\x07",
        "the script's title, not par-mux"
    );
    adhoc(&mut session, "");
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;build failed\x07",
        "a title-only notification has no trailing separator"
    );
    session.notify.delivery = NotifyDelivery::Off;
    adhoc(&mut session, "x");
    assert!(session.notify.outbox.is_empty(), "off delivers nothing");
    session.notify.delivery = NotifyDelivery::Inside;
    adhoc(&mut session, "see the log");
    let toast = session.renderer.toast.clone().expect("a toast is up");
    assert_eq!(toast.title, "build failed");
    assert_eq!(toast.body, "see the log");
    assert_eq!(toast.pane, None);
    assert!(session.notify.jump_targets.is_empty(), "no jump target");
    let (tx, ty, _, _) = session.renderer.toast_rect().expect("on screen");
    let (hx, hy) = session.geometry.frame_to_host(tx + 2, ty + 1);
    session.route_mouse(&mut conn, sgr(0, hx + 1, hy + 1, false));
    settle(&mut session, &mut conn);
    assert!(session.renderer.toast.is_none(), "the click dismisses it");
    assert!(
        !drained(&rx).iter().any(|l| l.starts_with("select-pane")),
        "an ad-hoc toast has no pane to focus"
    );
}

/// `show-notification` is rate-limited to one per second per client —
/// herdr's own `API_NOTIFICATION_RATE_LIMIT` bound: a second delivery
/// inside the window is dropped with a status flash, whatever the
/// channel, so a looping script cannot spawn an osascript per call or
/// flood the terminal escapes.
#[test]
fn adhoc_notifications_are_rate_limited_to_one_per_second() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snaprate", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;
    session.notify.host_is_kitty = false;
    session.refresh_status_facts(&mut conn).expect("seed");
    drained(&rx);
    session.handle_event(TmuxNotification::Notification {
        title: "build failed".to_string(),
        body: "1".to_string(),
    });
    session.handle_event(TmuxNotification::Notification {
        title: "build failed".to_string(),
        body: "2".to_string(),
    });
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    let raw = String::from_utf8(sink.raw.clone()).unwrap();
    assert_eq!(
        raw.matches("\x1b]9;").count(),
        1,
        "only the first delivery inside the window lands: {raw:?}"
    );
    assert_eq!(
        session.chrome.flash_text(),
        Some("notification rate-limited (1/s)")
    );
}

/// The terminal delivery's wire shapes are pinned per host, not per
/// environment: under a kitty host the escape is kitty's chunked OSC 99
/// (title chunk, then body chunk), elsewhere plain OSC 9 with the title
/// leading. The four OSC 9 assertions above set the resolved flag
/// explicitly for the same reason — whatever terminal launched
/// `cargo test` must not choose which assertions hold.
#[test]
fn terminal_delivery_shapes_are_pinned_per_host_not_per_environment() {
    use super::super::notify::NotifyDelivery;
    let (_rx, _conn) = fake_daemon("snapkitty", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;

    session.notify.host_is_kitty = true;
    session.handle_event(TmuxNotification::Notification {
        title: "build failed".to_string(),
        body: "3 errors".to_string(),
    });
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]99;i=1:d=0;build failed\x1b\\\x1b]99;i=1:p=body;3 errors\x1b\\",
        "under kitty the delivery is chunked OSC 99, title then body"
    );

    session.notify.host_is_kitty = false;
    session.notify.last_adhoc_at = None;
    session.handle_event(TmuxNotification::Notification {
        title: "build failed".to_string(),
        body: "3 errors".to_string(),
    });
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;build failed: 3 errors\x07",
        "off kitty the delivery is plain OSC 9, the title leading"
    );
}

/// An agent whose project the probe has not reported yet keeps the
/// roster polling on a slow cadence (the probe broadcasts nothing), and
/// the poll stops once every agent has one.
#[test]
fn missing_project_polls_the_roster_until_it_arrives() {
    let (_rx, mut conn) = fake_daemon("snapproject", snapshot_script());
    let mut session = snapshot_session();
    session.refresh_status_facts(&mut conn).expect("seed");
    let start = session.project_poll;
    assert!(
        !session.project_poll_due(start),
        "the clock starts at construction"
    );
    let later = start + super::super::sidebar::PROJECT_POLL_INTERVAL;
    assert!(session.project_poll_due(later), "an agent lacks a project");
    assert!(
        !session.project_poll_due(later),
        "the poll restarted the clock"
    );
    for pane in [1, 3] {
        session
            .status
            .patch_agent(&format!("%{pane}"), "x", Some("idle"), "+0", "");
    }
    session.status.set_projects_for_test("par-term");
    let much_later = later + super::super::sidebar::PROJECT_POLL_INTERVAL;
    assert!(
        !session.project_poll_due(much_later),
        "every agent has its project: no poll"
    );
}

/// `%agent-state-changed` and `%agent-released` patch the roster from
/// their payload and rebuild the side panel's agents section from the
/// cache: no status mark, no query (ARC-136).
#[test]
fn agent_event_patches_without_a_query() {
    let (rx, mut conn) = fake_daemon("snapagent", snapshot_script());
    let mut session = snapshot_session();
    session.refresh_status_facts(&mut conn).expect("seed");
    session.status_inflight.dirty = false;
    drained(&rx);

    let rows = |session: &WindowSession| -> Vec<SidebarRow> {
        session
            .renderer
            .sidebar_sections
            .iter()
            .flatten()
            .flat_map(|section| section.rows.clone())
            .collect()
    };
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%3".to_string(),
        agent: "kimi".to_string(),
        state: "working".to_string(),
        source: "hook".to_string(),
        ws: "+1".to_string(),
        msg: String::new(),
    });
    assert!(!session.status_inflight.dirty);
    assert!(
        rows(&session).contains(&arow(3, Some("side"), "kimi", "working", false)),
        "the ws token groups the inserted row under its workspace: {:?}",
        rows(&session)
    );
    assert!(
        !rows(&session)
            .iter()
            .any(|row| row.status == crate::mux::attach::panels::AgentStatus::Blocked),
        "{:?}",
        rows(&session)
    );

    session.handle_event(TmuxNotification::AgentReleased {
        pane_id: "%3".to_string(),
        agent: "kimi".to_string(),
    });
    assert!(!session.status_inflight.dirty);
    assert!(
        !rows(&session)
            .iter()
            .any(|row| row.id == SidebarRowId::Pane(3)),
        "{:?}",
        rows(&session)
    );
    assert!(received_within_300ms(&rx).is_empty(), "no round trip");
}

/// The bar never renders telemetry, so a telemetry sample leaves the
/// status clean.
#[test]
fn telemetry_event_does_not_mark_status_dirty() {
    let mut session = snapshot_session();
    session.status_inflight.dirty = false;
    session.handle_event(TmuxNotification::AgentTelemetryChanged {
        pane_id: "%1".to_string(),
        agent: "claude".to_string(),
    });
    assert!(!session.status_inflight.dirty);
}

/// `%window-renamed` and a set `%pane-title-changed` patch the tab and
/// the focused pane's title from their payloads, no status mark.
#[test]
fn rename_and_title_events_patch_without_a_query() {
    let (rx, mut conn) = fake_daemon("snaprename", snapshot_script());
    let mut session = snapshot_session();
    session.refresh_status_facts(&mut conn).expect("seed");
    session.status_inflight.dirty = false;
    drained(&rx);

    session.handle_event(TmuxNotification::WindowRenamed {
        window_id: "@1".to_string(),
        name: "tail".to_string(),
    });
    session.handle_event(TmuxNotification::PaneTitleChanged {
        pane_id: "%1".to_string(),
        title: "deploy".to_string(),
    });
    assert!(!session.status_inflight.dirty);
    assert_eq!(session.status.windows()[1].1, "tail");
    assert_eq!(session.status.pane_title(), "deploy");
    assert_eq!(session.renderer.border_label(1), "deploy");
    assert!(received_within_300ms(&rx).is_empty(), "no round trip");
}
