//! Status snapshots, the session chooser, and legacy daemons.

use super::*;

/// A fake daemon answering from a script keyed by the FULL command line,
/// falling back to the same hardcoded defaults as [`serve_one`] (version,
/// list-commands, list-panes, refresh-client). Answers everything `ok`
/// unless the script value ends in `!ERR` (then an error block).
fn serve_scripted(
    stream: crate::mux::LocalStream,
    tx: std::sync::mpsc::Sender<(String, String)>,
    script: std::collections::HashMap<String, String>,
) {
    use interprocess::TryClone as _;
    use std::io::Write as _;
    let mut writer = stream.try_clone().expect("clone stream");
    let mut reader = BufReader::new(stream);
    let mut number = 0u32;
    let mut line = String::new();
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
        let (reply, ok) = match script.get(trimmed) {
            Some(body) if body.ends_with("!ERR") => (String::new(), false),
            Some(body) => (body.clone(), true),
            None => match name.as_str() {
                "version" => ("9.9.9+deadbeef".to_string(), true),
                "list-commands" => (
                    "list-commands\nfeatures replay-held-state\n".to_string(),
                    true,
                ),
                _ => (String::new(), true),
            },
        };
        number += 1;
        tx.send((name, trimmed.to_owned())).ok();
        writer
            .write_all(emit_block(number, &reply, ok).as_bytes())
            .ok();
        writer.flush().ok();
    }
}

/// Bind a [`FakeDaemon`] whose serving loop answers from `script` (full
/// command line → reply body).
fn scripted_daemon(
    tag: &str,
    script: std::collections::HashMap<String, String>,
) -> (FakeDaemon, PathBuf) {
    let path = test_socket(tag);
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind fake daemon");
    let (tx, received) = channel();
    std::thread::spawn(move || {
        #[cfg(unix)]
        use interprocess::local_socket::traits::Listener as _;
        if let Ok(stream) = listener.accept() {
            serve_scripted(stream, tx, script);
        }
    });
    (FakeDaemon { received }, path)
}

/// The snapshot body one refresh reads: 2 workspaces (+0 active), the
/// owner session `$1 two`, panes `%1`/`%2` titled, one agent on `%1`.
const PASSTHROUGH_SNAPSHOT: &str = "snapshot 1\nworkspace +0 * main\nworkspace +1 - side\nsession $0 - +0 one\nsession $1 * +0 two\nwindow @0 * edit\npane %1 build output\npane %2 tests\nagent %1 claude working hook\nagent %2 kimi blocked hook";

/// ENH-043 port (card 01a1216d4780): against a daemon advertising
/// `client-snapshot v1`, one passthrough status refresh is exactly one
/// round trip, and every fact lands from that one reply.
#[test]
fn passthrough_status_refresh_uses_client_snapshot_when_advertised() {
    let mut script = std::collections::HashMap::new();
    script.insert(
        "list-commands".to_string(),
        "client-snapshot v1\nlist-commands\nfeatures replay-held-state\n".to_string(),
    );
    script.insert(
        "client-snapshot -t @0".to_string(),
        PASSTHROUGH_SNAPSHOT.to_string(),
    );
    let (daemon, path) = scripted_daemon("snap", script);
    let conn = conn::AttachConn::connect(&path).expect("connect");
    let mut session = passthrough_session(conn, "@0", "%1");
    recorded(&daemon.received, 4); // handshake

    session.refresh_status();
    let lines = recorded(&daemon.received, 1);
    assert_eq!(
        lines,
        vec![(
            "client-snapshot".to_string(),
            "client-snapshot -t @0".to_string()
        )]
    );
    let quiet = daemon.received.try_recv().err().is_some();
    assert!(quiet, "no further query after the single snapshot round");
    assert_eq!(session.session_id.as_deref(), Some("$1"));
    assert_eq!(session.session_name, "two");
    assert_eq!(
        session.workspaces,
        vec![
            ("+0".to_string(), "main".to_string()),
            ("+1".to_string(), "side".to_string())
        ]
    );
    assert_eq!(session.active_workspace.as_deref(), Some("+0"));
    assert_eq!(session.pane_title, "build output");
    assert_eq!(
        session.agents, 1,
        "one agent row for %1; %2's row is not this pane's"
    );
}

/// Without the `client-snapshot v1` token the legacy per-fact round
/// still runs, and the client never sends `client-snapshot`.
#[test]
fn passthrough_status_refresh_keeps_the_legacy_round_without_the_feature() {
    let mut script = std::collections::HashMap::new();
    script.insert("pane-info -t %1".to_string(), "%1 @0 80x24".to_string());
    let (daemon, path) = scripted_daemon("snaplegacy", script);
    let conn = conn::AttachConn::connect(&path).expect("connect");
    let mut session = passthrough_session(conn, "@0", "%1");
    recorded(&daemon.received, 4);

    session.refresh_status();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut names = Vec::new();
    while std::time::Instant::now() < deadline {
        match daemon
            .received
            .recv_timeout(std::time::Duration::from_millis(300))
        {
            Ok((name, line)) => {
                names.push(name.clone());
                assert!(!line.starts_with("client-snapshot"), "{line}");
            }
            Err(_) => break,
        }
    }
    assert!(names.contains(&"pane-info".to_string()), "{names:?}");
    assert!(names.contains(&"list-sessions".to_string()), "{names:?}");
    assert!(names.contains(&"list-agents".to_string()), "{names:?}");
    assert_eq!(
        session.window, "@0",
        "pane-info's second field still sets the window"
    );
}

/// A daemon that advertises the feature but refuses the snapshot for the
/// tracked window (unknown/stale id) falls back to the legacy round,
/// which re-derives the window — never a loop.
#[test]
fn passthrough_status_snapshot_refusal_falls_back_to_legacy() {
    let mut script = std::collections::HashMap::new();
    script.insert(
        "list-commands".to_string(),
        "client-snapshot v1\nlist-commands\nfeatures replay-held-state\n".to_string(),
    );
    script.insert("client-snapshot -t @9".to_string(), "!ERR".to_string());
    script.insert("pane-info -t %1".to_string(), "%1 @0 80x24".to_string());
    let (daemon, path) = scripted_daemon("snaprefused", script);
    let conn = conn::AttachConn::connect(&path).expect("connect");
    let mut session = passthrough_session(conn, "@9", "%1");
    recorded(&daemon.received, 4);

    session.refresh_status();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut saw_snapshot = false;
    let mut saw_pane_info = false;
    while std::time::Instant::now() < deadline {
        match daemon
            .received
            .recv_timeout(std::time::Duration::from_millis(300))
        {
            Ok((name, _)) => {
                if name == "client-snapshot" {
                    saw_snapshot = true;
                }
                if name == "pane-info" {
                    saw_pane_info = true;
                }
            }
            Err(_) => break,
        }
    }
    assert!(saw_snapshot, "the advertised feature was tried first");
    assert!(saw_pane_info, "the refusal fell back to the legacy round");
    assert_eq!(
        session.window, "@0",
        "the legacy round re-derived the window"
    );
}

/// A scripted daemon query for the picker resolution tests: the
/// two-session roster (`list-windows -a` gives `$1` two windows), with
/// every command it was asked recorded in `asked`.
fn two_session_query(
    asked: &mut Vec<String>,
) -> impl FnMut(&str) -> std::io::Result<crate::mux::client::Reply> + '_ {
    move |command| {
        asked.push(command.to_string());
        let body = match command {
            "list-sessions" => "+0: default: $0: att\n+0: default: $1: second",
            "list-windows -a" => "$0 @0 * one\n$1 @1 * two\n$1 @2 - three",
            _ => "",
        };
        Ok(crate::mux::client::Reply {
            body: body.lines().map(str::to_string).collect(),
            ok: true,
        })
    }
}

/// The picker on: several sessions reach the chooser as rows with their
/// window counts, and its pick becomes the target.
#[test]
fn picked_target_hands_several_sessions_to_the_chooser() {
    let mut asked = Vec::new();
    let mut seen = Vec::new();
    let picked = picked_target(
        &mut two_session_query(&mut asked),
        None,
        true,
        false,
        &mut |rows| {
            seen = rows.to_vec();
            Some(1)
        },
    )
    .expect("picked");
    assert_eq!(picked.as_deref(), Some("$1"));
    let summary: Vec<(String, String, Option<usize>)> = seen
        .iter()
        .map(|r| (r.id.clone(), r.name.clone(), r.windows))
        .collect();
    assert_eq!(
        summary,
        [
            ("$0".to_string(), "att".to_string(), Some(1)),
            ("$1".to_string(), "second".to_string(), Some(2)),
        ]
    );
    assert_eq!(asked, ["list-sessions", "list-windows -a"], "reads only");
}

/// An abort is the refusal, byte for byte.
#[test]
fn picked_target_abort_is_the_refusal() {
    let mut asked = Vec::new();
    let err = picked_target(
        &mut two_session_query(&mut asked),
        None,
        true,
        false,
        &mut |_| None,
    )
    .expect_err("aborted");
    assert_eq!(
        err,
        multi_session_error(&[
            ("$0".to_string(), "att".to_string()),
            ("$1".to_string(), "second".to_string()),
        ])
    );
}

/// Inactive (the setting off, or a non-terminal stdio), an explicit
/// target, or a single session: the chooser never runs, and an inactive
/// picker queries nothing, so the target-less rule (and its refusal) is
/// untouched.
#[test]
fn picked_target_stays_out_of_the_way() {
    let mut never =
        |_: &[picker::PickerRow]| -> Option<usize> { panic!("the chooser must not run") };
    let mut asked = Vec::new();
    assert_eq!(
        picked_target(
            &mut two_session_query(&mut asked),
            None,
            false,
            false,
            &mut never
        ),
        Ok(None)
    );
    assert_eq!(
        picked_target(
            &mut two_session_query(&mut asked),
            Some("$0"),
            true,
            false,
            &mut never
        ),
        Ok(Some("$0".to_string()))
    );
    assert!(asked.is_empty(), "nothing queried: {asked:?}");
    let mut single = |_: &str| {
        Ok(crate::mux::client::Reply {
            body: vec!["$0: only".to_string()],
            ok: true,
        })
    };
    assert_eq!(
        picked_target(&mut single, None, true, false, &mut never),
        Ok(None)
    );
}

/// The workspace preference (card 01a1988e3): with several workspaces,
/// the one named `default` resolves to its first session.
#[test]
fn prefer_default_workspace_picks_the_default_workspaces_session() {
    let mut asked = Vec::new();
    let picked = picked_target(
        &mut |command: &str| {
            asked.push(command.to_string());
            let body = match command {
                "list-workspaces" => "+4: default active\n+9: kanban".to_string(),
                "list-sessions -t +4" => "+4: default: $10: default".to_string(),
                _ => String::new(),
            };
            Ok(crate::mux::client::Reply {
                body: body.lines().map(str::to_string).collect(),
                ok: true,
            })
        },
        None,
        false,
        true,
        &mut |_| panic!("the chooser must not run"),
    )
    .expect("picked");
    assert_eq!(picked.as_deref(), Some("$10"));
    assert_eq!(
        asked,
        ["list-workspaces", "list-sessions -t +4"],
        "reads only"
    );
}

/// No `default` workspace: the first listed one resolves instead.
#[test]
fn prefer_default_workspace_falls_back_to_the_first_workspace() {
    let picked = picked_target(
        &mut |command: &str| {
            let body = match command {
                "list-workspaces" => "+9: kanban\n+22: par-rt-db".to_string(),
                "list-sessions -t +9" => "+9: kanban: $21: kanban".to_string(),
                _ => String::new(),
            };
            Ok(crate::mux::client::Reply {
                body: body.lines().map(str::to_string).collect(),
                ok: true,
            })
        },
        None,
        false,
        true,
        &mut |_| panic!("the chooser must not run"),
    )
    .expect("picked");
    assert_eq!(picked.as_deref(), Some("$21"));
}

/// One workspace (or the option off) keeps the target-less rule: nothing
/// resolves, the chooser never runs, and nothing is queried.
#[test]
fn prefer_default_workspace_keeps_the_single_workspace_rule() {
    for on in [false, true] {
        let mut asked = Vec::new();
        let picked = picked_target(
            &mut |command: &str| {
                asked.push(command.to_string());
                let body = match command {
                    "list-workspaces" => "+4: default active".to_string(),
                    _ => String::new(),
                };
                Ok(crate::mux::client::Reply {
                    body: body.lines().map(str::to_string).collect(),
                    ok: true,
                })
            },
            None,
            false,
            on,
            &mut |_| panic!("the chooser must not run"),
        )
        .expect("picked");
        assert_eq!(picked, None);
        assert_eq!(asked, if on { vec!["list-workspaces"] } else { vec![] });
    }
}

/// prefix M-5 (ESC 5) sends `select-layout tiled -t <window>`; an unbound
/// Alt chord is consumed without reaching the pane.
#[test]
fn passthrough_m5_sends_select_layout() {
    let (rx, mut session) = management_session("m5-layout");
    assert!(!session.route_bytes(&[0x02, 0x1b, b'5']));
    assert_eq!(
        wait_for_line(&rx, "select-layout"),
        "select-layout tiled -t @0"
    );
    assert!(!session.route_bytes(&[0x02, 0x1b, b'7']));
    match rx.recv_timeout(std::time::Duration::from_millis(300)) {
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        other => panic!("unbound Alt chord must be consumed: {other:?}"),
    }
}

#[test]
fn passthrough_select_layout_refusal_flashes() {
    let (_rx, mut session) = scripted_passthrough(
        "pt-layout-refused",
        &[("select-layout tiled -t @0", "!err")],
    );
    assert!(!session.route_bytes(&[0x02, 0x1b, b'5']));
    assert_eq!(session.flash.as_deref(), Some("layout tiled: refused"));
}

/// ARC-001: a daemon older than the `version` verb answers it with the
/// unknown-command `%error`. The handshake must continue in compatibility
/// mode (no daemon stamp, no features, one skew note), never fail the
/// connect, and still send the later steps.
#[test]
fn legacy_daemon_without_version_degrades_instead_of_failing() {
    let path = test_socket("legacyversion");
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        if let Ok(stream) = listener.accept() {
            serve_legacy(stream, tx);
        }
    });
    let conn = conn::AttachConn::connect(&path).expect("a legacy daemon is still attachable");
    assert_eq!(conn.daemon_stamp(), None, "no stamp from a legacy daemon");
    assert!(conn.daemon_features().is_empty());
    assert!(conn.warnings().pre_feature_daemon);
    let note = conn
        .warnings()
        .stamp_mismatch
        .as_deref()
        .expect("the skew is reported once");
    assert!(note.contains("compatibility mode"), "got: {note}");
    let lines = recorded(&rx, 4);
    let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "version",
            "list-commands",
            "set-client-colors",
            "refresh-client"
        ],
        "the handshake continues past the refused verbs: {lines:?}"
    );
}

/// A daemon predating both `version` and `list-commands`: `%error` on
/// each, everything else answered.
fn serve_legacy(stream: crate::mux::LocalStream, tx: std::sync::mpsc::Sender<(String, String)>) {
    use interprocess::TryClone as _;
    let mut writer = stream.try_clone().expect("clone stream");
    let mut reader = BufReader::new(stream);
    let mut number = 0u32;
    let mut line = String::new();
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
        number += 1;
        tx.send((name.clone(), trimmed.to_owned())).ok();
        let (reply, ok) = match name.as_str() {
            "version" | "list-commands" => (format!("unknown command: {name}"), false),
            _ => (String::new(), true),
        };
        writer
            .write_all(emit_block(number, &reply, ok).as_bytes())
            .ok();
        writer.flush().ok();
    }
}
