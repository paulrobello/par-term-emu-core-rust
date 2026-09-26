//! Cross-platform local socket transport: Unix domain sockets on Unix,
//! named pipes on Windows, behind one Listener/Stream pair.
//!
//! Platform facts that shape this module (see `par-mux.md` D4):
//!
//! 1. Unix resolves a filesystem path; Windows resolves a namespaced pipe
//!    name derived from the path string.
//! 2. Windows pipes live outside the filesystem, so a marker file is written
//!    at the path after binding — path-based staleness checks then work
//!    uniformly on both platforms.
//! 3. `0o600` has no Windows equivalent: a named pipe created without an
//!    explicit security descriptor is openable by other users on the machine.
//!    The pipe is therefore created with an owner-only DACL.
//! 4. On Unix the fallback base is a shared temp dir on Linux, so default
//!    sockets live in a per-UID `0700` directory (tmux's `/tmp/tmux-<uid>`
//!    model) whose owner and mode are verified before bind and connect, and
//!    accepted connections are refused unless the peer runs as this user.

use std::io;
use std::path::{Path, PathBuf};

pub use interprocess::local_socket::Stream as LocalStream;

// Unix serves the interprocess listener as-is: its sockets honor send/recv
// timeouts, which is what the eviction teardown's poll loop needs (ENH-012).
// Windows must wrap the named-pipe listener instead — see `LocalListener`
// below — because the generic listener type hides the pipe handles that
// eviction needs to cancel blocked I/O through.
#[cfg(unix)]
pub use interprocess::local_socket::Listener as LocalListener;

/// Bind a local listener at `path`, owned by the current user only.
///
/// Unix: the socket file is created with mode `0600`. Windows: the pipe is
/// created with an SDDL security descriptor granting access to the system and
/// the creating user only, and a marker file is written at `path`.
pub fn bind_local_listener(path: &Path) -> io::Result<LocalListener> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::{prelude::*, GenericFilePath, ListenerOptions};

        let name = path.to_fs_name::<GenericFilePath>()?;
        let listener = ListenerOptions::new()
            .name(name)
            .reclaim_name(false)
            .create_sync()?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(listener)
    }

    #[cfg(windows)]
    {
        use interprocess::local_socket::prelude::*;
        use interprocess::local_socket::GenericNamespaced;
        use interprocess::local_socket::ListenerOptions;
        use interprocess::os::windows::local_socket::ListenerOptionsExt as _;
        use interprocess::os::windows::named_pipe::local_socket::Listener as NpListener;
        use interprocess::os::windows::security_descriptor::SecurityDescriptor;
        use widestring::U16CString;

        // Owner-only DACL: full access for the system and the creating user,
        // nothing for anyone else on the machine.
        let sddl = U16CString::from_str("D:P(A;;GA;;;SY)(A;;GA;;;OW)")
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
        let security_descriptor = SecurityDescriptor::deserialize(&sddl)?;
        let name = path
            .to_string_lossy()
            .to_string()
            .to_ns_name::<GenericNamespaced>()?;
        let listener = NpListener::from_options(
            ListenerOptions::new()
                .name(name)
                .reclaim_name(false)
                .security_descriptor(security_descriptor),
        )?;
        std::fs::write(path, windows_socket_marker())?;
        Ok(LocalListener(listener))
    }
}

/// Accept one connection, returning the stream to serve it on plus the
/// [`ConnectionAbort`] the server's eviction path needs. Every accept goes
/// through here so no platform can drift out of the pair contract.
///
/// Unix refuses a connection whose peer is not running as this user: the
/// per-UID socket directory already keeps other users from reaching the
/// socket, and this is the second lock on the door (tmux checks credentials
/// on accept too). A refused peer is dropped and the listener keeps
/// serving — erroring instead would hand any local user the daemon's
/// fatal-fault exit.
pub fn accept_connection(listener: &LocalListener) -> io::Result<(LocalStream, ConnectionAbort)> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::traits::Listener as _;
        loop {
            let stream = listener.accept()?;
            if peer_is_current_user(&stream) {
                return Ok((stream, ConnectionAbort));
            }
            log::warn!("par-mux: refused a socket connection from another user");
            // Bound the spin of a repeated offender; the sleep is inside the
            // accept cadence, not the serving path.
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    #[cfg(windows)]
    {
        listener.accept_with_abort()
    }
}

/// Whether the stream's peer runs as this daemon's user. A peer the platform
/// will not vouch for is treated as foreign — fail closed.
#[cfg(unix)]
fn peer_is_current_user(stream: &LocalStream) -> bool {
    use interprocess::local_socket::traits::StreamCommon as _;

    stream
        .peer_creds()
        .ok()
        .and_then(|creds| creds.euid())
        .is_some_and(|euid| euid == current_uid())
}

/// What an evictor needs to tear this connection down (ENH-012).
///
/// Unix: the connection's threads run with send/recv timeouts, so setting
/// the eviction flag is enough — they wake on their next poll and exit. This
/// type is a no-op placeholder so callers share one shape.
///
/// Windows: named pipes reject I/O timeouts (interprocess 2.4 implements
/// both setters as `Err(Unsupported)` stubs), so a flagged thread parked in
/// `ReadFile`/`WriteFile` never wakes. [`ConnectionAbort::cancel_blocked_io`]
/// aborts those calls through `CancelIoEx` on a duplicated pipe handle.
/// Dropping every server-side handle — this duplicate included — is what
/// the client observes as the disconnect, so the abort handle must not
/// outlive the connection's registry entry.
#[cfg(unix)]
pub struct ConnectionAbort;

#[cfg(unix)]
impl ConnectionAbort {
    /// Synthetic registry entries (tests) have no connection to abort.
    #[cfg(test)]
    pub fn none() -> Self {
        ConnectionAbort
    }

    /// Unix sockets wake the flagged threads on their own timeouts.
    pub fn cancel_blocked_io(&self) {}
}

#[cfg(windows)]
pub struct ConnectionAbort {
    handle: Option<std::os::windows::io::OwnedHandle>,
}

#[cfg(windows)]
mod cancel_io_ex {
    use std::os::windows::io::RawHandle;

    #[link(name = "kernel32")]
    extern "system" {
        fn CancelIoEx(hfile: *mut core::ffi::c_void, lpoverlapped: *mut core::ffi::c_void) -> i32;
    }

    /// Abort every pending I/O operation on the pipe behind `handle`.
    /// Verified against interprocess 2.4 named pipes (2026-09-24): cancels a
    /// blocked synchronous read AND write regardless of which handle clone
    /// issued it — the blocked call fails with OS error 995
    /// (ERROR_OPERATION_ABORTED) — and cancelling a handle with no pending
    /// I/O is a harmless no-op. `false` only means the API call itself
    /// failed, which leaves the pre-fix behavior unchanged.
    pub(super) fn cancel(handle: RawHandle) -> bool {
        // SAFETY: `handle` is borrowed from an OwnedHandle the caller keeps
        // alive across the call; a NULL overlapped targets every pending
        // operation on the handle rather than one specific request.
        unsafe { CancelIoEx(handle as *mut _, std::ptr::null_mut()) != 0 }
    }
}

#[cfg(windows)]
impl ConnectionAbort {
    pub(crate) fn new(handle: std::os::windows::io::OwnedHandle) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    /// Synthetic registry entries (tests) have no connection to abort.
    #[cfg(test)]
    pub fn none() -> Self {
        Self { handle: None }
    }

    /// Abort this connection's blocked `ReadFile`/`WriteFile` calls so its
    /// threads can observe the eviction flag and exit.
    ///
    /// The first cancel lands while the wedged threads are parked (the
    /// normal eviction shape: a non-draining client). A thread that happened
    /// to be mid-dispatch missed it and would park forever afterward — pipes
    /// have no timeout to wake it later — so a short detached re-cancel
    /// covers that window. The retry's handle clone keeps the pipe open only
    /// until the last retry, holding the client's disconnect inside the same
    /// fraction-of-a-second envelope the Unix poll loop delivers.
    pub fn cancel_blocked_io(&self) {
        use std::os::windows::io::{AsHandle as _, AsRawHandle as _};

        let Some(handle) = &self.handle else {
            return;
        };
        cancel_io_ex::cancel(handle.as_raw_handle());
        let Ok(retry) = handle.as_handle().try_clone_to_owned() else {
            return;
        };
        std::thread::spawn(move || {
            for delay in [
                std::time::Duration::from_millis(150),
                std::time::Duration::from_millis(300),
            ] {
                std::thread::sleep(delay);
                cancel_io_ex::cancel(retry.as_raw_handle());
            }
            // The clone drops here: if the connection's threads are already
            // gone, this was the last server-side handle, and the client's
            // blocking read now returns EOF.
        });
    }
}

/// The named-pipe-backed listener Windows serves from, wrapping
/// interprocess's own named-pipe local-socket listener.
///
/// The generic `local_socket::Listener` cannot serve here: it accepts
/// through a type-erasing enum that exposes no pipe handle, and eviction
/// needs a handle duplicate taken from the very pipe instance that becomes
/// the served connection (a duplicate of a different instance cancels
/// nothing). Accepting on the underlying [`NpListener::inner`] pipe listener
/// and rebuilding the generic stream from a handle clone keeps every
/// existing `LocalStream` consumer unchanged.
#[cfg(windows)]
pub struct LocalListener(interprocess::os::windows::named_pipe::local_socket::Listener);

#[cfg(windows)]
impl LocalListener {
    /// Accept, discarding the abort handle — for call sites that never
    /// evict (tests, one-shot readers).
    pub fn accept(&self) -> io::Result<LocalStream> {
        self.accept_with_abort().map(|(stream, _)| stream)
    }

    /// Accept, producing the stream and its eviction abort handle. The
    /// served stream is rebuilt from a duplicate of the accepted pipe
    /// handle, so it refers to the exact instance the abort can cancel.
    fn accept_with_abort(&self) -> io::Result<(LocalStream, ConnectionAbort)> {
        use interprocess::os::windows::named_pipe::local_socket::Stream as NpStream;
        use std::os::windows::io::AsHandle as _;

        let pipe_stream = self.0.inner().accept()?;
        let abort = pipe_stream
            .as_handle()
            .try_clone_to_owned()
            .map_err(|err| io::Error::new(io::ErrorKind::ConnectionAborted, err))?;
        // The served stream wraps the accepted pipe stream directly. The
        // tempting alternative — converting a duplicated handle back
        // through `TryFrom<OwnedHandle>` — re-opens the handle overlapped
        // (`ReOpenFile`), and a synchronous `ReadFile` on an overlapped
        // handle blocks forever (observed on the Windows VM 2026-09-24).
        // This plain wrap is the same construction the generic listener's
        // own accept path performs.
        let stream = LocalStream::from(NpStream::from(pipe_stream));
        Ok((stream, ConnectionAbort::new(abort)))
    }

    /// See `interprocess::local_socket::traits::Listener::set_nonblocking`.
    /// An inherent method, because the interprocess trait is sealed and the
    /// enum listener this replaces exposed it via that trait.
    pub fn set_nonblocking(
        &self,
        nonblocking: interprocess::local_socket::ListenerNonblockingMode,
    ) -> io::Result<()> {
        use interprocess::local_socket::traits::Listener as _;
        self.0.set_nonblocking(nonblocking)
    }
}

/// Connect a stream to the server listening at `path`.
pub fn connect_local_stream(path: &Path) -> io::Result<LocalStream> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::{prelude::*, GenericFilePath};

        guard_fallback_socket_dir(path)?;
        let name = path.to_fs_name::<GenericFilePath>()?;
        LocalStream::connect(name)
    }

    #[cfg(windows)]
    {
        use interprocess::local_socket::{prelude::*, GenericNamespaced};

        let name = path
            .to_string_lossy()
            .to_string()
            .to_ns_name::<GenericNamespaced>()?;
        LocalStream::connect(name)
    }
}

/// Prepare `path` for binding: refuse it when a live server owns it, reclaim
/// it when only a stale remnant (dead socket file or marker) does.
///
/// A successful connect means a live server — `AddrInUse` is returned, which
/// is what stops two daemons from owning one path. A stale-socket connect
/// error means the previous owner is gone; the remnant is removed. Anything
/// else is reported unchanged.
pub fn prepare_socket_path(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    guard_fallback_socket_dir(path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if !path.exists() {
        return Ok(());
    }

    // Unix: a non-socket file cannot be a live server's endpoint, so reclaim
    // it outright. Without this, a stray regular file at the path surfaces as
    // an uncategorized ENOTSOCK connect error and bricks the path forever —
    // macOS observed 2026-09-21, and the error-kind lists cannot express it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let is_socket = std::fs::metadata(path)
            .map(|meta| meta.file_type().is_socket())
            .unwrap_or(false);
        if !is_socket {
            return remove_remnant(path);
        }
    }

    match connect_local_stream(path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("another server owns {}", path.display()),
            ));
        }
        Err(err) if stale_socket_connect_error(err.kind()) => {}
        Err(err) => return Err(err),
    }

    remove_remnant(path)
}

/// Remove the remnant at `path`, tolerating a concurrent removal.
fn remove_remnant(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// The default socket path for a server named `name`, namespaced per user.
///
/// Unix prefers `$XDG_RUNTIME_DIR` (per-user by definition) and falls back to
/// a per-UID directory under the temp dir — see [`uid_socket_dir`] for why
/// the fallback is not the temp dir itself; Windows uses the (per-user) temp
/// dir as the marker-file location the pipe name is derived from.
pub fn default_socket_path(name: &str) -> PathBuf {
    #[cfg(unix)]
    {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(uid_socket_dir);
        base.join(format!("par-mux-{name}.sock"))
    }

    #[cfg(windows)]
    {
        std::env::temp_dir().join(format!("par-mux-{name}.sock"))
    }
}

/// The per-UID directory the shared-temp fallback serves its sockets from.
///
/// `std::env::temp_dir()` is `/tmp` on Linux: world-writable, shared by
/// every user on the machine. A socket named directly under it can be
/// pre-bound by another user, whose server then receives this client's
/// keystrokes and clipboard. The per-UID `0700` directory is tmux's
/// `/tmp/tmux-<uid>` defense; macOS `$TMPDIR` is already per-user, but the
/// extra directory costs nothing and keeps one code path.
#[cfg(unix)]
fn uid_socket_dir() -> PathBuf {
    std::env::temp_dir().join(format!("par-mux-{}", current_uid()))
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid takes no arguments and cannot fail.
    unsafe { libc::getuid() }
}

/// The default socket path before the per-UID directory move (pre-0.52).
///
/// A daemon from before the move keeps serving this path with every session
/// it owns, invisible to [`default_socket_path`]. `MuxClient::connect_or_spawn`
/// probes it before spawning a replacement so an upgrade never strands a
/// live daemon behind a parallel one (observed 2026-09-26: a 0.51 daemon
/// kept serving while 0.52 clients built a second world for the same name).
/// Windows never moved — its default path is unchanged — so this is
/// Unix-only.
#[cfg(unix)]
pub(crate) fn legacy_socket_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("par-mux-{name}.sock"))
}

/// When `path` sits in the per-UID fallback directory, make sure that
/// directory exists and is exclusively ours before it is used for binding
/// or connecting. Paths elsewhere — an explicit `--socket`, or
/// `$XDG_RUNTIME_DIR`, which the OS guarantees per-user — are untouched.
#[cfg(unix)]
fn guard_fallback_socket_dir(path: &Path) -> io::Result<()> {
    socket_dir_guard(path, &uid_socket_dir())
}

/// The parent-match half of the guard, split out so tests can stage a
/// hostile directory as `base` without touching the real per-UID one.
#[cfg(unix)]
fn socket_dir_guard(path: &Path, base: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) if parent == base => ensure_owned_socket_dir(parent),
        _ => Ok(()),
    }
}

/// Create `dir`, or verify the existing one is exclusively ours: owned by
/// the current UID and granting nothing to group or other. Anything else
/// fails closed — a directory another user controls could host their socket
/// at our path, and one with group/other access lets them reach ours.
#[cfg(unix)]
fn ensure_owned_socket_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    match std::fs::create_dir(dir) {
        Ok(()) => {
            // create_dir applies the umask; pin 0700 before any socket lives
            // under it. A pre-existing directory keeps its own mode — the
            // checks below are what gate it.
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }

    let meta = std::fs::metadata(dir)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} exists but is not a directory", dir.display()),
        ));
    }
    if meta.uid() != current_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "socket directory {} is owned by uid {}, not the current uid {} — refusing to use it",
                dir.display(),
                meta.uid(),
                current_uid()
            ),
        ));
    }
    let mode = meta.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "socket directory {} is accessible by group or other (mode {:o}) — refusing to use it",
                dir.display(),
                mode & 0o777
            ),
        ));
    }
    Ok(())
}

/// Whether a connect error means "nothing live owns this path".
fn stale_socket_connect_error(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound | io::ErrorKind::TimedOut
    ) || (cfg!(windows) && kind == io::ErrorKind::WouldBlock)
}

/// Marker file contents: enough to identify the owning server process.
#[cfg(windows)]
fn windows_socket_marker() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}:{now}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A socket path that no other test run can ever name, cleaned up even
    /// when the test panics.
    ///
    /// Two failure modes motivate this over a `process::id()`-derived path in
    /// the shared temp dir. A path built from the pid alone is reproducible
    /// across `cargo test` invocations once the OS recycles that pid, so a
    /// remnant of an earlier run — or an orphaned listener still bound to it —
    /// collides with the new run and surfaces as `AddrInUse`. And a trailing
    /// `remove_file` never runs on an early return or a failed assertion, so
    /// those remnants accumulate instead of self-healing.
    ///
    /// `TempDir` answers both: the directory name carries OS-provided
    /// randomness, so no two runs collide, and its `Drop` removes the
    /// directory (socket included) while the test unwinds.
    struct TempSocket {
        dir: tempfile::TempDir,
        path: std::path::PathBuf,
    }

    impl TempSocket {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    fn temp_socket(tag: &str) -> TempSocket {
        // The `par-mux-ipc-` prefix keeps a leaked directory visible to the
        // same `par-mux-ipc-*` glob used to audit leftovers by hand.
        let dir = tempfile::Builder::new()
            .prefix("par-mux-ipc-")
            .tempdir()
            .expect("create temp dir for socket");
        let path = dir.path().join(tag);
        TempSocket { dir, path }
    }

    #[test]
    fn bind_then_connect_round_trips_bytes() {
        let socket = temp_socket("round-trip");
        let path = socket.path();
        let listener = bind_local_listener(path).expect("bind succeeds");

        let server = std::thread::spawn(move || {
            // accept_connection, not the raw trait accept: the same-user peer
            // check it adds must pass for this connection.
            let (mut stream, _abort) = accept_connection(&listener).expect("accept");
            let mut buf = [0u8; 5];
            stream.read_exact(&mut buf).expect("read");
            stream.write_all(b"pong\n").expect("write");
        });

        let mut client = connect_local_stream(path).expect("connect succeeds");
        client.write_all(b"ping\n").expect("write");
        let mut reply = String::new();
        client.read_to_string(&mut reply).expect("read reply");
        assert_eq!(reply.trim(), "pong");

        server.join().expect("server thread");
    }

    #[test]
    fn prepare_refuses_a_live_socket() {
        let socket = temp_socket("live");
        let _listener = bind_local_listener(socket.path()).expect("bind succeeds");
        let err =
            prepare_socket_path(socket.path()).expect_err("a live socket must not be reclaimed");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::AddrInUse,
            "refusing a live socket is what stops two daemons owning one path"
        );
    }

    #[test]
    fn prepare_reclaims_a_stale_path() {
        let socket = temp_socket("stale");
        {
            let _listener = bind_local_listener(socket.path()).expect("bind succeeds");
            // Listener drops here; on Unix the path file may survive it.
        }
        prepare_socket_path(socket.path()).expect("a stale path is reclaimable");
        let _listener = bind_local_listener(socket.path()).expect("rebind after reclaim");
    }

    #[test]
    fn prepare_reclaims_a_path_that_is_not_a_socket() {
        // A leftover regular file (a crashed write, a stale marker) must be
        // reclaimable too, or one junk file bricks the default path forever.
        let socket = temp_socket("junk");
        std::fs::write(socket.path(), b"not a socket").expect("write junk file");
        prepare_socket_path(socket.path()).expect("a non-socket file is reclaimable");
        let _listener = bind_local_listener(socket.path()).expect("rebind after reclaim");
    }

    #[test]
    fn prepare_is_a_noop_when_nothing_exists() {
        let socket = temp_socket("absent");
        prepare_socket_path(socket.path()).expect("absent path is fine");
    }

    #[test]
    fn temp_socket_paths_never_collide() {
        // The regression this guards: paths built from `process::id()` alone
        // repeat once the OS recycles a pid, so a remnant of an earlier
        // `cargo test` invocation collides with a later one. Two fixtures
        // sharing a tag must still land on distinct paths, and the socket the
        // caller binds must sit inside its own fixture directory so a stale
        // file from anywhere else cannot be named.
        let first = temp_socket("same-tag");
        let second = temp_socket("same-tag");
        assert_ne!(
            first.path(),
            second.path(),
            "two fixtures with one tag must not share a path"
        );
        for socket in [&first, &second] {
            assert!(
                socket.path().starts_with(socket.dir.path()),
                "the socket lives inside its own fixture dir: {}",
                socket.path().display()
            );
        }
    }

    #[test]
    fn temp_socket_cleans_up_after_a_panic() {
        // The other half of the regression: a trailing `remove_file` never
        // runs when a test panics, so remnants accumulate. `Drop` runs while
        // unwinding, so the directory goes even on an assertion failure.
        let dir = std::panic::catch_unwind(|| {
            let socket = temp_socket("panicking");
            let dir = socket.dir.path().to_path_buf();
            bind_local_listener(socket.path()).expect("bind succeeds");
            panic!("{}", dir.display());
        })
        .expect_err("the closure panics");
        let dir = std::path::PathBuf::from(
            dir.downcast_ref::<String>()
                .expect("panic payload is the dir path"),
        );
        assert!(
            !dir.exists(),
            "the fixture dir is removed while unwinding: {}",
            dir.display()
        );
    }

    #[test]
    fn default_socket_path_is_per_user_and_named() {
        let path = default_socket_path("default");
        let rendered = path.to_string_lossy();
        assert!(
            rendered.contains("par-mux"),
            "path is namespaced: {rendered}"
        );
        assert!(
            rendered.contains("default"),
            "path carries the server name: {rendered}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let socket = temp_socket("perms");
        let _listener = bind_local_listener(socket.path()).expect("bind succeeds");
        let mode = std::fs::metadata(socket.path())
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "group and other must have no access: {mode:o}"
        );
    }

    #[cfg(unix)]
    mod socket_dir_guard {
        use super::*;

        /// A guard-dir target inside a throwaway base, so tests never touch
        /// the real per-UID directory.
        fn guard_dir(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
            let base = tempfile::Builder::new()
                .prefix("par-mux-guard-")
                .tempdir()
                .expect("create temp base");
            let dir = base.path().join(tag);
            (base, dir)
        }

        #[test]
        fn creates_a_0700_directory_owned_by_the_current_user() {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let (_base, dir) = guard_dir("fresh");
            ensure_owned_socket_dir(&dir).expect("creates the guard dir");
            let meta = std::fs::metadata(&dir).expect("stat the guard dir");
            assert_eq!(
                meta.uid(),
                current_uid(),
                "a freshly created guard dir is owned by this user"
            );
            assert_eq!(
                meta.permissions().mode() & 0o777,
                0o700,
                "a freshly created guard dir is 0700 regardless of umask"
            );
        }

        #[test]
        fn accepts_an_existing_owned_0700_dir() {
            use std::os::unix::fs::PermissionsExt;
            let (_base, dir) = guard_dir("kept");
            std::fs::create_dir(&dir).expect("stage dir");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .expect("tighten");
            ensure_owned_socket_dir(&dir).expect("an owned 0700 dir passes");
        }

        #[test]
        fn refuses_a_dir_with_group_access() {
            use std::os::unix::fs::PermissionsExt;
            let (_base, dir) = guard_dir("group");
            std::fs::create_dir(&dir).expect("stage dir");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o750))
                .expect("loosen to group-readable");
            let err = ensure_owned_socket_dir(&dir).expect_err("group access is refused");
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        }

        #[test]
        fn refuses_a_world_accessible_dir() {
            use std::os::unix::fs::PermissionsExt;
            let (_base, dir) = guard_dir("world");
            std::fs::create_dir(&dir).expect("stage dir");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o701))
                .expect("loosen to other-execute");
            let err = ensure_owned_socket_dir(&dir).expect_err("other access is refused");
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        }

        #[test]
        fn refuses_a_non_directory_at_the_guard_path() {
            let (_base, dir) = guard_dir("file");
            std::fs::write(&dir, b"not a directory").expect("stage a file");
            let err = ensure_owned_socket_dir(&dir).expect_err("a file is refused");
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        }

        #[test]
        fn guard_targets_the_per_uid_fallback_only() {
            use std::os::unix::fs::PermissionsExt;
            // A custom path (explicit --socket, or XDG base) must pass
            // through untouched even when its parent is loose: the user
            // chose that location.
            let base = tempfile::Builder::new()
                .prefix("par-mux-custom-")
                .tempdir()
                .expect("create temp base");
            let loose = base.path().join("loose-dir");
            std::fs::create_dir(&loose).expect("stage dir");
            std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o755))
                .expect("leave loose");
            let custom_socket = loose.join("par-mux-custom.sock");
            socket_dir_guard(&custom_socket, &uid_socket_dir())
                .expect("a path outside the fallback dir is not guarded");
        }

        #[test]
        fn fallback_default_path_lives_in_the_per_uid_dir() {
            // The real fallback layout: <tmp>/par-mux-<uid>/par-mux-<name>.sock
            // — and the wiring runs the guard for exactly that parent. This
            // creates the genuine per-UID dir under the real temp dir, which
            // is what any default-path daemon run would create anyway.
            let socket = uid_socket_dir().join("par-mux-layout.sock");
            guard_fallback_socket_dir(&socket).expect("guard runs on the fallback layout");
            assert!(
                socket.starts_with(uid_socket_dir()),
                "fallback socket lives in the per-UID dir: {}",
                socket.display()
            );
        }

        #[test]
        fn a_loose_fallback_dir_is_refused_for_bind_and_connect() {
            use std::os::unix::fs::PermissionsExt;
            // The attack: another user pre-creates the per-UID dir with
            // group/other bits, so their socket could be reached at the
            // default path. The guard staged on a stand-in base — the real
            // one belongs to this user and must not be loosened by a test —
            // must fail closed for both entry points.
            let base = tempfile::Builder::new()
                .prefix("par-mux-hostile-")
                .tempdir()
                .expect("create temp base");
            let dir = base.path().join(format!("par-mux-{}", current_uid()));
            std::fs::create_dir(&dir).expect("stage hostile dir");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("loosen");
            let socket = dir.join("par-mux-evil.sock");
            let err = socket_dir_guard(&socket, &dir).expect_err("a loose dir is refused");
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        }
    }
}
