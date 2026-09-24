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
pub fn accept_connection(listener: &LocalListener) -> io::Result<(LocalStream, ConnectionAbort)> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::traits::Listener as _;
        Ok((listener.accept()?, ConnectionAbort))
    }
    #[cfg(windows)]
    {
        listener.accept_with_abort()
    }
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
/// the temp dir; Windows uses the (per-user) temp dir as the marker-file
/// location the pipe name is derived from.
pub fn default_socket_path(name: &str) -> PathBuf {
    #[cfg(unix)]
    {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!("par-mux-{name}.sock"))
    }

    #[cfg(windows)]
    {
        std::env::temp_dir().join(format!("par-mux-{name}.sock"))
    }
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
    // Windows serves its wrapper listener, whose accept is inherent.
    #[cfg(unix)]
    use interprocess::local_socket::traits::Listener as _;
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
            let mut stream = listener.accept().expect("accept");
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
}
