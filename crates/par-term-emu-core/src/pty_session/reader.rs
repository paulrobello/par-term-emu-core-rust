//! The PTY reader thread: reads the master, applies output to the
//! terminal under the write lock, answers device queries, and fans the raw
//! bytes out to coprocesses, the output callback, and observers.

#[cfg(unix)]
use super::send_sigwinch;
use super::PtySession;
use crate::debug;
use parking_lot::{Condvar, Mutex};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

/// Whether a PTY read error means the pty is dead and the reader must
/// stop. On Unix, reading the master after the child's slave side is gone
/// yields EIO (always on Linux; macOS/BSD may return a zero-byte read or
/// EIO depending on pending-output state), so EIO is the same
/// terminal-dead condition as EOF, not a retryable error — retrying it
/// busy-spins the reader on a dead pty.
fn read_error_is_terminal(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::BrokenPipe {
        return true;
    }
    #[cfg(unix)]
    if err.raw_os_error() == Some(libc::EIO) {
        return true;
    }
    false
}

/// Shared exit path for a dead PTY (a zero-byte read or a terminal read
/// error): publish the exit, wake anyone blocked on the update signal,
/// then advance the generation. The notify must come first — `running` is
/// part of `wait_for_update`'s exit condition, so a blocked waiter must
/// not sit out its timeout after the child is gone. The bump after the
/// notify is what a generation-poll observer (a frontend sleep-polling
/// `update_generation()`) sees on its next poll: without it, a child that
/// dies without producing output never advances the counter and such a
/// poller never notices the death.
fn reader_dead(
    running: &AtomicBool,
    update_signal: &(Mutex<()>, Condvar),
    update_generation: &AtomicU64,
) {
    running.store(false, Ordering::SeqCst);
    update_signal.1.notify_all();
    let old_gen = update_generation.fetch_add(1, Ordering::SeqCst);
    debug::log_generation_change(old_gen, old_gen + 1, "reader EOF");
}

impl PtySession {
    /// Start the reader thread that processes PTY output
    pub(super) fn start_reader_thread(
        &mut self,
        mut reader: Box<dyn Read + Send>,
        writer: Arc<Mutex<Box<dyn Write + Send>>>,
        child_pid: Option<u32>,
    ) {
        let terminal = Arc::clone(&self.terminal);
        let running = Arc::clone(&self.running);
        let update_generation = Arc::clone(&self.update_generation);
        let update_signal = Arc::clone(&self.update_signal);
        let reply_xtwinops = Arc::clone(&self.reply_xtwinops);
        let geometry = Arc::clone(&self.geometry);
        let output_callback = Arc::clone(&self.output_callback);
        let coprocess_manager = Arc::clone(&self.coprocess_manager);
        let reaped = Arc::clone(&self.reaped);
        // Only the unix SIGWINCH pulse on alt-screen entry signals the child.
        #[cfg(not(unix))]
        let _ = (child_pid, reaped);
        #[cfg(all(test, unix))]
        let signals_sent = Arc::clone(&self.signals_sent);
        #[cfg(test)]
        let first_read_delay = self.first_read_delay;

        let handle = thread::spawn(move || {
            let mut buffer = [0u8; 16384];
            #[cfg(test)]
            if let Some(delay) = first_read_delay {
                thread::sleep(delay);
            }

            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        // EOF - process has exited
                        reader_dead(&running, &update_signal, &update_generation);
                        break;
                    }
                    Ok(n) => {
                        debug::log_pty_read(n);

                        // Bump generation counter IMMEDIATELY on successful read,
                        // before any processing. This guarantees the counter always
                        // advances when PTY data arrives, even if processing encounters
                        // a panic or unexpected code path (e.g., Windows ConPTY after
                        // Ctrl+C where subsequent sequences may cause issues) — issue #60.
                        //
                        // This pre-processing bump alone is NOT sufficient: it advances
                        // the counter BEFORE the grid is written below, so a concurrent
                        // renderer that acquires the terminal lock in the window between
                        // here and the `terminal.write()` below reads the not-yet-updated
                        // grid but stamps its cell cache with this already-advanced
                        // generation. If this is the last read of a burst, the counter
                        // never advances again and that stale content is served until the
                        // next PTY read — the "some regions don't update" freeze in TUI
                        // apps (joe, vim) that repaint via partial line edits. A second
                        // bump after the grid write (see end of the write-guard block)
                        // closes that window.
                        let old_gen = update_generation.fetch_add(1, Ordering::SeqCst);
                        debug::log_generation_change(old_gen, old_gen + 1, "PTY read");

                        // Feed terminal output to coprocesses
                        {
                            let mut mgr = coprocess_manager.lock();
                            mgr.feed_output(&buffer[..n]);
                        }

                        // Process the bytes through the terminal.
                        //
                        // ARC-001: use `process_deferred` instead of `process` so
                        // observer callbacks (which may re-enter Python under the
                        // GIL, or otherwise block) are NOT invoked while the write
                        // guard below is held — that would stall every concurrent
                        // reader of this terminal (streaming clients, Python
                        // queries), nullifying the Mutex->RwLock migration
                        // (ARC-009). The returned batch is delivered after the
                        // write guard is dropped.
                        // ARC-027: bytes to write back to the PTY master for pending
                        // device-query responses. Populated inside the write guard
                        // below (where has_pending_responses/drain_responses/XTWINOPS
                        // filtering must run), but the actual blocking `write_all` +
                        // `flush` syscall is deferred until AFTER the guard is
                        // dropped, so it no longer stalls concurrent readers of this
                        // terminal (streaming clients, Python queries, screenshot
                        // rendering) for the syscall's duration.
                        let mut response_bytes: Vec<u8> = Vec::new();
                        let dispatch_batch = {
                            let mut term = terminal.write();
                            let was_alt_screen = term.is_alt_screen_active();
                            let batch = term.process_deferred(&buffer[..n]);
                            // Record output for session recording
                            term.record_output(&buffer[..n]);
                            // Process trigger scans on dirty rows
                            crate::terminal::TriggerEngine::process_trigger_scans(&mut term);
                            let is_alt_screen = term.is_alt_screen_active();

                            // Check for device query responses and stage them for
                            // writing back to the PTY after the write guard drops.
                            // This enables nested TUI applications (vim, htop, etc.) to work correctly
                            if term.has_pending_responses() {
                                let mut responses = term.drain_responses();

                                // Optional: filter out XTWINOPS (CSI t) replies to avoid shells
                                // echoing them visibly when ECHOCTL is enabled. Controlled by env
                                // PAR_TERM_REPLY_XTWINOPS (default: 1). Set to 0 to suppress.
                                // (Cached from env var at PtySession initialization)
                                if !reply_xtwinops.load(Ordering::Relaxed) {
                                    let mut filtered = Vec::with_capacity(responses.len());
                                    let mut i = 0;
                                    while i < responses.len() {
                                        if responses[i] == 0x1B
                                            && i + 1 < responses.len()
                                            && responses[i + 1] == b'['
                                        {
                                            // Collect until a final byte; drop if final is 't'
                                            let mut j = i + 2;
                                            let mut dropped = false;
                                            while j < responses.len() {
                                                let b = responses[j];
                                                if (b as char).is_ascii_alphabetic() {
                                                    // Alphabetic final byte for CSI
                                                    if b == b't' {
                                                        dropped = true;
                                                    }
                                                    j += 1;
                                                    break;
                                                }
                                                j += 1;
                                            }
                                            if !dropped {
                                                filtered.extend_from_slice(&responses[i..j]);
                                            }
                                            i = j;
                                        } else {
                                            filtered.push(responses[i]);
                                            i += 1;
                                        }
                                    }
                                    responses = filtered;
                                }

                                if !responses.is_empty() {
                                    response_bytes = responses;
                                }
                            }

                            // Send resize pulse (SIGWINCH) when entering alternate screen
                            // This helps applications like tmux recalculate their layout correctly
                            // (iTerm2 does this, which is why tmux works correctly there)
                            if !was_alt_screen && is_alt_screen {
                                debug::log(
                                    debug::DebugLevel::Trace,
                                    "ALT_SCREEN",
                                    "Entered alternate screen - sending SIGWINCH resize pulse",
                                );
                                // Best-effort pulse: skipped when the reap
                                // record is contended or says reaped, so a
                                // released PID is never signalled (SEC-125).
                                // try_lock keeps the terminal-then-record
                                // lock order deadlock-free.
                                #[cfg(unix)]
                                if let (Some(pid), Some(record)) = (child_pid, reaped.try_lock()) {
                                    if record.is_none() {
                                        #[cfg(test)]
                                        signals_sent.fetch_add(1, Ordering::SeqCst);
                                        // Current dimensions, not stale captured values
                                        let (current_cols, current_rows) = term.size();
                                        let _ = send_sigwinch(
                                            pid,
                                            "ALT_SCREEN",
                                            &format!(
                                                "alt-screen entry {current_cols}x{current_rows}"
                                            ),
                                        );
                                    }
                                }
                            }

                            // Second generation bump, now that the grid reflects
                            // this read's bytes (still inside the write guard, so any
                            // reader observing this new value either cannot yet take
                            // the terminal lock — and falls back to its cell cache —
                            // or takes it after the guard drops and sees the applied
                            // content). This guarantees the counter always moves PAST
                            // any value a renderer could have stamped with stale
                            // content during the pre-processing window above, so the
                            // next frame regenerates and catches up instead of
                            // freezing. Skipped only if processing panics — in which
                            // case the pre-processing bump above already preserved the
                            // issue #60 liveness guarantee.
                            let applied_gen = update_generation.fetch_add(1, Ordering::SeqCst);
                            debug::log_generation_change(
                                applied_gen,
                                applied_gen + 1,
                                "content applied",
                            );
                            // Still inside the write guard: refresh the
                            // wait-free geometry mirror so `size()` and
                            // `cursor_position()` track processed output
                            // without ever taking this lock (ENH-023).
                            geometry.publish(&term);

                            batch
                        }; // write guard (`term`) dropped here

                        // Applied content is now visible to read locks.
                        // Wake wait_for_update callers here — after the
                        // guard drop — so a woken reader takes the terminal
                        // lock without contending with this thread.
                        update_signal.1.notify_all();

                        // ARC-027: write staged device-query responses back to the
                        // PTY master now that the write guard is released, so the
                        // blocking write_all/flush syscall no longer holds up
                        // concurrent readers of this terminal. Done before
                        // dispatch_batch.deliver() below so device replies (which
                        // nested TUI apps like vim/htop block on) stay as prompt as
                        // possible.
                        if !response_bytes.is_empty() {
                            debug::log_device_query("pending", &response_bytes);
                            let mut w = writer.lock();
                            // Write responses back to PTY master so child can read them
                            let _ = w.write_all(&response_bytes);
                            let _ = w.flush();
                        }

                        // Call output callback if set (for streaming, logging,
                        // etc.). Fires after the bytes are applied to the
                        // terminal above, so a callback that reads terminal
                        // state (e.g. the mux daemon composing a reattach
                        // seed for a client it forwards these bytes to)
                        // observes state that already includes them — the
                        // forwarded output can never race ahead of the
                        // seed. Still raw, unmodified bytes.
                        {
                            let callback_guard = output_callback.lock();
                            if let Some(ref callback) = *callback_guard {
                                callback(&buffer[..n]);
                            }
                        }

                        // ARC-001: deliver observer callbacks now that the write
                        // guard is released, so slow/re-entrant observers don't
                        // block concurrent readers of this terminal.
                        dispatch_batch.deliver();
                    }
                    Err(e) => {
                        // Log error but continue (could be temporary)
                        crate::debug_error!("PTY", "PTY read error: {}", e);
                        // BrokenPipe, and on Unix EIO (the pty's form of
                        // EOF once the child's slave side is gone — see
                        // `read_error_is_terminal`), mean the pty is dead:
                        // exit like EOF rather than spinning on retries.
                        if read_error_is_terminal(&e) {
                            reader_dead(&running, &update_signal, &update_generation);
                            break;
                        }
                    }
                }
            }
        });

        self.reader_thread = Some(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// A terminal error must include BrokenPipe (the pre-existing fatal
    /// kind) and, on Unix, EIO — the pty's form of EOF after the child's
    /// slave side is gone (Linux always; macOS/BSD when no output is
    /// pending). Retryable errors (WouldBlock, Interrupted, UnexpectedEof)
    /// must not be classified as fatal, so ordinary transient failures
    /// keep the existing retry behavior.
    #[test]
    fn read_error_classifier_treats_broken_pipe_and_eio_as_terminal() {
        assert!(read_error_is_terminal(&std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "broken pipe"
        )));
        #[cfg(unix)]
        assert!(read_error_is_terminal(&std::io::Error::from_raw_os_error(
            libc::EIO
        )));
        for kind in [
            std::io::ErrorKind::WouldBlock,
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            let err = std::io::Error::new(kind, "transient");
            assert!(
                !read_error_is_terminal(&err),
                "{kind:?} must stay retryable"
            );
        }
    }

    /// A fake source that always fails with the given error, so the
    /// terminal-error path can be exercised without a real dead pty.
    struct FailingReader {
        err: std::io::Error,
        reads: Arc<AtomicUsize>,
    }

    impl Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Err(match self.err.raw_os_error() {
                Some(code) => std::io::Error::from_raw_os_error(code),
                None => std::io::Error::new(self.err.kind(), "injected"),
            })
        }
    }

    /// Regression test: the reader must treat a terminal read error as the
    /// dead-pty condition — store `running = false` and stop looping, so a
    /// dead pty cannot busy-spin the reader thread. Injected EIO on Unix
    /// (what a real dead pty read returns on Linux); BrokenPipe elsewhere,
    /// the one fatal kind the loop already recognized. Before the fix, EIO
    /// fell through to the retry path and the loop spun on the failing
    /// source forever.
    #[test]
    fn injected_terminal_read_error_stops_the_reader_without_spinning() {
        let mut session = PtySession::new(80, 24, 100);
        // Start from the live state a spawned session would have, so the
        // flip to false below is observable.
        session.running.store(true, Ordering::SeqCst);
        let reads = Arc::new(AtomicUsize::new(0));
        let injected = {
            #[cfg(unix)]
            {
                std::io::Error::from_raw_os_error(libc::EIO)
            }
            #[cfg(not(unix))]
            {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "injected")
            }
        };
        let reader = FailingReader {
            err: injected,
            reads: Arc::clone(&reads),
        };
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(Box::new(std::io::sink())));
        let gen_before = session.update_generation();

        session.start_reader_thread(Box::new(reader), writer, None);

        // The reader must exit promptly: running flips and the read count
        // stops growing (a spinning loop would keep incrementing `reads`).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while session.is_running() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            !session.is_running(),
            "a terminal read error must flip running to false"
        );

        // Let any concurrent final read land, then confirm the loop has
        // stopped consuming its source.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let settled = reads.load(Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(
            reads.load(Ordering::SeqCst),
            settled,
            "the reader must not spin on a dead pty"
        );
        assert!(
            session.update_generation() > gen_before,
            "the dead-pty exit must advance the generation"
        );
        let _ = session.reader_thread.take().unwrap().join();
    }

    /// Regression test: a child that exits without producing any output
    /// must be observable by a generation-poll consumer within one poll —
    /// the reader's EOF arm advances `update_generation` after the notify,
    /// so a poller that wakes (or sleep-polls) sees the counter move even
    /// though no bytes were ever read. Before the fix, a quiet death left
    /// the counter frozen.
    #[cfg(unix)]
    #[test]
    fn quiet_child_death_advances_the_generation() {
        let mut session = PtySession::new(80, 24, 100);
        assert!(session.spawn("/usr/bin/true", &[]).is_ok());
        let since = session.update_generation();

        // A generation-poll observer: bounded-window poll for the flip.
        // 30s: under a full parallel suite on the python-test build
        // (libpython linked), process spawn can outrun a tighter window.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let observed = loop {
            let gen = session.update_generation();
            if gen > since || std::time::Instant::now() >= deadline {
                break gen;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(
            observed > since,
            "a quiet child death must advance the generation within the window"
        );
        assert!(
            !session.is_running(),
            "the child is gone; running must be false"
        );
        let _ = session.wait();
    }
}
