//! Client CLI/config resolution, cycling, and non-blocking stdin/stdout.

use super::*;

/// ARC-123: the attach options with only the given CLI fields set.
fn cli_options(prefix: Option<&str>, reload: Option<&str>) -> AttachOptions {
    AttachOptions {
        socket: None,
        name: None,
        target: None,
        prefix: prefix.map(str::to_string),
        reload: reload.map(str::to_string),
        mode: None,
    }
}

/// ARC-123: `resolve_client` layers CLI over file over defaults: no
/// file and no flag is the built-in set; `--prefix C-a` sets the
/// prefix byte 0x01.
#[test]
fn resolve_client_cli_prefix_overrides_the_default() {
    let empty = crate::mux::config::ConfigFile::default();
    let base = resolve_client_with(&cli_options(None, None), empty.clone()).expect("defaults");
    assert_eq!(base.chords, crate::mux::config::Chords::with_defaults());
    let resolved =
        resolve_client_with(&cli_options(Some("C-a"), None), empty).expect("C-a resolves");
    assert_eq!(resolved.chords.prefix, 0x01);
}

/// ARC-123: the CLI tier wins over the file's `[client]` chords, and
/// the file still supplies everything the CLI does not name.
#[test]
fn resolve_client_cli_wins_over_the_file() {
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nprefix = \"C-a\"\nreload = \"C-a C-s\"\nsplit-right = \"|\"\n")
            .unwrap();
    let from_file = resolve_client_with(&cli_options(None, None), file.clone()).expect("file");
    assert_eq!(from_file.chords.prefix, 0x01);
    assert_eq!(from_file.chords.reload, 0x13);
    assert_eq!(
        from_file.chords.management.split_right,
        crate::mux::config::ChordKey::plain(b'|')
    );
    let cli = resolve_client_with(&cli_options(Some("C-x"), Some("C-x C-e")), file).expect("cli");
    assert_eq!(cli.chords.prefix, 0x18, "--prefix wins over the file");
    assert_eq!(cli.chords.reload, 0x05, "the CLI reload chord wins");
    assert_eq!(
        cli.chords.management.split_right,
        crate::mux::config::ChordKey::plain(b'|'),
        "the file still fills the rest"
    );
}

/// ARC-123: a malformed `--prefix` is one `Handshake` error with the
/// passthrough spelling, whichever mode is attaching.
#[test]
fn resolve_client_rejects_a_malformed_prefix() {
    let err = resolve_client_with(
        &cli_options(Some("bogus-key"), None),
        crate::mux::config::ConfigFile::default(),
    )
    .expect_err("a bad prefix fails");
    match err {
        AttachError::Handshake(err) => {
            assert!(err.to_string().contains("invalid --prefix"), "{err}")
        }
        other => panic!("expected a handshake error, got {other:?}"),
    }
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// ARC-124: the shared cycle rule — forward/back step, wrap at both
/// ends, a one-element ring stays put, a missing current is `None`.
#[test]
fn next_in_cycle_steps_wraps_and_rejects_a_missing_current() {
    let ring = ids(&["@0", "@1", "@2"]);
    assert_eq!(
        next_in_cycle(&ring, "@0", true).map(String::as_str),
        Some("@1")
    );
    assert_eq!(
        next_in_cycle(&ring, "@1", false).map(String::as_str),
        Some("@0")
    );
    assert_eq!(
        next_in_cycle(&ring, "@2", true).map(String::as_str),
        Some("@0"),
        "wraps forward"
    );
    assert_eq!(
        next_in_cycle(&ring, "@0", false).map(String::as_str),
        Some("@2"),
        "wraps back"
    );
    assert_eq!(
        next_in_cycle(&ids(&["$3"]), "$3", true).map(String::as_str),
        Some("$3")
    );
    assert_eq!(next_in_cycle(&ring, "@9", true), None, "missing current");
    assert_eq!(next_in_cycle(&[], "@0", true), None, "empty ring");
}

/// ARC-124: the active-window rule both modes share — the `*` row,
/// else the first `@` row, else `None`.
#[test]
fn active_window_row_takes_the_marked_row_else_the_first_window() {
    assert_eq!(
        active_window_row(&ids(&["@0 - main", "@1 * vim"])).as_deref(),
        Some("@1")
    );
    assert_eq!(
        active_window_row(&ids(&["noise", "@4 - a", "@5 - b"])).as_deref(),
        Some("@4")
    );
    assert_eq!(active_window_row(&[]), None);
}

/// ARC-124: the workspace cycle steps from the ACTIVE row of the
/// `list-workspaces` reply (`+W: name [active]`), wrapping.
#[test]
fn next_workspace_steps_from_the_active_row() {
    let body = ids(&["+0: main", "+1: lab active", "+2: ops"]);
    assert_eq!(next_workspace(&body, true).as_deref(), Some("+2"));
    assert_eq!(next_workspace(&body, false).as_deref(), Some("+0"));
    let last = ids(&["+0: main", "+1: lab active"]);
    assert_eq!(next_workspace(&last, true).as_deref(), Some("+0"), "wraps");
    assert_eq!(
        next_workspace(&ids(&["+0: main"]), true),
        None,
        "no active row"
    );
}

/// A nonblocking host terminal must not end the stdin reader: the
/// pre-fix reader sent `Err(WouldBlock)` and exited, and the pump's
/// `Some(Err(_))` arm detached at the first idle poll (card 01a11d9687d4).
#[test]
#[cfg(unix)]
fn nonblocking_stdin_wouldblock_does_not_end_the_reader() {
    // A real pipe whose read end is O_NONBLOCK — the host-tty shape.
    let mut fds = [0 as libc::c_int; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let (read_fd, write_fd) = (fds[0], fds[1]);
    let flags = unsafe { libc::fcntl(read_fd, libc::F_GETFL) };
    assert_eq!(
        unsafe { libc::fcntl(read_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );

    struct FdReader(libc::c_int);
    impl std::io::Read for FdReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            loop {
                let n = unsafe { libc::read(self.0, buf.as_mut_ptr().cast(), buf.len()) };
                if n < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(err);
                }
                return Ok(n as usize);
            }
        }
    }
    impl Drop for FdReader {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }

    let (tx, rx) = crossbeam_channel::bounded::<StdinMsg>(64);
    Stdin::spawn_reader(FdReader(read_fd), tx);

    // Idle + nonblocking: nothing may arrive. Pre-fix this received
    // Err(WouldBlock) at once and the reader thread ended.
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(120))
            .is_err(),
        "an idle nonblocking read must deliver nothing while idle"
    );

    // The reader is still alive: bytes written now must flow through.
    let written = b"key";
    assert_eq!(
        unsafe { libc::write(write_fd, written.as_ptr().cast(), written.len()) },
        3
    );
    match rx.recv_timeout(std::time::Duration::from_millis(2_000)) {
        Ok(Ok(bytes)) => assert_eq!(bytes, b"key"),
        other => panic!("expected the written bytes, got {other:?}"),
    }

    // Closing the write end still delivers EOF.
    unsafe { libc::close(write_fd) };
    match rx.recv_timeout(std::time::Duration::from_millis(2_000)) {
        Ok(Ok(bytes)) => assert!(bytes.is_empty(), "expected EOF, got {bytes:?}"),
        other => panic!("expected EOF, got {other:?}"),
    }
}

/// The old `let _ = stdout.write_all(data)` shape dropped everything
/// after the first WouldBlock a nonblocking host terminal raised; the
/// blocking writer retries until the host drains (card 01a11d96654c).
#[test]
fn a_blocked_stdout_write_retries_instead_of_dropping_the_chunk() {
    struct BlocksOnce {
        blocked: bool,
        got: Vec<u8>,
    }
    impl std::io::Write for BlocksOnce {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if !self.blocked {
                self.blocked = true;
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            self.got.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut out = BlocksOnce {
        blocked: false,
        got: Vec::new(),
    };
    write_all_blocking(&mut out, b"pane-bytes-1").expect("retry completes the write");
    assert_eq!(
        out.got, b"pane-bytes-1",
        "no byte may be lost to the blocked first write"
    );
}
