//! Daemon discovery: a registry of running par-mux servers plus a probe
//! that separates the live ones from the stale entries.
//!
//! Each daemon writes one small pointer file under its state directory at
//! bind time — `<state-base>/servers/<socket-stem>.json` carrying the full
//! socket path, pid, start timestamp, and build stamp — and removes it on
//! the clean shutdown path (`--stop`/`kill-server`). A crash or SIGKILL
//! leaves the file behind; [`enumerate`] treats every entry as unproven,
//! probes each candidate socket with `version`, and prunes the entries
//! whose probe fails. Sockets without registry entries are picked up by a
//! scan of the named-default socket directory, so plain named daemons
//! (`par-mux work`) appear in the same list as explicitly-placed ones.
//!
//! The stem-keyed file name means two daemons on sockets that share a stem
//! in different directories share one file; the entry carries the full
//! socket path, so the collision resolves to "whoever registered last",
//! and the loser is simply not in the registry (the default-directory
//! scan still finds it when its path is a default-named one).

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::client::MuxClient;
use super::ipc::default_socket_path;

/// One live (probe-answered) par-mux server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// The socket path the server was probed on.
    pub socket: PathBuf,
    /// The daemon's build stamp (`version` reply), when it answered.
    pub stamp: Option<String>,
    /// Live session count (`list-sessions` reply lines), when it answered.
    pub sessions: Option<usize>,
}

/// What [`enumerate`] found under one state base: the servers that answered
/// the probe, and the registry entries whose probe failed (pruned from the
/// registry on the way out).
#[derive(Debug, Clone, Default)]
pub struct Enumeration {
    /// Sockets whose probe answered, sorted by path.
    pub live: Vec<ServerInfo>,
    /// Registry entries (socket paths) whose probe failed — already
    /// pruned from the registry on the way out.
    pub dead: Vec<PathBuf>,
}

/// The registry pointer file for one server.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
struct RegistryEntry {
    /// The probed socket path, full — the file name is only the stem.
    socket: String,
    /// The daemon process at registration time. Liveness is re-proven over
    /// the socket; this only short-circuits re-registration.
    pid: u32,
    started_unix_ms: u64,
    stamp: String,
}

/// The state base the server's own registration lands under: the platform
/// state dir (the same base `persist::state_file_path` resolves to) unless
/// `--state-dir` moved the daemon's persistence. Kept in step with
/// `persist::platform_state_dir` (that module is the owner of the concept;
/// this duplicates the three-line resolution because the base is needed by
/// client-mode code paths that never touch persistence).
pub fn default_registry_base() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
}

/// The registry directory for state base `base`:
/// `<base>/servers/`.
fn registry_dir(base: &Path) -> PathBuf {
    base.join("servers")
}

/// The registry file for the server on `socket`, under state base `base`.
/// Stem-keyed exactly like the state file (`persist::state_file_in`), so a
/// server's registry entry and its state file always pair up.
fn registry_file(base: &Path, socket: &Path) -> PathBuf {
    let stem = socket
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("default");
    registry_dir(base).join(format!("{stem}.json"))
}

/// A pid the OS says is gone. `kill` on it returns ESRCH on every unix and
/// `OpenProcess` fails on Windows; the pid space below the int32 max keeps
/// it clear of the `pid <= 0` kill(2) special meanings.
#[cfg(test)]
const DEAD_PID: u32 = 2_147_483_647;

/// Whether the OS still has a process with this pid.
///
/// A yes is a hint, not proof — pids recycle. The socket probe is the
/// authority; this only decides whether re-registering may skip a rewrite.
#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // SAFETY: kill(2) with signal 0 probes liveness without signaling; no
    // process state is touched.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether the OS still has a process with this pid.
///
/// [`PROCESS_QUERY_LIMITED_INFORMATION`](windows_sys::Win32::System::Threading::OpenProcess)
/// succeeds for any existing process the caller may query and fails with a
/// cleared handle otherwise — the same fail-closed shape as the mux
/// client's server-identity check (SEC-112).
#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: the returned handle is null-checked and closed exactly once
    // on the only path that reaches past the check.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        CloseHandle(handle);
        true
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Register this process's daemon (listening on `socket`) under state base
/// `base`.
///
/// Idempotent: an existing entry naming the same socket with a live pid is
/// left byte-for-byte alone (a daemon re-binding after `--restart` finds
/// its predecessor's live file — same socket, and the old process is gone
/// only if `--stop` cleaned it up; a live pid there means a stem collision
/// with another daemon, whose entry must not be stolen). A dead or
/// mismatched entry is replaced.
///
/// Failure to register never blocks serving — callers log it and bind
/// anyway; discovery then falls back to the default-directory scan.
pub fn register(base: &Path, socket: &Path) -> io::Result<()> {
    let file = registry_file(base, socket);
    if let Ok(bytes) = std::fs::read(&file) {
        if let Ok(entry) = serde_json::from_slice::<RegistryEntry>(&bytes) {
            if entry.socket == socket.to_string_lossy() && pid_alive(entry.pid) {
                return Ok(());
            }
        }
    }
    let entry = RegistryEntry {
        socket: socket.to_string_lossy().into_owned(),
        pid: std::process::id(),
        started_unix_ms: now_unix_ms(),
        stamp: crate::mux::build_stamp().to_owned(),
    };
    let dir = registry_dir(base);
    std::fs::create_dir_all(&dir)?;
    // Tmp-then-rename, the state file's atomicity shape: a reader mid-list
    // sees either the previous entry or the complete new one, never a torn
    // write.
    let tmp = dir.join(format!(
        "{}.tmp",
        file.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    std::fs::write(&tmp, serde_json::to_vec(&entry)?)?;
    std::fs::rename(&tmp, &file)
}

/// Remove this daemon's registry entry under state base `base`, on the
/// clean shutdown path.
///
/// Only an entry naming this exact socket is removed: a stem collision
/// means the file may belong to another daemon, and an unparsable file is
/// junk that is removed regardless.
pub fn unregister(base: &Path, socket: &Path) -> io::Result<()> {
    let file = registry_file(base, socket);
    let remove = match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice::<RegistryEntry>(&bytes)
            .map(|entry| entry.socket == socket.to_string_lossy())
            .unwrap_or(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if remove {
        std::fs::remove_file(&file)?;
    }
    Ok(())
}

/// The directory named-default sockets live in (the parent of
/// `default_socket_path`): `$XDG_RUNTIME_DIR` or the per-UID temp dir on
/// unix, the per-user temp dir on Windows.
pub fn default_socket_dir() -> PathBuf {
    default_socket_path("default")
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
}

/// Whether a directory entry name is a named-default control socket
/// (`par-mux-<name>.sock`), excluding pane endpoints
/// (`<stem>.pane-<N>.sock`, same directory) and anything unpar-mux.
fn is_named_default_socket_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".sock") else {
        return false;
    };
    let Some(name_part) = stem.strip_prefix("par-mux-") else {
        return false;
    };
    !name_part.is_empty() && !name_part.contains(".pane-")
}

/// The named-default sockets present in the default socket directory right
/// now, regardless of registry state.
fn scan_default_dir() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(default_socket_dir()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| is_named_default_socket_name(&entry.file_name().to_string_lossy()))
        .map(|entry| entry.path())
        .collect()
}

/// The human-facing label for a socket: its name when the socket is a
/// current named-default one, the full path otherwise.
pub fn display_label(socket: &Path) -> String {
    default_name_of(socket).unwrap_or_else(|| socket.to_string_lossy().into_owned())
}

/// The name behind a named-default socket path, or `None` when the path is
/// not one (explicit `--socket`, a pane endpoint, or a foreign layout).
fn default_name_of(socket: &Path) -> Option<String> {
    let name = socket.file_name()?.to_str()?;
    let name_part = name.strip_suffix(".sock")?.strip_prefix("par-mux-")?;
    if name_part.is_empty() || name_part.contains(".pane-") {
        return None;
    }
    let parent = socket.parent()?;
    (parent == default_socket_dir()).then(|| name_part.to_owned())
}

/// Probe one socket: connect, ask `version` and `list-sessions`.
///
/// `None` means nothing live owns the path. A connect that succeeds but
/// never answers is bounded by the client's reply timeout, so a non-par-mux
/// server squatting on a default-named path delays the list instead of
/// hanging it.
fn probe(socket: &Path) -> Option<ServerInfo> {
    let mut client = MuxClient::connect(socket).ok()?;
    let stamp = client
        .send_checked("version")
        .ok()
        .filter(|reply| reply.ok)
        .map(|reply| reply.body.join("\n"));
    let sessions = client
        .send_checked("list-sessions")
        .ok()
        .filter(|reply| reply.ok)
        .map(|reply| {
            reply
                .body
                .iter()
                .filter(|line| !line.trim().is_empty())
                .count()
        });
    Some(ServerInfo {
        socket: socket.to_path_buf(),
        stamp,
        sessions,
    })
}

/// Enumerate the par-mux servers known under state base `base`: every
/// registry entry, plus — only when `base` IS the platform default — every
/// named-default socket the directory scan turns up. Deduplicated, probed,
/// and sorted by socket path.
///
/// The scan is tied to the platform base, not offered beside it: an
/// explicit base (`--state-dir`, what tests and sandboxes pass) asks
/// "list THIS registry", and sweeping the shared default directory there
/// would drag in whatever unrelated servers the user happens to run. The
/// scan's production job is to catch named-default daemons that never
/// wrote a registry entry (an older binary, a failed write) — those always
/// live under the platform base.
///
/// Registry entries whose probe fails are dead: they are returned in
/// `dead` and their files pruned here, so one list-servers run cleans up
/// after a crashed daemon. A dead entry for a socket a NEW daemon has
/// since bound is impossible (the new daemon's probe answers), so pruning
/// never eats a live server's entry.
pub fn enumerate(base: &Path) -> Enumeration {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(registry_dir(base)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(parsed) = serde_json::from_slice::<RegistryEntry>(&bytes) {
                        let socket = PathBuf::from(&parsed.socket);
                        if !candidates.contains(&socket) {
                            candidates.push(socket);
                        }
                    }
                }
            }
        }
    }
    if base == default_registry_base() {
        for socket in scan_default_dir() {
            if !candidates.contains(&socket) {
                candidates.push(socket);
            }
        }
    }
    candidates.sort();

    let mut found = Enumeration::default();
    for socket in candidates {
        match probe(&socket) {
            Some(info) => found.live.push(info),
            None => {
                // Prune only through `unregister`, which refuses to remove
                // a file that has come to name a different socket since it
                // was read.
                let _ = unregister(base, &socket);
                found.dead.push(socket);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base(_tag: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("par-mux-disc-")
            .tempdir()
            .expect("create temp base")
    }

    #[test]
    fn pid_alive_answers_for_own_pid_and_refuses_a_dead_one() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(DEAD_PID));
    }

    #[test]
    fn register_creates_a_stem_keyed_entry() {
        let base = temp_base("create");
        let socket = base.path().join("par-mux-alpha.sock");
        register(base.path(), &socket).expect("register");
        let file = registry_dir(base.path()).join("par-mux-alpha.json");
        let entry: RegistryEntry =
            serde_json::from_slice(&std::fs::read(&file).expect("entry file")).expect("parses");
        assert_eq!(entry.socket, socket.to_string_lossy());
        assert_eq!(entry.pid, std::process::id());
        assert_eq!(entry.stamp, crate::mux::build_stamp());
    }

    #[test]
    fn register_over_a_live_entry_is_a_noop() {
        let base = temp_base("noop");
        let socket = base.path().join("par-mux-live.sock");
        register(base.path(), &socket).expect("register");
        let file = registry_dir(base.path()).join("par-mux-live.json");
        let before = std::fs::read(&file).expect("entry file");
        register(base.path(), &socket).expect("re-register");
        let after = std::fs::read(&file).expect("entry file");
        assert_eq!(before, after, "a live entry must not be rewritten");
    }

    #[test]
    fn register_replaces_a_dead_entry_for_the_same_socket() {
        let base = temp_base("replace");
        let socket = base.path().join("par-mux-dead.sock");
        let file = registry_dir(base.path()).join("par-mux-dead.json");
        std::fs::create_dir_all(registry_dir(base.path())).expect("mkdir");
        let stale = RegistryEntry {
            socket: socket.to_string_lossy().into_owned(),
            pid: DEAD_PID,
            started_unix_ms: 0,
            stamp: "0.0.0+dead".to_owned(),
        };
        std::fs::write(&file, serde_json::to_vec(&stale).expect("serialize")).expect("stage");
        register(base.path(), &socket).expect("register over the stale entry");
        let entry: RegistryEntry =
            serde_json::from_slice(&std::fs::read(&file).expect("entry file")).expect("parses");
        assert_eq!(entry.pid, std::process::id(), "the dead entry was replaced");
    }

    #[test]
    fn register_replaces_an_entry_naming_a_different_socket() {
        // Stem collision: another daemon's file at our stem names another
        // socket. Registering must replace it — the entry is stem-keyed and
        // we are the daemon that owns this stem now.
        let base = temp_base("collision");
        let socket = base.path().join("par-mux-mine.sock");
        let file = registry_dir(base.path()).join("par-mux-mine.json");
        std::fs::create_dir_all(registry_dir(base.path())).expect("mkdir");
        let foreign = RegistryEntry {
            socket: "/elsewhere/par-mux-mine.sock".to_owned(),
            pid: std::process::id(),
            started_unix_ms: 0,
            stamp: "0.0.0+other".to_owned(),
        };
        std::fs::write(&file, serde_json::to_vec(&foreign).expect("serialize")).expect("stage");
        register(base.path(), &socket).expect("register");
        let entry: RegistryEntry =
            serde_json::from_slice(&std::fs::read(&file).expect("entry file")).expect("parses");
        assert_eq!(entry.socket, socket.to_string_lossy());
    }

    #[test]
    fn unregister_removes_only_the_matching_socket() {
        let base = temp_base("unregister");
        let socket = base.path().join("par-mux-target.sock");
        register(base.path(), &socket).expect("register");
        let file = registry_dir(base.path()).join("par-mux-target.json");

        // A different socket sharing the stem must not remove the entry.
        let other = base.path().join("elsewhere").join("par-mux-target.sock");
        unregister(base.path(), &other).expect("unregister foreign socket");
        assert!(file.exists(), "the foreign unregister left the entry");

        unregister(base.path(), &socket).expect("unregister own socket");
        assert!(!file.exists(), "the owning unregister removed the entry");

        // Unregistering again is quiet.
        unregister(base.path(), &socket).expect("unregister absent is fine");
    }

    #[test]
    fn named_default_socket_names_exclude_pane_endpoints_and_junk() {
        assert!(is_named_default_socket_name("par-mux-work.sock"));
        assert!(is_named_default_socket_name("par-mux-default.sock"));
        assert!(!is_named_default_socket_name("par-mux-work.pane-3.sock"));
        assert!(!is_named_default_socket_name("unrelated.sock"));
        assert!(!is_named_default_socket_name("par-mux-.sock"));
        assert!(!is_named_default_socket_name("par-mux-state.json"));
        assert!(!is_named_default_socket_name("random.tmp"));
    }

    #[test]
    fn display_label_names_default_sockets_and_paths_the_rest() {
        let dir = default_socket_dir();
        assert_eq!(
            display_label(&dir.join("par-mux-work.sock")),
            "work",
            "a current named-default socket is labeled by name"
        );
        assert_eq!(
            display_label(&dir.join("par-mux-work.pane-3.sock")),
            dir.join("par-mux-work.pane-3.sock").to_string_lossy(),
            "a pane endpoint is never labeled by name"
        );
        assert_eq!(
            display_label(Path::new("/tmp/par-mux-explicit.sock")),
            "/tmp/par-mux-explicit.sock",
            "an explicit socket is labeled by its full path"
        );
    }

    /// A registry full of hostile files — garbage bytes, truncated JSON,
    /// an array where the object shape is expected, wrong field types, an
    /// empty file, a 1MB junk file — is skipped wholesale: `enumerate`
    /// yields no candidates, no panic, and no hostile string reaches its
    /// output (`live`/`dead` stay empty).
    #[test]
    fn enumerate_skips_corrupt_registry_files_without_panicking() {
        let base = temp_base("corrupt");
        let dir = registry_dir(base.path());
        std::fs::create_dir_all(&dir).expect("registry dir");
        std::fs::write(dir.join("garbage.json"), [0xFF, 0x00, 0xFE, b'{', b'}'])
            .expect("garbage file");
        std::fs::write(
            dir.join("truncated.json"),
            br#"{"socket":"/tmp/x.sock","pid":12"#,
        )
        .expect("truncated file");
        // An array where the object shape is expected.
        std::fs::write(dir.join("array.json"), b"[1,2,3]").expect("array file");
        // Valid JSON, wrong field types.
        std::fs::write(
            dir.join("wrong_types.json"),
            br#"{"socket":1,"pid":"x","started_unix_ms":null,"stamp":[]}"#,
        )
        .expect("wrong-type file");
        std::fs::write(dir.join("empty.json"), b"").expect("empty file");
        std::fs::write(dir.join("junk.json"), vec![b'x'; 1 << 20]).expect("junk file");
        // Not a registry entry at all.
        std::fs::write(dir.join("notes.txt"), b"not a registry file").expect("txt file");

        let found = enumerate(base.path());
        assert!(found.live.is_empty(), "{:?}", found.live);
        assert!(found.dead.is_empty(), "{:?}", found.dead);
    }

    /// An entry whose socket string carries control bytes and a newline
    /// parses fine, probes dead (nothing listens there), and is pruned —
    /// the path lands in `dead` exactly once, well-formed as a PathBuf,
    /// and the registry file is gone afterwards.
    #[test]
    fn enumerate_prunes_a_hostile_socket_entry() {
        let base = temp_base("hostile");
        let dir = registry_dir(base.path());
        std::fs::create_dir_all(&dir).expect("registry dir");
        let hostile = "/no\x1b[31msuch\ndir/par-mux-x.sock\t";
        let entry = serde_json::json!({
            "socket": hostile,
            "pid": 123,
            "started_unix_ms": 0,
            "stamp": "0.0.0+test"
        });
        std::fs::write(
            dir.join("par-mux-x.json"),
            serde_json::to_vec(&entry).expect("json"),
        )
        .expect("hostile entry");

        let found = enumerate(base.path());
        assert!(found.live.is_empty(), "{:?}", found.live);
        assert_eq!(found.dead, vec![std::path::PathBuf::from(hostile)]);
        assert!(
            !dir.join("par-mux-x.json").exists(),
            "the dead entry's file was pruned"
        );
    }
}
