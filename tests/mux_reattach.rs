//! Reattach: a new client resyncs a live pane's screen, not a blank one.

#![cfg(feature = "mux")]

use par_term_emu_core_rust::mux::{MuxClient, MuxServer};
use par_term_emu_core_rust::tmux_control::TmuxNotification;
use std::time::{Duration, Instant};

fn socket(tag: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("par-mux-reattach-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn a_reconnecting_client_resyncs_the_pane_screen() {
    let path = socket("resync");
    let server = MuxServer::bind(&path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    // First client: create a session and produce recognisable output.
    {
        let mut client = MuxClient::connect(&path).expect("first connect");
        client.send("new-session -s resync").expect("new-session");
        client
            .send("send-keys -t %0 echo par-mux-resync-marker")
            .expect("send-keys");

        let deadline = Instant::now() + Duration::from_secs(10);
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

    std::thread::sleep(Duration::from_millis(300));

    // Second client: reattach and ask the pane to replay its screen.
    let mut client = MuxClient::connect(&path).expect("reconnect to the live daemon");
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

    let _ = std::fs::remove_file(&path);
}

#[test]
fn connect_or_spawn_starts_a_daemon_when_none_is_running() {
    let path = socket("spawn");
    // Nothing is listening. The client must start one.
    let mut client =
        MuxClient::connect_or_spawn_at(&path).expect("connect_or_spawn starts a daemon");
    // A session first: an empty daemon's list-panes block has no body lines,
    // so prove liveness by creating a pane and listing it.
    client
        .send("new-session -s spawn")
        .expect("the spawned daemon answers");
    let panes = client
        .send("list-panes")
        .expect("list on spawned daemon")
        .join("");
    assert!(
        panes.contains('%'),
        "a spawned daemon should hold the new pane: {panes}"
    );
    let _ = std::fs::remove_file(&path);
}
