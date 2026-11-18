//! Passthrough window/session/workspace switching and paste handling.

use super::*;

/// The wire lines a passthrough action sent that select or land
/// (the status re-query burst filtered out), in order.
fn switch_lines(rx: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
    rx.try_iter()
        .filter(|l| {
            l.starts_with("select-")
                || l.starts_with("switch-client")
                || l.starts_with("list-panes -t")
                || l.starts_with("list-windows -t")
                || l.starts_with("list-sessions -t")
        })
        .collect()
}

/// Passthrough prefix n / p: the neighbor window (wrapping) is
/// landed on (`switch-client`, ARC-124) and the pump attaches its `*`-marked pane; an unlisted
/// shown window moves nowhere.
#[test]
fn passthrough_window_switch_wraps_and_attaches_the_marked_pane() {
    let (rx, mut session) = scripted_passthrough(
        "pt-win",
        &[
            ("list-sessions", "$0: work"),
            ("list-windows -t $0", "@0 * a\n@1 - b\n@2 - c"),
            ("list-panes -t @2", "%4 0 -\n%5 1 *"),
            ("pane-info -t %5", "%5 @2 80x24"),
        ],
    );
    assert!(!session.route_bytes(&[0x02, b'p']));
    assert_eq!(
        switch_lines(&rx)[..4].to_vec(),
        vec![
            "list-windows -t $0",
            "switch-client -t @2",
            "list-panes -t @2",
            "select-pane -t %5"
        ],
        "p from the first window wraps to the last"
    );
    assert_eq!(session.window, "@2");
    assert_eq!(session.pane, "%5", "the pump follows the marked pane");

    assert_eq!(session.session_id.as_deref(), Some("$0"));
    session.window = "@9".to_string();
    assert!(!session.route_bytes(&[0x02, b'n']));
    assert_eq!(switch_lines(&rx), vec!["list-windows -t $0".to_string()]);
    assert_eq!(session.pane, "%5", "an unlisted window moves nowhere");
}

/// Passthrough prefix ( / ): the neighbor session in roster order
/// (wrapping), its `*` window, that window's marked pane. The current
/// session counts only while it owns the shown window (the
/// `list-windows -t <current>` probe, ARC-139 — the render mode's rule);
/// a stale or unknown current falls back to the roster head.
#[test]
fn passthrough_session_switch_lands_on_the_neighbors_active_pane() {
    let (rx, mut session) = scripted_passthrough(
        "pt-sess",
        &[
            ("list-sessions", "$0: work\n$1: lab"),
            ("list-windows -t $0", "@0 * a"),
            ("list-windows -t $1", "@3 - x\n@4 * y"),
            ("list-panes -t @4", "%8 0 *"),
        ],
    );
    assert!(!session.route_bytes(&[0x02, b'(']));
    let lines = switch_lines(&rx);
    assert_eq!(
        lines[..4].to_vec(),
        vec![
            "list-windows -t $0",
            "list-windows -t $1",
            "switch-client -t @4",
            "list-panes -t @4"
        ],
        "( from the head wraps to $1: {lines:?}"
    );
    assert_eq!(session.session_id.as_deref(), Some("$1"));
    assert_eq!(session.window, "@4");
    assert_eq!(session.pane, "%8");

    // A stale current ($7 owns nothing): the head ($0) is the base, so
    // ( wraps to $1 again rather than freezing the view.
    session.session_id = Some("$7".to_string());
    assert!(!session.route_bytes(&[0x02, b'(']));
    let lines = switch_lines(&rx);
    assert_eq!(
        lines[..3].to_vec(),
        vec![
            "list-windows -t $7",
            "list-windows -t $1",
            "switch-client -t @4"
        ],
        "a stale current falls back to the head: {lines:?}"
    );
    assert_eq!(session.session_id.as_deref(), Some("$1"));
}

/// ARC-124: a landing the daemon refuses (`switch-client` AND the
/// `select-window` fallback both error) leaves the pump's window,
/// session, and pane put — the session switch used to discard the
/// select reply and move local state anyway.
#[test]
fn passthrough_refused_landing_leaves_local_state_put() {
    let (rx, mut session) = scripted_passthrough(
        "pt-refused",
        &[
            ("list-sessions", "$0: work\n$1: lab"),
            ("list-windows -t $0", "@0 * a"),
            ("list-windows -t $1", "@4 * y"),
            ("switch-client -t @4", "!err"),
            ("select-window -t @4", "!err"),
        ],
    );
    assert!(!session.route_bytes(&[0x02, b')']));
    let lines = switch_lines(&rx);
    assert_eq!(
        lines,
        vec![
            "list-windows -t $0",
            "list-windows -t $1",
            "switch-client -t @4",
            "select-window -t @4"
        ],
        "both landings tried, nothing after: {lines:?}"
    );
    assert_eq!(session.session_id.as_deref(), Some("$0"));
    assert_eq!(session.window, "@0");
    assert_eq!(session.pane, "%0");
}

/// Passthrough prefix o: the next pane of the window (wrapping) by
/// the daemon's list; a single-pane window does nothing.
#[test]
fn passthrough_cycle_moves_to_the_next_listed_pane() {
    let (rx, mut session) =
        scripted_passthrough("pt-cycle", &[("list-panes -t @0", "%0 0 *\n%1 1 -")]);
    assert!(!session.route_bytes(&[0x02, b'o']));
    assert_eq!(
        switch_lines(&rx)[..2].to_vec(),
        vec!["list-panes -t @0", "select-pane -t %1"]
    );
    assert_eq!(session.pane, "%1");
    assert!(!session.route_bytes(&[0x02, b'o']));
    assert_eq!(switch_lines(&rx)[1], "select-pane -t %0", "wraps");

    let (rx, mut session) = scripted_passthrough("pt-cycle1", &[("list-panes -t @0", "%0 0 *")]);
    assert!(!session.route_bytes(&[0x02, b'o']));
    assert_eq!(switch_lines(&rx), vec!["list-panes -t @0".to_string()]);
    assert_eq!(session.pane, "%0");
}

/// Passthrough prefix W: the next workspace is selected and the pump
/// lands in its first session's active window's marked pane; a
/// workspace with no sessions only moves the selection.
#[test]
fn passthrough_workspace_switch_lands_in_the_next_workspace() {
    let (rx, mut session) = scripted_passthrough(
        "pt-ws",
        &[
            ("list-workspaces", "+0: main active\n+1: lab"),
            ("list-sessions -t +1", "+1: lab: $2: build"),
            ("list-sessions", "+1: lab: $2: build"),
            ("switch-client", "$2"),
            ("list-windows -t $2", "@6 * logs"),
            ("list-panes -t @6", "%9 0 *"),
        ],
    );
    assert!(!session.route_bytes(&[0x02, b'W']));
    assert_eq!(
        switch_lines(&rx)[..6].to_vec(),
        vec![
            "select-workspace -t +1",
            "list-sessions -t +1",
            "switch-client",
            "list-windows -t $2",
            "switch-client -t @6",
            "list-panes -t @6"
        ]
    );
    assert_eq!(session.session_id.as_deref(), Some("$2"));
    assert_eq!(session.window, "@6");
    assert_eq!(session.pane, "%9");

    let (rx, mut session) = scripted_passthrough(
        "pt-ws-empty",
        &[("list-workspaces", "+0: main active\n+1: lab")],
    );
    assert!(!session.route_bytes(&[0x02, 0x17]));
    let lines = switch_lines(&rx);
    assert_eq!(
        lines[..2].to_vec(),
        vec!["select-workspace -t +1", "list-sessions -t +1"],
        "C-w from +0 wraps to +1: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with("select-window")),
        "an empty workspace lands nowhere: {lines:?}"
    );
    assert_eq!(session.pane, "%0");
}

/// ARC-139 decision 3 (card 01a11bd1): a workspace landing goes to the
/// session `select-workspace` resumed (the bare `switch-client` query),
/// not the first listed one — the render mode's rule, now shared.
#[test]
fn passthrough_workspace_switch_lands_on_the_resumed_session() {
    let (rx, mut session) = scripted_passthrough(
        "pt-ws-resumed",
        &[
            ("list-workspaces", "+0: main active\n+1: lab"),
            (
                "list-sessions -t +1",
                "+1: lab: $2: build\n+1: lab: $3: deploy",
            ),
            ("list-sessions", "+1: lab: $3: deploy"),
            ("switch-client", "$3"),
            ("list-windows -t $3", "@7 * shell"),
            ("list-panes -t @7", "%11 0 *"),
        ],
    );
    assert!(!session.route_bytes(&[0x02, b'W']));
    let lines = switch_lines(&rx);
    assert!(
        lines.contains(&"list-windows -t $3".to_string())
            && !lines.contains(&"list-windows -t $2".to_string()),
        "the resumed session, not the first listed: {lines:?}"
    );
    assert_eq!(session.session_id.as_deref(), Some("$3"));
    assert_eq!(session.window, "@7");
    assert_eq!(session.pane, "%11");
}

/// ARC-139: the shared switch resolvers — window and session cycles
/// wrap both ways, an unlisted shown window or an empty roster resolves
/// to nothing, and a stale current session falls back to the head.
#[test]
fn passthrough_switch_resolvers_wrap_and_fall_back() {
    let (_rx, mut session) = scripted_passthrough(
        "pt-resolvers",
        &[
            ("list-windows -t $0", "@0 * a\n@1 - b\n@2 - c"),
            ("list-sessions", "$0: work\n$1: lab\n$2: ops"),
            ("list-windows -t $1", "@4 * y"),
            ("list-windows -t $2", "@8 * z"),
        ],
    );
    let conn = &mut session.conn;
    let window = |w: &str| Landing {
        window: w.to_string(),
        session: None,
    };
    assert_eq!(
        window_switch_target(conn, "$0", "@0", true).unwrap(),
        Some(window("@1"))
    );
    assert_eq!(
        window_switch_target(conn, "$0", "@0", false).unwrap(),
        Some(window("@2"))
    );
    assert_eq!(
        window_switch_target(conn, "$0", "@2", true).unwrap(),
        Some(window("@0"))
    );
    assert_eq!(
        window_switch_target(conn, "$0", "@9", true).unwrap(),
        None,
        "unlisted"
    );
    assert_eq!(
        window_switch_target(conn, "$5", "@0", true).unwrap(),
        None,
        "no windows"
    );

    let landed = |w: &str, s: &str| Landing {
        window: w.to_string(),
        session: Some(s.to_string()),
    };
    assert_eq!(
        session_switch_target(conn, Some("$0"), "@0", true).unwrap(),
        Some(landed("@4", "$1"))
    );
    assert_eq!(
        session_switch_target(conn, Some("$0"), "@0", false).unwrap(),
        Some(landed("@8", "$2")),
        "wraps backward"
    );
    assert_eq!(
        session_switch_target(conn, Some("$1"), "@0", true).unwrap(),
        Some(landed("@4", "$1")),
        "$1 does not own @0: stale, so the head ($0) is the base"
    );
    assert_eq!(
        session_switch_target(conn, None, "@0", true).unwrap(),
        Some(landed("@4", "$1")),
        "no current: the head is the base"
    );

    let (_rx, mut empty) = scripted_passthrough("pt-resolvers-empty", &[]);
    assert_eq!(
        session_switch_target(&mut empty.conn, Some("$0"), "@0", true).unwrap(),
        None,
        "empty roster"
    );
    assert_eq!(
        workspace_session(&mut empty.conn, "+1").unwrap(),
        WorkspaceLanding::Empty
    );
}

/// The prefix chord is visible in passthrough too: the status line
/// says `prefix` while the second key is pending (tmux's status-line
/// prefix indication) and drops the cue once the chord completes.
#[test]
fn passthrough_prefix_arms_the_status_row_cue() {
    let (_rx, mut session) = management_session("prefix-cue");
    assert!(!session.route_bytes(&[0x02]));
    assert!(session.prefix_pending, "the chord is armed");
    assert!(
        session.status_line().contains(" prefix |"),
        "the armed prefix leads the line: {}",
        session.status_line()
    );
    assert!(!session.route_bytes(b"Q"));
    assert!(
        !session.prefix_pending,
        "the unbound key consumed the chord"
    );
    assert!(
        !session.status_line().contains("prefix"),
        "the completed chord clears the cue"
    );
}

/// SEC-208 (d): passthrough tracks bracketed paste over the raw stream —
/// a paste split across two bursts forwards whole (markers included, the
/// pane's own framing) and its embedded prefix + `x` never kills a pane.
#[test]
fn passthrough_paste_body_skips_the_prefix_scan() {
    let (rx, mut session) = management_session("paste-pt");
    assert!(!session.route_bytes(b"\x1b[200~ab"));
    assert!(!session.route_bytes(b"\x02xcd\x1b[201~"));
    assert!(
        !session.prefix_pending,
        "a pasted prefix byte never arms the prefix"
    );
    // After the paste closes, the prefix routes again.
    assert!(
        session.route_bytes(&[0x02, b'd']),
        "prefix d detaches after the paste"
    );
    // `send_checked` returns after the reply, and the fake records each
    // line before replying, so every line is already queued.
    let lines: Vec<String> = rx.try_iter().map(|(_, line)| line).collect();
    assert!(
        !lines.iter().any(|l| l.starts_with("kill-pane")),
        "the pasted chord did not fire: {lines:?}"
    );
    let sends: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("send-keys"))
        .collect();
    assert_eq!(
        sends,
        vec![
            "send-keys -t %0 -H 1b 5b 32 30 30 7e 61 62",
            "send-keys -t %0 -H 02 78 63 64 1b 5b 32 30 31 7e",
        ],
        "{lines:?}"
    );
}

/// IME / CJK input after the prefix (passthrough): a non-ASCII chord
/// key is swallowed whole, even split across bursts, so no torn UTF-8
/// tail reaches the pane; a fullwidth form (an IME in fullwidth mode)
/// folds to its ASCII chord; CJK typed without the prefix forwards
/// verbatim.
#[test]
fn passthrough_non_ascii_chord_key_is_swallowed_whole_or_folded() {
    let (rx, mut session) = management_session("cjk-prefix-pt");
    let kanji = "中".as_bytes();
    assert!(!session.route_bytes(&[0x02, kanji[0]]));
    assert!(!session.route_bytes(&kanji[1..]));
    assert!(!session.prefix_pending, "the character ends the chord");
    assert!(
        session
            .flash
            .as_deref()
            .is_some_and(|f| f.contains("ASCII")),
        "the hint names the fix: {:?}",
        session.flash
    );
    assert!(!session.route_bytes("日本".as_bytes()));
    // Fullwidth ｄ (U+FF44) after the prefix is prefix d: detach.
    let mut chord = vec![0x02];
    chord.extend_from_slice("ｄ".as_bytes());
    assert!(session.route_bytes(&chord), "fullwidth d detaches");
    let lines: Vec<String> = rx.try_iter().map(|(_, line)| line).collect();
    let sends: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("send-keys"))
        .collect();
    assert_eq!(
        sends,
        vec!["send-keys -t %0 -H e6 97 a5 e6 9c ac"],
        "only the unprefixed CJK reached the pane: {lines:?}"
    );
}

/// The passthrough status row fills its width in CELLS: a CJK session
/// name (two columns per glyph) is truncated and padded so the row
/// never wraps past the host's last column.
#[test]
fn passthrough_status_row_fits_cjk_names_by_display_width() {
    let (_rx, mut session) = management_session("cjk-status-pt");
    session.session_name = "日本語のセッション".repeat(8);
    let bytes = session.status_draw_bytes(24, 40);
    let text = String::from_utf8(bytes).expect("utf-8");
    let row = text
        .split("\x1b[7m")
        .nth(1)
        .and_then(|rest| rest.split("\x1b[0m").next())
        .expect("the reverse-video row");
    assert_eq!(
        crate::mux::attach::text_input::display_width(row),
        39,
        "{row:?}"
    );
}

/// SEC-208 (d'): a marker split across bursts still flips the paste
/// state (the five-byte carry), and the carry stays bounded.
#[test]
fn passthrough_paste_markers_split_across_bursts_are_recognized() {
    let mut tracker = PasteTracker::default();
    for &b in b"\x1b[20" {
        tracker.observe(b);
    }
    assert!(!tracker.in_paste);
    for &b in b"0~" {
        tracker.observe(b);
    }
    assert!(tracker.in_paste, "split opener recognized");
    for &b in b"body\x1b[2" {
        tracker.observe(b);
    }
    assert!(tracker.in_paste);
    for &b in b"01~" {
        tracker.observe(b);
    }
    assert!(!tracker.in_paste, "split terminator recognized");
    assert!(
        tracker.carry.len() < PASTE_START.len(),
        "the carry stays bounded"
    );
}

/// SEC-209 defence in depth: an OSC 0/2 pane title (pane output, not
/// sanitized at the daemon) and names carrying controls never reach the
/// host through the passthrough status row.
#[test]
fn status_line_strips_control_characters() {
    let (_rx, mut session) = management_session("status-strip");
    session.session_name = "wo\x1b[2Jrk".to_string();
    session.workspaces = vec![("+0".to_string(), "ma\x07in".to_string())];
    session.active_workspace = Some("+0".to_string());
    session.pane_title = "t\x1b]52;c;aGk=\x07\u{9b}x".to_string();
    let line = session.status_line();
    assert!(!line.chars().any(char::is_control), "{line:?}");
    assert!(
        line.contains("[main]") && line.contains("wo[2Jrk") && line.contains("t]52;c;aGk=x"),
        "{line:?}"
    );
}
