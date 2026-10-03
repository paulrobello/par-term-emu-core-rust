//! The restart arc (Phase 3, Task 3.6): what one daemon saved, the next
//! daemon on the same socket serves back — layout, pane ids, screen content
//! and scrollback survive, and the processes behind the panes are new.
//!
//! Unix-only: the stop is a SIGTERM, and the process-identity probe asks the
//! shells themselves (`$$`), which is only meaningful on Unix.

#![cfg(all(feature = "mux", unix))]

// ARC-106: cargo sets CARGO_BIN_EXE_par-mux even when the bin's
// required-features are unmet, so a plain-`mux` build would silently exec a
// stale target/debug/par-mux. Fail loudly instead.
#[cfg(not(feature = "mux-bin"))]
compile_error!("this test drives the par-mux binary: build it with --features mux-bin");

mod common;

use common::{
    command, pane_ids, sigterm_clean, spawn_daemon, wait_for, wait_for_pid, wait_listening,
    MuxFixture,
};
use interprocess::TryClone as _;
use par_term_emu_core_rust::mux::connect_local_stream;
use std::io::BufReader;

/// Pane titles survive a restart: the save carries the user title, and the
/// next daemon on the same socket reports it through the `pane-title`
/// query without waiting for a change to push.
#[test]
fn a_restart_restores_pane_titles() {
    let fixture = MuxFixture::new("title");
    let path = fixture.socket();

    let mut first = spawn_daemon(&fixture);
    wait_listening(path);
    {
        let stream = connect_local_stream(path).expect("first daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        command(&mut writer, &mut reader, "new-session -s titled");
        let pane = pane_ids(&command(&mut writer, &mut reader, "list-panes").join(""))
            .first()
            .expect("new-session created a pane")
            .clone();
        command(
            &mut writer,
            &mut reader,
            &format!("select-pane -t {pane} -T 'survives restarts'"),
        );
        let before = command(&mut writer, &mut reader, &format!("pane-title -t {pane}")).join("");
        assert!(
            before.lines().any(|l| l.trim() == "survives restarts"),
            "title set before the restart: {before}"
        );
    }
    sigterm_clean(&mut first);

    let mut second = spawn_daemon(&fixture);
    wait_listening(path);
    {
        let stream = connect_local_stream(path).expect("second daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        let listed = command(&mut writer, &mut reader, "list-panes").join("");
        let pane = pane_ids(&listed)
            .first()
            .expect("the pane survived the restart")
            .clone();
        let after = command(&mut writer, &mut reader, &format!("pane-title -t {pane}")).join("");
        assert!(
            after.lines().any(|l| l.trim() == "survives restarts"),
            "the user title survives the restart: {after}"
        );
    }
    sigterm_clean(&mut second);
}

/// Task 3.6: one daemon builds a session with a split, screen content and
/// scrollback; a SIGTERM stops it; a second daemon on the SAME socket must
/// serve the saved tree back — same session, same pane ids, same screen and
/// history — while the processes behind the panes are new (D3.5's scope
/// honesty, asserted by asking the shells for their pids).
#[test]
fn a_restart_serves_the_saved_tree_with_new_processes() {
    let fixture = MuxFixture::new("arc");
    let path = fixture.socket();

    // First daemon: a split layout with real content in both panes.
    let mut first = spawn_daemon(&fixture);
    wait_listening(path);
    let stream = connect_local_stream(path).expect("first daemon accepts");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(&mut writer, &mut reader, "new-session -s restart");
    let first_pane = pane_ids(&command(&mut writer, &mut reader, "list-panes").join(""))
        .first()
        .expect("new-session created a pane")
        .clone();
    let second_pane = pane_ids(
        &command(
            &mut writer,
            &mut reader,
            &format!("split-window -t {first_pane} -h"),
        )
        .join(""),
    )
    .first()
    .expect("split-window replies with the new pane id")
    .clone();

    // 30 output lines on a 24-row pane: the early lines land in history, the
    // pid line stays on screen. `$$` is the shell's own pid — POSIX-portable,
    // and printf pads the counter so `ZQX-HIST-03` cannot substring-match
    // `ZQX-HIST-30`.
    command(
        &mut writer,
        &mut reader,
        &format!(
            "send-keys -t {first_pane} 'for i in $(seq 1 30); do printf \"ZQX-HIST-%02d\\n\" $i; done' Enter"
        ),
    );
    command(
        &mut writer,
        &mut reader,
        &format!("send-keys -t {first_pane} 'echo ZQX-FIRST-PID $$' Enter"),
    );
    command(
        &mut writer,
        &mut reader,
        &format!("send-keys -t {second_pane} 'echo ZQX-RIGHT-PANE' Enter"),
    );

    let old_pid = wait_for_pid(&mut writer, &mut reader, &first_pane, "ZQX-FIRST-PID");
    // Ground the scrollback BEFORE the stop: ZQX-HIST-03 must already be off
    // the screen and inside the capture range, or the post-restart
    // assertion would prove nothing about the restart. The visible-screen
    // check reads the reply BODY — interleaved %output pushes replay the
    // pane's whole byte stream and would trivially contain the marker. It
    // uses capture-pane's default range (the visible screen): the
    // refresh-client reseed also replays the scrollback.
    wait_for(
        &mut writer,
        &mut reader,
        &format!("capture-pane -t {first_pane} -p -S -30"),
        "ZQX-HIST-03",
    );
    let visible = command(
        &mut writer,
        &mut reader,
        &format!("capture-pane -t {first_pane} -p"),
    )
    .join("")
    .lines()
    .filter(|l| !l.starts_with("%output"))
    .collect::<Vec<_>>()
    .join("\n");
    assert!(
        !visible.contains("ZQX-HIST-03"),
        "the history marker starts off-screen: {visible}"
    );
    wait_for(
        &mut writer,
        &mut reader,
        &format!("refresh-client -t {second_pane}"),
        "ZQX-RIGHT-PANE",
    );
    drop((writer, reader));

    // Clean stop: the SIGTERM save is the only one that captured the
    // send-keys content (content commands do not save per-dispatch, D3.3).
    sigterm_clean(&mut first);

    // Second daemon on the same socket.
    let mut second = spawn_daemon(&fixture);
    wait_listening(path);
    let stream = connect_local_stream(path).expect("second daemon accepts");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    // Layout and ids survived.
    let sessions = command(&mut writer, &mut reader, "list-sessions").join("");
    assert!(
        sessions.contains("restart"),
        "the session survived: {sessions}"
    );
    let panes = command(&mut writer, &mut reader, "list-panes").join("");
    assert!(
        panes.contains(&first_pane),
        "pane {first_pane} kept its id: {panes}"
    );
    assert!(panes.contains(&second_pane), "the split survived: {panes}");

    // Screen content and scrollback survived.
    let screen = command(
        &mut writer,
        &mut reader,
        &format!("refresh-client -t {first_pane}"),
    )
    .join("");
    assert!(
        screen.contains("ZQX-FIRST-PID"),
        "the saved screen came back: {screen}"
    );
    let history = command(
        &mut writer,
        &mut reader,
        &format!("capture-pane -t {first_pane} -p -S -30"),
    )
    .join("");
    assert!(
        history.contains("ZQX-HIST-03"),
        "the saved scrollback came back: {history}"
    );
    let right = command(
        &mut writer,
        &mut reader,
        &format!("refresh-client -t {second_pane}"),
    )
    .join("");
    assert!(
        right.contains("ZQX-RIGHT-PANE"),
        "the second pane's content came back: {right}"
    );

    // Processes are new: the restored pane runs a fresh shell, whose pid
    // must differ from the one the pre-restart shell printed.
    command(
        &mut writer,
        &mut reader,
        &format!("send-keys -t {first_pane} 'echo ZQX-SECOND-PID $$' Enter"),
    );
    let screen = wait_for_pid(&mut writer, &mut reader, &first_pane, "ZQX-SECOND-PID");
    assert_ne!(
        old_pid, screen,
        "the pane's process is new, not the pre-restart shell"
    );

    // Ids also survive the allocator: the NEXT pane is %2, above the
    // restored %0/%1 — restored identities and new ones cannot collide
    // (D3.5).
    let resplit = command(
        &mut writer,
        &mut reader,
        &format!("split-window -t {first_pane} -v"),
    )
    .join("");
    let next = pane_ids(&resplit)
        .first()
        .expect("split replies with the new id")
        .clone();
    assert_eq!(
        next, "%2",
        "the allocator resumed above the restored ids: {resplit}"
    );

    drop((writer, reader));
    sigterm_clean(&mut second);
}

/// The shutdown race: a burst of pane exits (children exiting with code 1,
/// the shape a SIGHUP'd shell presents — not a signal the daemon could
/// attribute), persisted by the reaper, with the daemon's SIGTERM landing
/// after. The next daemon must serve the PRE-EXIT layout, restored from the
/// last-good snapshot that reap saves never touch — not the raced
/// emptiness the old final save would have locked in.
#[test]
fn a_shutdown_race_restores_the_pre_exit_layout() {
    let fixture = MuxFixture::new("race");
    let path = fixture.socket();

    // Two panes — the structural saves put the 2-pane layout in the
    // last-good snapshot.
    let mut first = spawn_daemon(&fixture);
    wait_listening(path);
    let stream = connect_local_stream(path).expect("first daemon accepts");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(&mut writer, &mut reader, "new-session -s raced");
    let left = pane_ids(&command(&mut writer, &mut reader, "list-panes").join(""))
        .first()
        .expect("new-session created a pane")
        .clone();
    let right = pane_ids(
        &command(
            &mut writer,
            &mut reader,
            &format!("split-window -t {left} -h"),
        )
        .join(""),
    )
    .first()
    .expect("split-window replies with the new pane id")
    .clone();

    // Both shells exit with code 1 — a non-signal exit.
    command(
        &mut writer,
        &mut reader,
        &format!("send-keys -t {left} 'exit 1' Enter"),
    );
    command(
        &mut writer,
        &mut reader,
        &format!("send-keys -t {right} 'exit 1' Enter"),
    );

    // The race's losing branch: SIGTERM racing the reaper's death persist.
    // Dead panes are HELD now (remain-on-exit) — the tree never empties
    // on its own, so every interleaving of this race restores both panes.
    // Wait out the reaper by observation, not a fixed sleep: pane-info
    // flags a held-dead pane ` exited=`, and flagging happens in the very
    // reap pass that enqueues the death persist — so once BOTH panes show
    // it, the state file's mtime must move past that moment before
    // SIGTERM, and the exercised interleaving is the one where the deaths
    // ARE persisted first.
    let flagged_at = {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let left_info =
                command(&mut writer, &mut reader, &format!("pane-info -t {left}")).join("");
            let right_info =
                command(&mut writer, &mut reader, &format!("pane-info -t {right}")).join("");
            if left_info.contains(" exited=") && right_info.contains(" exited=") {
                break std::time::SystemTime::now();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the reaper never flagged both panes dead: {left_info}{right_info}"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    };
    {
        let state = fixture.state_path();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        // A structural command enqueues a save whose capture happens NOW —
        // strictly after both panes were flagged dead — so its write is a
        // post-death save by construction, whatever the reaper's own save
        // (same pass as the flag) already wrote before the flag was seen.
        command(&mut writer, &mut reader, "set-buffer reaped");
        loop {
            let written = std::fs::metadata(&state)
                .and_then(|meta| meta.modified())
                .is_ok_and(|mtime| mtime > flagged_at);
            if written {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the death persist never landed in {}",
                state.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
    drop((writer, reader));

    // The raced stop: SIGTERM after the burst. Its final save is empty and
    // must leave the last-good snapshot alone.
    sigterm_clean(&mut first);

    // The next daemon restores the pre-exit layout, both pane ids intact.
    let mut second = spawn_daemon(&fixture);
    wait_listening(path);
    {
        let stream = connect_local_stream(path).expect("second daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        let panes = command(&mut writer, &mut reader, "list-panes").join("");
        assert!(
            panes.contains(&left) && panes.contains(&right),
            "the pre-exit layout survived the raced shutdown: {panes}"
        );
    }
    sigterm_clean(&mut second);
}

// ---------------------------------------------------------------------------
// ARC-114: a pane held dead at save time comes back held dead.
// ---------------------------------------------------------------------------

use par_term_emu_core_rust::mux::MuxClient;
use par_term_emu_core_rust::tmux_control::TmuxNotification;
use std::time::{Duration, Instant};

/// Run a first daemon whose only pane prints a marker and exits with code 3,
/// wait until the reaper has flagged it held (`pane-info ... exited=3`) and
/// the marker is on the frozen screen, then SIGTERM it with a client still
/// connected so the final save carries the dead pane. Returns after the
/// daemon has exited; the fixture's state dir holds the save.
fn stop_a_daemon_holding_a_dead_pane(fixture: &MuxFixture) {
    let path = fixture.socket();
    let mut first = spawn_daemon(fixture);
    wait_listening(path);
    let stream = connect_local_stream(path).expect("first daemon accepts");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(&mut writer, &mut reader, "new-session -s held");
    // The sentinel makes a re-run distinguishable: a daemon that respawned
    // this pane on restore would run the command a second time and exit 9.
    let ran = fixture.state_dir().with_file_name("ran-once");
    command(
        &mut writer,
        &mut reader,
        &format!(
            "respawn-pane -t %0 -k [ -e {0} ] && exit 9; : > {0}; printf FROZEN-MARK; exit 3",
            ran.display()
        ),
    );
    common::wait_until(
        &mut writer,
        &mut reader,
        "pane-info -t %0",
        |info| info.contains(" exited=3"),
        "the reaper flagging %0 dead with code 3",
    );
    wait_for(
        &mut writer,
        &mut reader,
        "capture-pane -t %0 -p",
        "FROZEN-MARK",
    );
    // The client stays connected: with nobody connected, a daemon whose
    // every pane is dead collects itself before the SIGTERM can land.
    sigterm_clean(&mut first);
    drop((writer, reader));
}

/// The next notification matching `pred` within `wait`, if any.
fn next_matching(
    client: &MuxClient,
    wait: Duration,
    mut pred: impl FnMut(&TmuxNotification) -> bool,
) -> Option<TmuxNotification> {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if let Ok(note) = client
            .notifications()
            .recv_timeout(Duration::from_millis(100))
        {
            if pred(&note) {
                return Some(note);
            }
        }
    }
    None
}

fn is_pane_exited_0_code_3(note: &TmuxNotification) -> bool {
    matches!(
        note,
        TmuxNotification::PaneExited { pane_id, exit_code: Some(3) } if pane_id == "%0"
    )
}

/// The restored dead pane is dead on the query side (`pane-info exited=3`),
/// keeps its frozen screen, and has no process behind it.
#[test]
fn a_restart_restores_a_dead_pane_held_with_its_exit_code_and_screen() {
    let fixture = MuxFixture::new("deadinfo");
    let path = fixture.socket();
    stop_a_daemon_holding_a_dead_pane(&fixture);

    let mut second = spawn_daemon(&fixture);
    wait_listening(path);
    let mut client = MuxClient::connect(path).expect("second daemon accepts");
    let info = client.send("pane-info -t %0").expect("info").join("");
    assert!(
        info.trim_end().ends_with(" exited=3"),
        "the restored pane reports its persisted exit code: {info:?}"
    );
    assert!(
        !info.contains("cmd="),
        "a restored dead pane has no foreground process: {info:?}"
    );
    let screen = client
        .send("capture-pane -t %0 -p")
        .expect("capture")
        .join("");
    assert!(
        screen.contains("FROZEN-MARK"),
        "the frozen screen came back: {screen:?}"
    );
    sigterm_clean(&mut second);
}

/// Registration replay delivers the restored dead pane's `%pane-exited`
/// with its code, and the reaper never announces it again: it skips an
/// already-dead pane, so a born-dead pane is not polled or re-broadcast.
#[test]
fn a_restored_dead_pane_replays_its_exit_once_and_is_never_re_announced() {
    let fixture = MuxFixture::new("deadreplay");
    let path = fixture.socket();
    stop_a_daemon_holding_a_dead_pane(&fixture);

    let mut second = spawn_daemon(&fixture);
    wait_listening(path);
    let client = {
        let mut client = MuxClient::connect(path).expect("second daemon accepts");
        // The first command registers the client; the replay rides ahead
        // of its reply.
        client.send("list-panes").expect("register");
        client
    };
    assert!(
        next_matching(&client, Duration::from_secs(10), is_pane_exited_0_code_3).is_some(),
        "registration replays %pane-exited %0 3 for the restored dead pane"
    );
    // Many reap passes (250 ms each) pass; a reaper that re-observed the
    // born-dead pane would broadcast its death again.
    assert!(
        next_matching(&client, Duration::from_millis(1500), |note| {
            matches!(note, TmuxNotification::PaneExited { .. })
        })
        .is_none(),
        "the reaper must not re-announce an already-dead restored pane"
    );
    sigterm_clean(&mut second);
}

/// `respawn-pane` on a restored dead pane works without `-k` (there is no
/// process to refuse over) and clears the dead state.
#[test]
fn respawn_pane_restarts_a_restored_dead_pane_without_k() {
    let fixture = MuxFixture::new("deadrespawn");
    let path = fixture.socket();
    stop_a_daemon_holding_a_dead_pane(&fixture);

    let mut second = spawn_daemon(&fixture);
    wait_listening(path);
    let mut client = MuxClient::connect(path).expect("second daemon accepts");
    let reply = client
        .send_checked("respawn-pane -t %0 sleep 30")
        .expect("respawn");
    assert!(reply.ok, "respawn without -k succeeds: {:?}", reply.body);
    let info = client.send("pane-info -t %0").expect("info").join("");
    assert!(
        !info.contains("exited="),
        "the respawn cleared the held-dead state: {info:?}"
    );
    sigterm_clean(&mut second);
}

/// SEC-128 across a restart: a dead pane whose last OSC 7 report named a
/// REMOTE host (an SSH session) must not respawn in that remote path just
/// because the same path exists locally. The persisted host rides with the
/// cwd, so `respawn-pane` (no `-c`) falls through to the daemon default.
#[test]
fn respawn_of_a_restored_dead_pane_ignores_its_remote_osc7_cwd() {
    let fixture = MuxFixture::new("remoteosc7");
    let path = fixture.socket();
    // A directory that exists locally and that the remote report names.
    let remote_dir = fixture.state_dir().with_file_name("remote-cwd");
    std::fs::create_dir_all(&remote_dir).expect("create the remote-named dir");
    let recorded = fixture.state_dir().with_file_name("respawn-pwd");

    let mut first = spawn_daemon(&fixture);
    wait_listening(path);
    {
        let stream = connect_local_stream(path).expect("first daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        command(&mut writer, &mut reader, "new-session -s remote");
        command(
            &mut writer,
            &mut reader,
            &format!(
                "respawn-pane -t %0 -k printf 'REMOTE-MARK\\033]7;file://remote-host%s\\033\\\\' {}; exit 3",
                remote_dir.display()
            ),
        );
        common::wait_until(
            &mut writer,
            &mut reader,
            "pane-info -t %0",
            |info| info.contains(" exited=3"),
            "the reaper flagging %0 dead with code 3",
        );
        wait_for(
            &mut writer,
            &mut reader,
            "capture-pane -t %0 -p",
            "REMOTE-MARK",
        );
        // The client stays connected so the final save carries the dead pane.
        sigterm_clean(&mut first);
    }

    let mut second = spawn_daemon(&fixture);
    wait_listening(path);
    let mut client = MuxClient::connect(path).expect("second daemon accepts");
    let reply = client
        .send_checked(&format!(
            "respawn-pane -t %0 pwd > {}; sleep 30",
            recorded.display()
        ))
        .expect("respawn");
    assert!(reply.ok, "respawn succeeds: {:?}", reply.body);
    let deadline = Instant::now() + Duration::from_secs(10);
    let landed = loop {
        let text = std::fs::read_to_string(&recorded).unwrap_or_default();
        if text.ends_with('\n') || Instant::now() >= deadline {
            break text.trim().to_string();
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!landed.is_empty(), "the respawned command recorded its pwd");
    assert_ne!(
        std::fs::canonicalize(&landed).unwrap(),
        std::fs::canonicalize(&remote_dir).unwrap(),
        "a remote OSC 7 cwd must not pick the respawn directory"
    );
    sigterm_clean(&mut second);
}

/// A daemon restored with only dead panes and no client collects itself
/// after the exit-when-empty grace, the same as one that saw them die.
#[test]
fn an_all_dead_restore_with_no_client_exits_after_the_grace() {
    let fixture = MuxFixture::new("deadexit");
    stop_a_daemon_holding_a_dead_pane(&fixture);

    let mut second = spawn_daemon(&fixture);
    // Production grace is 5 s; the bound is a starvation guard, not a
    // timing assertion.
    let deadline = Instant::now() + Duration::from_secs(40);
    let status = loop {
        if let Some(status) = second.try_wait().expect("daemon waitable") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "an all-dead restored daemon with no client never exited"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success(), "exit-when-empty exits 0: {status:?}");
    // The final save still holds the pane dead with its original code: a
    // respawned pane would have re-run the sentinel command and saved 9.
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.state_path()).expect("state file"))
            .expect("state file parses");
    let pane = &saved["sessions"][0]["windows"][0]["panes"][0];
    assert_eq!(pane["dead"], true, "still held dead in the final save");
    assert_eq!(pane["exit_code"], 3, "the original exit code survives");
}

/// The exact send-keys error reply for a dead pane: the `%begin` block body
/// is the `NotStartedError` display string and the block closes with
/// `%error`, never `%end`.
fn assert_not_started_error_reply(reply: &str, kind: &str) {
    assert!(
        reply.contains("%begin "),
        "{kind}: the reply opens a block: {reply:?}"
    );
    assert!(
        reply.contains("PTY session has not been started"),
        "{kind}: the body names NotStartedError: {reply:?}"
    );
    assert!(
        reply.contains("%error "),
        "{kind}: the block closes with %error: {reply:?}"
    );
    assert!(
        !reply.contains("%end "),
        "{kind}: a failed block never also closes with %end: {reply:?}"
    );
}

/// `send-keys` to a born-dead pane — one restored held dead with no PTY
/// behind it — fails with the `NotStartedError` reply block.
#[test]
fn send_keys_to_a_born_dead_pane_errors_not_started() {
    let fixture = MuxFixture::new("deadkeys");
    let path = fixture.socket();
    stop_a_daemon_holding_a_dead_pane(&fixture);

    let mut second = spawn_daemon(&fixture);
    wait_listening(path);
    {
        let stream = connect_local_stream(path).expect("second daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        let reply = command(&mut writer, &mut reader, "send-keys -t %0 -l hello").join("");
        assert_not_started_error_reply(&reply, "born-dead pane");
    }
    sigterm_clean(&mut second);
}

/// `send-keys` to a pane that died at RUNTIME (reaper-flagged, PTY closed)
/// produces the identical reply block: both dead-pane kinds funnel through
/// the shared `PtySession::write` liveness check, so they are
/// indistinguishable on the send-keys wire.
#[test]
fn send_keys_to_a_runtime_died_pane_errors_the_same() {
    let fixture = MuxFixture::new("rundeadkeys");
    let path = fixture.socket();
    let mut daemon = spawn_daemon(&fixture);
    wait_listening(path);
    {
        let stream = connect_local_stream(path).expect("daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        command(&mut writer, &mut reader, "new-session -s deadkeys");
        command(&mut writer, &mut reader, "respawn-pane -t %0 -k exit 3");
        common::wait_until(
            &mut writer,
            &mut reader,
            "pane-info -t %0",
            |info| info.contains(" exited=3"),
            "the reaper flagging %0 dead with code 3",
        );
        let reply = command(&mut writer, &mut reader, "send-keys -t %0 -l hello").join("");
        assert_not_started_error_reply(&reply, "runtime-died pane");
    }
    sigterm_clean(&mut daemon);
}

// The three tests below drive `par-mux --restart` itself — the CLI
// handoff: stop the old daemon, wait for its socket to stop accepting,
// fork the fresh daemon, restore. The handoff used to race: the old
// daemon unlinked the socket BEFORE its final state save, so the stop's
// wait could return while the save was still in flight, and the fresh
// daemon's restore read a missing or stale state file ("fresh daemon came
// up empty"). The socket now drops only after the save, and these tests
// hold that contract under repetition.

/// Run `par-mux --restart` on the fixture with both stdio streams
/// captured, so the pre-fork announcements are assertable.
fn run_restart_cli(fixture: &MuxFixture) -> std::process::Child {
    use std::process::{Command, Stdio};
    Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("--socket")
        .arg(fixture.socket())
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .arg("--restart")
        .env_remove("PAR_MUX_ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("--restart spawns")
}

/// Wait for the `--restart` parent half to exit (bounded — an unfixed
/// in-process serve would hang the suite here) and return its output.
fn wait_exited(mut child: std::process::Child, what: &str) -> std::process::Output {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait().expect("--restart is waitable") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{what} never returned; --restart must detach, not serve in-process")
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    child.wait_with_output().expect("--restart pipes drain")
}

/// `par-mux --stop` on the fixture, then require the socket to go quiet —
/// the cleanup half of a test whose restarted daemon is a detached
/// grandchild no `Child` handle exists for.
fn stop_via_cli(fixture: &MuxFixture) {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let status = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("--socket")
        .arg(fixture.socket())
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .arg("--stop")
        .env_remove("PAR_MUX_ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("--stop runs");
    assert!(status.success(), "--stop exits 0: {status:?}");
    let deadline = Instant::now() + Duration::from_secs(15);
    while connect_local_stream(fixture.socket()).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the daemon on {} did not stop within 15s",
            fixture.socket().display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// (a) The CLI handoff restores a populated tree — five trials, because the
/// race this pins was intermittent: the old daemon's final save (fsync +
/// rename of a possibly large JSON) now provably lands before the socket
/// unlinks, so the fresh daemon's restore always reads it.
#[test]
fn cli_restart_restores_a_populated_tree_five_trials() {
    for trial in 0..5 {
        let fixture = MuxFixture::new("cli-restart");
        let path = fixture.socket();

        let mut first = spawn_daemon(&fixture);
        wait_listening(path);
        let marker = format!("RSTX-MARK-{trial}");
        {
            let stream = connect_local_stream(path).expect("first daemon accepts");
            let mut writer = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(stream);
            command(&mut writer, &mut reader, "new-session -s demo");
            let pane = pane_ids(&command(&mut writer, &mut reader, "list-panes").join(""))
                .first()
                .expect("new-session created a pane")
                .clone();
            command(
                &mut writer,
                &mut reader,
                &format!("send-keys -t {pane} 'printf RSTX-MARK-{trial}' Enter"),
            );
            wait_for(
                &mut writer,
                &mut reader,
                &format!("capture-pane -t {pane}"),
                &marker,
            );
        } // the client drops; the live pane keeps the daemon serving

        let output = wait_exited(run_restart_cli(&fixture), "--restart's parent half");
        assert!(
            output.status.success(),
            "--restart's parent half exits 0: {:?}",
            output.status
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("stopped the daemon on"),
            "the restart stopped a live daemon: {stderr}"
        );
        assert!(
            !stderr.contains("saved an empty tree"),
            "a populated save must not be announced as empty: {stderr}"
        );

        wait_listening(path);
        {
            let stream = connect_local_stream(path).expect("fresh daemon accepts");
            let mut writer = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(stream);
            let listed = command(&mut writer, &mut reader, "list-panes").join("");
            let pane = pane_ids(&listed)
                .first()
                .expect("the pane survived the CLI restart")
                .clone();
            wait_for(
                &mut writer,
                &mut reader,
                &format!("capture-pane -t {pane}"),
                &marker,
            );
        }

        stop_via_cli(&fixture);
        sigterm_clean(&mut first);
    }
}

/// (b) A restart after the tree was emptied is LOUD on both sides: the
/// pre-fork peek tells the caller the previous daemon saved an empty tree,
/// and the fresh daemon's log records that it restored nothing and is
/// exiting via exit-when-empty. The outcome matches the documented
/// behavior — the fresh daemon serves, finds nothing, and exits after the
/// grace.
#[test]
fn cli_restart_after_emptying_announces_the_empty_restore() {
    use std::time::{Duration, Instant};

    let fixture = MuxFixture::new("cli-empty");
    let path = fixture.socket();

    // The emptied daemon exits on its own (exit-when-empty); the guard is
    // the panic backstop that reaps it if the test fails first.
    // The emptied daemon exits on its own (exit-when-empty); the guard is
    // the panic backstop that reaps it if the test fails first.
    let _first = spawn_daemon(&fixture);
    wait_listening(path);
    // Build and empty the tree over one direct client, then drop it — no
    // client stays connected, so nothing holds the emptied daemon alive.
    // (--cmd cannot be used here: it conflicts with --state-dir.)
    {
        let stream = connect_local_stream(path).expect("daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        command(&mut writer, &mut reader, "new-session -s demo");
        command(&mut writer, &mut reader, "kill-session -t demo");
    }
    // Zero sessions and zero clients: after EXIT_EMPTY_GRACE the daemon
    // exits itself, and its final save lands as an empty tree (which also
    // clears the last-good snapshot, so the restart cannot resurrect).
    let deadline = Instant::now() + Duration::from_secs(15);
    while connect_local_stream(path).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the emptied daemon never exited; exit-when-empty is broken"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let output = wait_exited(run_restart_cli(&fixture), "--restart's parent half");
    assert!(
        output.status.success(),
        "--restart's parent half exits 0: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("previous daemon saved an empty tree"),
        "the pre-fork peek announces the empty save: {stderr}"
    );

    // The fresh daemon comes up, then exits-when-empty per the docs.
    wait_listening(path);
    let deadline = Instant::now() + Duration::from_secs(20);
    while connect_local_stream(path).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the empty-restored fresh daemon never exited"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // The restore side is recorded in the daemon log the fork points
    // stderr at (unreachable from the terminal once detached).
    let mut log_path = fixture.state_path().into_os_string();
    log_path.push(".log");
    let log = std::fs::read_to_string(std::path::PathBuf::from(log_path))
        .expect("the fresh daemon wrote its log");
    assert!(
        log.contains("restored tree is empty; daemon exiting (exit-when-empty)"),
        "the daemon log records the empty restore: {log}"
    );
}

/// (c) A fresh daemon whose serve fails AFTER the fork (here: a socket
/// path past the platform Unix-socket address limit, so the bind can never
/// succeed) leaves the failure visible in the daemon log beside the state
/// file — pre-fix, that stderr went to /dev/null and the restart failed
/// silently, which is exactly how the owner lost the daemon with no error
/// text.
#[test]
fn cli_restart_surfaces_a_fresh_daemon_startup_failure() {
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let fixture = MuxFixture::new("cli-fail");
    // Every platform's sun_path is 104 (macOS) or 108 (Linux); 200 always
    // exceeds it.
    let long_name = format!("long-{}", "x".repeat(200));
    let socket = fixture.socket().with_file_name(long_name);

    let output = wait_exited(
        Command::new(env!("CARGO_BIN_EXE_par-mux"))
            .arg("--socket")
            .arg(&socket)
            .arg("--state-dir")
            .arg(fixture.state_dir())
            .arg("--restart")
            .env_remove("PAR_MUX_ENV")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("--restart spawns"),
        "--restart's parent half",
    );
    assert!(
        output.status.success(),
        "the parent half succeeds before the detached serve fails: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no daemon running"),
        "the stop phase reports the absent daemon: {stderr}"
    );

    // Give the (doomed) fresh daemon its startup window, then confirm it
    // never bound.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        connect_local_stream(&socket).is_err(),
        "no daemon can bind a path past the address limit"
    );

    // The failure is on disk where the caller can find it.
    let state = par_term_emu_core_rust::mux::persist::state_file_in(&fixture.state_dir(), &socket);
    let mut log_path = state.into_os_string();
    log_path.push(".log");
    let log = std::fs::read_to_string(std::path::PathBuf::from(log_path))
        .expect("the fresh daemon wrote its log");
    assert!(
        log.contains("Error:"),
        "the bind failure reaches the daemon log: {log}"
    );
}

/// Workspaces survive a restart: roster, names, membership, and the active
/// pointers all come back from the saved state (FORMAT_VERSION 3).
#[test]
fn workspaces_survive_a_restart() {
    let fixture = MuxFixture::new("wsrestart");
    let path = fixture.socket();

    let mut first = spawn_daemon(&fixture);
    wait_listening(path);
    {
        let stream = connect_local_stream(path).expect("first daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        // The first session lazily creates the default `main` (+0); the
        // seed stays alive so workspace ids hold: `dev` is +1.
        command(&mut writer, &mut reader, "new-session -s seed");
        command(&mut writer, &mut reader, "new-workspace -n dev");
        command(&mut writer, &mut reader, "new-session -s in-dev");
        command(&mut writer, &mut reader, "select-workspace -t +0");
        command(&mut writer, &mut reader, "new-session -s in-main");
        command(&mut writer, &mut reader, "select-workspace -t dev");
    }
    sigterm_clean(&mut first);

    let mut second = spawn_daemon(&fixture);
    wait_listening(path);
    {
        let stream = connect_local_stream(path).expect("second daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        let listed = command(&mut writer, &mut reader, "list-workspaces").join("");
        assert!(
            listed.contains("+0: main") && listed.contains("+1: dev"),
            "both workspaces survived: {listed}"
        );
        assert!(
            listed.contains("+1: dev active"),
            "the active pointer survived: {listed}"
        );
        let sessions = command(&mut writer, &mut reader, "list-sessions").join("");
        assert!(
            sessions.contains("+0: main: $2: in-main"),
            "session 2 restored into its workspace: {sessions}"
        );
        assert!(
            sessions.contains("+1: dev: $1: in-dev"),
            "session 1 restored into its workspace: {sessions}"
        );
        // A new session after the restart lands in the restored ACTIVE
        // workspace.
        command(&mut writer, &mut reader, "new-session -s fresh");
        let sessions = command(&mut writer, &mut reader, "list-sessions").join("");
        assert!(
            sessions.contains("+1: dev: $3: fresh"),
            "a post-restart session joins the restored active workspace: {sessions}"
        );
    }
    sigterm_clean(&mut second);
}
