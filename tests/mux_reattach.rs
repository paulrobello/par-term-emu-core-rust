//! Reattach: a new client resyncs a live pane's screen, not a blank one.

#![cfg(feature = "mux")]

mod common;

use common::{wait_listening, MuxFixture};
use par_term_emu_core_rust::mux::{MuxClient, MuxServer};
use par_term_emu_core_rust::terminal::Terminal;
use par_term_emu_core_rust::tmux_control::TmuxNotification;
use std::time::{Duration, Instant};

#[test]
fn a_reconnecting_client_resyncs_the_pane_screen() {
    let fixture = MuxFixture::new("resync");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    // First client: create a session and produce recognisable output.
    {
        let mut client = MuxClient::connect(path).expect("first connect");
        client.send("new-session -s resync").expect("new-session");
        // The output marker must be a CASE-FOLDED transformation of the
        // command: the kernel echo of the typed text arrives long before a
        // login shell finishes init, and an echo that contains the marker
        // satisfies the wait before any output exists (measured: the
        // marker containment passed on the echoed command line alone).
        client
            .send("send-keys -t %0 'echo PAR-MUX-RESYNC-MARKER | tr A-Z a-z' Enter")
            .expect("send-keys");

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut saw = false;
        while Instant::now() < deadline && !saw {
            // Debug of Output renders the payload as byte numbers, so match
            // the variant and decode the bytes to find the marker.
            if let Ok(TmuxNotification::Output { data, .. }) = client
                .notifications()
                .recv_timeout(Duration::from_millis(250))
            {
                saw = String::from_utf8_lossy(&data).contains("par-mux-resync-marker");
            }
        }
        assert!(saw, "first client should see the marker as pushed output");
        // Client drops — par-term has "restarted".
    }

    // The daemon must still be listening after its client left. Poll the
    // socket (bounded) rather than sleeping a guessed 300ms — the reattach
    // below is the assertion, so it must run against a live listener.
    wait_listening(path);

    // Second client: reattach and ask the pane to replay its screen.
    let mut client = MuxClient::connect(path).expect("reconnect to the live daemon");
    let panes = client.send("list-panes").expect("list-panes").join("");
    assert!(panes.contains('%'), "the pane survived the client: {panes}");

    let replay = client
        .send("refresh-client -t %0")
        .expect("refresh")
        .join("");
    assert!(
        replay.contains("par-mux-resync-marker"),
        "a reattached client must receive the pane's CURRENT SCREEN, not a blank \
         pane — otherwise par-term reattaches to empty panes. Got: {replay}"
    );

    // The replay must RENDER the pane's screen in a client emulator — the
    // output line at column 0 of its row. A `\n`-joined plain-text reply
    // staircases (LF preserves the cursor column), which is the
    // blank/partial reattach render this contract forbids; plain text also
    // loses attributes. The styled, cursor-addressed reply (`\x1b[H` +
    // per-row CUP + SGR) renders exactly.
    let mut term = Terminal::with_scrollback(80, 24, 1000);
    term.process(replay.as_bytes());
    let rendered = term.content();
    let rendered_lines: Vec<&str> = rendered.lines().collect();
    assert!(
        replay.starts_with("\x1b[H"),
        "the replay must be styled and cursor-addressed (an anchor CUP first), \
         not plain text: {replay:?}"
    );
    assert!(
        rendered_lines
            .iter()
            .any(|l| l.starts_with("par-mux-resync-marker")),
        "the replayed screen must render the marker at column 0 of its row \
         (a staircase render here is the blank/partial reattach bug): {rendered_lines:?}"
    );
}

/// A reattached client's seed must replay the pane's STATE, not just its
/// text — and the pane's subsequent `%output` must land on that state.
///
/// The pane runs a full-screen-TUI-shaped program (alt screen on, a styled
/// tag at a fixed position, cursor hidden) that blocks on `read`, so the
/// second frame is only produced once this test releases it from the
/// REATTACHED side. A plain-`content()` seed fails every assertion group:
/// it carries no `\x1b[?1049h`/`\x1b[?25l`, replays the TUI onto the main
/// screen, and loses every attribute.
#[test]
fn a_reattached_client_receives_a_seed_that_replays_state_and_survives_output() {
    let fixture = MuxFixture::new("seeded");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    // Case-folded on-screen words (the kernel echo of the typed command
    // contains SEED/LATE uppercase only, so waiting for lowercase cannot
    // be satisfied by the echo — the measured trap the first test
    // documents).
    let tui = "printf \"\\033[?1049h\\033[2;3H\\033[1;44m\"; echo SEED | tr A-Z a-z; \
               printf \"\\033[0m\\033[?25l\"; read -r x; \
               printf \"\\033[4;5H\\033[1;42m\"; echo LATE | tr A-Z a-z; \
               printf \"\\033[0m\"; sleep 60";

    {
        let mut client = MuxClient::connect(path).expect("first connect");
        client.send("new-session -s seeded").expect("new-session");
        client
            .send(&format!("send-keys -t %0 '{tui}' Enter"))
            .expect("send-keys");

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut saw = false;
        while Instant::now() < deadline && !saw {
            if let Ok(TmuxNotification::Output { data, .. }) = client
                .notifications()
                .recv_timeout(Duration::from_millis(250))
            {
                saw = String::from_utf8_lossy(&data).contains("seed");
            }
        }
        assert!(saw, "the TUI's first frame should reach the first client");
        // Client drops — par-term has "restarted" mid-TUI.
    }

    wait_listening(path);

    let mut client = MuxClient::connect(path).expect("reconnect");

    // The reattach seed.
    let seed_lines = client.send("refresh-client -t %0").expect("refresh");
    let seed = seed_lines.join("");
    assert!(
        seed.contains("\x1b[?1049h"),
        "the seed must select the alt screen (a plain-text seed replays the \
         TUI onto the main screen): {seed:?}"
    );
    assert!(
        seed.contains("\x1b[?25l"),
        "the seed must restore the hidden cursor: {seed:?}"
    );

    // Apply the seed to a fresh emulator, exactly as par-term does.
    let mut live = Terminal::with_scrollback(80, 24, 1000);
    live.process(seed.as_bytes());
    assert!(
        live.is_alt_screen_active(),
        "the seed selects the alt screen"
    );

    // Release the TUI's second frame and feed every subsequent %output
    // into the SAME emulator — the deltas must land on the seeded state.
    client
        .send("send-keys -t %0 g Enter")
        .expect("release the second frame");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw = false;
    while Instant::now() < deadline && !saw {
        if let Ok(TmuxNotification::Output { data, .. }) = client
            .notifications()
            .recv_timeout(Duration::from_millis(250))
        {
            live.process(&data);
            saw = String::from_utf8_lossy(&data).contains("late");
        }
    }
    assert!(
        saw,
        "the TUI's second frame should reach the reattached client"
    );

    // Reference: `capture-pane -e` is the daemon's own view of the pane's
    // visible screen — one styled line per grid row. Render each line at
    // its own row (the framing contract is line-per-row, not cursor
    // addressing; LF alone staircases).
    let captured = client.send("capture-pane -t %0 -p -e").expect("capture -e");
    assert_eq!(captured.len(), 24, "one line per grid row: {captured:?}");
    let mut reference = Terminal::with_scrollback(80, 24, 1000);
    for (row, line) in captured.iter().enumerate() {
        reference.process(format!("\x1b[{};1H", row + 1).as_bytes());
        reference.process(line.as_bytes());
    }

    // Grid equality, attributes included: the styled export is SGR-diffed
    // per run over every cell, so byte equality means the two grids
    // render identically — text AND colors/attributes.
    assert_eq!(
        live.content(),
        reference.content(),
        "the reattached emulator's screen must match the daemon's pane"
    );
    let live_styled = live.export_visible_screen_styled();
    assert_eq!(
        live_styled,
        reference.export_visible_screen_styled(),
        "the reattached emulator must carry the pane's ATTRIBUTES (colors, \
         bold), not just its text — a plain-text seed fails here"
    );
    assert!(
        live_styled.contains("\x1b[0;37;44"),
        "the seed row keeps its blue-background run: {live_styled:?}"
    );
    assert!(
        live_styled.contains("\x1b[0;37;42"),
        "the late row keeps its green-background run: {live_styled:?}"
    );
}

/// Kills the client's spawned daemon on drop — the panic backstop.
///
/// `MuxClient::kill_spawned_daemon` is deliberately not a `Drop` impl on
/// `MuxClient` itself (a disconnecting client must not kill a daemon other
/// clients are using); this test owns the daemon it spawned, so it needs
/// its own guard. The call is idempotent after an explicit one, so calling
/// it again from `Drop` on the happy path (where the test already called
/// it) is harmless.
struct SpawnedClient(MuxClient);

impl Drop for SpawnedClient {
    fn drop(&mut self) {
        let _ = self.0.kill_spawned_daemon();
    }
}

#[test]
fn connect_or_spawn_starts_a_daemon_when_none_is_running() {
    let fixture = MuxFixture::new("spawn");
    let path = fixture.socket();
    // Nothing is listening. The client must start one.
    let mut client = SpawnedClient(
        MuxClient::connect_or_spawn_at(path).expect("connect_or_spawn starts a daemon"),
    );
    // A session first: an empty daemon's list-panes block has no body lines,
    // so prove liveness by creating a pane and listing it.
    client
        .0
        .send("new-session -s spawn")
        .expect("the spawned daemon answers");
    let panes = client
        .0
        .send("list-panes")
        .expect("list on spawned daemon")
        .join("");
    assert!(
        panes.contains('%'),
        "a spawned daemon should hold the new pane: {panes}"
    );
    // The tmux model is daemon-outlives-client, so the daemon must be ended
    // explicitly — dropping the client (or removing only the socket file)
    // leaves a live process behind, one leak per test run.
    client
        .0
        .kill_spawned_daemon()
        .expect("the spawned daemon is cleaned up");
    // The spawned daemon persisted its tree (D3.3) under the test socket's
    // stem in the real platform state dir (connect_or_spawn_at passes no
    // --state-dir); the fixture's Drop removes that file for this stem.
}
