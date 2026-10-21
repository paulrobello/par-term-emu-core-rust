//! `send-keys` payload semantics, measured against real tmux.
//!
//! Real tmux 3.7c (probed via a throwaway `-L` server, 2026-10-02) joins
//! whitespace-separated payload tokens with NOTHING between them in every
//! mode: `send-keys -t 0 -l echo LEFT` types `echoLEFT`, the quoted single
//! argument `-l 'echo LEFT'` types `echo LEFT`, and default mode
//! `send-keys -t 0 abc def` types `abcdef`. par-mux matches that contract;
//! these tests pin it so a well-meaning "join with spaces" change cannot
//! ship. To type a space, quote the run (`-l 'echo LEFT'`) or use the
//! `Space` key name; for bytes a shell line cannot express, `send-keys -H`
//! takes hex byte pairs, the lossless path.

// The two tests drive real unix-socket daemons; on Windows the imports
// would sit unused, so the whole surface is unix-scoped.
#![cfg(all(feature = "mux", unix))]

#[cfg(not(feature = "mux-bin"))]
compile_error!("this test drives the par-mux binary: build it with --features mux-bin");

mod common;

use common::{command, wait_for, MuxFixture};
use interprocess::TryClone as _;
use par_mux::mux::{connect_local_stream, MuxServer};
use std::io::BufReader;
use std::thread;
use std::time::Duration;

/// The card's repro, end to end: `send-keys -t %0 -l echo LEFT` types
/// `echoLEFT` into the pane — the token separator is dropped, exactly as
/// real tmux does. A shell echoes the typed line back, so the pane's
/// capture shows the literal concatenation.
#[cfg(unix)]
#[test]
fn literal_multi_token_payload_drops_the_separator_like_tmux() {
    let fixture = MuxFixture::new("skl");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    thread::spawn(move || server.run());
    common::wait_listening(path);

    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(&mut writer, &mut reader, "new-session -s skl");

    command(&mut writer, &mut reader, "send-keys -t %0 -l echo LEFT");
    let screen = wait_for(&mut writer, &mut reader, "capture-pane -t %0", "echoLEFT");
    assert!(
        !screen.contains("echo LEFT"),
        "the separator must not appear: tmux joins -l tokens with nothing: {screen}"
    );
}

/// A pane whose child never reads (`sleep`) fills its kernel PTY buffer,
/// after which any further input write parks in `write_all`. The parked
/// write must not hold the tree mutex: a control command on a second
/// connection (QA-225, the paste-buffer shape) still has to answer.
#[cfg(unix)]
#[test]
fn send_keys_does_not_stall_other_commands_behind_a_full_pty() {
    let fixture = MuxFixture::new("skf");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    thread::spawn(move || server.run());
    common::wait_listening(path);

    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(
        &mut writer,
        &mut reader,
        "new-session -s skf -- sh -c 'stty raw -echo; sleep 60'",
    );

    // ~512 KiB of literal input against a ~64 KiB kernel buffer: after a
    // handful of commands the daemon is parked in a PTY write that cannot
    // complete while `sleep` never reads. The thread stays parked on the
    // reply for the rest of the test — the pane is never drained.
    let writer_path = path.to_path_buf();
    thread::spawn(move || {
        let stream = connect_local_stream(&writer_path).expect("connect writer");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        let payload = "A".repeat(8 * 1024);
        for _ in 0..64 {
            command(
                &mut writer,
                &mut reader,
                &format!("send-keys -t %0 -l {payload}"),
            );
        }
    });

    // Let the writer fill the kernel buffer and park before probing.
    thread::sleep(Duration::from_secs(2));

    // A fresh connection's `list-sessions` must answer while the stalled
    // write is in flight. The query runs on its own thread so a stuck
    // daemon cannot hang the test body — the bounded recv fails it.
    let (tx, rx) = std::sync::mpsc::channel();
    let query_path = path.to_path_buf();
    thread::spawn(move || {
        let stream = connect_local_stream(&query_path).expect("connect query");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        let reply = command(&mut writer, &mut reader, "list-sessions");
        let _ = tx.send(reply);
    });
    let reply = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("list-sessions stalled behind the tree lock held across a PTY write");
    assert!(
        reply.iter().any(|l| l.contains("skf")),
        "expected the session name in the reply: {reply:?}"
    );
}

/// The quoted single-argument form is how a space survives: the shell
/// executes the typed `echo LEFT` and the pane shows its output.
#[cfg(unix)]
#[test]
fn quoted_literal_run_keeps_its_space_like_tmux() {
    let fixture = MuxFixture::new("skq");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    thread::spawn(move || server.run());
    common::wait_listening(path);

    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(&mut writer, &mut reader, "new-session -s skq");

    // `-l` makes even the word `Enter` literal text (tmux semantics, the
    // same probe that measured the joining rule), so the terminator goes in
    // a second default-mode command.
    command(&mut writer, &mut reader, "send-keys -t %0 -l 'echo LEFT'");
    command(&mut writer, &mut reader, "send-keys -t %0 Enter");
    // The shell's output "LEFT" only appears if the space survived the
    // payload parse; without it the shell would run `echoLEFT`.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let output = loop {
        let screen = command(&mut writer, &mut reader, "capture-pane -t %0").join("");
        // A line holding exactly LEFT (not the echoed command) means the
        // echo executed. The echoed command line itself contains "echo LEFT".
        if screen.lines().any(|l| l.trim() == "LEFT") || std::time::Instant::now() > deadline {
            break screen;
        }
        thread::sleep(Duration::from_millis(50));
    };
    assert!(
        output.lines().any(|l| l.trim() == "LEFT"),
        "the quoted run should execute `echo LEFT`, printing LEFT: {output}"
    );
}
