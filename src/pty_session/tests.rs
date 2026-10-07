use super::*;

#[test]
fn test_new_pty_session() {
    let session = PtySession::new(80, 24, 1000);
    assert_eq!(session.size(), (80, 24));
    assert!(!session.is_running());
}

/// QA-195: a mutation through the write guard republishes the geometry
/// mirror on drop, so `cursor_position()` is current without PTY output.
#[test]
fn terminal_write_guard_publishes_geometry() {
    let session = PtySession::new(80, 24, 100);
    {
        let mut term = session.terminal_write();
        term.process(b"\x1b[5;10H");
    }
    assert_eq!(session.cursor_position(), (9, 4));
}

/// QA-182: a pixel extent past `u16` saturates instead of overflowing.
#[test]
fn pixel_extent_saturates_instead_of_overflowing() {
    assert_eq!(pixel_extent(80, 10), 800);
    assert_eq!(pixel_extent(2000, 40), u16::MAX);
    assert_eq!(pixel_extent(u16::MAX, u16::MAX), u16::MAX);
}

/// A session with no child (par-mux panes) advances its generation only
/// through `mark_updated` — the reader-thread bump does not exist, and a
/// generation-keyed render cache (par-term's pane cells) depends on the
/// generation advancing to ever re-read fed content.
#[test]
fn mark_updated_advances_generation_for_childless_sessions() {
    let session = PtySession::new(80, 24, 1000);
    assert!(!session.is_running());
    let before = session.update_generation();

    session.mark_updated();
    assert!(session.update_generation() > before);
    assert!(session.has_updates_since(before));
}

#[test]
fn test_with_terminal_accessors() {
    let session = PtySession::new(80, 24, 1000);
    assert_eq!(session.with_terminal(|term| term.size()), (80, 24));

    session.with_terminal_mut(|term| term.process(b"hello"));
    assert!(session
        .with_terminal(|term| term.content())
        .contains("hello"));

    // Both guards are released on return, so a fresh write lock is free.
    assert!(session.terminal_ref().try_write().is_some());
}

#[test]
fn test_get_default_shell() {
    let shell = PtySession::get_default_shell();
    assert!(!shell.is_empty());
}

#[test]
fn test_spawn_and_exit() {
    let mut session = PtySession::new(80, 24, 1000);

    // Spawn a simple command that exits immediately
    #[cfg(unix)]
    let result = session.spawn("/bin/echo", &["hello"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo hello"]);

    assert!(result.is_ok());

    // Give it time to execute
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Process should have exited
    let exit_code = session.try_wait();
    assert!(exit_code.is_ok());
}

#[test]
fn test_write_to_pty() {
    let mut session = PtySession::new(80, 24, 1000);

    // Try writing without spawning - should fail
    let result = session.write(b"test");
    assert!(result.is_err());
}

/// `poll_running` must report an exited child even when the reader flag
/// is stale — the Windows ConPTY condition, where the pipe read never
/// observes EOF (conhost keeps its end open) so `running` stays true
/// after the child is gone. The flag is forced back up after a real
/// child's exit to simulate that reader, and only the OS handle says
/// otherwise.
#[test]
fn poll_running_reports_exit_despite_a_stale_reader_flag() {
    let mut session = PtySession::new(80, 24, 1000);
    #[cfg(unix)]
    session.spawn("/bin/echo", &["bye"]).expect("spawn");
    #[cfg(windows)]
    session
        .spawn("cmd.exe", &["/C", "echo bye"])
        .expect("spawn");

    // Wait for the real exit, then simulate the ConPTY reader that never
    // noticed: the flag goes back up while the OS holds exit status.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !matches!(session.try_wait(), Ok(Some(_))) {
        assert!(
            std::time::Instant::now() < deadline,
            "child never exited for the stale-flag setup"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(!session.poll_running(), "the exited child is dead");

    session.running.store(true, Ordering::SeqCst);
    assert!(
        !session.poll_running(),
        "the OS exit must outweigh a stale reader flag"
    );
    assert!(
        session.child_pid().is_none(),
        "a reaped child's PID is released and must not be served"
    );
}

/// Spawn `/bin/sh -c "exit 3"` and poll `try_wait` until it reports the
/// reap (10 s deadline) — the shared setup for the SEC-125 tests.
#[cfg(unix)]
fn spawn_and_reap_exit_3() -> PtySession {
    let mut session = PtySession::new(80, 24, 1000);
    session.spawn("/bin/sh", &["-c", "exit 3"]).expect("spawn");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match session.try_wait() {
            Ok(Some(code)) => {
                assert_eq!(code, 3, "exit code");
                break;
            }
            _ => {
                assert!(std::time::Instant::now() < deadline, "child never exited");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
    session
}

/// SEC-125: once `try_wait` has reaped the child, its PID belongs to
/// the OS again and may be recycled, so it is no longer served.
#[cfg(unix)]
#[test]
fn reaped_child_reports_no_pid() {
    let mut session = spawn_and_reap_exit_3();
    assert!(session.child_pid().is_none());
    assert_eq!(session.try_wait().ok().flatten(), Some(3), "code is kept");
    assert_eq!(session.wait().ok(), Some(3), "wait serves the record");
}

/// SEC-125: `kill` after the reap must not signal the released PID.
/// portable-pty's `kill` sends a raw SIGHUP first, which fails with
/// ESRCH on a reaped PID (or, once recycled, hits another process).
#[cfg(unix)]
#[test]
fn kill_after_reap_is_a_silent_noop() {
    let mut session = spawn_and_reap_exit_3();
    assert!(session.kill().is_ok(), "kill after reap is a no-op");
    assert!(!session.is_running());
}

/// SEC-125: a resize after the reap updates the grid but sends no
/// SIGWINCH to the released PID.
#[cfg(unix)]
#[test]
fn resize_after_reap_sends_no_signal() {
    let mut session = spawn_and_reap_exit_3();
    let before = session.signals_sent.load(Ordering::SeqCst);
    session.resize(100, 30).expect("resize");
    session
        .resize_with_pixels(100, 30, 1000, 600)
        .expect("resize_with_pixels");
    assert_eq!(session.size(), (100, 30));
    assert_eq!(
        session.signals_sent.load(Ordering::SeqCst),
        before,
        "no SIGWINCH may reach a reaped PID"
    );
}

/// SEC-125: the resize path still signals a live child, so the guard
/// is not simply suppressing every delivery.
#[cfg(unix)]
#[test]
fn resize_of_a_live_child_still_signals() {
    let mut session = PtySession::new(80, 24, 1000);
    session
        .spawn("/bin/sh", &["-c", "sleep 30"])
        .expect("spawn");
    let before = session.signals_sent.load(Ordering::SeqCst);
    session.resize(100, 30).expect("resize");
    assert!(session.signals_sent.load(Ordering::SeqCst) > before);
    session.kill().expect("kill");
}

/// SEC-125: a respawn resets the record, so the new child's PID is
/// served even though the previous child was reaped.
#[cfg(unix)]
#[test]
fn respawn_after_reap_serves_the_new_pid() {
    let mut session = spawn_and_reap_exit_3();
    session
        .spawn("/bin/sh", &["-c", "sleep 30"])
        .expect("respawn");
    assert!(session.child_pid().is_some(), "the new child has a pid");
    assert_eq!(session.try_wait().ok().flatten(), None, "and is running");
    session.kill().expect("kill");
}

#[test]
fn test_resize() {
    let mut session = PtySession::new(80, 24, 1000);
    session.resize(100, 30).ok();
    assert_eq!(session.size(), (100, 30));
}

/// Poll `export_text()` until `marker` appears (5s deadline), then
/// return the final text — the shared shape for spawn-then-assert tests.
fn wait_for_text(session: &PtySession, marker: &str) -> String {
    let start_wait = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);
    loop {
        let content = session.export_text();
        if content.contains(marker) || start_wait.elapsed() > timeout {
            return content;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[test]
fn test_set_env() {
    let mut session = PtySession::new(80, 24, 1000);
    session.set_env("TEST_VAR", "test_value");
    // The stored var must reach the spawned process: echo it through
    // the PTY and assert the value lands in the terminal.
    #[cfg(unix)]
    let result = session.spawn("/bin/sh", &["-c", "echo ZQX-$TEST_VAR"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo ZQX-%TEST_VAR%"]);
    assert!(result.is_ok());
    let content = wait_for_text(&session, "ZQX-test_value");
    assert!(
        content.contains("ZQX-test_value"),
        "env var reached the child: {content}"
    );
}

#[test]
fn test_set_multiple_env_vars() {
    let mut session = PtySession::new(80, 24, 1000);
    session.set_env("VAR1", "value1");
    session.set_env("VAR2", "value2");
    session.set_env("VAR3", "value3");
    // All three stored vars must reach the spawned process together.
    #[cfg(unix)]
    let result = session.spawn("/bin/sh", &["-c", "echo ZQX-$VAR1-$VAR2-$VAR3"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo ZQX-%VAR1%-%VAR2%-%VAR3%"]);
    assert!(result.is_ok());
    let content = wait_for_text(&session, "ZQX-value1-value2-value3");
    assert!(
        content.contains("ZQX-value1-value2-value3"),
        "all env vars reached the child: {content}"
    );
}

#[test]
fn test_set_cwd() {
    let mut session = PtySession::new(80, 24, 1000);
    let path = std::path::Path::new("/tmp");
    session.set_cwd(path);
    // The stored cwd must reach the spawned process: /bin/pwd prints it.
    #[cfg(unix)]
    let result = session.spawn("/bin/pwd", &[]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "cd"]);
    assert!(result.is_ok());
    let content = wait_for_text(&session, "tmp");
    // On macOS /tmp is a symlink to /private/tmp, so accept either
    #[cfg(unix)]
    assert!(
        content.contains("/tmp") || content.contains("private/tmp"),
        "child ran in the set cwd: {content}"
    );
    #[cfg(windows)]
    assert!(
        !content.trim().is_empty(),
        "child printed its cwd: {content}"
    );
}

#[test]
fn test_size_getters() {
    let session = PtySession::new(100, 50, 2000);
    let (cols, rows) = session.size();
    assert_eq!(cols, 100);
    assert_eq!(rows, 50);
}

#[test]
fn test_terminal_access() {
    let session = PtySession::new(80, 24, 1000);
    let terminal = session.terminal();
    let mut guard = terminal.write();
    guard.process(b"ZQX-TERMINAL-ACCESS");
    assert!(
        guard.content().contains("ZQX-TERMINAL-ACCESS"),
        "the write lock grants terminal access"
    );
}

#[test]
fn test_update_generation() {
    let session = PtySession::new(80, 24, 1000);
    let gen1 = session.update_generation();
    let gen2 = session.update_generation();
    assert_eq!(gen1, gen2); // Should be same if no updates
}

#[test]
fn test_wait_for_update_some_after_output() {
    let mut session = PtySession::new(80, 24, 1000);
    let since = session.update_generation();
    #[cfg(unix)]
    let result = session.spawn("/bin/echo", &["wait-marker"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo wait-marker"]);
    assert!(result.is_ok());
    let waited = session.wait_for_update(since, std::time::Duration::from_secs(5));
    assert!(waited.is_some(), "output should advance the generation");
}

#[cfg(unix)]
#[test]
fn test_wait_for_update_none_on_idle_timeout() {
    let mut session = PtySession::new(80, 24, 1000);
    assert!(session.spawn("sleep", &["5"]).is_ok());
    // Drain whatever the spawn itself produced before timing the idle wait.
    let _ = session.wait_for_update(
        session.update_generation(),
        std::time::Duration::from_millis(200),
    );
    let since = session.update_generation();
    let start = std::time::Instant::now();
    let waited = session.wait_for_update(since, std::time::Duration::from_secs(1));
    assert!(waited.is_none(), "idle session must time out");
    assert!(start.elapsed() >= std::time::Duration::from_secs(1));
}

#[cfg(unix)]
#[test]
fn test_wait_for_update_none_promptly_when_child_exits() {
    let mut session = PtySession::new(80, 24, 1000);
    assert!(session.spawn("/usr/bin/true", &[]).is_ok());
    // Wait out the child's lifetime (no output; EOF is the only signal).
    let _ = session.wait_for_update(
        session.update_generation(),
        std::time::Duration::from_secs(5),
    );
    // A wait past the current generation must return on the EOF wake,
    // not sit out the 30 s deadline: the child is gone, nothing coming.
    let since = session.update_generation();
    let start = std::time::Instant::now();
    let waited = session.wait_for_update(since, std::time::Duration::from_secs(30));
    assert!(waited.is_none());
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "EOF must wake the waiter, not the deadline"
    );
}

#[test]
fn test_wait_until_predicate_on_content() {
    let mut session = PtySession::new(80, 24, 1000);
    #[cfg(unix)]
    let result = session.spawn("/bin/echo", &["wait-until-marker"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo wait-until-marker"]);
    assert!(result.is_ok());
    // 30 s budget: the marker arrives in milliseconds unloaded, but the
    // echo child can be starved past 5 s under gate load (observed
    // 2026-09-23 alongside the generation-poll flakes).
    assert!(session.wait_until(std::time::Duration::from_secs(30), |t| {
        t.content().contains("wait-until-marker")
    }));
}

#[test]
fn test_is_running_initially_false() {
    let session = PtySession::new(80, 24, 1000);
    assert!(!session.is_running());
}

#[test]
fn test_new_with_different_sizes() {
    let session1 = PtySession::new(40, 20, 500);
    assert_eq!(session1.size(), (40, 20));

    let session2 = PtySession::new(120, 40, 2000);
    assert_eq!(session2.size(), (120, 40));

    let session3 = PtySession::new(200, 60, 5000);
    assert_eq!(session3.size(), (200, 60));
}

#[test]
fn test_resize_multiple_times() {
    let mut session = PtySession::new(80, 24, 1000);

    session.resize(100, 30).ok();
    assert_eq!(session.size(), (100, 30));

    session.resize(120, 40).ok();
    assert_eq!(session.size(), (120, 40));

    session.resize(60, 20).ok();
    assert_eq!(session.size(), (60, 20));
}

#[test]
fn test_resize_to_small_size() {
    let mut session = PtySession::new(80, 24, 1000);
    session.resize(10, 5).ok();
    assert_eq!(session.size(), (10, 5));
}

#[test]
fn test_resize_to_large_size() {
    let mut session = PtySession::new(80, 24, 1000);
    session.resize(500, 200).ok();
    assert_eq!(session.size(), (500, 200));
}

#[test]
fn test_write_empty_data() {
    let mut session = PtySession::new(80, 24, 1000);
    let result = session.write(b"");
    assert!(result.is_err()); // Should fail as not spawned
}

#[test]
fn test_get_default_shell_not_empty() {
    let shell = PtySession::get_default_shell();
    assert!(!shell.is_empty());
    #[cfg(unix)]
    assert!(shell.contains("sh") || shell.contains("bash"));
}

#[test]
fn test_terminal_locked_state() {
    let session = PtySession::new(80, 24, 1000);
    {
        let terminal = session.terminal();
        let _lock1 = terminal.write();
        // While holding the write lock, a second writer is excluded
        assert!(
            terminal.try_write().is_none(),
            "write lock excludes a second writer"
        );
    }
    // After releasing, the terminal is lockable again
    let terminal = session.terminal();
    let _lock2 = terminal.write();
    drop(_lock2); // Explicitly drop to avoid unused variable warning
}

#[test]
fn test_set_env_with_empty_values() {
    let mut session = PtySession::new(80, 24, 1000);
    session.set_env("EMPTY_VAR", "");
    // An empty NAME cannot round-trip on Windows: portable-pty builds the
    // CreateProcessW environment block as raw `name=value\0` strings, and
    // `=value` makes the whole spawn fail — the OS rejects nameless env
    // entries by design, so this edge case is only poison-checkable where
    // execve tolerates it.
    #[cfg(unix)]
    session.set_env("", "value");
    // The edge-case entries must not poison the next spawn: echo a
    // marker through and assert it lands.
    #[cfg(unix)]
    let result = session.spawn("/bin/sh", &["-c", "echo ZQX-EMPTY-ENV-OK"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo ZQX-EMPTY-ENV-OK"]);
    assert!(result.is_ok());
    let content = wait_for_text(&session, "ZQX-EMPTY-ENV-OK");
    assert!(
        content.contains("ZQX-EMPTY-ENV-OK"),
        "spawn works with edge-case env entries: {content}"
    );
}

#[test]
fn test_set_env_with_unicode() {
    let mut session = PtySession::new(80, 24, 1000);
    session.set_env("UNICODE_VAR", "Hello 世界 🌍");
    #[cfg(unix)]
    let result = session.spawn("/bin/sh", &["-c", "echo ZQX-$UNICODE_VAR"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo ZQX-%UNICODE_VAR%"]);
    assert!(result.is_ok());
    let content = wait_for_text(&session, "ZQX-Hello");
    // Wide glyphs occupy two cells, so export_text() renders them with
    // padding spaces — match per-character instead of as one substring.
    assert!(
        content.contains("ZQX-Hello") && ["世", "界", "🌍"].iter().all(|c| content.contains(c)),
        "unicode env var reached the child: {content}"
    );
}

#[test]
fn test_spawn_with_env() {
    let mut session = PtySession::new(80, 24, 1000);

    // Create env vars to pass
    let mut env = HashMap::new();
    env.insert("TEST_VAR".to_string(), "test_value".to_string());

    // Spawn with env vars
    #[cfg(unix)]
    let result = session.spawn_with_env("/bin/echo", &["hello"], Some(&env), None);
    #[cfg(windows)]
    let result = session.spawn_with_env("cmd.exe", &["/C", "echo hello"], Some(&env), None);

    assert!(result.is_ok());
    assert!(
        session.wait_until(std::time::Duration::from_secs(30), |t| t
            .content()
            .contains("hello")),
        "the spawned command's output never arrived"
    );
}

#[test]
fn test_spawn_shell_with_env() {
    let mut session = PtySession::new(80, 24, 1000);

    // Create env vars to pass
    let mut env = HashMap::new();
    env.insert("MY_SHELL_VAR".to_string(), "shell_value".to_string());

    // Spawn shell with env vars
    let result = session.spawn_shell_with_env(Some(&env), None);
    assert!(result.is_ok());

    // Shell should be running
    assert!(session.is_running());

    // Clean up
    let _ = session.kill();
}

#[test]
#[ignore = "spawns a real PTY and shell/cmd process and polls its live output for up to 5s; too slow/flaky for the default suite, run explicitly with --ignored"]
fn test_spawn_with_env_cwd() {
    let mut session = PtySession::new(80, 24, 1000);

    // Spawn with cwd set to /tmp
    #[cfg(unix)]
    let result = session.spawn_with_env("/bin/pwd", &[], None, Some("/tmp"));
    #[cfg(windows)]
    let result = session.spawn_with_env("cmd.exe", &["/C", "cd"], None, Some("C:\\"));

    assert!(result.is_ok());

    // Wait for expected output with timeout
    let start_wait = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);
    let mut content = String::new();
    let mut found = false;

    while start_wait.elapsed() < timeout {
        content = session.export_text();
        #[cfg(unix)]
        if content.contains("/tmp") || content.contains("private/tmp") {
            found = true;
            break;
        }
        #[cfg(windows)]
        if content.contains("C:\\") {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    assert!(found, "Expected directory path in output, got: {}", content);
}

#[test]
fn test_spawn_shell_with_env_cwd() {
    let mut session = PtySession::new(80, 24, 1000);

    // Spawn shell with cwd
    let result = session.spawn_shell_with_env(None, Some("/tmp"));
    assert!(result.is_ok());

    // Shell should be running
    assert!(session.is_running());

    // Clean up
    let _ = session.kill();
}

#[test]
fn test_env_not_leaked_to_parent() {
    // Set a unique env var name that won't exist in parent
    let unique_var = "PTY_TEST_UNIQUE_VAR_12345";

    // Verify it doesn't exist in parent before spawn
    assert!(
        std::env::var(unique_var).is_err(),
        "Test var should not exist in parent env before spawn"
    );

    let mut session = PtySession::new(80, 24, 1000);

    // Create env vars to pass
    let mut env = HashMap::new();
    env.insert(unique_var.to_string(), "test_value".to_string());

    // Spawn with env vars
    #[cfg(unix)]
    let result = session.spawn_with_env("/bin/echo", &["test"], Some(&env), None);
    #[cfg(windows)]
    let result = session.spawn_with_env("cmd.exe", &["/C", "echo test"], Some(&env), None);

    assert!(result.is_ok());

    // Verify env var was NOT leaked to parent process
    assert!(
        std::env::var(unique_var).is_err(),
        "Test var should NOT exist in parent env after spawn"
    );
}

#[test]
fn test_spawn_with_env_and_set_env_combined() {
    let mut session = PtySession::new(80, 24, 1000);

    // Set env vars via set_env()
    session.set_env("VAR_FROM_SET_ENV", "set_env_value");

    // Create additional env vars to pass
    let mut env = HashMap::new();
    env.insert("VAR_FROM_SPAWN".to_string(), "spawn_value".to_string());

    // Spawn with both set_env vars and additional env vars
    #[cfg(unix)]
    let result = session.spawn_with_env("/bin/echo", &["test"], Some(&env), None);
    #[cfg(windows)]
    let result = session.spawn_with_env("cmd.exe", &["/C", "echo test"], Some(&env), None);

    assert!(result.is_ok());
    assert!(
        session.wait_until(std::time::Duration::from_secs(30), |t| t
            .content()
            .contains("test")),
        "the spawned command's output never arrived"
    );
}

#[test]
fn test_spawn_with_empty_env() {
    let mut session = PtySession::new(80, 24, 1000);

    // Pass empty env HashMap
    let env = HashMap::new();

    #[cfg(unix)]
    let result = session.spawn_with_env("/bin/echo", &["hello"], Some(&env), None);
    #[cfg(windows)]
    let result = session.spawn_with_env("cmd.exe", &["/C", "echo hello"], Some(&env), None);

    assert!(result.is_ok());
}
#[test]
fn test_resize_with_pixels_before_spawn() {
    let mut session = PtySession::new(80, 24, 1000);
    let result = session.resize_with_pixels(100, 30, 800, 600);
    assert!(
        result.is_ok(),
        "resize_with_pixels before spawn should not error: {:?}",
        result
    );
    assert_eq!(session.size(), (100, 30));
}

#[test]
fn test_write_str_before_spawn_returns_error() {
    let mut session = PtySession::new(80, 24, 1000);
    let result = session.write_str("hello");
    assert!(
        result.is_err(),
        "write_str before spawn should return error"
    );
}

#[test]
fn test_bell_count_initial() {
    let session = PtySession::new(80, 24, 1000);
    assert_eq!(session.bell_count(), 0);
}

#[test]
fn test_scrollback_initial_empty() {
    let session = PtySession::new(80, 24, 1000);
    let sb = session.scrollback();
    assert!(
        sb.is_empty(),
        "scrollback should be empty before any output"
    );
}

#[test]
fn test_scrollback_len_initial() {
    let session = PtySession::new(80, 24, 1000);
    assert_eq!(session.scrollback_len(), 0);
}

#[test]
fn test_has_updates_since_same_generation() {
    let session = PtySession::new(80, 24, 1000);
    let gen = session.update_generation();
    assert!(!session.has_updates_since(gen));
}

#[test]
fn test_has_updates_since_older_generation() {
    let session = PtySession::new(80, 24, 1000);
    let gen = session.update_generation();
    // Only test if gen > 0 to avoid u64 underflow
    if gen > 0 {
        assert!(
            session.has_updates_since(gen - 1),
            "should have updates since an older generation"
        );
    }
    // If gen == 0, the session just started with no updates; skip the check.
}

/// Regression test for issue #60: generation counter must increment
/// on every successful PTY read, even if terminal processing encounters
/// unexpected sequences (e.g., Windows ConPTY after Ctrl+C).
///
/// A child that exits before the reader's first `read()` must not lose
/// its output. On macOS the pty discards unread output once the last
/// slave fd closes; the delay forces the loaded-scheduler case every run.
#[cfg(unix)]
#[test]
fn output_of_a_child_that_exits_before_the_first_read_is_kept() {
    let mut session = PtySession::new(80, 24, 1000);
    session.first_read_delay = Some(std::time::Duration::from_millis(1500));
    let gen_before = session.update_generation();
    session
        .spawn("/bin/echo", &["EARLY-EXIT-MARKER"])
        .expect("spawn echo");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while session.is_running() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        !session.is_running(),
        "reader should reach EOF after echo exits"
    );
    assert!(
        session
            .terminal()
            .read()
            .content()
            .contains("EARLY-EXIT-MARKER"),
        "output written before the first read was lost: {:?}",
        session.terminal().read().content().trim()
    );
    assert!(session.update_generation() > gen_before);
}

/// This test spawns a real PTY, writes data, and verifies that
/// `has_updates_since()` correctly detects the change.
#[test]
fn test_generation_counter_increments_on_pty_output() {
    let mut session = PtySession::new(80, 24, 1000);

    // Spawn a simple command that exits immediately
    #[cfg(unix)]
    let result = session.spawn("/bin/echo", &["hello"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo hello"]);

    assert!(result.is_ok());

    let gen_before = session.update_generation();

    // The reader thread bumps the generation as PTY bytes arrive; under
    // load that can trail the spawn by more than any fixed window (the
    // flake this poll replaced: "was 0, now 0" after 200 ms — and the
    // 5 s deadline here still expired under back-to-back gate load,
    // 2026-09-23). Wait on the counter itself with a deadline instead.
    // The deadline is a starvation bound, not a timing assertion —
    // green runs finish in milliseconds — so allow the same 30 s the
    // wait_for_update EOF test budgets above.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let gen_after = loop {
        let gen = session.update_generation();
        if gen > gen_before || std::time::Instant::now() > deadline {
            break gen;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(
        gen_after > gen_before,
        "generation counter should have incremented after PTY output: was {}, now {}",
        gen_before,
        gen_after
    );
    assert!(
        session.has_updates_since(gen_before),
        "has_updates_since() should return true after PTY output"
    );
}

/// Regression test for the "some regions don't update" freeze in TUI apps
/// (joe, vim, etc.) that repaint via partial line edits (DL/IL/EL).
///
/// The issue #60 pre-processing bump advances the generation BEFORE the grid
/// is written. A renderer that acquires the terminal lock in the window
/// between that bump and the write reads the not-yet-updated grid but stamps
/// its cell cache with the already-advanced generation; if this is the last
/// read of an output burst, the counter never advances again and that stale
/// content is served until the next PTY read. The second bump after the grid
/// write must move the counter PAST any value observed in that window so the
/// next frame regenerates instead of freezing.
///
/// Since the reader-loop reorder (output callback fires after the read's
/// bytes are applied), the callback observes the generation AFTER both
/// bumps of its read: it runs on the reader thread itself, so nothing of
/// that read can land after it. Asserting the callback observed >= 2
/// proves the second, content-applied bump happened before the callback
/// fired — the terminal state a callback reads is never missing the bytes
/// it is being handed. (Before the reorder the callback ran between the
/// two bumps and observed the pre-processing bump alone, value 1.)
#[test]
fn test_generation_advances_after_content_applied() {
    let mut session = PtySession::new(80, 24, 1000);

    // Mirror the session's internal generation counter into the callback.
    // The `tests` module can reach the private field directly.
    let gen_counter = Arc::clone(&session.update_generation);
    let observed_in_callback = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let observed_cb = Arc::clone(&observed_in_callback);
    session.set_output_callback(Arc::new(move |_bytes: &[u8]| {
        // Runs after both the pre-processing and content-applied bumps.
        observed_cb.store(gen_counter.load(Ordering::SeqCst), Ordering::SeqCst);
    }));

    #[cfg(unix)]
    let result = session.spawn("/bin/echo", &["hello"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo hello"]);
    assert!(result.is_ok());

    // Wait for the command to run, produce output, and reach the
    // callback. A fixed sleep flakes under load — the reader thread can
    // lag the wait (observed locally 2026-09-22 under back-to-back gate
    // runs) — so poll with a deadline before asserting. 30 s, not 5 s:
    // this deadline is a starvation bound, and the 5 s bound still
    // expired under back-to-back gate load (2026-09-23) while green
    // runs finish in milliseconds.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let observed = loop {
        let observed = observed_in_callback.load(Ordering::SeqCst);
        let final_gen = session.update_generation();
        if (observed > 0 && final_gen >= observed) || std::time::Instant::now() >= deadline {
            break observed;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };

    assert!(
        observed > 0,
        "output callback should have run and observed a post-application generation"
    );
    assert!(
        observed >= 2,
        "the content-applied bump (second bump of the read) must precede the \
         output callback; callback observed generation {observed}"
    );
}

/// Reader-loop ordering contract: the output callback must fire only
/// after the bytes it receives have been applied to the terminal, so a
/// callback that reads terminal state (e.g. the mux daemon composing a
/// reattach seed for a client it is forwarding those same bytes to) can
/// never observe state that lacks them. The callback runs on the reader
/// thread itself, so before the reorder this failed deterministically —
/// the thread executing the callback was the one that had not yet taken
/// the write guard to process the bytes.
#[test]
fn output_callback_sees_applied_terminal_state() {
    let mut session = PtySession::new(80, 24, 1000);

    let terminal = Arc::clone(session.terminal_ref());
    // 0 = marker not seen yet, 1 = seen and state contained it, 2 = seen
    // and state lacked it. Stored once, after the check completes, so the
    // test thread never samples a "seen but not yet checked" window.
    let outcome = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let outcome_probe = Arc::clone(&outcome);
    session.set_output_callback(Arc::new(move |bytes: &[u8]| {
        if bytes.windows(7).any(|w| w == b"MARKERZ") {
            let text = terminal.read().grid.export_text_buffer();
            outcome_probe.store(
                if text.contains("MARKERZ") { 1 } else { 2 },
                Ordering::SeqCst,
            );
        }
    }));

    #[cfg(unix)]
    let result = session.spawn("/bin/sh", &["-c", "printf 'MARKERZ'"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo MARKERZ"]);
    assert!(result.is_ok());
    let _ = session.wait();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while outcome.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_ne!(
        outcome.load(Ordering::SeqCst),
        0,
        "output callback should have received the marker bytes"
    );
    assert_eq!(
        outcome.load(Ordering::SeqCst),
        1,
        "terminal state must contain the callback's bytes by the time the callback fires"
    );
}

/// Test that generation counter increments are not blocked by terminal
/// processing issues. Simulates the Ctrl+C scenario from issue #60 by
/// spawning a shell, sending Ctrl+C, then sending normal data and
/// verifying the generation counter still advances.
#[test]
fn test_generation_counter_after_ctrl_c() {
    let mut session = PtySession::new(80, 24, 1000);

    // Spawn a shell
    #[cfg(unix)]
    let result = session.spawn_shell();
    #[cfg(windows)]
    let result = session.spawn_shell();

    assert!(result.is_ok());

    // Wait for the prompt itself, not just the first update: a Ctrl+C sent
    // before the shell installs its interrupt handling kills it. ConPTY's
    // first update is its own setup output on a still-empty screen, and a
    // Ctrl+C then ends cmd.exe with STATUS_CONTROL_C_EXIT.
    assert!(
        session.wait_until(std::time::Duration::from_secs(30), |t| {
            let content = t.content();
            let content = content.trim_end();
            #[cfg(windows)]
            return content.ends_with('>');
            #[cfg(not(windows))]
            return !content.is_empty();
        }),
        "shell printed a prompt"
    );

    // Send Ctrl+C, and let its ^C echo and prompt redraw land before the
    // command, so the shell has handled the interrupt first.
    let gen_before_ctrl_c = session.update_generation();
    session.write(b"\x03").unwrap();
    session
        .update_waiter()
        .wait_for_update(gen_before_ctrl_c, std::time::Duration::from_secs(30))
        .expect("the shell redrew after Ctrl+C");

    // Now send a normal command
    let gen_before_echo = session.update_generation();
    session.write(b"echo TEST_GENERATION\r\n").unwrap();

    assert!(
        session.wait_until(std::time::Duration::from_secs(30), |t| t
            .content()
            .contains("TEST_GENERATION")),
        "the echo never reached the screen after Ctrl+C"
    );

    // Generation MUST increment for the echo output
    let gen_after_echo = session.update_generation();
    assert!(
        gen_after_echo > gen_before_echo,
        "generation counter should increment for output after Ctrl+C: was {}, now {}",
        gen_before_echo,
        gen_after_echo
    );
    assert!(
        session.has_updates_since(gen_before_echo),
        "has_updates_since() must detect changes after Ctrl+C"
    );
}

/// ARC-001 regression: observer dispatch must run *after* the reader
/// thread's `RwLock<Terminal>` write guard is dropped, not while it is
/// still held.
///
/// Before the fix, `process()` invoked observer callbacks inline from
/// inside the write-guard scope in the reader thread's read loop. A
/// callback that then tried to lock the same `Arc<RwLock<Terminal>>`
/// (directly, or indirectly via a Python callback calling back into a
/// terminal method) would find the lock already held by this very
/// thread — `parking_lot::RwLock` is not reentrant, so `try_write()`
/// would fail every time. After the fix (`process_deferred()` +
/// delivering the returned `ObserverDispatchBatch` only after the write
/// guard is dropped), the same `try_write()` call must succeed, proving
/// no exclusive lock is held while observers run.
///
/// Unix-only: relies on `/bin/sh -c "printf '\007'"` to reliably emit a
/// single BEL byte through the PTY, which `Terminal` turns into a
/// `TerminalEvent::BellRang` observer event (see
/// `terminal::tests::observer_tests::test_observer_receives_bell_event`
/// for the non-PTY equivalent).
#[test]
#[cfg(unix)]
fn test_observer_dispatch_does_not_hold_write_lock() {
    use crate::observer::TerminalObserver;
    use crate::terminal::TerminalEvent;

    /// Observer that, on every event, probes whether the write lock on
    /// its own terminal is free.
    struct LockProbeObserver {
        terminal: Arc<RwLock<Terminal>>,
        saw_any_invocation: AtomicBool,
        lock_was_free: AtomicBool,
    }

    impl TerminalObserver for LockProbeObserver {
        fn on_event(&self, _event: &TerminalEvent) {
            if self.terminal.try_write().is_some() {
                self.lock_was_free.store(true, Ordering::SeqCst);
            }
            // Published last: the test polls this flag, and once it is
            // set this invocation's lock verdict is already visible.
            self.saw_any_invocation.store(true, Ordering::SeqCst);
        }
    }

    let mut session = PtySession::new(80, 24, 1000);
    let terminal_arc = Arc::clone(session.terminal_ref());

    let probe = Arc::new(LockProbeObserver {
        terminal: Arc::clone(&terminal_arc),
        saw_any_invocation: AtomicBool::new(false),
        lock_was_free: AtomicBool::new(false),
    });
    {
        let mut term = terminal_arc.write();
        term.add_observer(probe.clone());
    }

    let result = session.spawn("/bin/sh", &["-c", "printf '\\007'"]);
    assert!(result.is_ok(), "spawn should succeed: {:?}", result);

    // Wait for the reader thread to read the BEL byte and dispatch it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !probe.saw_any_invocation.load(Ordering::SeqCst) {
        assert!(
            std::time::Instant::now() < deadline,
            "observer should have been invoked for the BEL event"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        probe.lock_was_free.load(Ordering::SeqCst),
        "try_write() must succeed from inside the observer callback -- the \
         reader thread must drop its write guard before delivering observer \
         events (ARC-001)"
    );
}

#[test]
fn test_get_writer_before_spawn_is_none() {
    let session = PtySession::new(80, 24, 1000);
    assert!(
        session.get_writer().is_none(),
        "writer should be None before spawn"
    );
}

#[test]
fn test_try_wait_before_spawn_returns_error() {
    let mut session = PtySession::new(80, 24, 1000);
    let result = session.try_wait();
    assert!(result.is_err(), "try_wait before spawn should return error");
}

#[test]
fn test_kill_before_spawn_returns_error() {
    let mut session = PtySession::new(80, 24, 1000);
    let result = session.kill();
    assert!(result.is_err(), "kill before spawn should return error");
}

#[test]
fn test_set_and_clear_output_callback() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let mut session = PtySession::new(80, 24, 1000);
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = called.clone();
    session.set_output_callback(Arc::new(move |_data: &[u8]| {
        called_clone.store(true, Ordering::Relaxed);
    }));
    session.clear_output_callback();
    // Drive real output through the reader thread, then assert the
    // cleared callback never fired even though content arrived.
    #[cfg(unix)]
    let result = session.spawn("/bin/sh", &["-c", "echo ZQX-CALLBACK"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo ZQX-CALLBACK"]);
    assert!(result.is_ok());
    let content = wait_for_text(&session, "ZQX-CALLBACK");
    assert!(
        content.contains("ZQX-CALLBACK"),
        "output arrived: {content}"
    );
    assert!(
        !called.load(Ordering::Relaxed),
        "cleared callback must not fire"
    );
}

// ===================================================================
// Coverage-focused deterministic tests (no live PTY / no timing)
// ===================================================================

#[test]
fn test_child_pid_none_before_spawn() {
    // child_pid() must return None when no process has been spawned.
    let session = PtySession::new(80, 24, 1000);
    assert_eq!(
        session.child_pid(),
        None,
        "child_pid should be None before spawn"
    );
}

#[test]
fn test_terminal_ref_returns_same_underlying_arc() {
    // terminal_ref() must return a reference to the SAME Arc that
    // terminal() clones — both should resolve to the same allocation.
    let session = PtySession::new(80, 24, 1000);
    let cloned = session.terminal();
    let borrowed = session.terminal_ref();
    // Arc::ptr_eq confirms they point at the same allocation.
    assert!(
        Arc::ptr_eq(&cloned, borrowed),
        "terminal() and terminal_ref() must reference the same Arc"
    );
    // Sanity: the size reported through both is identical.
    assert_eq!(cloned.read().size(), borrowed.read().size());
}

#[test]
fn test_cursor_position_initial_origin() {
    // A freshly constructed terminal has its cursor at (0, 0).
    let session = PtySession::new(80, 24, 1000);
    let (col, row) = session.cursor_position();
    assert_eq!(col, 0);
    assert_eq!(row, 0);
}

#[test]
fn test_get_line_in_bounds_initially_blank() {
    // A fresh terminal row exists but consists of blank (' ') cells.
    let session = PtySession::new(80, 24, 1000);
    let line = session.get_line(0);
    assert!(line.is_some(), "row 0 should exist");
    let line = line.unwrap();
    assert_eq!(
        line.len(),
        80,
        "row 0 should have one cell per column initially"
    );
    assert!(
        line.chars().all(|c| c == ' '),
        "fresh row should be entirely blank spaces"
    );
}

#[test]
fn test_get_line_out_of_bounds_returns_none() {
    // Out-of-range rows must return None (no panic).
    let session = PtySession::new(80, 24, 1000);
    assert!(
        session.get_line(24).is_none(),
        "row == height is out of range"
    );
    assert!(
        session.get_line(usize::MAX).is_none(),
        "huge row index is out of range"
    );
}

#[test]
fn test_content_initially_blank_or_empty() {
    // content() on a fresh terminal must not panic. It returns the
    // visible screen content; on a brand-new terminal it is blank.
    let session = PtySession::new(80, 24, 1000);
    let content = session.content();
    // Every visible character should be whitespace (blank cells).
    assert!(
        content.chars().all(|c| c.is_whitespace() || c == '\n'),
        "fresh content() must contain only whitespace, got: {:?}",
        content
    );
}

#[test]
fn test_export_text_and_styled_on_fresh_terminal() {
    // export_text / export_styled must succeed on a fresh terminal
    // without panicking, even though no output has ever been processed.
    let session = PtySession::new(80, 24, 1000);
    let text = session.export_text();
    let styled = session.export_styled();
    // Both must be valid UTF-8 strings (returned as String already).
    // text should be all-whitespace-or-newline; styled may contain
    // ANSI escapes so we only assert it does not panic and is a String.
    assert!(
        text.chars().all(|c| c.is_whitespace() || c == '\n'),
        "export_text on fresh terminal should be blank"
    );
    // Smoke check: styled is a string we can call .len() on without panic.
    let _ = styled.len();
}

#[test]
fn test_list_coprocesses_initially_empty() {
    // With no coprocess started, list must return an empty Vec.
    let session = PtySession::new(80, 24, 1000);
    assert!(
        session.list_coprocesses().is_empty(),
        "list_coprocesses() must be empty before any start_coprocess"
    );
}

#[test]
fn test_coprocess_status_unknown_id_is_none() {
    // status of an unregistered coprocess id must return None
    // (distinguishes "no such id" from "stopped").
    let session = PtySession::new(80, 24, 1000);
    assert_eq!(
        session.coprocess_status(999),
        None,
        "status for unknown id should be None"
    );
    assert_eq!(
        session.coprocess_status(0),
        None,
        "status for id 0 should be None before any coprocess started"
    );
}

#[test]
fn test_read_from_unknown_coprocess_is_err() {
    // read_from_coprocess must surface a deterministic error for an
    // unknown id (no panic, no blocking).
    let session = PtySession::new(80, 24, 1000);
    let result = session.read_from_coprocess(42);
    assert!(
        result.is_err(),
        "read_from_coprocess on unknown id must return Err"
    );
    let msg = result.unwrap_err();
    assert!(
        msg.contains("42") || msg.to_lowercase().contains("not found"),
        "error message should reference the missing id or 'not found': {}",
        msg
    );
}

#[test]
fn test_read_coprocess_errors_unknown_id_is_err() {
    let session = PtySession::new(80, 24, 1000);
    let result = session.read_coprocess_errors(7);
    assert!(
        result.is_err(),
        "read_coprocess_errors on unknown id must return Err"
    );
}

#[test]
fn test_write_to_unknown_coprocess_is_err() {
    // Writing to an unknown coprocess id must be a deterministic error
    // without touching any I/O.
    let session = PtySession::new(80, 24, 1000);
    let result = session.write_to_coprocess(123, b"data");
    assert!(
        result.is_err(),
        "write_to_coprocess on unknown id must return Err"
    );
}

#[test]
fn test_stop_unknown_coprocess_is_err() {
    // stop_coprocess on an unknown id must return Err (no panic).
    let session = PtySession::new(80, 24, 1000);
    let result = session.stop_coprocess(256);
    assert!(
        result.is_err(),
        "stop_coprocess on unknown id must return Err"
    );
}

#[test]
fn test_clear_output_callback_without_set_is_noop() {
    // clear_output_callback before any set must be safe (no panic) and
    // must not break the next spawn: drive output through afterwards.
    let mut session = PtySession::new(80, 24, 1000);
    session.clear_output_callback();
    session.clear_output_callback(); // idempotent
    #[cfg(unix)]
    let result = session.spawn("/bin/sh", &["-c", "echo ZQX-NOOP-CLEAR"]);
    #[cfg(windows)]
    let result = session.spawn("cmd.exe", &["/C", "echo ZQX-NOOP-CLEAR"]);
    assert!(result.is_ok());
    let content = wait_for_text(&session, "ZQX-NOOP-CLEAR");
    assert!(
        content.contains("ZQX-NOOP-CLEAR"),
        "output flows: {content}"
    );
}

#[test]
fn test_set_env_does_not_affect_terminal_state() {
    // set_env stores vars for later spawn; it must NOT mutate the
    // terminal grid/size/cursor. Regression guard for accidental
    // side effects.
    let mut session = PtySession::new(80, 24, 1000);
    let size_before = session.size();
    let cursor_before = session.cursor_position();
    let gen_before = session.update_generation();

    session.set_env("FOO", "bar");
    session.set_env("BAZ", "qux");

    assert_eq!(session.size(), size_before, "size must not change");
    assert_eq!(
        session.cursor_position(),
        cursor_before,
        "cursor must not move"
    );
    assert_eq!(
        session.update_generation(),
        gen_before,
        "generation must not advance from set_env"
    );
}

#[test]
fn test_set_cwd_does_not_affect_terminal_state() {
    // set_cwd stores a path string; it must not alter terminal state.
    let mut session = PtySession::new(80, 24, 1000);
    let size_before = session.size();
    let gen_before = session.update_generation();

    session.set_cwd(std::path::Path::new("/tmp"));

    assert_eq!(session.size(), size_before);
    assert_eq!(session.update_generation(), gen_before);
}

#[test]
fn test_resize_does_not_advance_generation() {
    // resize() mutates terminal size but, unlike PTY reads, does NOT
    // bump update_generation (the counter only advances on PTY reads).
    // This pins that contract so callers relying on it for redraw
    // detection keep working.
    let mut session = PtySession::new(80, 24, 1000);
    let gen_before = session.update_generation();
    let res = session.resize(90, 30);
    assert!(res.is_ok(), "resize before spawn should be ok");
    assert_eq!(session.size(), (90, 30), "size must reflect new dimensions");
    assert_eq!(
        session.update_generation(),
        gen_before,
        "resize must NOT advance the update_generation counter"
    );
}

#[test]
fn test_resize_with_pixels_zero_dimensions_no_div_by_zero() {
    // resize_with_pixels guards against cols==0 / rows==0 in its
    // per-cell division. Verify that path doesn't panic.
    let mut session = PtySession::new(80, 24, 1000);
    // cols = 0 -> cell math skipped; must not divide-by-zero.
    let res = session.resize_with_pixels(0, 0, 100, 100);
    assert!(res.is_ok(), "resize_with_pixels(0,0,...) should not panic");
    // Valid resize with pixels should also work and be observable via size().
    let res = session.resize_with_pixels(40, 12, 400, 240);
    assert!(res.is_ok());
    assert_eq!(session.size(), (40, 12));
}

#[test]
fn test_resize_with_pixels_advances_size_not_generation() {
    // Same generation contract as plain resize (no PTY read happened).
    let mut session = PtySession::new(80, 24, 1000);
    let gen_before = session.update_generation();
    let res = session.resize_with_pixels(100, 30, 700, 600);
    assert!(res.is_ok());
    assert_eq!(session.size(), (100, 30));
    assert_eq!(
        session.update_generation(),
        gen_before,
        "resize_with_pixels must NOT advance update_generation"
    );
}

#[test]
fn test_get_default_shell_is_absolute_path_on_unix() {
    // On Unix the default-shell resolver returns either $SHELL (when it
    // is an existing file) or /bin/sh. Either way, the result must be
    // non-empty and, on unix, start with '/' (absolute).
    let shell = PtySession::get_default_shell();
    assert!(!shell.is_empty());
    #[cfg(unix)]
    assert!(
        shell.starts_with('/'),
        "Unix default shell should be an absolute path, got: {}",
        shell
    );
}

#[test]
fn test_wait_before_spawn_returns_not_started_error() {
    // wait() on a session that never spawned must surface NotStartedError,
    // not block. (It cannot block because self.child is None.)
    let mut session = PtySession::new(80, 24, 1000);
    let result = session.wait();
    assert!(
        result.is_err(),
        "wait() before spawn should return an error"
    );
    match result {
        Err(PtyError::NotStartedError) => {}
        other => panic!("expected NotStartedError, got {:?}", other),
    }
}

#[test]
fn test_try_wait_before_spawn_returns_not_started_error() {
    // Mirror of the existing try_wait test, but pin the specific variant
    // so future refactors don't silently swap error types.
    let mut session = PtySession::new(80, 24, 1000);
    assert!(matches!(session.try_wait(), Err(PtyError::NotStartedError)));
}

#[test]
fn test_kill_before_spawn_returns_not_started_error() {
    let mut session = PtySession::new(80, 24, 1000);
    assert!(matches!(session.kill(), Err(PtyError::NotStartedError)));
}

#[test]
fn test_write_before_spawn_returns_not_started_error() {
    // write() must fail deterministically with NotStartedError before spawn.
    let mut session = PtySession::new(80, 24, 1000);
    assert!(matches!(
        session.write(b"data"),
        Err(PtyError::NotStartedError)
    ));
}

#[test]
fn test_write_str_before_spawn_returns_not_started_error() {
    // write_str delegates to write(), so the same error path applies.
    let mut session = PtySession::new(80, 24, 1000);
    assert!(matches!(
        session.write_str("data"),
        Err(PtyError::NotStartedError)
    ));
}

#[test]
fn test_get_writer_is_none_before_and_after_no_spawn() {
    // The writer field stays None until a successful spawn.
    let session = PtySession::new(80, 24, 1000);
    assert!(session.get_writer().is_none());
}

#[test]
fn test_scrollback_accessors_consistent_on_fresh_terminal() {
    // scrollback() and scrollback_len() must agree on a fresh terminal:
    // both should report "empty".
    let session = PtySession::new(80, 24, 1000);
    let sb_vec = session.scrollback();
    let sb_len = session.scrollback_len();
    assert_eq!(sb_len, 0);
    assert_eq!(sb_vec.len(), 0);
    assert!(
        sb_len == sb_vec.len(),
        "scrollback_len() ({}) and scrollback().len() ({}) must agree",
        sb_len,
        sb_vec.len()
    );
}

#[test]
fn test_has_updates_since_future_generation_is_false() {
    // has_updates_since(gen) where gen > current must be false
    // (no updates between now and a future generation).
    let session = PtySession::new(80, 24, 1000);
    let current = session.update_generation();
    assert!(
        !session.has_updates_since(current + 1),
        "has_updates_since(future_gen) must be false"
    );
    assert!(
        !session.has_updates_since(current + 1000),
        "has_updates_since(far_future_gen) must be false"
    );
}

/// The geometry mirror (ENH-023): `size()` is served from atomics the
/// resize path refreshes, so a resize is visible with no `process()`
/// call, and the cursor tracks fed output without a reader thread.
#[test]
fn size_and_cursor_serve_from_the_geometry_mirror() {
    let mut session = PtySession::new(80, 24, 1000);
    assert_eq!(session.size(), (80, 24));
    assert_eq!(session.cursor_position(), (0, 0));

    session.resize(100, 30).expect("resize");
    assert_eq!(
        session.size(),
        (100, 30),
        "resize is visible from size() without any process() call"
    );

    // Fed output moves the mirrored cursor: two lines, cursor parked
    // after the text on the second row.
    session.with_terminal_mut(|term| {
        term.process(b"AB\r\nCD");
    });
    assert_eq!(session.cursor_position(), (2, 1));
    assert_eq!(
        session.snapshot_geometry(),
        ((100, 30), (2, 1)),
        "the locked snapshot agrees with the mirror"
    );
}

/// Dropping a session that was constructed but never spawned must
/// run Drop cleanly (kills coprocesses, signals reader) without
/// panicking — there is no child, no reader thread, no writer.
#[test]
fn test_drop_on_unspawned_session_does_not_panic() {
    // Dropping a session that was constructed but never spawned must
    // run Drop cleanly (kills coprocesses, signals reader) without
    // panicking — there is no child, no reader thread, no writer.
    let _session = PtySession::new(80, 24, 1000);
    // Bound the scope so Drop runs before the assertion completes.
    let dropped_ok = std::panic::catch_unwind(|| {
        let _s = PtySession::new(10, 5, 100);
        // explicit drop
        drop(_s);
    });
    assert!(
        dropped_ok.is_ok(),
        "Dropping an unspawned PtySession must not panic"
    );
}

/// A PTY spawned from an environment carrying PAR_MUX_* vars gets none of
/// them (they carry the OUTER pane's identity), unless set_env adds them
/// back — which is exactly how mux panes seed their own values. The
/// synthetic var proves the prefix rule covers identity vars beyond the
/// ones a daemon exports today.
/// A parent env for the env-drop tests: the real env's spawn essentials
/// (cmd.exe will not start without `SystemRoot`/`COMSPEC`, and a present
/// `HOME` keeps portable-pty off its non-thread-safe `getpwuid` fallback)
/// plus `outer`, the vars under test. Injected through
/// `parent_env_override`, so no test mutates the process env (QA-196).
fn parent_env_with(outer: &[(&str, &str)]) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    const ESSENTIALS: &[&str] = &[
        "PATH",
        "HOME",
        "SystemRoot",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
        "USERPROFILE",
    ];
    let mut env: Vec<(std::ffi::OsString, std::ffi::OsString)> = ESSENTIALS
        .iter()
        .filter_map(|&key| std::env::var_os(key).map(|value| (key.into(), value)))
        .collect();
    env.extend(outer.iter().map(|&(key, value)| (key.into(), value.into())));
    env
}

#[test]
fn par_mux_env_does_not_leak_into_spawned_ptys() {
    let mut session = PtySession::new(80, 24, 1000);
    session.parent_env_override = Some(parent_env_with(&[(
        "PAR_MUX_LEAK_PROBE",
        "stale-outer-value",
    )]));
    // set_env runs after the drop, so this one survives as the pane's own.
    session.set_env("PAR_MUX_PANE_ID", "42");

    #[cfg(unix)]
    let (shell, flag, probe) = (
        "/bin/sh",
        "-c",
        "echo LEAK=[$PAR_MUX_LEAK_PROBE] OWN=[$PAR_MUX_PANE_ID]",
    );
    #[cfg(windows)]
    let (shell, flag, probe) = (
        "cmd.exe",
        "/C",
        "echo LEAK=[%PAR_MUX_LEAK_PROBE%] OWN=[%PAR_MUX_PANE_ID%]",
    );
    session.spawn(shell, &[flag, probe]).expect("probe spawns");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let screen = session.with_terminal(|term| term.content());
        if screen.contains("LEAK=[") {
            // sh expands an unset var to empty; cmd.exe echoes %VAR%
            // literally — both shapes prove the drop. OWN=[42] expands
            // on either shell and proves set_env survives it.
            #[cfg(unix)]
            let dropped = screen.contains("LEAK=[]");
            #[cfg(windows)]
            let dropped = screen.contains("LEAK=[%PAR_MUX_LEAK_PROBE%]");
            assert!(dropped, "the outer pane's PAR_MUX_* leaked in: {screen}");
            assert!(
                screen.contains("OWN=[42]"),
                "set_env values survive the drop: {screen}"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "probe never ran; screen so far: {screen}"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// A PTY spawned from an environment carrying the OUTER agent session's
/// identity vars gets none of them (herdr parity, pane.rs
/// apply_pane_launch_env): a pane is not a child agent of whatever
/// started the daemon, and nested-session detection keyed on these vars
/// (omp's OMPCODE check) would otherwise hide the pane's own agents
/// from rosters. set_env opts back in for an intentional child session.
#[test]
fn outer_agent_identity_env_does_not_leak_into_spawned_ptys() {
    let mut session = PtySession::new(80, 24, 1000);
    session.parent_env_override = Some(parent_env_with(&[
        ("CLAUDECODE", "1"),
        ("CLAUDE_CODE_SESSION_ID", "outer-session"),
        ("CLAUDE_CODE_CHILD_SESSION", "1"),
        ("CLAUDE_CODE_MESSAGING_TOKEN", "outer-token"),
        ("OMPCODE", "1"),
        ("CODEX_THREAD_ID", "outer-thread"),
    ]));
    // set_env runs after the drop, so an intentional child session can
    // opt back in.
    session.set_env("OMPCODE", "1");

    #[cfg(unix)]
    let (shell, flag, probe) = (
        "/bin/sh",
        "-c",
        "echo CC=[$CLAUDECODE] CS=[$CLAUDE_CODE_SESSION_ID] CH=[$CLAUDE_CODE_CHILD_SESSION] CT=[$CLAUDE_CODE_MESSAGING_TOKEN] OC=[$OMPCODE] CX=[$CODEX_THREAD_ID]",
    );
    #[cfg(windows)]
    let (shell, flag, probe) = (
        "cmd.exe",
        "/C",
        "echo CC=[%CLAUDECODE%] CS=[%CLAUDE_CODE_SESSION_ID%] CH=[%CLAUDE_CODE_CHILD_SESSION%] CT=[%CLAUDE_CODE_MESSAGING_TOKEN%] OC=[%OMPCODE%] CX=[%CODEX_THREAD_ID%]",
    );
    session.spawn(shell, &[flag, probe]).expect("probe spawns");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let screen = session.with_terminal(|term| term.content());
        if screen.contains("CC=[") {
            // sh expands an unset var to empty ([]); cmd.exe echoes the
            // literal %VAR% — both prove the drop. The opted-in OMPCODE
            // expands to its value on either shell.
            #[cfg(unix)]
            let (cc, cs, ch, ct, cx) = ("CC=[]", "CS=[]", "CH=[]", "CT=[]", "CX=[]");
            #[cfg(windows)]
            let (cc, cs, ch, ct, cx) = (
                "CC=[%CLAUDECODE%]",
                "CS=[%CLAUDE_CODE_SESSION_ID%]",
                "CH=[%CLAUDE_CODE_CHILD_SESSION%]",
                "CT=[%CLAUDE_CODE_MESSAGING_TOKEN%]",
                "CX=[%CODEX_THREAD_ID%]",
            );
            assert!(screen.contains(cc), "CLAUDECODE leaked: {screen}");
            assert!(
                screen.contains(cs),
                "CLAUDE_CODE_SESSION_ID leaked: {screen}"
            );
            assert!(
                screen.contains(ch),
                "CLAUDE_CODE_CHILD_SESSION leaked: {screen}"
            );
            assert!(
                screen.contains(ct),
                "CLAUDE_CODE_MESSAGING_TOKEN leaked: {screen}"
            );
            assert!(
                screen.contains("OC=[1]"),
                "set_env opt-back-in must survive the drop: {screen}"
            );
            assert!(screen.contains(cx), "CODEX_THREAD_ID leaked: {screen}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "probe never ran; screen so far: {screen}"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
