#![cfg(feature = "pty_session")]
//! Card 01a218f2 criterion 1: PtySession::write throughput for 512 MiB of
//! input in 64 KiB chunks while a reader holds terminal read locks
//! concurrently — the upload shape the par-term frontend reported (585 MB
//! base64, 64 KiB chunks, frame gathers contending with the input path).
//!
//! The pre-queue write path (exclusive Terminal lock per chunk for the
//! `record_input` check, then a blocking `write_all`+`flush` under the
//! writer mutex) is emulated inline as the baseline, from the same public
//! handles the old code used (`terminal()` + `get_writer()`); the new path
//! is the shipped `PtySession::write`. Interleaved A/B rounds (same
//! child, same load) guard against drift between runs.
//!
//! `#[ignore = "load-sensitive: throughput-ratio assertion on a real PTY"]`
//! — evidence run (release build, serial):
//! `cargo test -p par-term-emu-core --features pty_session --release
//! --test pty_input_throughput -- --ignored --test-threads=1`

#[cfg(unix)]
mod pty_input_throughput {
    use par_term_emu_core::pty_session::PtySession;
    use par_term_emu_core::terminal::Terminal;
    use parking_lot::{Mutex, RwLock};
    use std::io::Write;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const CHUNK: usize = 64 * 1024;
    const CHUNKS: usize = 8192; // 512 MiB per run, the criterion's shape
    const ROUNDS: usize = 3;

    /// One payload chunk: `CHUNK` bytes, newline-terminated so canonical
    /// mode delivers it (echo left on — the par-term paste-upload shape).
    fn make_chunk(payload: &[u8]) -> Vec<u8> {
        let mut bytes = payload[..CHUNK].to_vec();
        bytes[CHUNK - 1] = b'\n';
        bytes
    }

    /// A stable terminal generation for `quiet` — the reader thread has
    /// drained the echo backlog before the next A/B round, so each round
    /// starts from the same state.
    fn wait_echo_settled(pty: &PtySession) {
        let mut last = pty.update_generation();
        let mut stable_since: Option<Instant> = None;
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            let now = pty.update_generation();
            if now != last {
                last = now;
                stable_since = None;
            } else if stable_since.is_some_and(|t| t.elapsed() > Duration::from_millis(200)) {
                return;
            } else if stable_since.is_none() {
                stable_since = Some(Instant::now());
            }
        }
        panic!("echo backlog never settled between rounds");
    }

    /// The pre-queue write path, emulated from the public handles.
    fn old_style_write(
        terminal: &Arc<RwLock<Terminal>>,
        writer: &Arc<Mutex<Box<dyn Write + Send>>>,
        data: &[u8],
    ) -> Result<(), par_term_emu_core::pty_error::PtyError> {
        {
            let mut term = terminal.write();
            term.record_input(data);
        }
        let mut w = writer.lock();
        w.write_all(data)
            .map_err(par_term_emu_core::pty_error::PtyError::IoError)?;
        w.flush()
            .map_err(par_term_emu_core::pty_error::PtyError::IoError)?;
        Ok(())
    }

    /// One full upload over the given path; returns (elapsed, MB/s).
    fn run_upload(pty: &mut PtySession, old_style: bool, payload: &[u8]) -> (Duration, f64) {
        let terminal = pty.terminal();
        let writer = pty.get_writer().expect("writer after spawn");
        let bytes = make_chunk(payload);
        let start = Instant::now();
        for i in 0..CHUNKS {
            if old_style {
                if let Err(e) = old_style_write(&terminal, &writer, &bytes) {
                    println!(
                        "old-style write failed at chunk {i}: {e}; running={} try_wait={:?}",
                        pty.is_running(),
                        pty.try_wait()
                    );
                    panic!("old-style chunk write failed");
                }
            } else {
                pty.write(&bytes).expect("queued chunk write");
            }
        }
        let elapsed = start.elapsed();
        let mbps = (CHUNKS * CHUNK) as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64();
        (elapsed, mbps)
    }

    #[test]
    #[ignore = "load-sensitive: throughput-ratio assertion on a real PTY (not a gate; excluded from the ci profile like the render-latency harness, run manually with --ignored)"]
    fn new_path_beats_old_path_10x_under_read_lock_contention() {
        let payload = vec![b'x'; CHUNK];

        // One session per round-pair would contend with a draining
        // predecessor; one long-lived session, settled between rounds.
        // The child keeps the DEFAULT tty settings (canonical + echo) —
        // the paste-like upload shape par-term measured: every chunk ends
        // in a newline (real pastes and base64 wraps are line-terminated)
        // so canonical delivery works, and the echo drives the reader
        // thread's write-lock churn that made the old per-chunk exclusive
        // lock wait behind frame gathers and output processing.
        let mut pty = PtySession::new(80, 24, 100);
        pty.spawn("/bin/sh", &["-c", "cat > /dev/null"])
            .expect("spawn echo cat");

        // The concurrent frame-gather simulation: holds terminal read
        // locks for realistic frame-composition durations — a frontend
        // snapshotting a large terminal holds the read lock for
        // milliseconds, not microseconds (par-term measured 21-43 s frame
        // gathers under upload churn; 15 ms models the unpathological
        // steady case). Hold 15 ms, release 2 ms, for the whole window.
        let holder = {
            let terminal = pty.terminal();
            std::thread::spawn(move || {
                let until = Instant::now() + Duration::from_secs(600);
                while Instant::now() < until {
                    let _guard = terminal.read();
                    std::thread::sleep(Duration::from_millis(15));
                    drop(_guard);
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        };

        let mut old_samples: Vec<f64> = Vec::new();
        let mut new_samples: Vec<f64> = Vec::new();
        for round in 0..ROUNDS {
            let (t_old, mbps_old) = run_upload(&mut pty, true, &payload);
            wait_echo_settled(&pty);
            let (t_new, mbps_new) = run_upload(&mut pty, false, &payload);
            wait_echo_settled(&pty);
            println!(
                "round {round}: old {:.0} ms ({mbps_old:.1} MB/s)  new {:.0} ms ({mbps_new:.1} MB/s)  ratio {:.1}x",
                t_old.as_secs_f64() * 1e3,
                t_new.as_secs_f64() * 1e3,
                mbps_new / mbps_old,
            );
            old_samples.push(mbps_old);
            new_samples.push(mbps_new);
        }

        holder.join().expect("read-lock holder");

        // Median ratio over interleaved rounds — a single round's ratio is
        // host-noise-sensitive.
        let median = |mut v: Vec<f64>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        };
        let ratio = median(new_samples.clone()) / median(old_samples.clone());
        println!(
            "median ratio: {ratio:.2}x  (old {:.1} MB/s, new {:.1} MB/s)",
            median(old_samples),
            median(new_samples)
        );
        assert!(
            ratio >= 10.0,
            "queued write throughput must be >=10x the old per-chunk-lock path (median ratio {ratio:.2}x)"
        );
    }
}
