//! The restart arc (Phase 3, Task 3.6): what one daemon saved, the next
//! daemon on the same socket serves back — layout, pane ids, screen content
//! and scrollback survive, and the processes behind the panes are new.
//!
//! Unix-only: the stop is a SIGTERM, and the process-identity probe asks the
//! shells themselves (`$$`), which is only meaningful on Unix.

#![cfg(all(feature = "mux", unix))]

mod common;

use common::{
    command, pane_ids, sigterm_clean, spawn_daemon, wait_for, wait_for_pid, wait_listening,
};
use interprocess::TryClone as _;
use par_term_emu_core_rust::mux::connect_local_stream;
use std::io::BufReader;

fn socket(tag: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("par-mux-restart-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_file(&path);
    path
}

/// Task 3.6: one daemon builds a session with a split, screen content and
/// scrollback; a SIGTERM stops it; a second daemon on the SAME socket must
/// serve the saved tree back — same session, same pane ids, same screen and
/// history — while the processes behind the panes are new (D3.5's scope
/// honesty, asserted by asking the shells for their pids).
#[test]
fn a_restart_serves_the_saved_tree_with_new_processes() {
    let path = socket("arc");
    let state_path = par_term_emu_core_rust::mux::persist::state_file_path(&path);
    let _ = std::fs::remove_file(&state_path);

    // First daemon: a split layout with real content in both panes.
    let mut first = spawn_daemon(&path);
    wait_listening(&path);
    let stream = connect_local_stream(&path).expect("first daemon accepts");
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
    // pane's whole byte stream and would trivially contain the marker.
    wait_for(
        &mut writer,
        &mut reader,
        &format!("capture-pane -t {first_pane} -p -S -30"),
        "ZQX-HIST-03",
    );
    let visible = command(
        &mut writer,
        &mut reader,
        &format!("refresh-client -t {first_pane}"),
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
    let mut second = spawn_daemon(&path);
    wait_listening(&path);
    let stream = connect_local_stream(&path).expect("second daemon accepts");
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

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&state_path);
}
