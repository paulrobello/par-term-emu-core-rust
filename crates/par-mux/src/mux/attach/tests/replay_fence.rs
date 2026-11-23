//! The passthrough replay fence (card 01a20644): a `%output` chunk
//! stamped at or before the resync reply's `gen=` is already on the
//! replayed screen and must not double-apply.

use super::*;

/// Serve one connection answering the pane replay with `SEED` and the
/// scripted closing flags (`None`: a pre-fence daemon, plain `1`).
fn serve_with_fence(stream: crate::mux::LocalStream, generation: Option<u64>) {
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
        number += 1;
        let block = if trimmed.starts_with("refresh-client -t %0") && !trimmed.contains("-C") {
            let flags = generation.map_or_else(|| "1".to_string(), |g| format!("1 gen={g}"));
            crate::mux::emit_block_flags(number, "\x1b[1;1HSEED", true, &flags)
        } else {
            emit_block(number, "", true)
        };
        writer.write_all(block.as_bytes()).ok();
        writer.flush().ok();
    }
}

fn fenced_session(tag: &str, generation: Option<u64>) -> Session {
    let path = test_socket(tag);
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    std::thread::spawn(move || {
        if let Ok(stream) = listener.accept() {
            serve_with_fence(stream, generation);
        }
    });
    passthrough_session(
        conn::AttachConn::connect(&path).expect("connect"),
        "@0",
        "%0",
    )
}

fn row0(session: &Session) -> String {
    let grid = session.emulator.terminal().active_grid();
    (0..40)
        .map(|col| grid.get(col, 0).expect("cell").c().to_string())
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn output(data: &[u8], generation: Option<u64>) -> TmuxNotification {
    TmuxNotification::Output {
        pane_id: "%0".to_string(),
        data: data.to_vec(),
        generation,
    }
}

/// A chunk stamped at or before the reply's generation is dropped; one
/// above it (and an unstamped one) reaches the shadow.
#[test]
fn a_replay_covered_chunk_arriving_after_the_reply_is_dropped() {
    let mut session = fenced_session("pt-fence-drop", Some(10));
    session.resync();
    assert_eq!(session.output_fence, Some(10));
    let mut dirty = StatusDirty::dirty();

    assert!(session.handle_event(&output(b"STALE", Some(10)), &mut dirty));
    assert!(session.handle_event(&output(b"OLDER", Some(4)), &mut dirty));
    assert_eq!(row0(&session), "SEED", "covered chunks did not apply");

    assert!(session.handle_event(&output(b"+FRESH", Some(11)), &mut dirty));
    assert_eq!(row0(&session), "SEED+FRESH");
    assert!(session.handle_event(&output(b"+RAW", None), &mut dirty));
    assert_eq!(
        row0(&session),
        "SEED+FRESH+RAW",
        "unstamped feeds as always"
    );
}

/// A replay whose reply carries no `gen=` clears a prior fence.
#[test]
fn an_unfenced_replay_clears_the_prior_fence() {
    let mut session = fenced_session("pt-fence-clear", None);
    session.output_fence = Some(100);
    session.resync();
    assert_eq!(session.output_fence, None);
    let mut dirty = StatusDirty::dirty();
    assert!(session.handle_event(&output(b"+X", Some(1)), &mut dirty));
    assert_eq!(row0(&session), "SEED+X");
}

/// `%pane-respawned` resets the fence: the fresh terminal has a fresh
/// generation chain.
#[test]
fn a_respawn_clears_the_fence() {
    let mut session = passthrough_session(
        super::prefix_and_resync::test_dead_conn("pt-fence-respawn"),
        "@0",
        "%0",
    );
    session.output_fence = Some(100);
    let mut dirty = StatusDirty::dirty();
    assert!(session.handle_event(
        &TmuxNotification::PaneRespawned {
            pane_id: "%0".to_string()
        },
        &mut dirty
    ));
    assert_eq!(session.output_fence, None);
    assert!(session.handle_event(&output(b"X", Some(1)), &mut dirty));
    assert_eq!(row0(&session), "X");
}
