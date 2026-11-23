//! Split/kill/new-window/resize/swap management chords.

use super::*;

/// prefix %: `split-window -t %0 -h` rides the wire, and the pump
/// lands on the new pane — select-pane %1 plus the switch contract's
/// refresh-client resync, with the fresh pane now the Session's
/// target (subsequent typing forwards to it).
#[test]
fn split_right_chord_sends_split_window_and_lands_on_the_new_pane() {
    let (rx, mut session) = management_session("split-h");
    assert!(!session.route_bytes(&[0x02, b'%']));
    let lines = recorded(&rx, 6);
    let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "version",
            "list-commands",
            "set-client-colors",
            "refresh-client",
            "split-window",
            "select-pane",
        ],
        "the chord then the landing: {lines:?}"
    );
    assert_eq!(
        lines[4].1, "split-window -t %0 -h",
        "the split targets the FOCUSED pane with -h: {lines:?}"
    );
    assert_eq!(
        lines[5].1, "select-pane -t %1",
        "the pump followed the new pane from the reply: {lines:?}"
    );
    // Forwarding follows the landing: the switch contract's resync
    // burst (refresh-client, the status re-queries) rides first; the
    // next plain byte lands on the fresh pane %1.
    assert!(!session.route_bytes(b"q"));
    let keys = wait_for_line(&rx, "send-keys");
    assert_eq!(
        keys, "send-keys -t %1 -H 71",
        "typing forwards to the landed pane"
    );
}

/// prefix ": the vertical spelling (`split-window -t %0` with no -h)
/// and the same land-on-the-new-pane contract.
#[test]
fn split_down_chord_sends_the_vertical_split_spelling() {
    let (rx, mut session) = management_session("split-v");
    assert!(!session.route_bytes(&[0x02, b'"']));
    let lines = recorded(&rx, 6);
    assert_eq!(
        lines[4].1, "split-window -t %0",
        "the default direction rides bare (below): {lines:?}"
    );
    assert_eq!(lines[5].1, "select-pane -t %1", "lands on the new pane");
}

/// prefix x: `kill-pane -t %0` rides the wire and the pump follows
/// the window's surviving active pane.
#[test]
fn kill_chord_sends_kill_pane_and_lands_on_the_survivor() {
    let (rx, mut session) = management_session("kill");
    assert!(!session.route_bytes(&[0x02, b'x']));
    let lines = recorded(&rx, 7);
    let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "version",
            "list-commands",
            "set-client-colors",
            "refresh-client",
            "kill-pane",
            "list-panes",
            "select-pane",
        ],
        "kill, survivor lookup, landing: {lines:?}"
    );
    assert_eq!(lines[4].1, "kill-pane -t %0");
    assert_eq!(
        lines[5].1, "list-panes -t @0",
        "the survivor query targets the focused pane's window: {lines:?}"
    );
    assert_eq!(
        lines[6].1, "select-pane -t %1",
        "the *-marked survivor wins the landing: {lines:?}"
    );
}

/// prefix x on the window's LAST pane: no survivor — the dead guard
/// engages (the exited cue shows; typing is dropped, prefix chords
/// keep routing) instead of the pump forwarding into a corpse.
#[test]
fn kill_of_the_last_pane_engages_the_dead_guard() {
    let path = test_socket("kill-last");
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let (tx, rx) = channel();
    let sender = tx.clone();
    std::thread::spawn(move || {
        if let Ok(stream) = listener.accept() {
            use interprocess::TryClone as _;
            let mut writer = stream.try_clone().expect("clone stream");
            let mut reader = BufReader::new(stream);
            let mut number = 0u32;
            let mut line = String::new();
            let tx = sender;
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    continue;
                }
                let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
                let reply = match name.as_str() {
                    "version" => "9.9.9+deadbeef".to_string(),
                    "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
                    "kill-pane -t %0" => String::new(),
                    _ => String::new(),
                };
                // The survivor query (list-panes -t @0) is the one
                // command this script errors on: an %error block, no
                // body, so `marked_pane` fails and the guard engages.
                let ok = name != "list-panes";
                if !ok {
                    number += 1;
                    tx.send((name.clone(), trimmed.to_owned())).ok();
                    writer
                        .write_all(emit_block(number, "can't identify a pane", false).as_bytes())
                        .ok();
                    writer.flush().ok();
                    continue;
                }
                number += 1;
                tx.send((name, trimmed.to_owned())).ok();
                writer
                    .write_all(emit_block(number, &reply, true).as_bytes())
                    .ok();
                writer.flush().ok();
            }
        }
    });
    let conn = conn::AttachConn::connect(&path).expect("connect");
    drop(tx);
    let mut session = Session {
        conn,
        socket_path: path,
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: "@0".to_string(),
        session_id: Some("$0".to_string()),
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
        output_fence: None,
    };
    session.drawn_size = Some((80, 24));
    assert!(!session.route_bytes(&[0x02, b'x']));
    assert!(
        session.exited.is_some(),
        "no survivor -> the dead guard engages"
    );
    // The dead guard drops typing but keeps prefix chords live.
    assert!(!session.route_bytes(b"typing"), "no detach, bytes dropped");
    assert!(
        session.route_bytes(&[0x02, b'd']),
        "prefix d still detaches"
    );
    drop(rx);
}

/// prefix c: `new-window -t $0` rides the wire, the fresh window is
/// landed on (`switch-client`, ARC-124), and the pump attaches to its active pane — the same
/// follow the window-switch chords make.
#[test]
fn new_window_chord_follows_the_new_windows_active_pane() {
    let (rx, mut session) = management_session("newwin");
    assert!(!session.route_bytes(&[0x02, b'c']));
    let lines = recorded(&rx, 7);
    let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "version",
            "list-commands",
            "set-client-colors",
            "refresh-client",
            "new-window",
            "switch-client",
            "list-panes",
        ],
        "new-window, landing, active-pane lookup: {lines:?}"
    );
    assert_eq!(lines[4].1, "new-window -t $0");
    assert_eq!(
        lines[5].1, "switch-client -t @1",
        "the fresh window: {lines:?}"
    );
    // The active-pane lookup ran against the NEW window (its marked
    // pane %1 from the scripted table), and the Session's window
    // tracks it — the next switch_window list-windows query targets
    // @1 through session_id.
    assert_eq!(lines[6].1, "list-panes -t @1");
    assert_eq!(session.window, "@1");
    // And the pump landed on the new window's active pane.
    assert_eq!(session.pane, "%1");
}

/// Config: a chord override parses and routes — `%` remapped to `s`
/// splits, and the default `%` no longer intercepts (it forwards as
/// an ordinary byte through the dead-conn fallback).
#[test]
fn chord_override_remaps_the_split_chord() {
    let file: crate::mux::config::ConfigFile = toml::from_str(
        "[client]\nsplit-right = \"s\"\nsplit-down = \"v\"\nkill-pane = \"K\"\nnew-window = \"w\"\n",
    )
    .expect("parse");
    let (rx, mut session) = management_session("remap");
    let chords = crate::mux::config::reload_client_chords(
        &file,
        &crate::mux::config::Chords {
            prefix: session.prefix,
            reload: session.reload_key,
            management: session.management,
            ..crate::mux::config::Chords::with_defaults()
        },
    )
    .expect("chords parse");
    session.management = chords.management;
    assert!(
        !session.route_bytes(&[0x02, b's']),
        "the REMAPPED key splits"
    );
    let split = wait_for_line(&rx, "split-window");
    assert_eq!(
        split, "split-window -t %0 -h",
        "the remap targets %0 with -h"
    );
    // The landing burst ends with refresh_status's roster query;
    // after it drains, the wire is quiet.
    let _ = wait_for_line(&rx, "list-agents");
    // The OLD default key `%` no longer intercepts — and being
    // unbound now, the fixed table consumes it silently (the
    // unknown-chord rule: ignored), so nothing rides the wire.
    assert!(!session.route_bytes(&[0x02, b'%']));
    match rx.recv_timeout(std::time::Duration::from_millis(300)) {
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        other => panic!("the old key must be consumed, not acted on: {other:?}"),
    }
}

/// prefix R (the resize chord) enters the sticky resize mode; each
/// arrow sends the wire's relative resize-pane for the FOCUSED pane
/// at the configured step; q exits and normal forwarding resumes.
#[test]
fn resize_chord_arrows_send_resize_pane_and_q_exits() {
    let (rx, mut session) = management_session("resize-chord");
    assert!(
        !session.route_bytes(&[0x02, b'R']),
        "the resize chord enters the mode"
    );
    assert!(session.resize_mode, "the mode is sticky");
    // Right arrow: the wire's relative form at the default step 1.
    assert!(!session.route_bytes(b"\x1b[C"));
    assert_eq!(
        wait_for_line(&rx, "resize-pane"),
        "resize-pane -t %0 -R 1",
        "the right arrow resizes the focused pane's right edge"
    );
    // Up arrow: -U at the same step.
    assert!(!session.route_bytes(b"\x1b[A"));
    assert_eq!(
        wait_for_line(&rx, "resize-pane"),
        "resize-pane -t %0 -U 1",
        "the up arrow is -U"
    );
    // q exits; typing forwards again afterward.
    assert!(!session.route_bytes(b"q"));
    assert!(!session.resize_mode);
    assert!(!session.route_bytes(b"z"));
    assert_eq!(
        wait_for_line(&rx, "send-keys"),
        "send-keys -t %0 -H 7a",
        "typing forwards after the mode exits"
    );
}

/// In resize mode, a non-arrow key exits the mode and is REPROCESSED
/// by the normal router (the cancelling key still does its job — as
/// plain typing, since chords always need the prefix).
#[test]
fn resize_mode_cancelling_key_reprocesses_normally() {
    let (rx, mut session) = management_session("resize-cancel");
    assert!(!session.route_bytes(&[0x02, b'R']));
    // 'x' exits and reprocesses: a plain byte forwards to the pane.
    assert!(!session.route_bytes(b"x"));
    assert!(!session.resize_mode);
    assert_eq!(
        wait_for_line(&rx, "send-keys"),
        "send-keys -t %0 -H 78",
        "the cancelling key routed through the normal router"
    );
}

/// Config overrides: a resize-step of 3 rides each arrow, and a
/// remapped resize chord (Z) enters the mode while the old default
/// key is consumed unbound.
#[test]
fn resize_chord_and_step_follow_the_config() {
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nresize = \"Z\"\nresize-step = 3\n").expect("parse");
    let (rx, mut session) = management_session("resize-cfg");
    let chords = crate::mux::config::reload_client_chords(
        &file,
        &crate::mux::config::Chords {
            prefix: session.prefix,
            reload: session.reload_key,
            management: session.management,
            ..crate::mux::config::Chords::with_defaults()
        },
    )
    .expect("chords parse");
    session.management = chords.management;
    session.resize_step = chords.resize_step;
    assert!(!session.route_bytes(&[0x02, b'Z']));
    assert!(session.resize_mode, "the REMAPPED key enters the mode");
    assert!(!session.route_bytes(b"\x1b[C"));
    assert_eq!(
        wait_for_line(&rx, "resize-pane"),
        "resize-pane -t %0 -R 3",
        "the configured step rides the arrow"
    );
    // The old default key R is consumed unbound (nothing rides the
    // wire); drain the burst first, then require quiet.
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}
    assert!(!session.route_bytes(&[0x02, b'R']));
    match rx.recv_timeout(std::time::Duration::from_millis(300)) {
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        other => panic!("the old key must be consumed, not acted on: {other:?}"),
    }
}

/// prefix {: swap with the layout-order neighbor — `swap-pane -s
/// <focused> -t <prev>` — from the same list-panes roster cycle_pane
/// walks.
#[test]
fn swap_prev_chord_sends_swap_pane() {
    let (rx, mut session) = management_session("swap-prev");
    assert!(!session.route_bytes(&[0x02, b'{']));
    let line = wait_for_line(&rx, "swap-pane");
    assert_eq!(
        line, "swap-pane -s %0 -t %1",
        "focused %0 swaps with its next-roster neighbor (wraps to %1)"
    );
}
