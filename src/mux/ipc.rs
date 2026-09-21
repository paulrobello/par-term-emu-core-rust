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

pub use interprocess::local_socket::{Listener as LocalListener, Stream as LocalStream};

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
        let listener = ListenerOptions::new()
            .name(name)
            .reclaim_name(false)
            .security_descriptor(security_descriptor)
            .create_sync()?;
        std::fs::write(path, windows_socket_marker())?;
        Ok(listener)
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
    use interprocess::local_socket::traits::Listener as _;
    use std::io::{Read, Write};

    fn temp_socket(tag: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("par-mux-ipc-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn bind_then_connect_round_trips_bytes() {
        let path = temp_socket("round-trip");
        let listener = bind_local_listener(&path).expect("bind succeeds");

        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().expect("accept");
            let mut buf = [0u8; 5];
            stream.read_exact(&mut buf).expect("read");
            stream.write_all(b"pong\n").expect("write");
        });

        let mut client = connect_local_stream(&path).expect("connect succeeds");
        client.write_all(b"ping\n").expect("write");
        let mut reply = String::new();
        client.read_to_string(&mut reply).expect("read reply");
        assert_eq!(reply.trim(), "pong");

        server.join().expect("server thread");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prepare_refuses_a_live_socket() {
        let path = temp_socket("live");
        let _listener = bind_local_listener(&path).expect("bind succeeds");
        let err = prepare_socket_path(&path).expect_err("a live socket must not be reclaimed");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::AddrInUse,
            "refusing a live socket is what stops two daemons owning one path"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prepare_reclaims_a_stale_path() {
        let path = temp_socket("stale");
        {
            let _listener = bind_local_listener(&path).expect("bind succeeds");
            // Listener drops here; on Unix the path file may survive it.
        }
        prepare_socket_path(&path).expect("a stale path is reclaimable");
        let _listener = bind_local_listener(&path).expect("rebind after reclaim");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prepare_reclaims_a_path_that_is_not_a_socket() {
        // A leftover regular file (a crashed write, a stale marker) must be
        // reclaimable too, or one junk file bricks the default path forever.
        let path = temp_socket("junk");
        std::fs::write(&path, b"not a socket").expect("write junk file");
        prepare_socket_path(&path).expect("a non-socket file is reclaimable");
        let _listener = bind_local_listener(&path).expect("rebind after reclaim");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prepare_is_a_noop_when_nothing_exists() {
        let path = temp_socket("absent");
        prepare_socket_path(&path).expect("absent path is fine");
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
        let path = temp_socket("perms");
        let _listener = bind_local_listener(&path).expect("bind succeeds");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(
            mode & 0o077,
            0,
            "group and other must have no access: {mode:o}"
        );
        let _ = std::fs::remove_file(&path);
    }
}
