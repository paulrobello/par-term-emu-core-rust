//! Bounded staging for client file uploads — the materialize half of the
//! documented input semantic (STREAMING.md "Inline images"): a client
//! (the phone, over the mux-control channel) begins an upload, streams
//! chunks, and commits; the daemon materializes the assembled bytes as a
//! file under one staging directory and `upload-commit` bracket-pastes
//! that file's PATH into the target pane. The bytes themselves never ride
//! into pane input: the daemon never injects an escape sequence carrying
//! them (the pane app reads the file it is pointed at).
//!
//! Staging is bounded four ways — per-chunk bytes, total bytes per
//! upload, chunk count per upload, and concurrent uploads — and two
//! timeouts (idle and total) abandon an upload that stops moving. The
//! sweep runs inside every upload verb and on the server's reap tick, so
//! an abandoned upload's spool file is deleted without waiting for the
//! client to clean up. Committed files deliberately outlive the upload:
//! the pane app owns the file once the path is pasted (lifetime and
//! directory documented in MUX.md's Uploads section and SECURITY.md's
//! caps table).
//!
//! One upload at a time per client is the streaming shape (one active
//! upload per pane session); the daemon-side registry still keys by id so
//! several clients can upload to several panes concurrently.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// cap: Base64-decoded bytes accepted for one `upload-chunk`.
pub const MAX_UPLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// cap: Total bytes one upload may stage.
pub const MAX_UPLOAD_TOTAL_BYTES: u64 = 100 * 1024 * 1024;

/// cap: Chunks one upload may accept (bounds a tiny-chunk flood below the
/// byte cap).
pub const MAX_UPLOAD_CHUNKS: usize = 4 * 1024;

/// cap: Uploads staged concurrently daemon-wide.
pub const MAX_CONCURRENT_UPLOADS: usize = 8;

/// cap: Bytes of one `upload-begin` filename (a bare file name, never a
/// path); longer is refused.
pub const MAX_UPLOAD_NAME_BYTES: usize = 200;

/// Idle time before an upload is abandoned and its spool file deleted.
pub const UPLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Time from `upload-begin` before an upload is abandoned regardless of
/// progress, so a slow-drip upload cannot stage forever.
pub const UPLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(600);

/// The directory committed and in-flight upload files live under. Files
/// are 0600 in a 0700 directory (same-uid only, like the control socket).
pub fn staging_dir() -> PathBuf {
    std::env::temp_dir().join("par-mux-uploads")
}

/// One in-flight upload: a spool file chunks append to, the declared
/// total, and the two deadlines.
struct StagedUpload {
    file: std::fs::File,
    path: PathBuf,
    declared_total: u64,
    written: u64,
    chunks: usize,
    last_activity: Instant,
    began: Instant,
}

/// The daemon-wide upload table. A global like the git badge cache: the
/// control verbs are the only writers, ids are unique daemon-wide, and
/// `reset_for_tests` isolates the test binaries that share the process.
#[derive(Default)]
struct UploadRegistry {
    next_id: u64,
    uploads: HashMap<u64, StagedUpload>,
}

static REGISTRY: OnceLock<Mutex<UploadRegistry>> = OnceLock::new();

fn registry() -> &'static Mutex<UploadRegistry> {
    REGISTRY.get_or_init(|| Mutex::new(UploadRegistry::default()))
}

/// `upload-begin <filename> <total-bytes>`: open a spool file for the
/// sanitized bare name and reply with the upload id. `total` pre-checks
/// the size cap; the per-chunk and total caps enforce it for real as
/// chunks land.
pub fn begin(filename: &str, total_bytes: u64) -> Result<u64, String> {
    let name = sanitize_name(filename)?;
    if total_bytes > MAX_UPLOAD_TOTAL_BYTES {
        return Err(format!(
            "upload total {total_bytes} exceeds the {}-byte cap",
            MAX_UPLOAD_TOTAL_BYTES
        ));
    }
    let mut registry = registry().lock();
    sweep(&mut registry, Instant::now());
    if registry.uploads.len() >= MAX_CONCURRENT_UPLOADS {
        return Err(format!(
            "too many uploads in progress (cap {})",
            MAX_CONCURRENT_UPLOADS
        ));
    }
    std::fs::create_dir_all(staging_dir())
        .map_err(|err| format!("cannot create the upload staging directory: {err}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(staging_dir(), std::fs::Permissions::from_mode(0o700));
    }
    registry.next_id += 1;
    let id = registry.next_id;
    // A name collision is possible across daemon restarts (ids restart,
    // committed files persist), so a `-N` suffix disambiguates instead of
    // failing the begin.
    let stem = format!("upload-{id:06}-{name}");
    let mut path = staging_dir().join(&stem);
    let mut suffix = 0u32;
    let file = loop {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true).read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => break file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && suffix < 1000 => {
                suffix += 1;
                path = staging_dir().join(format!("{stem}-{suffix}"));
            }
            Err(err) => return Err(format!("cannot open the upload spool file: {err}")),
        }
    };
    registry.uploads.insert(
        id,
        StagedUpload {
            file,
            path,
            declared_total: total_bytes,
            written: 0,
            chunks: 0,
            last_activity: Instant::now(),
            began: Instant::now(),
        },
    );
    Ok(id)
}

/// `upload-chunk <id> <base64>`: append one chunk. Any rejection aborts
/// the upload (the id becomes invalid and the spool file is deleted), so
/// a client cannot hold a half-broken staging slot or keep pushing past a
/// cap.
pub fn chunk(id: u64, data: &[u8]) -> Result<(), String> {
    let mut registry = registry().lock();
    sweep(&mut registry, Instant::now());
    // The cap checks run in one scope that ends before any abort, so the
    // slot's borrow is never held across the delete.
    let rejection = {
        let Some(upload) = registry.uploads.get(&id) else {
            return Err(format!("no such upload: {id}"));
        };
        if data.is_empty() {
            Some(format!("chunk is empty (upload {id} aborted)"))
        } else if data.len() > MAX_UPLOAD_CHUNK_BYTES {
            Some(format!(
                "chunk of {} bytes exceeds the {}-byte per-chunk cap (upload {id} aborted)",
                data.len(),
                MAX_UPLOAD_CHUNK_BYTES
            ))
        } else if upload.chunks >= MAX_UPLOAD_CHUNKS {
            Some(format!(
                "upload {id} exceeded the {}-chunk cap (aborted)",
                MAX_UPLOAD_CHUNKS
            ))
        } else {
            let new_total = upload.written + data.len() as u64;
            if new_total > upload.declared_total {
                Some(format!(
                    "chunk pushes upload {id} past its declared {}-byte total (aborted)",
                    upload.declared_total
                ))
            } else if new_total > MAX_UPLOAD_TOTAL_BYTES {
                Some(format!(
                    "upload {id} exceeded the {}-byte total cap (aborted)",
                    MAX_UPLOAD_TOTAL_BYTES
                ))
            } else {
                None
            }
        }
    };
    if let Some(message) = rejection {
        drop_abort(&mut registry.uploads, id);
        return Err(message);
    }
    let upload = registry.uploads.get_mut(&id).expect("just checked");
    if upload.file.write_all(data).is_err() {
        drop_abort(&mut registry.uploads, id);
        return Err(format!("upload {id} spool write failed (aborted)"));
    }
    upload.written += data.len() as u64;
    upload.chunks += 1;
    upload.last_activity = Instant::now();
    Ok(())
}

/// `upload-commit <id>`: flush and seal the spool file, consume the
/// staging slot, and return the materialized path for the caller to paste.
/// The file is left in place — the pane app owns it once it has the path.
/// A commit whose bytes never reached the declared total is refused and
/// cleaned up.
pub fn commit(id: u64) -> Result<PathBuf, String> {
    let mut registry = registry().lock();
    sweep(&mut registry, Instant::now());
    let Some(mut upload) = registry.uploads.remove(&id) else {
        return Err(format!("no such upload: {id}"));
    };
    if upload.written != upload.declared_total {
        let _ = std::fs::remove_file(&upload.path);
        return Err(format!(
            "upload {id} is incomplete: {} of {} bytes",
            upload.written, upload.declared_total
        ));
    }
    if upload.file.flush().is_err() || upload.file.sync_all().is_err() {
        let _ = std::fs::remove_file(&upload.path);
        return Err(format!("upload {id} could not be flushed to disk"));
    }
    drop(upload.file);
    Ok(upload.path)
}

/// `upload-abort <id>`: drop the staging slot and delete the spool file.
pub fn abort(id: u64) -> Result<(), String> {
    let mut registry = registry().lock();
    sweep(&mut registry, Instant::now());
    let Some(upload) = registry.uploads.remove(&id) else {
        return Err(format!("no such upload: {id}"));
    };
    drop(upload.file);
    if let Err(err) = std::fs::remove_file(&upload.path) {
        if err.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("upload {id} could not be cleaned up: {err}"));
        }
    }
    Ok(())
}

/// Delete the spool file of `id` and remove its slot.
fn drop_abort(uploads: &mut HashMap<u64, StagedUpload>, id: u64) {
    if let Some(upload) = uploads.remove(&id) {
        drop(upload.file);
        let _ = std::fs::remove_file(&upload.path);
    }
}

/// Delete every upload past its idle or total deadline. Runs inside every
/// verb and on the server's reap tick, so an abandoned upload's spool file
/// is deleted even when its client never returns.
fn sweep(registry: &mut UploadRegistry, now: Instant) {
    let expired: Vec<u64> = registry
        .uploads
        .iter()
        .filter(|(_, upload)| {
            now.duration_since(upload.last_activity) > UPLOAD_IDLE_TIMEOUT
                || now.duration_since(upload.began) > UPLOAD_TOTAL_TIMEOUT
        })
        .map(|(id, _)| *id)
        .collect();
    for id in expired {
        drop_abort(&mut registry.uploads, id);
    }
}

/// The reap tick's sweep entry point.
pub(crate) fn sweep_expired() {
    let mut registry = registry().lock();
    sweep(&mut registry, Instant::now());
}

/// A `upload-begin` filename must be a bare file name: path components
/// are stripped (a client cannot point the paste outside the staging
/// directory), control characters and Windows-reserved punctuation are
/// refused, and the name is length-capped.
fn sanitize_name(raw: &str) -> Result<String, String> {
    let name = Path::new(raw)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "upload filename must be a UTF-8 bare file name".to_string())?;
    if name.is_empty() {
        return Err("upload filename must not be empty".to_string());
    }
    if name.len() > MAX_UPLOAD_NAME_BYTES {
        return Err(format!(
            "upload filename exceeds the {}-byte cap",
            MAX_UPLOAD_NAME_BYTES
        ));
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\'))
    {
        return Err(
            "upload filename must not contain control characters or <>:\"|?*\\".to_string(),
        );
    }
    Ok(name.to_string())
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    let mut registry = registry().lock();
    let ids: Vec<u64> = registry.uploads.keys().copied().collect();
    for id in ids {
        drop_abort(&mut registry.uploads, id);
    }
    registry.next_id = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn init() {
        reset_for_tests();
        // Committed files persist by design, so reruns (and earlier failed
        // runs) can hold the exact names this test claims; free them so
        // the primary (unsuffixed) paths are deterministic.
        for stale in [
            "upload-000001-photo.png",
            "upload-000002-photo.png",
            "upload-000001-passwd",
        ] {
            let _ = std::fs::remove_file(staging_dir().join(stale));
        }
    }

    fn read_path(path: &Path) -> Vec<u8> {
        let mut buf = Vec::new();
        std::fs::File::open(path)
            .expect("open")
            .read_to_end(&mut buf)
            .expect("read");
        buf
    }

    // One test fn on purpose: the registry is a process-global, so
    // scenarios sharing it must run sequentially (parallel test fns would
    // reset each other's slots mid-scenario).
    #[test]
    fn the_upload_staging_lifecycle_and_bounds_hold() {
        // Round trip: begin -> chunk -> commit materializes the bytes and
        // leaves the file; the id is consumed; a repeated name gets a
        // fresh id.
        init();
        let id = begin("photo.png", 5).expect("begin");
        chunk(id, b"hello").expect("chunk");
        let path = commit(id).expect("commit");
        assert!(
            path.starts_with(staging_dir()),
            "the materialized path lives in the staging directory: {path:?}"
        );
        assert!(path.to_string_lossy().ends_with("upload-000001-photo.png"));
        assert_eq!(read_path(&path), b"hello", "the bytes landed verbatim");
        assert!(chunk(id, b"more").is_err(), "commit consumes the id");
        assert!(std::fs::read(&path).is_ok(), "the committed file persists");
        let second = begin("photo.png", 0).expect("second begin");
        assert_ne!(id, second);
        let second_path = commit(second).expect("second commit");
        assert!(second_path
            .to_string_lossy()
            .ends_with("upload-000002-photo.png"));
        // This test's own committed files are removed, so a rerun (and a
        // parallel suite sharing the staging directory) starts clean.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&second_path);

        // Per-chunk cap: an oversize chunk aborts the upload.
        init();
        let id = begin("big.bin", MAX_UPLOAD_TOTAL_BYTES).expect("begin");
        let err = chunk(id, &vec![0u8; MAX_UPLOAD_CHUNK_BYTES + 1]).unwrap_err();
        assert!(err.contains("per-chunk cap"), "unexpected: {err}");
        assert!(chunk(id, b"x").is_err(), "the aborted id is dead");
        assert!(commit(id).is_err(), "the aborted id cannot commit");

        // Declared-total cap: a chunk pushing past the total aborts.
        init();
        let id = begin("ten.bin", 10).expect("begin");
        chunk(id, &[0u8; 8]).expect("first chunk");
        let err = chunk(id, &[0u8; 3]).unwrap_err();
        assert!(err.contains("declared"), "unexpected: {err}");
        assert!(commit(id).is_err(), "the aborted id cannot commit");

        // Chunk-count cap: a tiny-chunk flood is bounded.
        init();
        let id = begin("drip.bin", MAX_UPLOAD_CHUNKS as u64).expect("begin");
        for _ in 0..MAX_UPLOAD_CHUNKS {
            chunk(id, b"x").expect("chunk within cap");
        }
        let err = chunk(id, b"x").unwrap_err();
        assert!(err.contains("chunk cap"), "unexpected: {err}");

        // Begin caps: an over-cap total is refused up front; the
        // concurrent-upload cap refuses the ninth.
        init();
        let err = begin("big.bin", MAX_UPLOAD_TOTAL_BYTES + 1).unwrap_err();
        assert!(err.contains("exceeds"), "unexpected: {err}");
        for i in 0..MAX_CONCURRENT_UPLOADS {
            begin(&format!("f{i}.bin"), 1).expect("begin within cap");
        }
        let err = begin("one-too-many.bin", 1).unwrap_err();
        assert!(err.contains("too many uploads"), "unexpected: {err}");

        // An incomplete commit is refused and cleaned up.
        init();
        let id = begin("partial.bin", 4).expect("begin");
        chunk(id, b"ab").expect("chunk");
        let err = commit(id).unwrap_err();
        assert!(err.contains("incomplete"), "unexpected: {err}");
        let path = staging_dir().join(format!("upload-{id:06}-partial.bin"));
        assert!(!path.exists(), "the refused upload's spool file is gone");

        // Abort deletes the spool file and invalidates the id.
        init();
        let id = begin("gone.bin", 2).expect("begin");
        chunk(id, b"x").expect("chunk");
        abort(id).expect("abort");
        let path = staging_dir().join(format!("upload-{id:06}-gone.bin"));
        assert!(!path.exists(), "abort deletes the spool file");
        assert!(chunk(id, b"x").is_err(), "abort invalidates the id");
        assert!(abort(id).is_err(), "double abort reports no such upload");

        // The sweep deletes idle and overdue uploads.
        init();
        let id = begin("stale.bin", 1).expect("begin");
        {
            let mut registry = registry().lock();
            let upload = registry.uploads.get_mut(&id).expect("staged");
            upload.last_activity = Instant::now() - UPLOAD_IDLE_TIMEOUT - Duration::from_secs(1);
            upload.began = Instant::now() - UPLOAD_TOTAL_TIMEOUT - Duration::from_secs(1);
        }
        sweep_expired();
        assert!(chunk(id, b"x").is_err(), "the swept upload's id is dead");
        let path = staging_dir().join(format!("upload-{id:06}-stale.bin"));
        assert!(!path.exists(), "the sweep deleted the spool file");

        // Filename sanitization: path components are stripped (never a
        // traversal), hostile names are refused, long names are refused.
        init();
        let id = begin("/etc/passwd", 0).expect("begin");
        let path = commit(id).expect("commit");
        assert_eq!(path.parent(), Some(staging_dir().as_path()));
        assert!(path.to_string_lossy().ends_with("upload-000001-passwd"));
        for bad in ["..", ".", "", "a<b", "a|b", "a\nb"] {
            assert!(sanitize_name(bad).is_err(), "{bad:?} must be refused");
        }
        let long = "x".repeat(MAX_UPLOAD_NAME_BYTES + 1);
        assert!(sanitize_name(&long).is_err(), "over-cap names are refused");
        let _ = std::fs::remove_file(&path);

        // Empty chunks and unknown ids are refused.
        init();
        assert!(begin("", 0).is_err());
        let err = chunk(999, b"x").unwrap_err();
        assert!(err.contains("no such upload"), "unexpected: {err}");
        let id = begin("e.bin", 1).expect("begin");
        let err = chunk(id, b"").unwrap_err();
        assert!(err.contains("empty"), "unexpected: {err}");
    }
}
