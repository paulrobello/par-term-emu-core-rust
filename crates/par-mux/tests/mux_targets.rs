//! Name targets over the wire: `-t` accepts a pane's user title, a window
//! name, or a session name, resolved daemon-side against the real tree.
//! Ambiguity and ids-first are wire contracts, not resolver internals.

#![cfg(feature = "mux")]

mod common;

use common::{command, wait_for, MuxFixture};
use interprocess::TryClone as _;
use par_mux::mux::{connect_local_stream, MuxServer};
use std::io::BufReader;

/// The replies to one command, joined — assert against this.
fn ask(writer: &mut impl std::io::Write, reader: &mut impl std::io::BufRead, line: &str) -> String {
    command(writer, reader, line).join("")
}

/// A reply block's body lines: framing (`%begin`/`%end`) and interleaved
/// pushes (`%output`, …) stripped, body lines kept. Pane ids themselves
/// start with `%`, so the prefix alone cannot classify — framing is the
/// `%<word> ` shape, body lines never are.
fn body_lines(reply: &str) -> Vec<&str> {
    let framing = ["%begin", "%end", "%output", "%exit", "%error"];
    reply
        .lines()
        .filter(|l| !l.is_empty())
        .filter(|l| !framing.iter().any(|f| l.starts_with(f)))
        .collect()
}

/// The reply block's body only: the lines between `%begin` and `%end` —
/// pushes (`%layout-change`, …) that raced the reply are dropped, which
/// the `body_lines` prefix filter cannot do (they start with `%` too).
fn block_lines(reply: &str) -> Vec<&str> {
    reply
        .lines()
        .skip_while(|l| !l.starts_with("%begin"))
        .skip(1)
        .take_while(|l| !l.starts_with("%end"))
        .collect()
}

/// A pane target that is a user title resolves to the titled pane, and an
/// unknown name is a reported error, not a silent miss.
#[test]
fn pane_commands_accept_a_user_title_target() {
    let fixture = MuxFixture::new("ptitle");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    command(&mut writer, &mut reader, "new-session -s alpha");
    command(&mut writer, &mut reader, "split-window -t %0");
    command(&mut writer, &mut reader, "select-pane -t %1 -T build");

    // The name addresses exactly the pane carrying the title: pane-info's
    // reply echoes the RESOLVED id, so a wrong pick would show %0.
    let info = ask(&mut writer, &mut reader, "pane-info -t build");
    assert!(
        info.contains("%1"),
        "name resolved to the titled pane: {info}"
    );

    // A quoted name keeps its spaces, same grammar as -s/-n names.
    command(
        &mut writer,
        &mut reader,
        "select-pane -t %1 -T 'my build pane'",
    );
    let titled = ask(&mut writer, &mut reader, "pane-title -t 'my build pane'");
    assert!(
        titled.contains("my build pane"),
        "spaced name round-trips: {titled}"
    );

    // Unknown name: an error block, not an empty success.
    let missing = ask(&mut writer, &mut reader, "pane-info -t nosuch");
    assert!(
        missing.contains("%error") && missing.contains("no such pane: nosuch"),
        "unknown name errors: {missing}"
    );

    drop(writer);
    let _ = handle;
}

/// Window and session targets accept names: a window is addressable by its
/// name, and a session name picks the RIGHT session (observed through the
/// environment a pane spawned in that session inherits).
#[test]
fn window_and_session_commands_accept_name_targets() {
    let fixture = MuxFixture::new("wtitle");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    // Two sessions, alpha's pane %0 and beta's pane %1; distinct env per
    // session so a later spawn reveals WHICH session was targeted.
    command(&mut writer, &mut reader, "new-session -s alpha");
    command(&mut writer, &mut reader, "new-session -s beta");
    command(
        &mut writer,
        &mut reader,
        "set-environment -t alpha WHO alpha-a",
    );
    command(
        &mut writer,
        &mut reader,
        "set-environment -t beta WHO beta-b",
    );

    // new-window -n names its window; rename-window then addresses that
    // window BY NAME. The same call targets session alpha BY NAME, so the
    // window's pane %2 spawns inside alpha.
    command(&mut writer, &mut reader, "new-window -t alpha -n editor");
    let renamed = ask(&mut writer, &mut reader, "rename-window -t editor logs");
    assert!(
        !renamed.contains("%error"),
        "window name resolves: {renamed}"
    );
    let listed = ask(&mut writer, &mut reader, "list-windows");
    assert!(
        listed.contains("logs"),
        "rename landed on the named window: {listed}"
    );

    // The session-name proof: pane %2 was spawned by the `-t alpha`
    // new-window, so it carries alpha's env — the shell echoes alpha's
    // value. A wrong session pick (or the name silently failing) never
    // prints the marker. ($WHO on sh, %WHO% on cmd.exe — the typed echo
    // shows the unexpanded form on both.)
    #[cfg(unix)]
    let echo_mark = "echo MARK-$WHO";
    #[cfg(windows)]
    let echo_mark = "echo MARK-%WHO%";
    command(
        &mut writer,
        &mut reader,
        &format!("send-keys -t %2 '{echo_mark}' Enter"),
    );
    let output = wait_for(
        &mut writer,
        &mut reader,
        "capture-pane -t %2",
        "MARK-alpha-a",
    );
    assert!(
        !output.contains("MARK-beta-b"),
        "session name picked alpha, not beta: {output}"
    );

    // Unknown names error for both kinds.
    let no_window = ask(&mut writer, &mut reader, "select-window -t nosuch");
    assert!(
        no_window.contains("%error") && no_window.contains("no such window: nosuch"),
        "unknown window name errors: {no_window}"
    );
    let no_session = ask(&mut writer, &mut reader, "set-environment -t nosession K V");
    assert!(
        no_session.contains("%error") && no_session.contains("no such session: nosession"),
        "unknown session name errors: {no_session}"
    );

    drop(writer);
    let _ = handle;
}

/// A name held by more than one pane errors listing the candidate ids and
/// acts on nothing: an ambiguous kill leaves every candidate alive.
#[test]
fn ambiguous_pane_target_errors_and_acts_on_nothing() {
    let fixture = MuxFixture::new("pambig");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    command(&mut writer, &mut reader, "new-session -s alpha");
    command(&mut writer, &mut reader, "split-window -t %0");
    command(&mut writer, &mut reader, "select-pane -t %0 -T dup");
    command(&mut writer, &mut reader, "select-pane -t %1 -T dup");

    let kill = ask(&mut writer, &mut reader, "kill-pane -t dup");
    assert!(kill.contains("%error"), "ambiguity must fail: {kill}");
    assert!(
        kill.contains("%0") && kill.contains("%1"),
        "the error lists both candidate ids: {kill}"
    );
    assert!(
        kill.contains("ambiguous pane target: dup"),
        "the error names the ambiguity: {kill}"
    );

    // Acts on nothing: both panes still answer.
    for pane in ["%0", "%1"] {
        let info = ask(&mut writer, &mut reader, &format!("pane-info -t {pane}"));
        assert!(
            !info.contains("%error"),
            "pane {pane} must survive the refused kill: {info}"
        );
    }

    drop(writer);
    let _ = handle;
}

/// A window name held by two windows is ambiguous and refuses to act.
#[test]
fn ambiguous_window_target_errors_and_acts_on_nothing() {
    let fixture = MuxFixture::new("wambig");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    // new-session names the session's first window after the session, so
    // two sessions named alike start with two alike-named windows; rename
    // makes the collision explicit either way.
    command(&mut writer, &mut reader, "new-session -s work");
    command(&mut writer, &mut reader, "new-session -s other");
    command(&mut writer, &mut reader, "rename-window -t @1 work");

    let select = ask(&mut writer, &mut reader, "select-window -t work");
    assert!(
        select.contains("%error"),
        "window ambiguity must fail: {select}"
    );
    assert!(
        select.contains("@0") && select.contains("@1"),
        "the error lists both candidate windows: {select}"
    );

    drop(writer);
    let _ = handle;
}

/// The attach path's tree reconstruction: `list-windows -t <session>`
/// returns that session's windows in order with the active one marked, and
/// `list-panes -t <window>` returns a window's panes in layout leaf order
/// with the active one marked and a deterministic leaf index. The bare
/// forms keep their exact pre-existing shapes.
#[test]
fn targeted_list_windows_and_list_panes_reconstruct_the_tree() {
    let fixture = MuxFixture::new("treeq");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    // Session alpha: window @0 (pane %0), a split -> %1, then two more
    // windows @2 (pane %2) and @3 (pane %3). Session beta follows so the
    // scoping is visible: its windows must NOT leak into alpha's reply.
    command(&mut writer, &mut reader, "new-session -s alpha");
    command(&mut writer, &mut reader, "split-window -t %0");
    // %1 is active now; move it back so @0's marker is the asserted state.
    command(&mut writer, &mut reader, "select-pane -t %0");
    command(&mut writer, &mut reader, "new-window -t alpha -n second");
    command(&mut writer, &mut reader, "rename-window -t second two");
    command(&mut writer, &mut reader, "new-window -t alpha -n third");
    command(&mut writer, &mut reader, "select-window -t two");
    command(&mut writer, &mut reader, "new-session -s beta");
    command(&mut writer, &mut reader, "new-window -t beta -n bwin");

    // Window ids are daemon-monotonic, so alpha's windows are @0 (its
    // first pane's window), @1 (`second`), @2 (`third`); beta's is @3.
    // list-windows -t alpha: in session order with the active window @1
    // marked and the name as the line remainder.
    let listed = ask(&mut writer, &mut reader, "list-windows -t alpha");
    let lines: Vec<&str> = listed
        .lines()
        .filter(|l| !l.is_empty())
        .filter(|l| !l.starts_with('%'))
        .collect();
    assert_eq!(
        lines,
        vec!["@0 - alpha", "@1 * two", "@2 - third"],
        "session-scoped windows in order with the active marker: {listed}"
    );

    // The session target also works as a typed id, and beta's list does
    // not contain alpha's windows.
    let session_id = ask(&mut writer, &mut reader, "list-sessions")
        .lines()
        .find_map(|l| l.strip_prefix("$0: ").map(str::to_string))
        .filter(|name| name == "alpha")
        .map(|_| "$0".to_string());
    let by_id = ask(&mut writer, &mut reader, "list-windows -t $0");
    let strip = |text: String| -> Vec<String> {
        text.lines()
            .filter(|l| !l.is_empty() && !l.starts_with('%'))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        strip(by_id),
        strip(listed.clone()),
        "typed id and name targets agree"
    );
    let _ = session_id;
    let beta_listed = ask(&mut writer, &mut reader, "list-windows -t beta");
    assert!(
        beta_listed.contains("@4 - bwin") && !beta_listed.contains("@0"),
        "beta's list is scoped to beta (its windows are @3, @4): {beta_listed}"
    );

    // Wrong session name: an error block.
    let missing = ask(&mut writer, &mut reader, "list-windows -t nosuch");
    assert!(
        missing.contains("%error") && missing.contains("no such session: nosuch"),
        "unknown session errors: {missing}"
    );

    // list-panes -t @0: layout leaf order with the active pane marked.
    let panes = ask(&mut writer, &mut reader, "list-panes -t @0");
    let pane_lines: Vec<&str> = body_lines(&panes);
    assert_eq!(
        pane_lines,
        vec!["%0 0 *", "%1 1 -"],
        "window-scoped panes in leaf order with active marker and index: {panes}"
    );

    // The leaf index is deterministic across a re-split: split %0
    // (leaf 0) — the new pane takes leaf 1, %1 shifts to 2, %0 stays 0.
    command(&mut writer, &mut reader, "split-window -h -t %0");
    let panes = ask(&mut writer, &mut reader, "list-panes -t @0");
    let pane_lines: Vec<&str> = body_lines(&panes);
    // The split target keeps its leaf index (0) and its old neighbor shifts
    // to 2; the new pane takes leaf 1 and the focus (tmux semantics), so
    // the marker rides on it — the index is deterministic, the marker is
    // activity.
    assert_eq!(
        pane_lines[0], "%0 0 -",
        "the split target keeps its leaf index: {panes}"
    );
    assert_eq!(
        pane_lines[1], "%6 1 *",
        "the new pane takes leaf 1 and the focus: {panes}"
    );
    assert_eq!(
        pane_lines[2], "%1 2 -",
        "the old leaf 1 shifts to 2: {panes}"
    );
    assert_eq!(pane_lines.len(), 3, "the split added a pane: {panes}");

    // A dead/bare window id: an error block.
    let no_window = ask(&mut writer, &mut reader, "list-panes -t @999");
    assert!(
        no_window.contains("%error") && no_window.contains("no such window: @999"),
        "unknown window errors: {no_window}"
    );

    // The bare forms keep their pre-existing shapes exactly: every pane id
    // alone on its line; every window as `@N: name`.
    let global_panes = ask(&mut writer, &mut reader, "list-panes");
    for line in body_lines(&global_panes) {
        assert!(
            line.starts_with('%') && line.split_whitespace().count() == 1,
            "bare list-panes stays one id per line, got: {line:?}"
        );
    }
    let global_windows = ask(&mut writer, &mut reader, "list-windows");
    assert!(
        body_lines(&global_windows).iter().all(|l| l
            .split_once(": ")
            .map(|(id, _)| id.starts_with('@'))
            .unwrap_or(false)),
        "bare list-windows keeps the `@N: name` shape: {global_windows}"
    );
    assert_eq!(
        body_lines(&global_windows).len(),
        5,
        "every window across both sessions is listed: {global_windows}"
    );

    drop(writer);
    let _ = handle;
}

/// Ids win over names: a pane TITLED "%2" cannot capture the "%2" target —
/// the sigil prefix always means the id.
#[test]
fn typed_ids_win_over_names_shaped_like_ids() {
    let fixture = MuxFixture::new("idfirst");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    command(&mut writer, &mut reader, "new-session -s alpha");
    command(&mut writer, &mut reader, "split-window -t %0");
    command(&mut writer, &mut reader, "split-window -t %0");
    // Pane %0 now carries the user title "%2".
    command(&mut writer, &mut reader, "select-pane -t %0 -T '%2'");

    // The reply echoes the RESOLVED id: %2 the pane, never %0 the titled.
    let info = ask(&mut writer, &mut reader, "pane-info -t %2");
    assert!(
        info.contains("%2 @"),
        "the id target must resolve to pane %2, not the pane titled %2: {info}"
    );
    assert!(
        !info.contains("%0"),
        "the titled pane must not capture the id target: {info}"
    );

    drop(writer);
    let _ = handle;
}

/// tmux parity: `split-window` accepts window (`@N`) and session (`$N`)
/// targets — `@N` splits that window's active pane, `$N` the session's
/// active window's active pane — the reply body stays the new pane id, and
/// unknown ids error per kind.
#[test]
fn split_window_accepts_window_and_session_targets() {
    let fixture = MuxFixture::new("swtgt");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    // alpha: window @0 with panes %0, %1 (split); window @1 ("logs") with
    // panes %2, %3 (split), active via select-window below.
    command(&mut writer, &mut reader, "new-session -s alpha");
    command(&mut writer, &mut reader, "split-window -t %0");
    command(&mut writer, &mut reader, "new-window -t alpha -n logs");
    command(&mut writer, &mut reader, "split-window -t %2");
    command(&mut writer, &mut reader, "select-window -t @0");

    // @0's active pane is %1 — a @N target splits IT, so the new pane
    // lands in @0.
    let reply = ask(&mut writer, &mut reader, "split-window -t @0");
    let pane = block_lines(&reply)
        .last()
        .copied()
        .expect("the reply body is the new pane id")
        .to_string();
    assert!(
        pane.starts_with('%') && block_lines(&reply).len() == 1,
        "the reply body is exactly the new pane id: {reply}"
    );
    let info = ask(&mut writer, &mut reader, &format!("pane-info -t {pane}"));
    assert!(
        info.contains(&format!("{pane} @0 ")),
        "the @N split landed in window @0: {info}"
    );

    // A $N target splits the session's active window's active pane: alpha
    // is still on @0, so the same window again.
    let reply = ask(&mut writer, &mut reader, "split-window -t $0");
    let pane = block_lines(&reply)
        .last()
        .copied()
        .expect("the reply body is the new pane id")
        .to_string();
    let info = ask(&mut writer, &mut reader, &format!("pane-info -t {pane}"));
    assert!(
        info.contains(&format!("{pane} @0 ")),
        "the $N split followed alpha's active window @0: {info}"
    );

    // Selection moves the session target: with @1 active, a $N split
    // lands there, not in @0.
    command(&mut writer, &mut reader, "select-window -t @1");
    let reply = ask(&mut writer, &mut reader, "split-window -t $0");
    let pane = block_lines(&reply)
        .last()
        .copied()
        .expect("the reply body is the new pane id")
        .to_string();
    let info = ask(&mut writer, &mut reader, &format!("pane-info -t {pane}"));
    assert!(
        info.contains(&format!("{pane} @1 ")),
        "after selecting @1, the $N split follows it: {info}"
    );

    // Unknown ids error per kind.
    let reply = ask(&mut writer, &mut reader, "split-window -t @999");
    assert!(
        reply.contains("%error") && reply.contains("no such window: @999"),
        "unknown window target errors: {reply}"
    );
    let reply = ask(&mut writer, &mut reader, "split-window -t $999");
    assert!(
        reply.contains("%error") && reply.contains("no such session: $999"),
        "unknown session target errors: {reply}"
    );
    let reply = ask(&mut writer, &mut reader, "split-window -t %999");
    assert!(
        reply.contains("%error") && reply.contains("no such pane: %999"),
        "unknown pane target errors: {reply}"
    );

    drop(writer);
    let _ = handle;
}

/// tmux parity: `new-window` accepts window (`@N`) and pane (`%N`)
/// targets — the new window sits IMMEDIATELY AFTER that window in the
/// session's order (verified through `list-windows -t`), the reply body
/// stays the new window id, and unknown ids error per kind.
#[test]
fn new_window_window_and_pane_targets_insert_after_their_window() {
    let fixture = MuxFixture::new("nwtgt");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    // alpha: windows @0 ("alpha"), @1 ("two"), @2 ("three").
    command(&mut writer, &mut reader, "new-session -s alpha");
    command(&mut writer, &mut reader, "new-window -t alpha -n two");
    command(&mut writer, &mut reader, "new-window -t alpha -n three");

    // @N: insert right after window @0.
    let reply = ask(&mut writer, &mut reader, "new-window -t @0 -n ins");
    let window = block_lines(&reply)
        .last()
        .copied()
        .expect("the reply body is the new window id")
        .to_string();
    assert!(
        window.starts_with('@') && block_lines(&reply).len() == 1,
        "the reply body is exactly the new window id: {reply}"
    );
    let listed = ask(&mut writer, &mut reader, "list-windows -t alpha");
    let lines: Vec<&str> = listed
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('%'))
        .collect();
    assert_eq!(
        lines,
        vec!["@0 * alpha", "@3 - ins", "@1 - two", "@2 - three"],
        "the new window took @0's next slot in session order: {listed}"
    );

    // %N: the pane's window is the target, same insertion rule.
    let reply = ask(&mut writer, &mut reader, "new-window -t %1 -n ins-pane");
    assert!(
        block_lines(&reply).last() == Some(&"@4"),
        "the reply body is the new window id: {reply}"
    );
    let listed = ask(&mut writer, &mut reader, "list-windows -t alpha");
    let lines: Vec<&str> = listed
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('%'))
        .collect();
    assert_eq!(
        lines,
        vec![
            "@0 * alpha",
            "@3 - ins",
            "@1 - two",
            "@4 - ins-pane",
            "@2 - three"
        ],
        "the pane target inserted after its window @1: {listed}"
    );

    // The $N form keeps appending (and the active marker stays put —
    // par-mux's new-window never selects).
    command(&mut writer, &mut reader, "new-window -t $0 -n appended");
    let listed = ask(&mut writer, &mut reader, "list-windows -t alpha");
    let lines: Vec<&str> = listed
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('%'))
        .collect();
    assert_eq!(
        lines.last(),
        Some(&"@5 - appended"),
        "append stays last: {listed}"
    );

    // Unknown ids error per kind, and nothing was inserted.
    let reply = ask(&mut writer, &mut reader, "new-window -t @999");
    assert!(
        reply.contains("%error") && reply.contains("no such window: @999"),
        "unknown window target errors: {reply}"
    );
    let reply = ask(&mut writer, &mut reader, "new-window -t %999");
    assert!(
        reply.contains("%error") && reply.contains("no such pane: %999"),
        "unknown pane target errors: {reply}"
    );
    let listed = ask(&mut writer, &mut reader, "list-windows -t alpha");
    let count = listed
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('%'))
        .count();
    assert_eq!(count, 6, "the refused targets inserted nothing: {listed}");

    drop(writer);
    let _ = handle;
}
