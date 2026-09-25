//! Name targets over the wire: `-t` accepts a pane's user title, a window
//! name, or a session name, resolved daemon-side against the real tree.
//! Ambiguity and ids-first are wire contracts, not resolver internals.

#![cfg(feature = "mux")]

mod common;

use common::{command, wait_for, MuxFixture};
use interprocess::TryClone as _;
use par_term_emu_core_rust::mux::{connect_local_stream, MuxServer};
use std::io::BufReader;

/// The replies to one command, joined — assert against this.
fn ask(writer: &mut impl std::io::Write, reader: &mut impl std::io::BufRead, line: &str) -> String {
    command(writer, reader, line).join("")
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
    // prints the marker.
    command(
        &mut writer,
        &mut reader,
        "send-keys -t %2 'echo MARK-$WHO' Enter",
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
