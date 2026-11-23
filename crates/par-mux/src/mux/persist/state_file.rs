//! Where a daemon's state file lives (D3.4) and the daemon's exclusive
//! claim on it.

use std::fs;
use std::path::{Path, PathBuf};

/// The state file for a server listening on `socket_path` (D3.4):
/// `<state_dir>/par-mux/<socket-stem>.state.json`. The socket itself lives
/// in the ephemeral temp dir by design; state must not — pane content is as
/// private as the terminal it came from.
pub fn state_file_path(socket_path: &Path) -> PathBuf {
    state_file_in(&platform_state_dir(), socket_path)
}

/// The platform state dir, falling back to the data dir on platforms
/// without a distinct one (D3.4: macOS has no XDG state dir, so state lives
/// under `~/Library/Application Support` there).
pub fn platform_state_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
}

/// The pure half of [`state_file_path`]: a socket at
/// `<any>/par-mux-<stem>.sock` maps to `<base>/par-mux/<stem>.state.json`,
/// so two servers on different sockets never share state. `base` is exposed
/// (ARC-017) so `par-mux --state-dir <dir>` can override
/// `platform_state_dir` without duplicating this join logic.
pub fn state_file_in(base: &Path, socket_path: &Path) -> PathBuf {
    let stem = socket_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("default");
    base.join("par-mux").join(format!("{stem}.state.json"))
}

/// The daemon's exclusive claim on a state file: an advisory `flock` on
/// `<state>.lock`, held for the daemon's whole lifetime. A second daemon
/// resolving to the same state path fails here instead of racing the
/// first daemon's saves — the 2026-10-27 crash loop: two live daemons'
/// fixed-`<target>.tmp` saves interleaved, corrupted the 161 MB state at
/// byte ~107 MB, and the quarantine/restart churn dropped every attached
/// client. The claim survives the `--restart` fork: the child inherits
/// the open file description, and a `flock` lives as long as any fd to
/// that description does. Windows takes an exclusive `LockFileEx` byte-range
/// lock on the same `<state>.lock` file (SEC-005), held for the daemon's
/// lifetime and released with the handle; the locked byte sits far past the
/// pid line so another daemon can still read the holder's pid. Windows
/// `--restart` serves in-process, so there is no cross-fork inheritance to
/// preserve.
#[derive(Debug)]
pub struct StateLock {
    /// Keeps the `flock` alive; dropping it releases the claim.
    _file: Option<fs::File>,
}

/// The lock file's first line names its holder's pid, so a refusal can
/// point the operator at the process to stop.
#[cfg(any(unix, windows))]
fn lock_holder_pid(lock_path: &Path) -> Option<u32> {
    fs::read_to_string(lock_path)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

/// Claim `state_path` for this daemon, refusing while another live daemon
/// holds it. The retry window absorbs the `--stop`/`--restart` handoff,
/// where the previous daemon is mid-exit and releases within a beat; a
/// holder that outlasts it is refused by pid so the operator can act.
pub fn acquire_state_lock(state_path: &Path) -> std::io::Result<StateLock> {
    use std::io::Write;
    let mut os = state_path.as_os_str().to_os_string();
    os.push(".lock");
    let lock_path = PathBuf::from(os);
    // The claim runs before the first save, so the state dir may not
    // exist yet (a fresh --state-dir); the daemon owns its state base
    // and creates it as write_job would.
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut held = try_lock_exclusive(&file);
    while held.is_err() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
        held = try_lock_exclusive(&file);
    }
    if held.is_err() {
        let holder = lock_holder_pid(&lock_path)
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "an unknown process".to_string());
        return Err(std::io::Error::other(format!(
            "state {} is already served by another daemon (pid {holder}); \
             stop that daemon first (`par-mux --stop`) — two daemons on \
             one state file corrupt each other's saves",
            state_path.display()
        )));
    }
    file.set_len(0)?;
    file.write_all(format!("{}\n", std::process::id()).as_bytes())?;
    Ok(StateLock { _file: Some(file) })
}

/// Non-blocking exclusive advisory lock on the whole file (`flock`).
#[cfg(unix)]
fn try_lock_exclusive(file: &fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Non-blocking exclusive `LockFileEx` on one byte far beyond the pid line.
/// Windows byte-range locks are mandatory for other handles, so the locked
/// range must not overlap the bytes a refused daemon reads to name the holder.
#[cfg(windows)]
fn try_lock_exclusive(file: &fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;
    // SAFETY: an all-zero OVERLAPPED is its documented initial state; the
    // handle is owned by `file`, which outlives the call.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.Anonymous.Anonymous.Offset = 0;
    overlapped.Anonymous.Anonymous.OffsetHigh = 0x7FFF_0000;
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
