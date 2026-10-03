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
use par_term_emu_core_rust::mux::{connect_local_stream, MuxServer};
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
