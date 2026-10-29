//! The mux-backed streaming suite against a real in-process daemon.

use super::control::parse_pane_info;
use super::mirror::SEED_GRAPHICS_ID_OFFSET;
use super::test_wait::CEILING;
use super::*;
use crate::streaming::protocol::ServerMessage;
use crate::streaming::server::SessionFactory;
use crate::streaming::session::StreamSessionState;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

// --- Against a real in-process daemon -------------------------------

use crate::mux::{MuxClient, MuxServer};
use crate::streaming::{ConnectionParams, StreamingConfig};
use std::time::{Duration, Instant};

/// Unix echoes the typed line with the quotes; cmd.exe with the caret.
/// Only the EXECUTED output holds the joined marker.
#[cfg(unix)]
const TYPED_BEFORE: &str = r#"echo SEED""-BEFORE"#;
#[cfg(windows)]
const TYPED_BEFORE: &str = "echo SEED^-BEFORE";
#[cfg(unix)]
const TYPED_AFTER: &str = r#"echo DELTA""-AFTER"#;
#[cfg(windows)]
const TYPED_AFTER: &str = "echo DELTA^-AFTER";

/// Bound on the graceful exit of [`daemon`]'s serving thread. Only a
/// wedged `run()` exceeds it, and by then the test is over anyway.
const SERVING_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Owns [`daemon`]'s serving thread: Drop raises the shutdown flag and
/// joins — bounded — so a finished test cannot leak a detached daemon
/// thread holding the socket into whatever runs next (QA-185).
struct ServingGuard {
    shutdown: std::sync::Arc<AtomicBool>,
    serving: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ServingGuard {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(serving) = self.serving.take() {
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = serving.join();
                let _ = done_tx.send(());
            });
            if done_rx.recv_timeout(SERVING_EXIT_TIMEOUT).is_err() {
                eprintln!(
                    "mux_factory daemon still serving {SERVING_EXIT_TIMEOUT:?} after shutdown"
                );
            }
        }
    }
}

/// A served daemon with one session; returns its socket dir (keep it
/// alive), socket path, the serving-thread guard, a control client, and
/// the first pane id `%N`.
fn daemon() -> (tempfile::TempDir, PathBuf, ServingGuard, MuxClient, String) {
    let dir = tempfile::Builder::new()
        .prefix("par-mux-str-")
        .tempdir()
        .expect("temp dir");
    let socket = dir.path().join("s");
    let server = MuxServer::bind(&socket).expect("bind");
    let shutdown = server.shutdown_handle();
    let serving = std::thread::spawn(move || server.run());
    let guard = ServingGuard {
        shutdown,
        serving: Some(serving),
    };
    let mut control = MuxClient::connect(&socket).expect("control client");
    control.send("new-session -s stream").expect("new-session");
    let pane = control
        .send("list-panes")
        .expect("list-panes")
        .into_iter()
        .map(|l| l.trim().to_string())
        .find(|l| l.starts_with('%'))
        .expect("a pane");
    (dir, socket, guard, control, pane)
}

/// The session machinery spawns tasks (`resolve_session` starts the
/// broadcaster, `setup_session` the resize forwarder), so tests enter a
/// multi-thread runtime. The test body stays on the plain test thread:
/// its blocking waits and writes never occupy a runtime worker.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

/// A streaming server whose sessions mirror panes of the daemon. The
/// handshake's reply deadline is the hang ceiling: the production 10 s
/// expired on `pane-info` at host load ~200 against a healthy daemon.
fn streaming(socket: &std::path::Path) -> Arc<StreamingServer> {
    let factory = Arc::new(
        MuxSessionFactory::new(socket, MuxPaneSelector::FromSessionId).with_reply_timeout(CEILING),
    );
    let server = Arc::new(StreamingServer::with_factory(
        "127.0.0.1:0".to_string(),
        StreamingConfig::default(),
        Arc::clone(&factory) as Arc<dyn SessionFactory>,
    ));
    factory.set_streaming_server(Arc::clone(&server));
    server
}

/// Open the streaming session mirroring `pane` (`%N`).
fn mirror(server: &Arc<StreamingServer>, pane: &str) -> Arc<StreamSessionState> {
    let params = ConnectionParams {
        session_id: format!("pane-{}", pane.trim_start_matches('%')),
        readonly: false,
        preset: None,
    };
    server.resolve_session(&params).expect("mux-backed session")
}

fn has_line(text: &str, marker: &str) -> bool {
    text.lines().any(|l| l.trim() == marker)
}

fn wait(what: &str, mut ready: impl FnMut() -> Option<String>) {
    let deadline = Instant::now() + CEILING;
    loop {
        match ready() {
            None => return,
            Some(state) => assert!(
                Instant::now() < deadline,
                "never saw {what}; last state:\n{state}"
            ),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_mirror(session: &StreamSessionState, marker: &str) {
    wait(marker, || {
        let text = session.terminal.read().content();
        (!has_line(&text, marker)).then_some(text)
    });
}

/// Type `line` + Enter through the session's input path, the way the
/// streaming server forwards a client's Input message.
fn type_line(session: &StreamSessionState, line: &str) {
    let writer = session
        .pty_writer
        .read()
        .as_ref()
        .cloned()
        .expect("a mux-backed session accepts input");
    let mut w = writer.lock();
    w.write_all(line.as_bytes()).expect("input write");
    w.write_all(b"\r").expect("enter");
    w.flush().expect("flush");
}

fn pane_size(control: &mut MuxClient, pane: &str) -> (usize, usize) {
    let info = control
        .send(&format!("pane-info -t {pane}"))
        .expect("pane-info")
        .join("");
    let (_, cols, rows) = parse_pane_info(&info).expect("pane-info reply");
    (cols, rows)
}

/// Wait until `marker` shows in the DAEMON's capture of `pane`.
fn wait_pane(control: &mut MuxClient, pane: &str, marker: &str) {
    wait(marker, || {
        let text = control
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        (!has_line(&text, marker)).then_some(text)
    });
}

/// Open the streaming session with an explicit id (the
/// `workspace-active` tests use the follow mode's id directly).
fn mirror_session(server: &Arc<StreamingServer>, session_id: &str) -> Arc<StreamSessionState> {
    let params = ConnectionParams {
        session_id: session_id.to_string(),
        readonly: false,
        preset: None,
    };
    server.resolve_session(&params).expect("mux-backed session")
}

/// Card 01a16ab7 criterion 3: with a mirror following the daemon's
/// active workspace, a desktop `select-workspace` mid-mirror moves the
/// viewer to the new workspace's pane — same streaming session, no
/// reconnect, and the input path follows with it.
#[test]
fn a_workspace_active_mirror_follows_a_desktop_workspace_switch() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane0) = daemon();

    // A second workspace; `new-workspace` selects it, so switch back
    // to `main` before the viewer attaches.
    control
        .send("new-workspace -n second")
        .expect("new-workspace");
    let second_pane = control
        .send("list-panes -F '#{pane_id} #{session_name}'")
        .expect("list-panes")
        .into_iter()
        .find(|l| l.trim().ends_with("second"))
        .and_then(|l| {
            l.split_whitespace()
                .next()
                .and_then(|p| p.trim_start_matches('%').parse::<u32>().ok())
        })
        .expect("the second workspace's pane");
    let pane1 = format!("%{second_pane}");
    control
        .send("select-workspace -t main")
        .expect("back to main");

    // Seed each pane's screen with its workspace marker.
    control
        .send(&format!("send-keys -t {pane0} 'echo WS1-MARKER' Enter"))
        .expect("send-keys");
    wait_pane(&mut control, &pane0, "WS1-MARKER");
    control
        .send(&format!("send-keys -t {pane1} 'echo WS2-MARKER' Enter"))
        .expect("send-keys");
    wait_pane(&mut control, &pane1, "WS2-MARKER");

    // Criterion 1: `workspace-active` resolves the ACTIVE workspace's
    // pane (main's), seeded from it, with nothing of workspace two.
    let server = streaming(&socket);
    let session = mirror_session(&server, ACTIVE_WORKSPACE_SESSION_ID);
    let seeded = session.terminal.read().content();
    assert!(
        has_line(&seeded, "WS1-MARKER"),
        "the mirror seeds from the active workspace's pane:\n{seeded}"
    );
    assert!(
        !has_line(&seeded, "WS2-MARKER"),
        "the other workspace must not leak into the fresh mirror:\n{seeded}"
    );
    let registered = server
        .get_session(ACTIVE_WORKSPACE_SESSION_ID)
        .expect("session registered");

    // Criteria 2+3: the desktop switches workspaces; the SAME session
    // re-seeds to the new workspace's pane without a reconnect.
    control.send("select-workspace -t second").expect("switch");
    wait_mirror(&session, "WS2-MARKER");
    let switched = session.terminal.read().content();
    assert!(
        !has_line(&switched, "WS1-MARKER"),
        "the re-seed reset the mirror; old pane content must be gone:\n{switched}"
    );
    let after = server
        .get_session(ACTIVE_WORKSPACE_SESSION_ID)
        .expect("session still registered after the switch");
    assert!(
        Arc::ptr_eq(&registered, &after),
        "the switch must not close or recreate the streaming session"
    );

    // Input follows: typed lines now reach the new workspace's pane.
    type_line(&session, "echo FOLLOW-INPUT");
    wait_pane(&mut control, &pane1, "FOLLOW-INPUT");
    let old = control
        .send(&format!("capture-pane -t {pane0}"))
        .expect("capture")
        .join("\n");
    assert!(
        !has_line(&old, "FOLLOW-INPUT"),
        "post-switch input must reach the new pane only:\n{old}"
    );
}

/// Card 01a18391648f7679948a984246785911: a `workspace-active` dial's
/// Connected handshake reports the RESOLVED pane id (`pane-N`), not the
/// sentinel — at attach and again after a desktop workspace switch —
/// while any other id keeps echoing verbatim.
#[test]
fn a_workspace_active_handshake_names_the_resolved_pane() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane0) = daemon();
    let pane0_num: u32 = pane0.trim_start_matches('%').parse().expect("pane number");

    // A second workspace; `new-workspace` selects it, so switch back
    // to `main` before the viewer attaches.
    control
        .send("new-workspace -n second")
        .expect("new-workspace");
    let second_pane = control
        .send("list-panes -F '#{pane_id} #{session_name}'")
        .expect("list-panes")
        .into_iter()
        .find(|l| l.trim().ends_with("second"))
        .and_then(|l| {
            l.split_whitespace()
                .next()
                .and_then(|p| p.trim_start_matches('%').parse::<u32>().ok())
        })
        .expect("the second workspace's pane");
    let pane1 = format!("%{second_pane}");
    control
        .send("select-workspace -t main")
        .expect("back to main");

    let server = streaming(&socket);
    let session = mirror_session(&server, ACTIVE_WORKSPACE_SESSION_ID);

    // The handshake names the pane the mirror landed on, not the
    // sentinel the client dialed.
    let connected = session.build_connect_message("dialer", false);
    let ServerMessage::Connected { session_id, .. } = connected else {
        panic!("expected a Connected message");
    };
    assert_eq!(
        session_id,
        format!("pane-{pane0_num}"),
        "the workspace-active handshake names the resolved pane"
    );

    // Any other id echoes verbatim (here: the first-pane fallback).
    let fallback = mirror_session(&server, "fallback");
    let connected = fallback.build_connect_message("dialer", false);
    let ServerMessage::Connected { session_id, .. } = connected else {
        panic!("expected a Connected message");
    };
    assert_eq!(
        session_id, "fallback",
        "non-sentinel ids keep the verbatim echo"
    );

    // After a desktop workspace switch a fresh handshake names the
    // pane being mirrored NOW: the retarget wrote the shared pane.
    control
        .send(&format!("send-keys -t {pane1} 'echo WS2-MARKER' Enter"))
        .expect("send-keys");
    wait_pane(&mut control, &pane1, "WS2-MARKER");
    control.send("select-workspace -t second").expect("switch");
    wait_mirror(&session, "WS2-MARKER");
    let connected = session.build_connect_message("late-joiner", false);
    let ServerMessage::Connected { session_id, .. } = connected else {
        panic!("expected a Connected message");
    };
    assert_eq!(
        session_id,
        format!("pane-{second_pane}"),
        "the handshake follows the retarget"
    );
}

/// A `workspace-active` retarget re-tags frames with the NEW pane:
/// the re-seed replaces the store (the old pane's frames and their
/// tags are gone), and a fresh `Connected.graphics` replay — stamped
/// from the factory's side map — carries the new pane's session id.
#[test]
fn a_workspace_reseed_tags_frames_with_the_new_pane() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane0) = daemon();
    let pane0_num: u32 = pane0.trim_start_matches('%').parse().expect("pane number");

    control
        .send("new-workspace -n second")
        .expect("new-workspace");
    let second_pane = control
        .send("list-panes -F '#{pane_id} #{session_name}'")
        .expect("list-panes")
        .into_iter()
        .find(|l| l.trim().ends_with("second"))
        .and_then(|l| {
            l.split_whitespace()
                .next()
                .and_then(|p| p.trim_start_matches('%').parse::<u32>().ok())
        })
        .expect("the second workspace's pane");
    let pane1 = format!("%{second_pane}");
    control
        .send("select-workspace -t main")
        .expect("back to main");

    // Pane 0 (the active workspace's pane) holds a graphic BEFORE the
    // mirror attaches, so its replayed frame carries the OLD pane's tag.
    let line =
        b"printf '\\033_Ga=T,f=24,i=77,s=2,v=2;AAAAAAAAAAAAAAAA\\033\\\\'; echo GFX1-PLACED\r";
    let hex: String = line
        .iter()
        .map(|b| format!("{b:02x} "))
        .collect::<String>()
        .trim_end()
        .to_string();
    control
        .send(&format!("send-keys -t {pane0} -H {hex}"))
        .expect("send-keys");
    wait("GFX1-PLACED in pane0", || {
        let text = control
            .send(&format!("capture-pane -t {pane0}"))
            .expect("capture")
            .join("\n");
        (!has_line(&text, "GFX1-PLACED")).then_some(text)
    });

    let server = streaming(&socket);
    let session = mirror_session(&server, ACTIVE_WORKSPACE_SESSION_ID);
    let connected = session.build_connect_message("late-joiner", false);
    let ServerMessage::Connected { graphics, .. } = connected else {
        panic!("expected a Connected message");
    };
    assert_eq!(
        graphics.len(),
        1,
        "the pre-attach graphic replays to the late joiner: {graphics:?}"
    );
    assert_eq!(
        graphics[0].pane.as_deref(),
        Some(format!("pane-{pane0_num}").as_str()),
        "the seed's frame carries the OLD pane's tag"
    );

    // Pane 1 holds its own graphic; the desktop switches workspaces
    // and the mirror re-seeds IN PLACE.
    let line =
        b"printf '\\033_Ga=T,f=24,i=88,s=2,v=2;BBBBBBBBBBBBBBBB\\033\\\\'; echo GFX2-PLACED\r";
    let hex: String = line
        .iter()
        .map(|b| format!("{b:02x} "))
        .collect::<String>()
        .trim_end()
        .to_string();
    control
        .send(&format!("send-keys -t {pane1} -H {hex}"))
        .expect("send-keys");
    wait("GFX2-PLACED in pane1", || {
        let text = control
            .send(&format!("capture-pane -t {pane1}"))
            .expect("capture")
            .join("\n");
        (!has_line(&text, "GFX2-PLACED")).then_some(text)
    });

    control.send("select-workspace -t second").expect("switch");
    wait_mirror(&session, "GFX2-PLACED");

    let connected = session.build_connect_message("late-joiner", false);
    let ServerMessage::Connected { graphics, .. } = connected else {
        panic!("expected a Connected message");
    };
    assert_eq!(
        graphics.len(),
        1,
        "the re-seed replaced the store; the old pane's frame is gone: {graphics:?}"
    );
    assert!(
        graphics[0].image_id >= SEED_GRAPHICS_ID_OFFSET,
        "the surviving frame is a seed frame"
    );
    assert_eq!(
        graphics[0].pane.as_deref(),
        Some(format!("pane-{second_pane}").as_str()),
        "the re-seeded frame carries the NEW pane's tag"
    );
}

#[test]
fn a_mirror_is_seeded_from_the_pane_then_follows_its_output() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane) = daemon();

    // Output that exists BEFORE the viewer attaches arrives via the seed.
    control
        .send(&format!("send-keys -t {pane} '{TYPED_BEFORE}' Enter"))
        .expect("send-keys");
    wait("SEED-BEFORE in the pane", || {
        let text = control
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        (!has_line(&text, "SEED-BEFORE")).then_some(text)
    });

    let server = streaming(&socket);
    let session = mirror(&server, &pane);
    let seeded = session.terminal.read().content();
    assert!(
        has_line(&seeded, "SEED-BEFORE"),
        "the seed replays the pane's existing screen:\n{seeded}"
    );

    // Output produced AFTER the attach arrives as applied %output, and it
    // is driven by the viewer's own input (send-keys -H).
    type_line(&session, TYPED_AFTER);
    wait_mirror(&session, "DELTA-AFTER");
}

/// A pane graphic streams to the mirror's viewers: the raw `%output`
/// decodes on the mirror, and the raised GraphicsAdded becomes the cheap
/// announce plus the pane-tagged InlineImage frame — pixels included,
/// sent once (a bounded quiet window after the first frame stays empty;
/// the drain acts per event, never per poll).
#[test]
fn a_pane_graphic_streams_as_an_inline_image_frame() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane) = daemon();
    let pane_num: u32 = pane.trim_start_matches('%').parse().expect("pane number");

    let server = streaming(&socket);
    let session = mirror(&server, &pane);
    let mut updates = session.broadcast_tx.subscribe();

    // A 2x2 RGB kitty image as PTY output: the hex line is a printf the
    // pane's shell runs (input queues in the pty until the shell reads
    // it, so one send executes exactly once whenever the shell is up).
    // RGB decodes to RGBA in the store: 4 pixels x 4 bytes.
    let line = b"printf '\\033_Ga=T,f=24,i=77,s=2,v=2;AAAAAAAAAAAAAAAA\\033\\\\'\r";
    let hex: String = line
        .iter()
        .map(|b| format!("{b:02x} "))
        .collect::<String>()
        .trim_end()
        .to_string();
    control
        .send(&format!("send-keys -t {pane} -H {hex}"))
        .expect("send-keys");

    let deadline = Instant::now() + CEILING;
    let mut frames = Vec::new();
    let mut announces = 0usize;
    let mut announce_panes: Vec<Option<String>> = Vec::new();
    loop {
        while let Ok(msg) = updates.try_recv() {
            match msg {
                ServerMessage::GraphicsAdded { pane, .. } => {
                    announces += 1;
                    announce_panes.push(pane);
                }
                ServerMessage::InlineImage { .. } => frames.push(msg),
                _ => {}
            }
        }
        if !frames.is_empty() || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !frames.is_empty(),
        "the graphic never streamed as a frame; pane capture:\n{}\nmirror graphics: {}",
        control
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n"),
        session.terminal.read().graphics_count()
    );
    let ServerMessage::InlineImage {
        image_id,
        pane_id,
        format,
        width,
        height,
        col,
        row,
        data,
        pane,
    } = &frames[0]
    else {
        panic!("not an InlineImage frame");
    };
    assert_eq!(*pane_id, Some(pane_num), "the frame names its pane");
    assert_eq!(
        pane.as_deref(),
        Some(format!("pane-{pane_num}").as_str()),
        "the frame names its pane in session-id form"
    );
    assert_eq!(format, "rgba", "store pixels are RGBA");
    assert_eq!((*width, *height), (2, 2));
    assert_eq!(data.len(), 16, "2x2 RGB decoded to RGBA");
    assert!(*image_id > 0);
    let terminal = session.terminal.read();
    let stored = terminal
        .all_graphics()
        .iter()
        .find(|g| g.id == *image_id)
        .expect("the mirror store holds the graphic");
    assert_eq!(
        stored.position,
        (*col as usize, *row as usize),
        "the frame carries the graphic's placement"
    );
    assert!(announces >= 1, "the cheap announce rides along");
    assert_eq!(
        announce_panes.first(),
        Some(&Some(format!("pane-{pane_num}"))),
        "the GraphicsAdded announce names the serving pane"
    );

    // Sent once: a bounded quiet window after the first frame sees no
    // second frame for the same placement.
    std::thread::sleep(Duration::from_millis(300));
    let dupes = std::iter::from_fn(|| updates.try_recv().ok())
        .filter(|m| matches!(m, ServerMessage::InlineImage { .. }))
        .count();
    assert_eq!(dupes, 0, "the frame must not re-stream on later polls");
}

/// A graphic placed BEFORE the mirror attaches replays to a late
/// joiner through `Connected.graphics`: the seed's `-G` section
/// carries the pane's placements into the mirror store (the card's
/// regression — the seed reproduced the visible grid but not the
/// images, so late joiners saw a pane without its graphics), and a
/// placement after attach still streams live as an InlineImage frame.
#[test]
fn a_graphic_placed_before_attach_replays_in_connected_graphics() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane) = daemon();
    let pane_num: u32 = pane.trim_start_matches('%').parse().expect("pane number");

    // The image emission and a marker ride one send-keys: the marker
    // reaching the pane capture proves the pane terminal processed
    // the image bytes above it, so the mirror below attaches AFTER
    // the placement deterministically.
    let line =
        b"printf '\\033_Ga=T,f=24,i=77,s=2,v=2;AAAAAAAAAAAAAAAA\\033\\\\'; echo GFX-PLACED\r";
    let hex: String = line
        .iter()
        .map(|b| format!("{b:02x} "))
        .collect::<String>()
        .trim_end()
        .to_string();
    control
        .send(&format!("send-keys -t {pane} -H {hex}"))
        .expect("send-keys");
    wait("GFX-PLACED in the pane", || {
        let text = control
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        (!has_line(&text, "GFX-PLACED")).then_some(text)
    });

    let server = streaming(&socket);
    let session = mirror(&server, &pane);
    let connected = session.build_connect_message("late-joiner", false);
    let ServerMessage::Connected { graphics, .. } = connected else {
        panic!("expected a Connected message");
    };
    assert_eq!(
        graphics.len(),
        1,
        "the pre-attach graphic replays to the late joiner: {graphics:?}"
    );
    let frame = &graphics[0];
    assert_eq!(frame.format, "rgba", "store pixels replay as RGBA");
    assert_eq!((frame.width, frame.height), (2, 2));
    assert_eq!(frame.data.len(), 16, "2x2 RGB decoded to RGBA");
    assert_eq!(
        frame.pane.as_deref(),
        Some(format!("pane-{pane_num}").as_str()),
        "the replay names the pane the seed came from"
    );
    assert!(
        frame.image_id >= SEED_GRAPHICS_ID_OFFSET,
        "seed ids ride the offset, out of the mirror counter's range"
    );
    // The read guard is scoped: holding it past this block would block
    // the drain's write for the live leg below, and the live frame
    // would never arrive.
    {
        let mirror = session.terminal.read();
        let stored = mirror
            .all_graphics()
            .iter()
            .find(|g| g.id == frame.image_id)
            .expect("the seed ingested into the mirror store");
        assert_eq!((stored.width, stored.height), (2, 2));
        assert_eq!(
            stored.position,
            (frame.col as usize, frame.row as usize),
            "the replay carries the stored placement"
        );
    }

    // The live leg is untouched: a placement after attach still
    // arrives as the pane-tagged InlineImage frame.
    let mut updates = session.broadcast_tx.subscribe();
    let line = b"printf '\\033_Ga=T,f=24,i=88,s=2,v=2;BBBBBBBBBBBBBBBB\\033\\\\'\r";
    let hex: String = line
        .iter()
        .map(|b| format!("{b:02x} "))
        .collect::<String>()
        .trim_end()
        .to_string();
    control
        .send(&format!("send-keys -t {pane} -H {hex}"))
        .expect("send-keys");
    let deadline = Instant::now() + CEILING;
    let mut frames = Vec::new();
    loop {
        while let Ok(msg) = updates.try_recv() {
            if let ServerMessage::InlineImage { .. } = msg {
                frames.push(msg);
            }
        }
        if !frames.is_empty() || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !frames.is_empty(),
        "a placement after attach must still stream live; mirror graphics: {} pane capture:\n{}",
        session.terminal.read().graphics_count(),
        control
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n"),
    );
    let ServerMessage::InlineImage {
        image_id,
        pane_id,
        data,
        ..
    } = &frames[0]
    else {
        panic!("not an InlineImage frame");
    };
    assert_eq!(*pane_id, Some(pane_num), "the live frame names its pane");
    assert!(
        *image_id < SEED_GRAPHICS_ID_OFFSET,
        "live frames draw ids from the mirror's own counter"
    );
    assert_eq!(data.len(), 16, "the live frame carries the decoded pixels");
}

#[test]
fn the_mirror_starts_at_the_pane_size_and_a_viewer_resize_refits_the_pane() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane) = daemon();
    control
        .send(&format!("refresh-client -t {pane} -C 100x30"))
        .expect("the desktop's size");

    let server = streaming(&socket);
    let session = mirror(&server, &pane);
    assert_eq!(
        session.terminal.read().size(),
        (100, 30),
        "the mirror seeds at the PANE's size, not the viewer's"
    );
    assert_eq!(
        pane_size(&mut control, &pane),
        (100, 30),
        "connecting a viewer must not resize the pane"
    );

    // A viewer resize is a deliberate request: it is recorded as the
    // viewer's sizing contribution and the pane re-fits to the
    // smallest-viewer minimum — here the viewer alone, 70x20. The
    // mirror follows via %layout-change.
    session.resize_tx.send((70, 20)).expect("resize request");
    wait("the pane and mirror at 70x20", || {
        let pane_now = pane_size(&mut control, &pane);
        let mirror_now = session.terminal.read().size();
        (pane_now != (70, 20) || mirror_now != (70, 20))
            .then(|| format!("pane {pane_now:?}, mirror {mirror_now:?}"))
    });

    // A larger report from elsewhere (the desktop) does NOT lift the
    // grid: the attached viewer still displays the window at 70x20,
    // and the window's extent is the componentwise minimum over its
    // viewers. The desktop's report only becomes its own contribution.
    control
        .send(&format!("refresh-client -t {pane} -C 90x25"))
        .expect("desktop resize");
    wait("the pane and mirror still at the viewer's 70x20", || {
        let pane_now = pane_size(&mut control, &pane);
        let mirror_now = session.terminal.read().size();
        (pane_now != (70, 20) || mirror_now != (70, 20))
            .then(|| format!("pane {pane_now:?}, mirror {mirror_now:?}"))
    });
}

#[test]
fn viewer_status_tracks_attach_membership_and_unchanged_grid_constraints() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane) = daemon();
    control
        .send(&format!("refresh-client -t {pane} -C 80x24"))
        .unwrap();
    let server = streaming(&socket);
    let session = mirror(&server, &pane);
    let initial = session.pane_size_status().expect("initial status");
    let ServerMessage::PaneSizeStatus {
        cols,
        rows,
        column_viewer: Some(first),
        ..
    } = initial
    else {
        panic!("status")
    };
    assert_eq!((cols, rows), (80, 24));
    let mut updates = session.broadcast_tx.subscribe();
    let mut other = MuxClient::connect(&socket).unwrap();
    other
        .send(&format!("refresh-client -t {pane} -C 80x24"))
        .unwrap();
    wait("two attach viewers", || {
        let presence = session.viewer_presence();
        (!matches!(presence, ServerMessage::ViewerPresence { count: 2, .. }))
            .then(|| format!("{presence:?}"))
    });
    assert!(std::iter::from_fn(|| updates.try_recv().ok())
        .any(|m| matches!(m, ServerMessage::ViewerPresence { count: 2, .. })));
    drop(control);
    wait("constraint owner changed without resizing", || {
        let status = session.pane_size_status();
        (!matches!(&status, Some(ServerMessage::PaneSizeStatus { cols: 80, rows: 24, column_viewer: Some(owner), .. }) if owner != &first))
            .then(|| format!("{status:?}"))
    });
    assert!(
        std::iter::from_fn(|| updates.try_recv().ok()).any(|m| matches!(
            m,
            ServerMessage::PaneSizeStatus {
                cols: 80,
                rows: 24,
                ..
            }
        ))
    );
    wait("one remaining attach viewer", || {
        let presence = session.viewer_presence();
        (!matches!(presence, ServerMessage::ViewerPresence { count: 1, .. }))
            .then(|| format!("{presence:?}"))
    });
    drop(other);
    wait("last attach viewer left", || {
        let presence = session.viewer_presence();
        (!matches!(presence, ServerMessage::ViewerPresence { count: 0, .. }))
            .then(|| format!("{presence:?}"))
    });
}

#[test]
fn closing_a_resized_mirror_withdraws_its_size_claim() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane) = daemon();
    control
        .send(&format!("refresh-client -t {pane} -C 100x30"))
        .expect("desktop size");
    let server = streaming(&socket);
    let session = mirror(&server, &pane);
    session.resize_tx.send((40, 12)).expect("phone fit");
    wait("phone size", || {
        let size = pane_size(&mut control, &pane);
        (size != (40, 12)).then(|| format!("pane {size:?}"))
    });

    assert!(server.close_session(&session.id, "viewer left".to_string()));
    wait("desktop size after mirror teardown", || {
        let size = pane_size(&mut control, &pane);
        (size != (100, 30)).then(|| format!("pane {size:?}"))
    });
}

async fn next_wire_message(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
    mut matches: impl FnMut(&ServerMessage) -> bool,
) -> ServerMessage {
    use futures_util::StreamExt;
    tokio::time::timeout(CEILING, async {
        loop {
            let frame = socket.next().await.expect("open websocket").expect("frame");
            let message = match frame {
                tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                    crate::streaming::decode_server_message(&bytes).expect("protobuf")
                }
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    serde_json::from_str(&text).expect("JSON")
                }
                _ => continue,
            };
            if matches(&message) {
                return message;
            }
        }
    })
    .await
    .expect("wire message before deadline")
}

/// Card criterion 3: a phone drives a chunked upload through the
/// mux-control channel (the streamer's per-session daemon connection
/// surface, reached here over the shared WebSocket dispatch), and the
/// pane receives the bracket-pasted materialized path. The daemon-side
/// file holds exactly the bytes the client sent.
#[test]
fn a_client_upload_reaches_the_pane_as_a_bracket_pasted_path() {
    use crate::streaming::protocol::ClientMessage;

    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane) = daemon();
    let server = streaming(&socket);
    let session = mirror(&server, &pane);
    let pane_num: u32 = pane.trim_start_matches('%').parse().expect("pane number");

    // A WebSocket client (the phone) attaches to the mirrored pane.
    let params = ConnectionParams::from_uri_query(Some(&format!("session={}", session.id)));
    let (mut phone, connection) = rt.block_on(async {
        let (phone, connection) = tokio::io::duplex(65536);
        (
            tokio_tungstenite::WebSocketStream::from_raw_socket(
                phone,
                tokio_tungstenite::tungstenite::protocol::Role::Client,
                None,
            )
            .await,
            tokio_tungstenite::WebSocketStream::from_raw_socket(
                connection,
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await,
        )
    });
    let serving = Arc::clone(&server);
    let websocket = rt.spawn(async move {
        serving
            .handle_test_ws(connection, &params)
            .await
            .expect("websocket session");
    });
    wait("websocket registered", || {
        (session.client_count() != 1).then(|| "no websocket client".to_string())
    });

    // The wire is binary protobuf (Text frames are logged and skipped
    // by both transports), so the phone's messages encode through the
    // same encoder a real client uses.
    async fn send_msg(
        phone: &mut tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
        msg: ClientMessage,
    ) {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;
        let bytes =
            crate::streaming::proto::encode_client_message(&msg).expect("encode client message");
        phone
            .send(Message::binary(bytes))
            .await
            .expect("client send");
    }
    // Begin: the daemon id rides back as the direct upload_started reply.
    rt.block_on(send_msg(
        &mut phone,
        ClientMessage::UploadBegin {
            filename: "phone-upload.txt".into(),
            total_bytes: 11,
        },
    ));
    let upload_id = match rt.block_on(async {
        next_wire_message(&mut phone, |m| {
            matches!(m, ServerMessage::UploadStarted { .. })
        })
        .await
    }) {
        ServerMessage::UploadStarted { upload_id } => upload_id,
        other => panic!("expected upload_started, got {other:?}"),
    };
    assert!(upload_id > 0, "begin must report the daemon id");

    // Chunk + commit back-to-back: the pipe's single worker serializes
    // them, so commit can never overtake the chunk it follows.
    rt.block_on(send_msg(
        &mut phone,
        ClientMessage::UploadChunk {
            data: b"hello world".to_vec(),
        },
    ));
    rt.block_on(send_msg(&mut phone, ClientMessage::UploadCommit));
    let path = match rt.block_on(async {
        next_wire_message(&mut phone, |m| {
            matches!(m, ServerMessage::UploadFinished { .. })
        })
        .await
    }) {
        ServerMessage::UploadFinished {
            upload_id: finished_id,
            path,
            error,
        } => {
            assert!(
                error.is_none(),
                "commit failed: {error:?} (id {finished_id})"
            );
            assert_eq!(finished_id, upload_id, "the finished id matches begin");
            path.expect("a committed upload_finished carries the path")
        }
        other => panic!("expected upload_finished, got {other:?}"),
    };
    assert!(
        path.contains("phone-upload.txt"),
        "the pasted path carries the client's name: {path}"
    );
    assert_eq!(
        std::fs::read(&path).expect("materialized bytes"),
        b"hello world".to_vec(),
        "the staged file holds exactly the chunk bytes"
    );
    std::fs::remove_file(&path).expect("test cleanup");

    // The pane received the path (plain shell: no bracketed-paste
    // markers, the raw path lands at the prompt). Substring match:
    // the shared staging directory suffixes the name -N when a
    // previous run's committed file still holds the primary name.
    wait("the pasted path in the pane", || {
        let text = control
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        (!text.contains("phone-upload.txt")).then_some(text)
    });

    // Settled state allows a fresh begin; the abort settles with an
    // `aborted` upload_finished (its spool file is deleted daemon-side).
    rt.block_on(send_msg(
        &mut phone,
        ClientMessage::UploadBegin {
            filename: "phone-upload-2.txt".into(),
            total_bytes: 2,
        },
    ));
    rt.block_on(send_msg(&mut phone, ClientMessage::UploadAbort));
    match rt.block_on(async {
        next_wire_message(&mut phone, |m| {
            matches!(m, ServerMessage::UploadFinished { .. })
        })
        .await
    }) {
        ServerMessage::UploadFinished {
            error: Some(reason),
            ..
        } => {
            assert_eq!(reason, "aborted");
        }
        other => panic!("expected the aborted settlement, got {other:?}"),
    };

    // The upload ran against pane %N — the mirror's pane, addressed by
    // the daemon verb the pipe forwarded (pane_num only pins the test).
    assert_eq!(link_pane(&session.id), Some(pane_num));

    rt.block_on(async {
        phone.close(None).await.expect("phone closes");
        websocket.await.expect("websocket task");
    });
}

/// Test-only bridge: the mirror link's pane for `session_id`, so the
/// e2e pins the upload landed on the mirrored pane.
fn link_pane(session_id: &str) -> Option<u32> {
    // The mirror id is `pane-N` by construction in these tests.
    session_id
        .strip_prefix("pane-")
        .and_then(|n| n.parse().ok())
}

#[test]
fn last_websocket_disconnect_releases_the_mirror() {
    use tokio_tungstenite::tungstenite::protocol::Role;

    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane) = daemon();
    control
        .send(&format!("refresh-client -t {pane} -C 100x30"))
        .expect("desktop size");
    let server = streaming(&socket);
    let session = mirror(&server, &pane);
    let params = ConnectionParams::from_uri_query(Some(&format!("session={}", session.id)));
    let (mut phone, connection) = rt.block_on(async {
        let (phone, connection) = tokio::io::duplex(65536);
        (
            tokio_tungstenite::WebSocketStream::from_raw_socket(phone, Role::Client, None).await,
            tokio_tungstenite::WebSocketStream::from_raw_socket(connection, Role::Server, None)
                .await,
        )
    });
    let serving = Arc::clone(&server);
    let websocket = rt.spawn(async move {
        serving
            .handle_test_ws(connection, &params)
            .await
            .expect("websocket session");
    });
    wait("websocket registered", || {
        (session.client_count() != 1).then(|| "no websocket client".to_string())
    });
    rt.block_on(async {
        next_wire_message(&mut phone, |m| {
            matches!(
                m,
                ServerMessage::PaneSizeStatus {
                    cols: 100,
                    rows: 30,
                    ..
                }
            )
        })
        .await;
        next_wire_message(&mut phone, |m| {
            matches!(m, ServerMessage::ViewerPresence { count: 2, .. })
        })
        .await;
    });
    session.resize_tx.send((40, 12)).expect("phone fit");
    wait("phone size", || {
        let size = pane_size(&mut control, &pane);
        (size != (40, 12)).then(|| format!("pane {size:?}"))
    });
    rt.block_on(async {
        next_wire_message(&mut phone, |m| matches!(m, ServerMessage::PaneSizeStatus { cols: 40, rows: 12, column_viewer: Some(owner), .. } if owner.starts_with("stream:"))).await;
    });
    let (mut second_phone, second_connection) = rt.block_on(async {
        let (phone, connection) = tokio::io::duplex(65536);
        (
            tokio_tungstenite::WebSocketStream::from_raw_socket(phone, Role::Client, None).await,
            tokio_tungstenite::WebSocketStream::from_raw_socket(connection, Role::Server, None)
                .await,
        )
    });
    let serving = Arc::clone(&server);
    let params = ConnectionParams::from_uri_query(Some(&format!("session={}", session.id)));
    let second_websocket = rt.spawn(async move {
        serving
            .handle_test_ws(second_connection, &params)
            .await
            .expect("second viewer");
    });
    wait("second websocket registered", || {
        (session.client_count() != 2).then(|| "second viewer absent".to_string())
    });
    rt.block_on(async {
        next_wire_message(&mut phone, |m| {
            matches!(m, ServerMessage::ViewerPresence { count: 3, .. })
        })
        .await;
        next_wire_message(&mut second_phone, |m| {
            matches!(m, ServerMessage::ViewerPresence { count: 3, .. })
        })
        .await;
    });
    rt.block_on(async {
        phone.close(None).await.expect("phone closes");
        websocket.await.expect("websocket task");
    });
    rt.block_on(async {
        next_wire_message(&mut second_phone, |m| {
            matches!(m, ServerMessage::ViewerPresence { count: 2, .. })
        })
        .await;
    });
    assert_eq!(session.client_count(), 1);
    assert!(server.get_session(&session.id).is_some());
    assert_eq!(pane_size(&mut control, &pane), (40, 12));
    rt.block_on(async {
        second_phone.close(None).await.expect("last viewer closes");
        second_websocket.await.expect("last websocket task");
    });
    wait("desktop size after websocket disconnect", || {
        let size = pane_size(&mut control, &pane);
        (size != (100, 30)).then(|| format!("pane {size:?}"))
    });
    assert!(server.get_session(&session.id).is_none());
    let reconnected = mirror(&server, &pane);
    assert_eq!(reconnected.terminal.read().size(), (100, 30));
    type_line(&reconnected, TYPED_AFTER);
    wait_mirror(&reconnected, "DELTA-AFTER");
}

/// Mouse reports: the server's Mouse arm encodes with the session
/// terminal's `report_mouse` and writes the bytes to `pty_writer`. For a
/// mirror that only works if the pane's mouse mode reached it as a delta.
/// Unix-only: the pane runs `cat -v` to make the received report visible.
#[cfg(unix)]
#[test]
fn a_mouse_report_encoded_by_the_mirror_reaches_the_pane() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, mut control, pane) = daemon();
    let server = streaming(&socket);
    let session = mirror(&server, &pane);

    type_line(&session, r#"printf '\033[?1000h'; echo MOUSE""-ON; cat -v"#);
    wait_mirror(&session, "MOUSE-ON");
    wait("the mirror in mouse mode", || {
        let mode = session.terminal.read().mouse_mode();
        (mode == crate::mouse::MouseMode::Off).then(|| format!("{mode:?}"))
    });

    let report = session
        .terminal
        .write()
        .report_mouse(crate::mouse::MouseEvent::new(0, 4, 2, true, 0));
    assert_eq!(report, b"\x1b[M %#", "an X10 press at col 4 row 2");
    let writer = session
        .pty_writer
        .read()
        .as_ref()
        .cloned()
        .expect("input path");
    {
        let mut w = writer.lock();
        w.write_all(&report).expect("mouse write");
        w.write_all(b"\r").expect("flush the line to cat");
        w.flush().expect("flush");
    }
    wait("the report echoed by cat -v in the pane", || {
        let text = control
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture")
            .join("\n");
        (!text.contains("^[[M %#")).then_some(text)
    });
}

#[test]
fn two_viewers_of_one_pane_both_stay_live() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, _serving, _control, pane) = daemon();

    // Two streaming servers stand for two independent mobile viewers:
    // each holds its own daemon connection to the same pane.
    let viewer_a = streaming(&socket);
    let viewer_b = streaming(&socket);
    let a = mirror(&viewer_a, &pane);
    let b = mirror(&viewer_b, &pane);

    type_line(&a, TYPED_AFTER);
    wait_mirror(&a, "DELTA-AFTER");
    wait_mirror(&b, "DELTA-AFTER");
}

/// Every typed frame, alternating the queued path and the direct writer
/// across resizes and a second viewer, lands exactly once in the
/// pane-side file.
#[test]
fn stress_input_frames_of_varied_sizes_all_land_exactly_once() {
    let rt = runtime();
    let _entered = rt.enter();
    let (dir, socket, _serving, _control, pane) = daemon();
    let viewer_a = streaming(&socket);
    // A second viewer doubles the pane's %output fanout and the daemon
    // connections racing the input socket.
    let viewer_b = streaming(&socket);
    let a = mirror(&viewer_a, &pane);
    let _b = mirror(&viewer_b, &pane);

    // Keystroke-sized frames (the original observation was ~6-16 B
    // sends): everything typed behind a running `cat` is one file line
    // per marker, so a 4-char marker + CR is a true 5 B input frame.
    #[cfg(unix)]
    let mut expected_cat: Vec<String> = Vec::new();
    #[cfg(unix)]
    {
        let cat_file = dir.path().join("keystrokes.txt");
        type_line(&a, &format!("cat >> {}", cat_file.display()));
        // The shell opens the redirect target before exec, so the file
        // existing means `cat` owns the terminal's input from here on.
        let deadline = Instant::now() + CEILING;
        while !cat_file.exists() {
            assert!(Instant::now() < deadline, "cat never started");
            std::thread::sleep(Duration::from_millis(20));
        }
        for i in 0..30u32 {
            let marker = format!("T{i:02}");
            expected_cat.push(marker.clone());
            let frame = format!("{marker}\r");
            if i % 2 == 0 {
                a.enqueue_pty_input(frame.into_bytes());
            } else {
                let writer = a.pty_writer.read().as_ref().cloned().expect("input path");
                let mut w = writer.lock();
                w.write_all(frame.as_bytes()).expect("keystroke write");
                w.flush().expect("flush");
            }
        }
        // EOF ends `cat`: the pane is a shell again for phase 2. It
        // must take the same queue as the frames above it — a direct
        // writer write races the drain and lands first, so `cat`
        // exits and the queued frames arrive at the shell instead of
        // the file.
        a.enqueue_pty_input(b"\x04".to_vec());
        let deadline = Instant::now() + CEILING;
        while expected_cat.len()
            != std::fs::read_to_string(&cat_file)
                .map(|t| t.lines().count())
                .unwrap_or(0)
        {
            assert!(
                Instant::now() < deadline,
                "keystroke frames never fully appended; rerun with DEBUG_LEVEL=1 — \
                 the debug log names the dropping side"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut want = expected_cat;
        let mut got: Vec<String> = std::fs::read_to_string(&cat_file)
            .expect("keystrokes file")
            .lines()
            .map(str::trim)
            .map(str::to_string)
            .collect();
        want.sort();
        got.sort();
        assert_eq!(got, want, "keystroke frames lost or duplicated");
    }

    let file = dir.path().join("frames.txt");
    // Marker byte lengths spanning the observed small-frame range; the
    // full typed line (echo + redirect + temp path) runs ~60-330 B.
    const SIZES: [usize; 6] = [12, 24, 64, 120, 180, 240];
    let mut expected: Vec<String> = Vec::new();
    let have = || {
        std::fs::read_to_string(&file)
            .map(|t| t.lines().count())
            .unwrap_or(0)
    };
    for batch in 0..3 {
        for cycle in 0..5 {
            for (i, &size) in SIZES.iter().enumerate() {
                let index = batch * 5 * SIZES.len() + cycle * SIZES.len() + i;
                let mut marker = format!("F{batch}_{cycle:02}_{size}");
                marker.extend(std::iter::repeat_n('x', size - marker.len()));
                expected.push(marker.clone());
                let cmd = format!("echo {marker} >> {}", file.display());
                if index.is_multiple_of(2) {
                    let mut bytes = cmd.into_bytes();
                    bytes.push(b'\r');
                    a.enqueue_pty_input(bytes);
                } else {
                    type_line(&a, &cmd);
                }
                // Resizes interleaved with typing: the original
                // observation sat next to resize traffic.
                if index % 10 == 9 {
                    let to = if (index / 10).is_multiple_of(2) {
                        (120, 40)
                    } else {
                        (80, 24)
                    };
                    a.resize_tx.send(to).expect("resize request");
                }
                // Wait for each line before the next: a PTY's kernel
                // input queue (~1 KB, canonical while the shell runs a
                // child) drops bytes typed faster than the pane drains
                // — plain bash + script(1) loses 24/30 the same way, so
                // that is tty semantics, not this input path. The
                // barrier keeps the send rate within what a pane can
                // consume while preserving the size coverage.
                let want = expected.len();
                let deadline = Instant::now() + CEILING;
                while have() < want {
                    assert!(
                        Instant::now() < deadline,
                        "frame {index}: {}/{} lines reached the pane; rerun \
                         with DEBUG_LEVEL=1 — the debug log names the side",
                        have(),
                        want
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    let got: Vec<String> = std::fs::read_to_string(&file)
        .expect("frames file")
        .lines()
        .map(str::trim)
        .map(str::to_string)
        .collect();
    let mut want = expected;
    let mut got_sorted = got.clone();
    want.sort();
    got_sorted.sort();
    let missing: Vec<&String> = want.iter().filter(|m| !got.contains(*m)).collect();
    let extra: Vec<&String> = got.iter().filter(|m| !want.contains(*m)).collect();
    assert_eq!(
        got_sorted, want,
        "frames lost = {missing:?}, extra/duplicated = {extra:?}"
    );

    // Teardown: the drain task `enqueue_pty_input` spawned lives until
    // every channel sender drops, and this session outlives the test in
    // the streaming server's registry — so drop the sender here or
    // tokio's blocking-pool shutdown blocks past the test's end.
    *a.pty_input_tx.write() = None;
}

#[test]
fn a_killed_pane_marks_its_mirror_dead() {
    let (_dir, socket, _serving, mut control, pane) = daemon();
    let second = control
        .send(&format!("split-window -h -t {pane}"))
        .expect("split")
        .join("")
        .trim()
        .to_string();
    let factory = Arc::new(MuxSessionFactory::new(
        &socket,
        MuxPaneSelector::FromSessionId,
    ));
    let session_id = format!("pane-{}", second.trim_start_matches('%'));
    factory
        .create_session(&session_id, 80, 24, None)
        .expect("mirror the split pane");
    assert!(factory.is_session_alive(&session_id));

    control
        .send(&format!("kill-pane -t {second}"))
        .expect("kill-pane");
    wait("the mirror marked dead", || {
        factory
            .is_session_alive(&session_id)
            .then(|| "still alive".to_string())
    });
}

/// The drain thread closes the mirror off the Tokio runtime when the
/// daemon dies mid-session: the close must complete — registry removal
/// AND the delayed factory teardown — instead of panicking on a bare
/// `tokio::spawn` (the production streamer panic at server.rs:551 in
/// 0.61.x, observed when the upstream daemon died under live sessions).
#[test]
fn dead_upstream_closes_the_mirror_off_the_runtime() {
    let rt = runtime();
    let _entered = rt.enter();
    let (_dir, socket, serving, _control, pane) = daemon();
    let factory = Arc::new(
        MuxSessionFactory::new(&socket, MuxPaneSelector::FromSessionId).with_reply_timeout(CEILING),
    );
    let server = Arc::new(StreamingServer::with_factory(
        "127.0.0.1:0".to_string(),
        StreamingConfig::default(),
        Arc::clone(&factory) as Arc<dyn SessionFactory>,
    ));
    factory.set_streaming_server(Arc::clone(&server));
    let session_id = format!("pane-{}", pane.trim_start_matches('%'));
    let session = mirror(&server, &pane);

    // The mirror is live: typed input lands in the pane and its echo
    // reaches the mirror's terminal.
    type_line(&session, "echo MIRROR-ALIVE-MARKER");
    wait_mirror(&session, "MIRROR-ALIVE-MARKER");

    // Kill the upstream: the drain thread sees EOF and closes the
    // session from a plain std thread, outside any Tokio context.
    drop(serving);
    wait("closed mirror and torn-down factory link", || {
        let closed = server.get_session(&session_id).is_none();
        let link_count = factory.mirror_link_count();
        (!(closed && link_count == 0)).then(|| format!("closed={closed} links={link_count}"))
    });
}

/// A daemon that accepts and never replies must fail `create_session`
/// by its reply deadline instead of blocking the caller forever.
#[test]
fn create_session_times_out_against_a_silent_daemon() {
    let dir = tempfile::Builder::new()
        .prefix("par-mux-silent-")
        .tempdir()
        .expect("temp dir");
    let socket = dir.path().join("s");
    let listener = crate::mux::ipc::bind_local_listener(&socket).expect("bind");
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let silent = std::thread::spawn(move || {
        let (stream, _abort) = crate::mux::ipc::accept_connection(&listener).expect("accept");
        // Hold the connection open, unanswered, until the test is done.
        let _ = release_rx.recv_timeout(Duration::from_secs(30));
        drop(stream);
    });

    let timeout = Duration::from_millis(300);
    let factory =
        MuxSessionFactory::new(&socket, MuxPaneSelector::FromSessionId).with_reply_timeout(timeout);
    let started = Instant::now();
    // "main" is not `pane-N`, so the factory must ask with list-panes.
    let result = factory.create_session("main", 80, 24, None);
    let elapsed = started.elapsed();
    let _ = release_tx.send(());
    silent.join().expect("silent daemon thread");

    let err = match result {
        Ok(_) => panic!("a silent daemon cannot produce a session"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("no reply"), "unexpected error: {err}");
    assert!(
        elapsed >= timeout && elapsed < timeout + Duration::from_secs(5),
        "create_session returned after {elapsed:?}, deadline {timeout:?}"
    );
}
